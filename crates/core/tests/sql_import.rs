use klyndb_connections::{Connection, Store};
use klyndb_core::{
    Engine,
    import::{ImportStatus, SqlImportRequest},
};
use klyndb_driver_api::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn sql(driver: &dyn Session, sql: String) -> Result<Vec<Row>> {
    let (output, mut input) = mpsc::channel(2);
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
async fn done(engine: &Engine, id: &str) -> ImportStatus {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let status = engine.imports.status(id).unwrap();
            if status.done {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SQL import terminated")
}
async fn run_file(
    engine: &Engine,
    config: &Connection,
    path: &std::path::Path,
    text: &[u8],
) -> ImportStatus {
    std::fs::write(path, text).unwrap();
    let source = engine
        .prepare_sql_import(path.into(), &config.id)
        .await
        .unwrap();
    let id = engine
        .start_sql_import(SqlImportRequest {
            source: source.id,
            connection: config.id.clone(),
            timeout_seconds: 60,
            confirmed: true,
        })
        .await
        .unwrap();
    let status = done(engine, &id).await;
    engine.imports.release(&id).unwrap();
    status
}

#[tokio::test]
async fn streaming_sql_import_real_engines() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixtures = vec![
        (
            "sqlite",
            "sqlite",
            directory
                .path()
                .join("sql.db")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "duckdb",
            "duckdb",
            directory
                .path()
                .join("sql.duckdb")
                .to_string_lossy()
                .into_owned(),
        ),
    ];
    for (name, engine, variable) in [
        ("postgres", "postgres", "KLYNDB_TEST_POSTGRES_URL"),
        ("mysql", "mysql", "KLYNDB_TEST_MYSQL_URL"),
        ("mariadb", "mysql", "KLYNDB_TEST_MARIADB_URL"),
        ("mssql", "mssql", "KLYNDB_TEST_MSSQL_URL"),
    ] {
        if let Ok(url) = std::env::var(variable) {
            fixtures.push((name, engine, url));
        }
    }
    for (name, dialect, address) in fixtures {
        let engine =
            Engine::new(Store::open(&directory.path().join(format!("{name}-state.db"))).unwrap());
        let mut config = Connection {
            id: String::new(),
            name: format!("SQL import {name}"),
            engine: dialect.into(),
            address,
            environment: "production".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: matches!(dialect, "sqlite" | "duckdb"),
        };
        let embedded_password = config.validate().unwrap();
        engine.store.save(&config).unwrap();
        // Disposable SQL fixtures use session credentials, not the user's keychain.
        let password = Some(if dialect == "mssql" {
            std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap()
        } else {
            embedded_password
                .map(|secret| secret.to_string())
                .unwrap_or_default()
        });
        engine
            .connect(&config.id, password.clone(), None, None)
            .await
            .unwrap();
        let driver = engine.driver(&config.id).await.unwrap();
        assert!(driver.capabilities().import_sql);
        let table = format!("sql_import_{}", uuid::Uuid::new_v4().simple());
        let quote = driver.quote_identifier(&table);
        let binary = match dialect {
            "postgres" => "BYTEA",
            "mssql" => "VARBINARY(MAX)",
            _ => "BLOB",
        };
        let text = if dialect == "mssql" {
            "NVARCHAR(MAX)"
        } else {
            "TEXT"
        };
        let boundary = if dialect == "mssql" { "\nGO\n" } else { "\n" };
        let begin = if dialect == "mssql" {
            "BEGIN TRANSACTION"
        } else {
            "BEGIN"
        };
        let mut file = format!("\u{feff}-- SQL file; whole-file preflight\nCREATE TABLE {quote} (id BIGINT PRIMARY KEY, label {text}, precise {text}, payload {binary});{boundary}").into_bytes();
        let label = format!("  'quoted';\\ line\nGO\nnext é {}  ", "x".repeat(3600));
        let rows = (0..1201).map(|i| {
            Ok(vec![
                Cell::Number(i.to_string()),
                Cell::Text(label.clone()),
                Cell::Text("18446744073709551615".into()),
                Cell::Binary("00ff27".into()),
            ])
        });
        klyndb_export::export_for_engine(
            &mut file,
            &[
                "id".into(),
                "label".into(),
                "precise".into(),
                "payload".into(),
            ],
            rows,
            "sql",
            &table,
            dialect,
        )
        .unwrap();
        assert!(
            file.len() > klyndb_query::SQL_LIMIT,
            "{name}: whole file exceeds editor limit"
        );
        let path = directory.path().join(format!("{name}.sql"));
        std::fs::write(&path, &file).unwrap();
        let source = engine
            .prepare_sql_import(path.clone(), &config.id)
            .await
            .unwrap();
        assert_eq!(source.preview.statements, 1202);
        assert_eq!(
            source.preview.unit,
            if dialect == "mssql" {
                "batches"
            } else {
                "statements"
            }
        );
        assert_eq!(source.preview.sample.len(), 5);
        assert!(source.preview.sample.iter().all(|s| s.len() <= 516));
        let request = |confirmed| SqlImportRequest {
            source: source.id.clone(),
            connection: config.id.clone(),
            timeout_seconds: 60,
            confirmed,
        };
        assert!(
            engine
                .start_sql_import(request(false))
                .await
                .unwrap_err()
                .message
                .contains("confirm")
        );
        std::fs::write(&path, b"DROP TABLE definitely_not_the_snapshot;").unwrap();
        let id = engine.start_sql_import(request(true)).await.unwrap();
        assert!(engine.start_sql_import(request(true)).await.is_err());
        let status = done(&engine, &id).await;
        assert!(status.error.is_none(), "{name}: {:?}", status.error);
        assert_eq!(status.completed_statements, 1202);
        assert_eq!(status.transaction, Some(TransactionState::Idle));
        engine.imports.release(&id).unwrap();
        let stored = sql(
            driver.as_ref(),
            format!("SELECT * FROM {quote} ORDER BY id"),
        )
        .await
        .unwrap();
        assert_eq!(stored.len(), 1201);
        assert_eq!(stored[0][1].text(), label);
        assert_eq!(stored[0][2].text(), "18446744073709551615");
        assert_eq!(
            stored[0][3].text(),
            if dialect == "postgres" {
                "\\x00ff27"
            } else {
                "00ff27"
            }
        );
        let binary_sql = match dialect {
            "postgres" => "encode(payload, 'hex')",
            "mssql" => "CONVERT(varchar(max),payload,2)",
            _ => "hex(payload)",
        };
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT {binary_sql} FROM {quote} WHERE id=0")
            )
            .await
            .unwrap()[0][0]
                .text()
                .to_ascii_lowercase(),
            "00ff27"
        );

        // Export from the actual native result spool, then execute that export again.
        let query = engine
            .start(
                config.id.clone(),
                format!("SELECT * FROM {quote} ORDER BY id"),
                10_000,
                30,
                true,
            )
            .await
            .unwrap();
        loop {
            if engine.job(&query).unwrap().status().unwrap().done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut exported = vec![];
        engine
            .job(&query)
            .unwrap()
            .export(&mut exported, 0, "sql", &table)
            .unwrap();
        engine.release(&query).unwrap();
        sql(driver.as_ref(), format!("DELETE FROM {quote}"))
            .await
            .unwrap();
        let status = run_file(&engine, &config, &path, &exported).await;
        assert!(
            status.error.is_none(),
            "{name} native export: {:?}",
            status.error
        );
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT * FROM {quote} ORDER BY id")
            )
            .await
            .unwrap(),
            stored
        );

        // Syntax failures are found before any statement executes.
        let invalid = format!("INSERT INTO {quote} (id) VALUES (2001); SELECT 'unclosed");
        std::fs::write(&path, invalid).unwrap();
        assert!(
            engine
                .prepare_sql_import(path.clone(), &config.id)
                .await
                .is_err()
        );
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT count(*) FROM {quote} WHERE id=2001")
            )
            .await
            .unwrap()[0][0]
                .text(),
            "0"
        );

        // A native error reports the successful prefix, with no invented whole-file rollback.
        let failed = format!(
            "INSERT INTO {quote} (id) VALUES (2002);{boundary}INSERT INTO {quote} (id) VALUES (0);{boundary}INSERT INTO {quote} (id) VALUES (2003);"
        );
        let status = run_file(&engine, &config, &path, failed.as_bytes()).await;
        assert_eq!(status.completed_statements, 1, "{name}");
        assert!(status.error.unwrap().contains("Earlier effects"));
        assert_eq!(
            sql(
                driver.as_ref(),
                format!("SELECT id FROM {quote} WHERE id>=2002 ORDER BY id")
            )
            .await
            .unwrap(),
            vec![vec![if dialect == "postgres" {
                Cell::Text("2002".into())
            } else {
                Cell::Number("2002".into())
            }]]
        );

        let manual = format!("{begin}; INSERT INTO {quote} (id) VALUES (2004);");
        let status = run_file(&engine, &config, &path, manual.as_bytes()).await;
        assert_eq!(status.transaction, Some(TransactionState::Active));
        sql(driver.as_ref(), "ROLLBACK".into()).await.unwrap();
        assert!(
            sql(
                driver.as_ref(),
                format!("SELECT id FROM {quote} WHERE id=2004")
            )
            .await
            .unwrap()
            .is_empty()
        );

        // Hold an incomplete producer open. Another tab's COMMIT cannot interleave.
        let (send, input) = mpsc::channel(2);
        let (output, mut results) = mpsc::channel(2);
        let drain = tokio::spawn(async move { while results.recv().await.is_some() {} });
        let completed = Arc::new(AtomicU64::new(0));
        let cancel = CancellationToken::new();
        let worker_driver = driver.clone();
        let worker_cancel = cancel.clone();
        let worker_count = completed.clone();
        let writer = tokio::spawn(async move {
            worker_driver
                .execute_script(input, output, worker_cancel, worker_count)
                .await
        });
        send.send(Ok(ScriptBatch::Statement(begin.into())))
            .await
            .unwrap();
        send.send(Ok(ScriptBatch::Statement(format!(
            "INSERT INTO {quote} (id) VALUES (2005)"
        ))))
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while completed.load(Ordering::Relaxed) < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let competing = driver.clone();
        let mut commit =
            tokio::spawn(async move { sql(competing.as_ref(), "COMMIT".into()).await });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut commit)
                .await
                .is_err(),
            "{name}: COMMIT interleaved"
        );
        send.send(Ok(ScriptBatch::Complete)).await.unwrap();
        drop(send);
        writer.await.unwrap().unwrap();
        drain.await.unwrap();
        commit.await.unwrap().unwrap();
        assert_eq!(
            driver.transaction_state().await.unwrap(),
            TransactionState::Idle
        );

        // A dropped producer fails closed, and cancelling the core stops a running server statement.
        let (send, input) = mpsc::channel(1);
        drop(send);
        let (output, mut results) = mpsc::channel(1);
        let drain = async move { while results.recv().await.is_some() {} };
        let (failed, _) = tokio::join!(
            driver.execute_script(
                input,
                output,
                CancellationToken::new(),
                Arc::new(AtomicU64::new(0))
            ),
            drain
        );
        assert!(failed.unwrap_err().message.contains("reader stopped"));
        let slow = match dialect {
            "postgres" => "SELECT pg_sleep(30);",
            "mysql" => "SELECT SLEEP(30);",
            "mssql" => "EXEC(N'WAITFOR DELAY ''00:00:30''');",
            "duckdb" => {
                "SELECT sum(a.i+b.i) FROM range(1000000000) a(i) CROSS JOIN range(1000000000) b(i);"
            }
            _ => {
                "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n;"
            }
        };
        std::fs::write(&path, slow).unwrap();
        let source = engine
            .prepare_sql_import(path.clone(), &config.id)
            .await
            .unwrap();
        let id = engine
            .start_sql_import(SqlImportRequest {
                source: source.id,
                connection: config.id.clone(),
                timeout_seconds: 1,
                confirmed: true,
            })
            .await
            .unwrap();
        let status = done(&engine, &id).await;
        assert!(
            status
                .error
                .as_ref()
                .is_some_and(|error| error.contains("timed out")),
            "{name}: {:?}",
            status.error
        );
        engine.imports.release(&id).unwrap();
        assert_eq!(
            sql(driver.as_ref(), "SELECT 42".into()).await.unwrap()[0][0].text(),
            "42"
        );

        if dialect == "duckdb" {
            // Discarded SELECTs do not need display decoding or the editor's retained-row cap.
            let native = format!(
                "SELECT ['340282366920938463463374607431768211455'::UHUGEINT]; INSERT INTO {quote}(id) VALUES(2010); SELECT * FROM range(6001);"
            );
            let status = run_file(&engine, &config, &path, native.as_bytes()).await;
            assert!(status.error.is_none(), "{:?}", status.error);
            assert_eq!(status.completed_statements, 3);
            assert_eq!(
                sql(
                    driver.as_ref(),
                    format!("SELECT id FROM {quote} WHERE id=2010")
                )
                .await
                .unwrap()[0][0]
                    .text(),
                "2010"
            );
            let failed = format!(
                "BEGIN; INSERT INTO {quote}(id) VALUES(2011); INSERT INTO {quote}(id) VALUES(0);"
            );
            let status = run_file(&engine, &config, &path, failed.as_bytes()).await;
            assert_eq!(status.completed_statements, 2);
            assert!(status.error.is_some());
            assert_eq!(status.transaction, Some(TransactionState::Failed));
            sql(driver.as_ref(), "ROLLBACK".into()).await.unwrap();
            assert!(
                sql(
                    driver.as_ref(),
                    format!("SELECT id FROM {quote} WHERE id=2011")
                )
                .await
                .unwrap()
                .is_empty()
            );

            // Cancellation releases a session held while the producer is still waiting for input.
            let (send, input) = mpsc::channel(1);
            let (output, mut results) = mpsc::channel(2);
            let drain = tokio::spawn(async move { while results.recv().await.is_some() {} });
            let cancel = CancellationToken::new();
            let completed = Arc::new(AtomicU64::new(0));
            let worker = driver.clone();
            let token = cancel.clone();
            let count = completed.clone();
            let task =
                tokio::spawn(
                    async move { worker.execute_script(input, output, token, count).await },
                );
            send.send(Ok(ScriptBatch::Statement("SELECT 42".into())))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while completed.load(Ordering::Relaxed) != 1 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            cancel.cancel();
            assert!(
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err()
                    .message
                    .contains("cancelled")
            );
            drop(send);
            drain.await.unwrap();
            assert_eq!(
                driver.transaction_state().await.unwrap(),
                TransactionState::Idle
            );
        }
        if dialect == "mssql" {
            // Native GO boundaries preserve variables within a batch and reset their scope afterwards.
            let native = format!(
                "DECLARE @n bigint=2010; INSERT INTO {quote} (id,label) VALUES(@n,N'雪;\nGO\nquoted');{boundary}SELECT TOP(6001) a.object_id FROM sys.all_objects a CROSS JOIN sys.all_objects b;{boundary}"
            );
            let status = run_file(&engine, &config, &path, native.as_bytes()).await;
            assert!(status.error.is_none(), "{:?}", status.error);
            assert_eq!(status.completed_statements, 2);
            assert_eq!(
                sql(
                    driver.as_ref(),
                    format!("SELECT label FROM {quote} WHERE id=2010")
                )
                .await
                .unwrap()[0][0]
                    .text(),
                "雪;\nGO\nquoted"
            );
            let id = engine
                .start(
                    config.id.clone(),
                    "DECLARE @n int=42; SELECT @n AS value;\nGO\nSELECT 43 AS next_value".into(),
                    100,
                    5,
                    true,
                )
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                while !engine.job(&id).unwrap().status().unwrap().done {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let job = engine.job(&id).unwrap();
            assert!(job.status().unwrap().error.is_none());
            assert_eq!(job.page(0, 0, 10).unwrap()[0][0].text(), "42");
            assert_eq!(job.page(1, 0, 10).unwrap()[0][0].text(), "43");
            engine.release(&id).unwrap();
            for invalid in [
                format!("INSERT INTO {quote}(id) VALUES(2011);{boundary}GO 2"),
                format!(
                    "INSERT INTO {quote}(id) VALUES(2011);{boundary}SET QUOTED_IDENTIFIER OFF;"
                ),
            ] {
                std::fs::write(&path, invalid).unwrap();
                assert!(
                    engine
                        .prepare_sql_import(path.clone(), &config.id)
                        .await
                        .is_err()
                );
            }
            assert!(
                sql(
                    driver.as_ref(),
                    format!("SELECT id FROM {quote} WHERE id=2011")
                )
                .await
                .unwrap()
                .is_empty()
            );
            sql(driver.as_ref(), "SET QUOTED_IDENTIFIER OFF".into())
                .await
                .unwrap();
            let status = run_file(&engine, &config, &path, b"SELECT 42").await;
            assert_eq!(status.completed_statements, 0);
            assert!(status.error.unwrap().contains("QUOTED_IDENTIFIER"));
            sql(driver.as_ref(), "SET QUOTED_IDENTIFIER ON".into())
                .await
                .unwrap();
            sql(driver.as_ref(), "SET IMPLICIT_TRANSACTIONS ON".into())
                .await
                .unwrap();
            let status = run_file(&engine, &config, &path, b"SELECT 42").await;
            assert!(status.error.is_none());
            assert_eq!(
                sql(driver.as_ref(), "SELECT @@TRANCOUNT".into())
                    .await
                    .unwrap()[0][0]
                    .text(),
                "0"
            );
            sql(driver.as_ref(), "SET IMPLICIT_TRANSACTIONS OFF".into())
                .await
                .unwrap();
        }

        // Close the writable file instance before reopening DuckDB in native read-only mode.
        sql(driver.as_ref(), format!("DROP TABLE {quote}"))
            .await
            .unwrap();
        engine.disconnect(&config.id).await.unwrap();
        let mut read_only = config.clone();
        read_only.id.clear();
        read_only.read_only = true;
        read_only.validate().unwrap();
        engine.store.save(&read_only).unwrap();
        engine
            .connect(&read_only.id, password, None, None)
            .await
            .unwrap();
        assert!(
            engine
                .prepare_sql_import(path.clone(), &read_only.id)
                .await
                .is_err()
        );
        engine.disconnect(&read_only.id).await.unwrap();
        eprintln!(
            "{name}: streamed SQL/export roundtrip, snapshot, partial error, transaction/serialization, deadline/cancel and read-only checks passed"
        );
    }
}
