use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::*;
use klyndb_import::{CsvOptions, Mapping, Snapshot, ValueKind};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn sql(driver: &dyn Session, sql: String) -> Result<Vec<Row>> {
    let (output, mut input) = mpsc::channel(4);
    let drain = async move {
        let mut rows = vec![];
        while let Some(batch) = input.recv().await {
            if let Batch::Rows(batch) = batch {
                rows.extend(batch);
            }
        }
        rows
    };
    let (executed, rows) = tokio::join!(
        driver.execute(sql, output, CancellationToken::new(), 10_000),
        drain
    );
    executed?;
    Ok(rows)
}
fn insert(id: i64) -> Change {
    Change::Insert {
        values: BTreeMap::from([
            ("id".into(), Cell::Number(id.to_string())),
            (
                "label".into(),
                Cell::Text(format!("Imported row {id}, \"quoted\"\nnext")),
            ),
        ]),
    }
}
async fn stream(
    driver: Arc<dyn Session>,
    table: Table,
    batches: Vec<Result<InsertBatch>>,
) -> Result<MutationResult> {
    let (output, input) = mpsc::channel(1);
    let producer = tokio::spawn(async move {
        for batch in batches {
            if output.send(batch).await.is_err() {
                break;
            }
        }
    });
    let result = driver
        .insert_stream(table, input, CancellationToken::new())
        .await;
    producer.await.unwrap();
    result
}
async fn count(driver: &dyn Session, table: &str) -> u64 {
    sql(driver, format!("SELECT count(*) FROM {table}"))
        .await
        .unwrap()[0][0]
        .text()
        .parse()
        .unwrap()
}

async fn csv_stream(
    driver: Arc<dyn Session>,
    table: Table,
    snapshot: Snapshot,
) -> Result<MutationResult> {
    let (output, input) = mpsc::channel(2);
    let token = CancellationToken::new();
    let producer_token = token.clone();
    let producer = tokio::task::spawn_blocking(move || {
        snapshot.produce(
            &CsvOptions::default(),
            &[
                Mapping {
                    column: Some("id".into()),
                    kind: ValueKind::Number,
                },
                Mapping {
                    column: Some("label".into()),
                    kind: ValueKind::Text,
                },
            ],
            output,
            producer_token,
            &std::sync::atomic::AtomicU64::new(0),
        )
    });
    let written = driver.insert_stream(table, input, token.clone()).await;
    if written.is_err() {
        token.cancel();
    }
    let parsed = producer.await.unwrap();
    if let Ok(written) = &written {
        assert_eq!(written.affected, parsed.unwrap());
    }
    written
}

#[tokio::test]
async fn atomic_stream_import_contract() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut configs = vec![(
        "sqlite",
        directory
            .path()
            .join("source.db")
            .to_string_lossy()
            .into_owned(),
    )];
    if let Ok(url) = std::env::var("KLYNDB_TEST_POSTGRES_URL") {
        configs.push(("postgres", url));
    }
    if let Ok(url) = std::env::var("KLYNDB_TEST_MYSQL_URL") {
        configs.push(("mysql", url));
    }
    for (name, address) in configs {
        let mut config = Connection {
            id: String::new(),
            name: "Import contract".into(),
            engine: name.into(),
            address,
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: name == "sqlite",
        };
        config.validate().unwrap();
        engine.store.save(&config).unwrap();
        engine.connect(&config.id, None).await.unwrap();
        let driver = engine.driver(&config.id).await.unwrap();
        assert!(driver.capabilities().import_rows);
        let table = Table {
            schema: match name {
                "sqlite" => "main",
                "postgres" => "public",
                _ => "klyndb_test",
            }
            .into(),
            name: format!("import_{}", uuid::Uuid::new_v4().simple()),
            kind: "table".into(),
        };
        let qualified = format!(
            "{}.{}",
            driver.quote_identifier(&table.schema),
            driver.quote_identifier(&table.name)
        );
        sql(driver.as_ref(), format!("CREATE TABLE {qualified}(id BIGINT PRIMARY KEY, label TEXT NOT NULL, doubled BIGINT GENERATED ALWAYS AS (id * 2) STORED){}", if name=="mysql" { " ENGINE=InnoDB" } else { "" })).await.unwrap();
        let source = directory.path().join(format!("{name}.csv"));
        let mut file = std::fs::File::create(&source).unwrap();
        klyndb_export::export(
            &mut file,
            &["id".into(), "label".into()],
            (1..=1201).map(|id| {
                Ok(vec![
                    Cell::Number(id.to_string()),
                    Cell::Text(format!("Imported row {id}, \"quoted\"\nnext")),
                ])
            }),
            "csv",
            "",
        )
        .unwrap();
        let imported = csv_stream(
            driver.clone(),
            table.clone(),
            Snapshot::copy(&source).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(imported.affected, 1201);
        assert!(!imported.pending_transaction);
        assert_eq!(count(driver.as_ref(), &qualified).await, 1201, "{name}");
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT label FROM {qualified} WHERE id=1201")
            )
            .await
            .unwrap()[0][0]
                .text(),
            "Imported row 1201, \"quoted\"\nnext"
        );
        let mut file = std::fs::File::create(&source).unwrap();
        klyndb_export::export(
            &mut file,
            &["id".into(), "label".into()],
            (2001..=3201).map(|id| {
                Ok(vec![
                    Cell::Number(id.to_string()),
                    Cell::Text("New row".into()),
                ])
            }),
            "csv",
            "",
        )
        .unwrap();
        std::io::Write::write_all(&mut file, b"bad,invalid number\n").unwrap();
        let failure = csv_stream(
            driver.clone(),
            table.clone(),
            Snapshot::copy(&source).unwrap(),
        )
        .await
        .unwrap_err();
        assert!(
            failure.message.contains("CSV record 1203"),
            "{}",
            failure.message
        );
        assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
        for terminal in [
            Some(Ok(InsertBatch::Rows(vec![insert(1)]))),
            Some(Err(Error::new("CSV record failed validation"))),
            None,
        ] {
            let mut batches = vec![Ok(InsertBatch::Rows(vec![insert(2000)]))];
            if let Some(terminal) = terminal {
                batches.push(terminal);
            }
            assert!(
                stream(driver.clone(), table.clone(), batches)
                    .await
                    .is_err(),
                "{name}"
            );
            assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
            assert_eq!(
                driver.transaction_state().await.unwrap(),
                TransactionState::Idle
            );
        }
        let invalid = Change::Insert {
            values: BTreeMap::from([("doubled".into(), Cell::Number("2".into()))]),
        };
        assert!(
            stream(
                driver.clone(),
                table.clone(),
                vec![
                    Ok(InsertBatch::Rows(vec![invalid])),
                    Ok(InsertBatch::Complete)
                ]
            )
            .await
            .is_err()
        );
        assert!(
            stream(
                driver.clone(),
                table.clone(),
                vec![
                    Ok(InsertBatch::Rows(vec![Change::Delete { old: vec![] }])),
                    Ok(InsertBatch::Complete)
                ]
            )
            .await
            .is_err()
        );
        sql(driver.as_ref(), "BEGIN".into()).await.unwrap();
        driver
            .apply_changes(table.clone(), vec![insert(9000)])
            .await
            .unwrap();
        let pending = stream(
            driver.clone(),
            table.clone(),
            vec![
                Ok(InsertBatch::Rows(vec![insert(9001)])),
                Ok(InsertBatch::Complete),
            ],
        )
        .await
        .unwrap();
        assert!(pending.pending_transaction);
        assert!(
            stream(
                driver.clone(),
                table.clone(),
                vec![
                    Ok(InsertBatch::Rows(vec![insert(9002)])),
                    Ok(InsertBatch::Rows(vec![insert(1)])),
                    Ok(InsertBatch::Complete)
                ]
            )
            .await
            .is_err()
        );
        assert_eq!(count(driver.as_ref(), &qualified).await, 1203);
        assert_eq!(
            driver.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        sql(driver.as_ref(), "ROLLBACK".into()).await.unwrap();
        assert_eq!(count(driver.as_ref(), &qualified).await, 1201);

        // A second tab must not COMMIT an unfinished import. Cancel while the reader
        // is still alive and idle; dropped-producer cancellation is not sufficient.
        let (output, input) = mpsc::channel(1);
        output
            .send(Ok(InsertBatch::Rows(vec![insert(3100)])))
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let task_driver = driver.clone();
        let task_table = table.clone();
        let task_cancel = cancel.clone();
        let import = tokio::spawn(async move {
            task_driver
                .insert_stream(task_table, input, task_cancel)
                .await
        });
        while output.capacity() == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let other = driver.clone();
        let mut concurrent =
            tokio::spawn(async move { sql(other.as_ref(), "COMMIT".into()).await });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut concurrent)
                .await
                .is_err()
        );
        cancel.cancel();
        let cancelled = tokio::time::timeout(Duration::from_secs(4), import)
            .await
            .unwrap()
            .unwrap();
        assert!(cancelled.is_err(), "{name}");
        let queued_commit = concurrent.await.unwrap();
        if name == "sqlite" {
            assert!(
                queued_commit
                    .unwrap_err()
                    .message
                    .contains("no transaction")
            );
        } else {
            queued_commit.unwrap();
        }
        drop(output);
        assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
        assert_eq!(
            driver.transaction_state().await.unwrap(),
            TransactionState::Idle
        );

        if name != "sqlite" {
            let mut blocker = config.clone();
            blocker.id.clear();
            blocker.validate().unwrap();
            engine.store.save(&blocker).unwrap();
            engine.connect(&blocker.id, None).await.unwrap();
            let locking = engine.driver(&blocker.id).await.unwrap();
            sql(
                locking.as_ref(),
                format!("BEGIN; UPDATE {qualified} SET label='Locked temporarily' WHERE id=1"),
            )
            .await
            .unwrap();
            let (output, input) = mpsc::channel(1);
            output
                .send(Ok(InsertBatch::Rows(vec![insert(6100), insert(1)])))
                .await
                .unwrap();
            let token = CancellationToken::new();
            let task_token = token.clone();
            let target = driver.clone();
            let target_table = table.clone();
            let import =
                tokio::spawn(
                    async move { target.insert_stream(target_table, input, task_token).await },
                );
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
            let result = tokio::time::timeout(Duration::from_secs(4), import)
                .await
                .unwrap()
                .unwrap();
            assert!(result.is_err(), "{name}");
            assert_eq!(
                driver.transaction_state().await.unwrap(),
                TransactionState::Idle
            );
            assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
            sql(locking.as_ref(), "ROLLBACK".into()).await.unwrap();
            drop(output);
            engine.disconnect(&blocker.id).await.unwrap();
        }
        if name == "mysql" {
            sql(driver.as_ref(), "SET autocommit=0".into())
                .await
                .unwrap();
            assert!(
                stream(
                    driver.clone(),
                    table.clone(),
                    vec![
                        Ok(InsertBatch::Rows(vec![insert(4000)])),
                        Ok(InsertBatch::Complete)
                    ]
                )
                .await
                .unwrap()
                .pending_transaction
            );
            sql(driver.as_ref(), "ROLLBACK; SET autocommit=1".into())
                .await
                .unwrap();
            assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
            let myisam = Table {
                name: format!("myisam_{}", uuid::Uuid::new_v4().simple()),
                ..table.clone()
            };
            let target = format!(
                "{}.{}",
                driver.quote_identifier(&myisam.schema),
                driver.quote_identifier(&myisam.name)
            );
            sql(
                driver.as_ref(),
                format!("CREATE TABLE {target}(id BIGINT PRIMARY KEY,label TEXT) ENGINE=MyISAM"),
            )
            .await
            .unwrap();
            assert!(
                stream(
                    driver.clone(),
                    myisam,
                    vec![
                        Ok(InsertBatch::Rows(vec![insert(1)])),
                        Ok(InsertBatch::Complete)
                    ]
                )
                .await
                .is_err()
            );
            assert_eq!(count(driver.as_ref(), &target).await, 0);
            sql(driver.as_ref(), format!("DROP TABLE {target}"))
                .await
                .unwrap();
        }

        let mut read_only = config.clone();
        read_only.id.clear();
        read_only.read_only = true;
        read_only.create_file = false;
        read_only.validate().unwrap();
        engine.store.save(&read_only).unwrap();
        engine.connect(&read_only.id, None).await.unwrap();
        let ro = engine.driver(&read_only.id).await.unwrap();
        assert!(!ro.capabilities().import_rows);
        assert!(
            stream(
                ro,
                table.clone(),
                vec![
                    Ok(InsertBatch::Rows(vec![insert(5000)])),
                    Ok(InsertBatch::Complete)
                ]
            )
            .await
            .is_err()
        );
        engine.disconnect(&read_only.id).await.unwrap();
        if name == "mysql" {
            let audit = format!(
                "{}.{}",
                driver.quote_identifier(&table.schema),
                driver.quote_identifier(&format!("audit_{}", uuid::Uuid::new_v4().simple()))
            );
            sql(
                driver.as_ref(),
                format!("CREATE TABLE {audit}(id BIGINT) ENGINE=MyISAM"),
            )
            .await
            .unwrap();
            let trigger = driver.quote_identifier(&format!("{}_audit", table.name));
            sql(driver.as_ref(), format!("CREATE TRIGGER {trigger} AFTER INSERT ON {qualified} FOR EACH ROW INSERT INTO {audit}(id) VALUES(NEW.id)")).await.unwrap();
            let failed = stream(
                driver.clone(),
                table.clone(),
                vec![
                    Ok(InsertBatch::Rows(vec![insert(7000)])),
                    Err(Error::new("Late CSV validation failure")),
                ],
            )
            .await
            .unwrap_err();
            assert!(
                failed.message.contains("rollback could not be confirmed"),
                "{}",
                failed.message
            );
            assert!(driver.transaction_state().await.is_err());
            let mut observer = config.clone();
            observer.id.clear();
            observer.validate().unwrap();
            engine.store.save(&observer).unwrap();
            engine.connect(&observer.id, None).await.unwrap();
            let observing = engine.driver(&observer.id).await.unwrap();
            assert_eq!(count(observing.as_ref(), &qualified).await, 1201);
            assert_eq!(count(observing.as_ref(), &audit).await, 1);
            sql(
                observing.as_ref(),
                format!("DROP TABLE {qualified}; DROP TABLE {audit}"),
            )
            .await
            .unwrap();
            engine.disconnect(&observer.id).await.unwrap();
        } else {
            sql(driver.as_ref(), format!("DROP TABLE {qualified}"))
                .await
                .unwrap();
        }
        engine.disconnect(&config.id).await.unwrap();
    }
}
