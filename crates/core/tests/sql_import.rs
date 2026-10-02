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
    let mut fixtures = vec![(
        "sqlite",
        "sqlite",
        directory
            .path()
            .join("sql.db")
            .to_string_lossy()
            .into_owned(),
    )];
    for (name, engine, variable) in [
        ("postgres", "postgres", "KLYNDB_TEST_POSTGRES_URL"),
        ("mysql", "mysql", "KLYNDB_TEST_MYSQL_URL"),
        ("mariadb", "mysql", "KLYNDB_TEST_MARIADB_URL"),
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
            create_file: dialect == "sqlite",
        };
        config.validate().unwrap();
        engine.store.save(&config).unwrap();
        engine.connect(&config.id, None, None, None).await.unwrap();
        let driver = engine.driver(&config.id).await.unwrap();
        assert!(driver.capabilities().import_sql);
        let table = format!("sql_import_{}", uuid::Uuid::new_v4().simple());
        let quote = driver.quote_identifier(&table);
        let binary = if dialect == "postgres" {
            "BYTEA"
        } else {
            "BLOB"
        };
        let mut file = format!("\u{feff}-- SQL file; whole-file preflight\nCREATE TABLE {quote} (id BIGINT PRIMARY KEY, label TEXT, precise TEXT, payload {binary});\n").into_bytes();
        let label = format!("  'quoted';\\ line\nnext é {}  ", "x".repeat(3600));
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
        let binary_sql = if dialect == "postgres" {
            "encode(payload, 'hex')"
        } else {
            "hex(payload)"
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
            "INSERT INTO {quote} (id) VALUES (2002); INSERT INTO {quote} (id) VALUES (0); INSERT INTO {quote} (id) VALUES (2003);"
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

        let manual = format!("BEGIN; INSERT INTO {quote} (id) VALUES (2004);");
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
        send.send(Ok(ScriptBatch::Statement("BEGIN".into())))
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
        assert!(status.error.unwrap().contains("timed out"), "{name}");
        engine.imports.release(&id).unwrap();
        assert_eq!(
            sql(driver.as_ref(), "SELECT 42".into()).await.unwrap()[0][0].text(),
            "42"
        );

        let mut read_only = config.clone();
        read_only.id.clear();
        read_only.read_only = true;
        read_only.validate().unwrap();
        engine.store.save(&read_only).unwrap();
        engine
            .connect(&read_only.id, None, None, None)
            .await
            .unwrap();
        assert!(
            engine
                .prepare_sql_import(path.clone(), &read_only.id)
                .await
                .is_err()
        );
        engine.disconnect(&read_only.id).await.unwrap();
        sql(driver.as_ref(), format!("DROP TABLE {quote}"))
            .await
            .unwrap();
        engine.disconnect(&config.id).await.unwrap();
        eprintln!(
            "{name}: streamed SQL/export roundtrip, snapshot, partial error, transaction/serialization, deadline/cancel and read-only checks passed"
        );
    }
}
