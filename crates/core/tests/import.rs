use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_core::import::{ImportRequest, ImportStatus};
use klyndb_driver_api::*;
use klyndb_import::{ImportOptions, Mapping, Snapshot, ValueKind};
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
            &ImportOptions::default(),
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
async fn finished(engine: &Engine, id: &str) -> ImportStatus {
    tokio::time::timeout(Duration::from_secs(65), async {
        loop {
            let status = engine.imports.status(id).unwrap();
            if status.done {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn duckdb_csv_json_jobs_exact_roundtrips_confirmation_and_atomic_failures() {
    use klyndb_import::ImportFormat;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut config = Connection {
        id: String::new(),
        name: "DuckDB import".into(),
        engine: "duckdb".into(),
        address: directory
            .path()
            .join("native.duckdb")
            .to_string_lossy()
            .into_owned(),
        environment: "production".into(),
        group: String::new(),
        color: "#93d4b5".into(),
        favorite: false,
        read_only: false,
        create_file: true,
    };
    config.validate().unwrap();
    engine.store.save(&config).unwrap();
    engine.connect(&config.id, None, None, None).await.unwrap();
    let driver = engine.driver(&config.id).await.unwrap();
    let table = Table {
        schema: "main".into(),
        name: "imports".into(),
        kind: "table".into(),
    };
    sql(driver.as_ref(), "CREATE TABLE imports(id UBIGINT PRIMARY KEY,label VARCHAR,precise DECIMAL(38,18),derived BIGINT GENERATED ALWAYS AS(length(label)))".into()).await.unwrap();
    let mapping = [ValueKind::Number, ValueKind::Text, ValueKind::Number]
        .into_iter()
        .zip(["id", "label", "precise"])
        .map(|(kind, column)| Mapping {
            column: Some(column.into()),
            kind,
        })
        .collect::<Vec<_>>();
    let file = directory.path().join("rows.data");
    let label = "  é, \"quoted\"\nnext  ";
    let precise = "12345678901234567890.123456789012345678";
    let expected = (1..=1201)
        .map(|id| {
            vec![
                Cell::Number(id.to_string()),
                if id == 1 {
                    Cell::Null
                } else {
                    Cell::Text(label.into())
                },
                Cell::Number(precise.into()),
            ]
        })
        .collect::<Vec<_>>();
    for format in [
        ImportFormat::Csv,
        ImportFormat::Json,
        ImportFormat::KlyndbJson,
    ] {
        let options = ImportOptions {
            format,
            null_value: Some("\\N".into()),
            ..Default::default()
        };
        let good = match format {
            ImportFormat::Csv => format!(
                "id,label,precise\n{}",
                (1..=1201)
                    .map(|id| format!(
                        "{id},{},{precise}\n",
                        if id == 1 {
                            "\\N".into()
                        } else {
                            format!("\"{}\"", label.replace('"', "\"\""))
                        }
                    ))
                    .collect::<String>()
            )
            .into_bytes(),
            ImportFormat::Json => format!(
                "[{}]",
                (1..=1201)
                    .map(|id| format!(
                        "{{\"id\":{id},\"label\":{},\"precise\":{precise}}}",
                        if id == 1 {
                            "null".into()
                        } else {
                            serde_json::to_string(label).unwrap()
                        }
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .into_bytes(),
            ImportFormat::KlyndbJson => {
                let mut bytes = vec![];
                klyndb_export::export(
                    &mut bytes,
                    &["id".into(), "label".into(), "precise".into()],
                    expected.clone().into_iter().map(Ok),
                    "json",
                    "",
                )
                .unwrap();
                bytes
            }
        };
        std::fs::write(&file, &good).unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        assert_eq!(source.preview.rows[0][2].as_deref(), Some(precise));
        let request = |source: String, confirmed: bool| ImportRequest {
            source,
            connection: config.id.clone(),
            table: table.clone(),
            options: options.clone(),
            mapping: mapping.clone(),
            timeout_seconds: 60,
            confirmed,
        };
        assert!(
            engine
                .start_import(request(source.id.clone(), false))
                .await
                .unwrap_err()
                .message
                .contains("Confirmation")
        );
        let mut generated = request(source.id.clone(), true);
        generated.mapping[0].column = Some("derived".into());
        assert!(engine.start_import(generated).await.is_err());
        std::fs::write(&file, b"changed after selection").unwrap();
        let id = engine
            .start_import(request(source.id.clone(), true))
            .await
            .unwrap();
        let status = finished(&engine, &id).await;
        engine.imports.release(&id).unwrap();
        assert!(status.error.is_none(), "{:?}: {:?}", format, status.error);
        assert_eq!(status.read_rows, 1201);
        let result = status.result.unwrap();
        assert_eq!(result.affected, 1201);
        assert!(!result.pending_transaction);
        assert_eq!(status.transaction, Some(TransactionState::Idle));
        assert_eq!(
            sql(
                driver.as_ref(),
                "SELECT id,label,precise FROM imports ORDER BY id".into()
            )
            .await
            .unwrap(),
            expected
        );
        // Export the stored native values through the real core result spool.
        let job_id = engine
            .start(
                config.id.clone(),
                "SELECT id,label,precise FROM imports ORDER BY id".into(),
                2000,
                30,
                true,
            )
            .await
            .unwrap();
        let job = engine.job(&job_id).unwrap();
        while !job.status().unwrap().done {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(job.status().unwrap().error.is_none());
        let mut exported = vec![];
        assert_eq!(job.export(&mut exported, 0, "json", "").unwrap(), 1201);
        sql(driver.as_ref(), "DELETE FROM imports".into())
            .await
            .unwrap();
        std::fs::write(&file, &exported).unwrap();
        let typed = ImportOptions {
            format: ImportFormat::KlyndbJson,
            ..Default::default()
        };
        let source = engine
            .imports
            .prepare(file.clone(), typed.clone())
            .await
            .unwrap();
        let mut request = request(source.id.clone(), true);
        request.options = typed;
        let id = engine.start_import(request).await.unwrap();
        assert!(finished(&engine, &id).await.error.is_none());
        engine.imports.release(&id).unwrap();
        assert_eq!(
            sql(
                driver.as_ref(),
                "SELECT id,label,precise FROM imports ORDER BY id".into()
            )
            .await
            .unwrap(),
            expected
        );
        sql(driver.as_ref(), "DELETE FROM imports".into())
            .await
            .unwrap();
    }
    let options = ImportOptions::default();
    let request = |source: String| ImportRequest {
        source,
        connection: config.id.clone(),
        table: table.clone(),
        options: options.clone(),
        mapping: mapping.clone(),
        timeout_seconds: 60,
        confirmed: true,
    };
    // A failure beyond multiple native batches must roll back every prefix row.
    let prefix = format!(
        "id,label,precise\n{}",
        (1..=1201)
            .map(|id| format!("{id},prefix,{precise}\n"))
            .collect::<String>()
    );
    for suffix in [
        format!("1,duplicate,{precise}\n"),
        "1202,broken,not-a-number\n".into(),
    ] {
        std::fs::write(&file, format!("{prefix}{suffix}")).unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        let id = engine.start_import(request(source.id)).await.unwrap();
        let status = finished(&engine, &id).await;
        engine.imports.release(&id).unwrap();
        assert!(status.error.is_some());
        assert_eq!(status.transaction, Some(TransactionState::Idle));
        assert_eq!(count(driver.as_ref(), "imports").await, 0);
    }
    std::fs::write(&file, format!("id,label,precise\n2,new,{precise}\n")).unwrap();
    let source = engine
        .imports
        .prepare(file.clone(), options.clone())
        .await
        .unwrap();
    sql(
        driver.as_ref(),
        "BEGIN; INSERT INTO imports(id,label,precise) VALUES(1,'caller',1)".into(),
    )
    .await
    .unwrap();
    let id = engine.start_import(request(source.id)).await.unwrap();
    let status = finished(&engine, &id).await;
    engine.imports.release(&id).unwrap();
    assert!(status.error.unwrap().contains("no savepoints"));
    assert_eq!(status.transaction, Some(TransactionState::Active));
    assert_eq!(count(driver.as_ref(), "imports").await, 1);
    sql(driver.as_ref(), "ROLLBACK".into()).await.unwrap();
    let change = Change::Insert {
        values: BTreeMap::from([("id".into(), Cell::Number("3".into()))]),
    };
    assert!(
        engine
            .apply_changes(&config.id, table.clone(), vec![change.clone()], false)
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .apply_changes(&config.id, table.clone(), vec![change.clone()], true)
            .await
            .unwrap()
            .affected,
        1
    );
    // The production job deadline interrupts native default evaluation, then rolls back.
    sql(driver.as_ref(), "CREATE TABLE slow(id BIGINT PRIMARY KEY,label VARCHAR,precise DECIMAL(38,18),expensive VARCHAR DEFAULT sha256(repeat(uuid()::VARCHAR,1000000)))".into()).await.unwrap();
    std::fs::write(&file, &prefix).unwrap();
    let source = engine
        .imports
        .prepare(file.clone(), options.clone())
        .await
        .unwrap();
    let mut timed = request(source.id);
    timed.table.name = "slow".into();
    timed.timeout_seconds = 1;
    let id = engine.start_import(timed).await.unwrap();
    let status = finished(&engine, &id).await;
    engine.imports.release(&id).unwrap();
    assert!(status.error.unwrap().contains("timed out"));
    assert_eq!(status.transaction, Some(TransactionState::Idle));
    assert_eq!(count(driver.as_ref(), "slow").await, 0);
    assert_eq!(
        sql(driver.as_ref(), "SELECT 42".into()).await.unwrap()[0][0].text(),
        "42"
    );
    engine.disconnect(&config.id).await.unwrap();
    config.read_only = true;
    engine.store.save(&config).unwrap();
    let caps = engine.connect(&config.id, None, None, None).await.unwrap();
    assert!(!caps.edit_rows && !caps.import_rows);
    let readonly = engine.driver(&config.id).await.unwrap();
    assert!(!readonly.inspect(&table).await.unwrap().editable);
    assert!(
        readonly
            .apply_changes(table.clone(), vec![change])
            .await
            .is_err()
    );
    let (_, rx) = mpsc::channel(1);
    assert!(
        readonly
            .insert_stream(table, rx, CancellationToken::new())
            .await
            .is_err()
    );
    engine.disconnect(&config.id).await.unwrap();
}
async fn reconnect_if_closed(
    engine: &Engine,
    config: &Connection,
    driver: &mut Arc<dyn Session>,
    message: &str,
) {
    if message.to_lowercase().contains("connection closed") {
        engine.disconnect(&config.id).await.unwrap();
        engine.connect(&config.id, None, None, None).await.unwrap();
        *driver = engine.driver(&config.id).await.unwrap();
    }
}

#[tokio::test]
async fn core_import_jobs_confirmation_deadline_and_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut configs = vec![(
        "sqlite",
        directory
            .path()
            .join("jobs.db")
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
        let engine = Engine::new(
            Store::open(&directory.path().join(format!("{name}-jobs-state.db"))).unwrap(),
        );
        let mut config = Connection {
            id: String::new(),
            name: "Import job contract".into(),
            engine: name.into(),
            address,
            environment: "production".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: name == "sqlite",
        };
        config.validate().unwrap();
        engine.store.save(&config).unwrap();
        engine.connect(&config.id, None, None, None).await.unwrap();
        let mut driver = engine.driver(&config.id).await.unwrap();
        let table = Table {
            schema: match name {
                "sqlite" => "main",
                "postgres" => "public",
                _ => "klyndb_test",
            }
            .into(),
            name: format!("jobs_{}", uuid::Uuid::new_v4().simple()),
            kind: "table".into(),
        };
        let qualified = format!(
            "{}.{}",
            driver.quote_identifier(&table.schema),
            driver.quote_identifier(&table.name)
        );
        sql(driver.as_ref(), format!("CREATE TABLE {qualified}(id BIGINT PRIMARY KEY,label TEXT,doubled BIGINT GENERATED ALWAYS AS (id * 2) STORED){}", if name == "mysql" { " ENGINE=InnoDB" } else { "" })).await.unwrap();
        let options = ImportOptions {
            delimiter: ";".into(),
            null_value: Some("\\N".into()),
            ..Default::default()
        };
        let mapping = vec![
            Mapping {
                column: Some("id".into()),
                kind: ValueKind::Number,
            },
            Mapping {
                column: Some("label".into()),
                kind: ValueKind::Text,
            },
            Mapping {
                column: None,
                kind: ValueKind::Text,
            },
        ];
        let request = |source: String| ImportRequest {
            source,
            connection: config.id.clone(),
            table: table.clone(),
            options: options.clone(),
            mapping: mapping.clone(),
            timeout_seconds: 10,
            confirmed: true,
        };
        let file = directory.path().join(format!("{name}-job.csv"));
        std::fs::write(
            &file,
            b"\xef\xbb\xbfid;label;ignored\r\n1;\"hello;quoted\nnext\";x\r\n2;\\N;x\r\n3;last;x",
        )
        .unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        assert_eq!(source.preview.rows.len(), 3);
        assert!(
            !serde_json::to_string(&source)
                .unwrap()
                .contains(directory.path().to_str().unwrap())
        );
        let mut unconfirmed = request(source.id.clone());
        unconfirmed.confirmed = false;
        assert!(
            engine
                .start_import(unconfirmed)
                .await
                .unwrap_err()
                .message
                .contains("Confirmation")
        );
        assert!(engine.imports.status(&source.id).is_err());
        let mut generated = request(source.id.clone());
        generated.mapping[0].column = Some("doubled".into());
        assert!(engine.start_import(generated).await.is_err());
        let mut duplicate = request(source.id.clone());
        duplicate.mapping[1].column = Some("id".into());
        assert!(engine.start_import(duplicate).await.is_err());
        std::fs::write(&file, b"changed after selection").unwrap();
        let id = engine
            .start_import(request(source.id.clone()))
            .await
            .unwrap();
        let status = finished(&engine, &id).await;
        assert!(status.error.is_none(), "{name}: {:?}", status.error);
        assert_eq!(status.read_rows, 3);
        assert_eq!(status.result.unwrap().affected, 3);
        assert_eq!(status.transaction, Some(TransactionState::Idle));
        assert_eq!(count(driver.as_ref(), &qualified).await, 3);
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT label FROM {qualified} WHERE id=1")
            )
            .await
            .unwrap()[0][0]
                .text(),
            "hello;quoted\nnext"
        );
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT label FROM {qualified} WHERE id=2")
            )
            .await
            .unwrap()[0][0],
            Cell::Null
        );
        assert!(engine.start_import(request(id.clone())).await.is_err());
        engine.imports.release(&id).unwrap();
        assert!(engine.imports.preview(&id, options.clone()).await.is_err());

        std::fs::write(&file, b"id;label;ignored\n50;pending;x").unwrap();
        sql(driver.as_ref(), "BEGIN".into()).await.unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        let id = engine.start_import(request(source.id)).await.unwrap();
        let status = finished(&engine, &id).await;
        assert!(status.result.unwrap().pending_transaction);
        assert_eq!(status.transaction, Some(TransactionState::Active));
        sql(driver.as_ref(), "ROLLBACK".into()).await.unwrap();
        assert_eq!(count(driver.as_ref(), &qualified).await, 3);
        engine.imports.release(&id).unwrap();

        // A real independent lock exercises the per-file deadline and whole-file undo.
        let mut blocker = config.clone();
        blocker.id.clear();
        blocker.validate().unwrap();
        engine.store.save(&blocker).unwrap();
        engine.connect(&blocker.id, None, None, None).await.unwrap();
        let locking = engine.driver(&blocker.id).await.unwrap();
        sql(
            locking.as_ref(),
            format!("BEGIN; UPDATE {qualified} SET label='locked' WHERE id=1"),
        )
        .await
        .unwrap();
        std::fs::write(&file, b"id;label;ignored\n6000;before lock;x\n1;locked;x").unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        let mut timed = request(source.id);
        timed.timeout_seconds = 1;
        let id = engine.start_import(timed).await.unwrap();
        let status = finished(&engine, &id).await;
        let failure = status.error.unwrap();
        if name == "sqlite" {
            // SQLite can reject a read-to-write lock upgrade immediately rather
            // than waiting for its busy timeout; exercise VM timeout separately.
            assert!(
                failure.contains("locked") || failure.contains("busy"),
                "{failure}"
            );
        } else {
            assert!(failure.contains("timed out after 1 seconds"), "{failure}");
        }
        assert!(status.result.is_none());
        sql(locking.as_ref(), "ROLLBACK".into()).await.unwrap();
        engine.disconnect(&blocker.id).await.unwrap();
        reconnect_if_closed(&engine, &config, &mut driver, &failure).await;
        assert_eq!(count(driver.as_ref(), &qualified).await, 3);
        engine.imports.release(&id).unwrap();

        if name == "sqlite" {
            let trigger = driver.quote_identifier(&format!("{}_slow", table.name));
            sql(driver.as_ref(), format!("CREATE TRIGGER {trigger} BEFORE INSERT ON {qualified} WHEN NEW.id=6000 BEGIN SELECT (WITH RECURSIVE n(v) AS (VALUES(1) UNION ALL SELECT v+1 FROM n WHERE v<100000000) SELECT sum(v) FROM n); END")).await.unwrap();
            let data = format!(
                "id;label;ignored\n{}6000;slow;x\n",
                (7000..7256)
                    .map(|id| format!("{id};before timeout;x\n"))
                    .collect::<String>()
            );
            std::fs::write(&file, data).unwrap();
            let source = engine
                .imports
                .prepare(file.clone(), options.clone())
                .await
                .unwrap();
            let mut timed = request(source.id);
            timed.timeout_seconds = 1;
            let id = engine.start_import(timed).await.unwrap();
            let status = finished(&engine, &id).await;
            let failure = status.error.unwrap();
            assert!(failure.contains("timed out after 1 seconds"), "{failure}");
            assert!(status.result.is_none());
            reconnect_if_closed(&engine, &config, &mut driver, &failure).await;
            assert_eq!(count(driver.as_ref(), &qualified).await, 3);
            engine.imports.release(&id).unwrap();
            sql(
                driver.as_ref(),
                format!(
                    "BEGIN; INSERT INTO {qualified}(id,label) VALUES(5500,'earlier uncommitted')"
                ),
            )
            .await
            .unwrap();
            let source = engine
                .imports
                .prepare(file.clone(), options.clone())
                .await
                .unwrap();
            let mut timed = request(source.id);
            timed.timeout_seconds = 1;
            let id = engine.start_import(timed).await.unwrap();
            let status = finished(&engine, &id).await;
            let failure = status.error.unwrap();
            assert!(failure.contains("earlier uncommitted changes"), "{failure}");
            reconnect_if_closed(&engine, &config, &mut driver, &failure).await;
            assert_eq!(count(driver.as_ref(), &qualified).await, 3);
            engine.imports.release(&id).unwrap();
            sql(driver.as_ref(), format!("DROP TRIGGER {trigger}"))
                .await
                .unwrap();
        }
        let data = format!(
            "id;label;ignored\n{}",
            (10000..40000)
                .map(|id| format!("{id};new;x\n"))
                .collect::<String>()
        );
        std::fs::write(&file, &data).unwrap();
        for disconnect in [false, true] {
            let source = engine
                .imports
                .prepare(file.clone(), options.clone())
                .await
                .unwrap();
            let id = engine.start_import(request(source.id)).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while engine.imports.status(&id).unwrap().read_rows < 512 {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
            .await
            .unwrap();
            assert!(!engine.imports.status(&id).unwrap().done, "{name}");
            assert!(engine.imports.release(&id).is_err());
            if disconnect {
                tokio::time::timeout(Duration::from_secs(8), engine.disconnect(&config.id))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(engine.driver(&config.id).await.is_err());
            } else {
                engine.imports.cancel(&id).unwrap();
            }
            let status = finished(&engine, &id).await;
            let failure = status.error.unwrap();
            assert!(status.result.is_none());
            if disconnect {
                engine.connect(&config.id, None, None, None).await.unwrap();
                driver = engine.driver(&config.id).await.unwrap();
            } else {
                reconnect_if_closed(&engine, &config, &mut driver, &failure).await;
            }
            assert_eq!(count(driver.as_ref(), &qualified).await, 3);
            engine.imports.release(&id).unwrap();
        }
        let mut read_only = config.clone();
        read_only.id.clear();
        read_only.read_only = true;
        read_only.create_file = false;
        read_only.validate().unwrap();
        engine.store.save(&read_only).unwrap();
        engine
            .connect(&read_only.id, None, None, None)
            .await
            .unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        let mut ro = request(source.id.clone());
        ro.connection = read_only.id.clone();
        assert!(engine.start_import(ro).await.is_err());
        engine.imports.release(&source.id).unwrap();
        engine.disconnect(&read_only.id).await.unwrap();

        let mut files = vec![];
        for _ in 0..4 {
            files.push(
                engine
                    .imports
                    .prepare(file.clone(), options.clone())
                    .await
                    .unwrap()
                    .id,
            );
        }
        assert!(
            engine
                .imports
                .prepare(file.clone(), options.clone())
                .await
                .is_err()
        );
        for id in files {
            engine.imports.release(&id).unwrap();
        }
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        let id = engine.start_import(request(source.id)).await.unwrap();
        assert!(engine.imports.begin_shutdown());
        tokio::time::timeout(Duration::from_secs(8), engine.imports.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert!(finished(&engine, &id).await.error.is_some());
        assert_eq!(count(driver.as_ref(), &qualified).await, 3);
        engine.imports.release(&id).unwrap();
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        assert!(
            engine
                .start_import(request(source.id.clone()))
                .await
                .is_err()
        );
        engine.imports.release(&source.id).unwrap();
        sql(driver.as_ref(), format!("DROP TABLE {qualified}"))
            .await
            .unwrap();
        engine.disconnect(&config.id).await.unwrap();
    }
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
        engine.connect(&config.id, None, None, None).await.unwrap();
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
            engine.connect(&blocker.id, None, None, None).await.unwrap();
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
        engine
            .connect(&read_only.id, None, None, None)
            .await
            .unwrap();
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
            engine
                .connect(&observer.id, None, None, None)
                .await
                .unwrap();
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

#[tokio::test]
async fn json_import_roundtrip_and_late_error_rollback() {
    use klyndb_import::ImportFormat;
    let directory = tempfile::tempdir().unwrap();
    let mut configs = vec![(
        "sqlite",
        directory
            .path()
            .join("json.db")
            .to_string_lossy()
            .into_owned(),
    )];
    for (name, variable) in [
        ("postgres", "KLYNDB_TEST_POSTGRES_URL"),
        ("mysql", "KLYNDB_TEST_MYSQL_URL"),
        ("mariadb", "KLYNDB_TEST_MARIADB_URL"),
    ] {
        if let Ok(url) = std::env::var(variable) {
            configs.push((name, url));
        }
    }
    for (name, address) in configs {
        let engine = Engine::new(
            Store::open(&directory.path().join(format!("{name}-json-state.db"))).unwrap(),
        );
        let mut config = Connection {
            id: String::new(),
            name: "JSON import contract".into(),
            engine: if name == "mariadb" { "mysql" } else { name }.into(),
            address,
            environment: "production".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: name == "sqlite",
        };
        config.validate().unwrap();
        engine.store.save(&config).unwrap();
        engine.connect(&config.id, None, None, None).await.unwrap();
        let driver = engine.driver(&config.id).await.unwrap();
        let table = Table {
            schema: match name {
                "sqlite" => "main",
                "postgres" => "public",
                _ => "klyndb_test",
            }
            .into(),
            name: format!("json_import_{}", uuid::Uuid::new_v4().simple()),
            kind: "table".into(),
        };
        let qualified = format!(
            "{}.{}",
            driver.quote_identifier(&table.schema),
            driver.quote_identifier(&table.name)
        );
        sql(driver.as_ref(), format!("CREATE TABLE {qualified}(id BIGINT PRIMARY KEY,label TEXT,precise TEXT,document JSON){}", if matches!(name, "mysql" | "mariadb") { " ENGINE=InnoDB" } else { "" })).await.unwrap();
        let file = directory.path().join(format!("{name}.json"));
        let good = format!("[{}]", (1..=1201).map(|id| format!(r#"{{"id":{id},"label":{},"precise":1.234567890123456789,"document":{{"n":123456789012345678901234567890,"list":[true,null,"😀"]}}}}"#, if id == 1 { "null".to_string() } else { serde_json::to_string(&format!("  row {id}, \"quoted\"\nnext  ")).unwrap() })).collect::<Vec<_>>().join(","));
        std::fs::write(&file, &good).unwrap();
        let options = ImportOptions {
            format: ImportFormat::Json,
            trim: true,
            empty_as_null: true,
            null_value: Some("null".into()),
            ..Default::default()
        };
        let mapping = [
            ValueKind::Number,
            ValueKind::Text,
            ValueKind::Text,
            ValueKind::Json,
        ]
        .into_iter()
        .zip(["id", "label", "precise", "document"])
        .map(|(kind, column)| Mapping {
            column: Some(column.into()),
            kind,
        })
        .collect::<Vec<_>>();
        let request = |source: String, options: ImportOptions, confirmed: bool| ImportRequest {
            source,
            connection: config.id.clone(),
            table: table.clone(),
            options,
            mapping: mapping.clone(),
            timeout_seconds: 10,
            confirmed,
        };
        let source = engine
            .imports
            .prepare(file.clone(), options.clone())
            .await
            .unwrap();
        assert_eq!(source.preview.rows[0][1], None);
        assert_eq!(
            source.preview.rows[0][2],
            Some("1.234567890123456789".into())
        );
        assert!(
            engine
                .start_import(request(source.id.clone(), options.clone(), false))
                .await
                .unwrap_err()
                .message
                .contains("Confirmation")
        );
        // Selection captured a private immutable file; later edits cannot change it.
        std::fs::write(&file, b"changed").unwrap();
        let id = engine
            .start_import(request(source.id.clone(), options.clone(), true))
            .await
            .unwrap();
        let status = finished(&engine, &id).await;
        assert!(status.error.is_none(), "{name}: {:?}", status.error);
        assert_eq!(status.result.unwrap().affected, 1201);
        assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
        let rows = sql(
            driver.as_ref(),
            format!(
                "SELECT label,precise,document FROM {qualified} WHERE id IN (1,1201) ORDER BY id"
            ),
        )
        .await
        .unwrap();
        assert_eq!(rows[0][0], Cell::Null);
        assert_eq!(rows[1][0].text(), "  row 1201, \"quoted\"\nnext  ");
        assert_eq!(rows[1][1].text(), "1.234567890123456789");
        if name == "mysql" {
            // MySQL's native JSON storage converts this out-of-range integer to
            // a double; the input parser and TEXT mapping must still stay exact.
            assert!(rows[1][2].text().contains("1.2345678901234568e+29"));
        } else {
            assert!(
                rows[1][2].text().contains("123456789012345678901234567890"),
                "{name}: {}",
                rows[1][2].text()
            );
        }
        engine.imports.release(&source.id).unwrap();
        // More than one batch was written before these late parser errors occur.
        let new_rows = (2001..=3201)
            .map(|id| format!(r#"{{"id":{id},"label":"new","precise":2,"document":null}}"#))
            .collect::<Vec<_>>()
            .join(",");
        for ending in [
            "] trailing",
            ",]",
            ", {\"id\":4000,\"label\":\"x\",\"precise\":2,\"document\":null,\"id\":4001}]",
            ", {\"id\":4000,\"label\":\"x\",\"precise\":2,\"other\":null}]",
            "",
        ] {
            std::fs::write(&file, format!("[{new_rows}{ending}")).unwrap();
            let source = engine
                .imports
                .prepare(file.clone(), options.clone())
                .await
                .unwrap();
            let id = engine
                .start_import(request(source.id.clone(), options.clone(), true))
                .await
                .unwrap();
            let status = finished(&engine, &id).await;
            assert!(
                status
                    .error
                    .as_ref()
                    .is_some_and(|error| error.contains("JSON")),
                "{name}: {:?}",
                status.error
            );
            assert!(status.read_rows >= 1000);
            assert_eq!(count(driver.as_ref(), &qualified).await, 1201, "{name}");
            assert_eq!(
                driver.transaction_state().await.unwrap(),
                TransactionState::Idle
            );
            engine.imports.release(&source.id).unwrap();
        }
        // Roundtrip actual native JSON export cells, inside the caller's transaction.
        let mut out = std::fs::File::create(&file).unwrap();
        klyndb_export::export(
            &mut out,
            &[
                "id".into(),
                "label".into(),
                "precise".into(),
                "document".into(),
            ],
            (5001..=5010).map(|id| {
                Ok(vec![
                    Cell::Number(id.to_string()),
                    Cell::Null,
                    Cell::Number("18446744073709551615".into()),
                    Cell::Json(serde_json::json!("plain JSON string")),
                ])
            }),
            "json",
            "",
        )
        .unwrap();
        drop(out);
        let native_options = ImportOptions {
            format: ImportFormat::KlyndbJson,
            ..Default::default()
        };
        let source = engine
            .imports
            .prepare(file.clone(), native_options.clone())
            .await
            .unwrap();
        sql(driver.as_ref(), "BEGIN".into()).await.unwrap();
        let id = engine
            .start_import(request(source.id.clone(), native_options, true))
            .await
            .unwrap();
        let status = finished(&engine, &id).await;
        assert!(status.error.is_none(), "{name}: {:?}", status.error);
        let result = status.result.unwrap();
        assert_eq!(result.affected, 10);
        assert!(result.pending_transaction);
        let rows = sql(
            driver.as_ref(),
            format!("SELECT label,precise,document FROM {qualified} WHERE id=5001"),
        )
        .await
        .unwrap();
        assert_eq!(rows[0][0], Cell::Null);
        assert_eq!(rows[0][1].text(), "18446744073709551615");
        assert_eq!(rows[0][2].text(), "\"plain JSON string\"");
        sql(driver.as_ref(), "ROLLBACK".into()).await.unwrap();
        assert_eq!(count(driver.as_ref(), &qualified).await, 1201);
        engine.imports.release(&source.id).unwrap();
        sql(driver.as_ref(), format!("DROP TABLE {qualified}"))
            .await
            .unwrap();
        engine.disconnect(&config.id).await.unwrap();
    }
}
