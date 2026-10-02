use klyndb_connections::{Connection, Store};
use klyndb_core::{Engine, QueryStatus, import::SqlImportRequest};
use klyndb_driver_api::TransactionState;
use std::{sync::Arc, time::Duration};

async fn done(engine: &Engine, id: &str) -> QueryStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = engine.job(id).unwrap().status().unwrap();
            if status.done {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("query cleanup completed")
}
async fn query(engine: &Engine, config: &Connection, sql: String) -> String {
    let id = engine
        .start(config.id.clone(), sql, 100, 60, true)
        .await
        .unwrap();
    let status = done(engine, &id).await;
    assert!(
        status.error.is_none(),
        "{}: {:?}",
        config.name,
        status.error
    );
    id
}
async fn value(engine: &Engine, config: &Connection, sql: String) -> String {
    let id = query(engine, config, sql).await;
    let rows = engine.job(&id).unwrap().page(0, 0, 100).unwrap();
    engine.release(&id).unwrap();
    rows[0][0].text()
}

#[tokio::test]
async fn reconnect_real_engines_resets_session_without_replaying_work() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixtures = vec![(
        "sqlite",
        "sqlite",
        directory
            .path()
            .join("data.db")
            .to_string_lossy()
            .into_owned(),
    )];
    for (name, dialect, variable) in [
        ("postgres", "postgres", "KLYNDB_TEST_POSTGRES_URL"),
        ("mysql", "mysql", "KLYNDB_TEST_MYSQL_URL"),
        ("mariadb", "mysql", "KLYNDB_TEST_MARIADB_URL"),
    ] {
        if let Ok(address) = std::env::var(variable) {
            fixtures.push((name, dialect, address));
        }
    }
    for (name, dialect, address) in fixtures {
        let engine =
            Engine::new(Store::open(&directory.path().join(format!("{name}-state.db"))).unwrap());
        let mut config = Connection {
            id: String::new(),
            name: format!("Reconnect {name}"),
            engine: dialect.into(),
            address,
            environment: "production".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: dialect == "sqlite",
        };
        let password = config
            .validate()
            .unwrap()
            .map(|p| p.to_string())
            .unwrap_or_default();
        engine.store.save(&config).unwrap();
        engine
            .connect(&config.id, Some(password.clone()), None, None)
            .await
            .unwrap();
        let before = engine.driver(&config.id).await.unwrap();
        let table =
            before.quote_identifier(&format!("reconnect_{}", uuid::Uuid::new_v4().simple()));
        let temp =
            before.quote_identifier(&format!("reconnect_temp_{}", uuid::Uuid::new_v4().simple()));
        let create = query(&engine, &config, format!("CREATE TABLE {table} (id INT PRIMARY KEY, label TEXT); INSERT INTO {table} VALUES (1,'original'); CREATE TEMPORARY TABLE {temp} (id INT)")).await;
        engine.release(&create).unwrap();
        let snapshot = query(&engine, &config, format!("SELECT label FROM {table}")).await;
        let update = query(
            &engine,
            &config,
            format!("BEGIN; UPDATE {table} SET label='uncommitted' WHERE id=1"),
        )
        .await;
        engine.release(&update).unwrap();
        assert_eq!(
            before.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        let refused = engine
            .reconnect(&config.id, Some(password.clone()), None, None, false)
            .await
            .unwrap_err();
        assert!(refused.message.contains("Confirmation required"));
        assert!(Arc::ptr_eq(
            &before,
            &engine.driver(&config.id).await.unwrap()
        ));
        assert_eq!(
            value(&engine, &config, format!("SELECT label FROM {table}")).await,
            "uncommitted"
        );

        let capabilities = engine
            .reconnect(&config.id, Some(password.clone()), None, None, true)
            .await
            .unwrap();
        assert!(capabilities.import_sql);
        let after = engine.driver(&config.id).await.unwrap();
        assert!(!Arc::ptr_eq(&before, &after));
        assert!(
            before.transaction_state().await.is_err(),
            "{name}: previous session closed"
        );
        assert_eq!(
            after.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        assert_eq!(
            value(&engine, &config, format!("SELECT label FROM {table}")).await,
            "original"
        );
        let absent = engine
            .start(
                config.id.clone(),
                format!("SELECT * FROM {temp}"),
                10,
                10,
                true,
            )
            .await
            .unwrap();
        assert!(
            done(&engine, &absent).await.error.is_some(),
            "{name}: temporary table reset"
        );
        engine.release(&absent).unwrap();
        let mut exported = vec![];
        assert_eq!(
            engine
                .job(&snapshot)
                .unwrap()
                .export(&mut exported, 0, "csv", "unused")
                .unwrap(),
            1
        );
        assert!(String::from_utf8(exported).unwrap().contains("original"));
        engine.release(&snapshot).unwrap();

        // An immediate reconnect must see a query registered before its worker starts.
        let slow = match dialect {
            "postgres" => "SELECT pg_sleep(30)",
            "mysql" => "SELECT SLEEP(30)",
            _ => {
                "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT SUM(x) FROM n"
            }
        };
        let running = engine
            .start(config.id.clone(), slow.into(), 10, 60, true)
            .await
            .unwrap();
        engine
            .reconnect(&config.id, Some(password.clone()), None, None, true)
            .await
            .unwrap();
        assert!(
            done(&engine, &running).await.error.is_some(),
            "{name}: old query cancelled"
        );
        engine.release(&running).unwrap();
        assert_eq!(value(&engine, &config, "SELECT 42".into()).await, "42");

        // Reconnect awaits SQL reader/native cleanup and rolls back this caller-owned transaction.
        let path = directory.path().join(format!("{name}-reconnect.sql"));
        std::fs::write(
            &path,
            format!("BEGIN; INSERT INTO {table} VALUES (2,'pending import'); {slow};"),
        )
        .unwrap();
        let source = engine.prepare_sql_import(path, &config.id).await.unwrap();
        let importing = engine
            .start_sql_import(SqlImportRequest {
                source: source.id,
                connection: config.id.clone(),
                timeout_seconds: 60,
                confirmed: true,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while engine
                .imports
                .status(&importing)
                .unwrap()
                .completed_statements
                < 2
            {
                assert!(!engine.imports.status(&importing).unwrap().done);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        engine
            .reconnect(&config.id, Some(password.clone()), None, None, true)
            .await
            .unwrap();
        let imported = engine.imports.status(&importing).unwrap();
        assert!(
            imported.done && imported.error.is_some(),
            "{name}: native import cleanup finished"
        );
        engine.imports.release(&importing).unwrap();
        assert_eq!(
            value(
                &engine,
                &config,
                format!("SELECT COUNT(*) FROM {table} WHERE id=2")
            )
            .await,
            "0"
        );

        // A failed replacement leaves the saved connection disconnected, with no silent replay.
        let mut broken = config.clone();
        if dialect == "sqlite" {
            broken.address = directory
                .path()
                .join("missing.db")
                .to_string_lossy()
                .into_owned();
            broken.create_file = false;
        } else {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut url = url::Url::parse(&config.address).unwrap();
            url.set_port(Some(listener.local_addr().unwrap().port()))
                .unwrap();
            broken.address = url.to_string();
            drop(listener);
        }
        engine.store.save(&broken).unwrap();
        assert!(
            engine
                .reconnect(&config.id, Some(password.clone()), None, None, true)
                .await
                .is_err()
        );
        assert!(engine.driver(&config.id).await.is_err());
        engine.store.save(&config).unwrap();
        engine
            .connect(&config.id, Some(password), None, None)
            .await
            .unwrap();
        assert_eq!(
            value(&engine, &config, format!("SELECT label FROM {table}")).await,
            "original"
        );
        let cleanup = query(&engine, &config, format!("DROP TABLE {table}")).await;
        engine.release(&cleanup).unwrap();
        engine.disconnect(&config.id).await.unwrap();
        eprintln!(
            "{name}: confirmation, native reset/rollback, retained export, query/import cleanup and failed reconnect recovery passed"
        );
    }
}
