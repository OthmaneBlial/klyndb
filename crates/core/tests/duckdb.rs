use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{Cell, TransactionState};
use std::time::Duration;

async fn query(engine: &Engine, connection: &Connection, sql: &str) -> String {
    let id = engine
        .start(connection.id.clone(), sql.into(), 10000, 10, true)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = engine.job(&id).unwrap().status().unwrap();
            if status.done {
                assert!(status.error.is_none(), "{:?}", status.error);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    id
}
#[tokio::test]
async fn duckdb_saved_session_spool_exports_test_isolation_and_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
    let mut config = Connection {
        id: String::new(),
        name: "DuckDB core contract".into(),
        engine: "duckdb".into(),
        address: dir
            .path()
            .join("data.duckdb")
            .to_string_lossy()
            .into_owned(),
        environment: "development".into(),
        group: String::new(),
        color: "#79c7a4".into(),
        favorite: false,
        read_only: false,
        create_file: true,
    };
    assert!(
        engine
            .test_connection(config.clone(), None, None, None)
            .await
            .is_err()
    );
    assert!(!std::path::Path::new(&config.address).exists());
    assert!(engine.store.connections().unwrap().is_empty());
    config.validate().unwrap();
    engine.store.save(&config).unwrap();
    let caps = engine.connect(&config.id, None, None, None).await.unwrap();
    assert!(caps.table_browse && caps.cancel && caps.transactions && caps.affected_rows);
    assert!(caps.import_sql);
    assert!(caps.edit_rows && caps.import_rows);
    let writes = query(&engine,&config,"CREATE TABLE t(id UBIGINT PRIMARY KEY, label VARCHAR, exact DECIMAL(38,18), payload BLOB); INSERT INTO t VALUES(18446744073709551615,'é;''\\next',12345678901234567890.123456789012345678,from_hex('00ff'))").await;
    assert_eq!(
        engine.job(&writes).unwrap().status().unwrap().sets[1].affected,
        Some(1)
    );
    let id = query(
        &engine,
        &config,
        "SELECT id,label,exact,payload FROM t; SELECT 42 AS next_set",
    )
    .await;
    let job = engine.job(&id).unwrap();
    assert_eq!(job.status().unwrap().sets.len(), 2);
    let expected = vec![
        Cell::Number("18446744073709551615".into()),
        Cell::Text("é;'\\next".into()),
        Cell::Number("12345678901234567890.123456789012345678".into()),
        Cell::Binary("00ff".into()),
    ];
    assert_eq!(job.page(0, 0, 100).unwrap(), vec![expected.clone()]);
    let mut csv = vec![];
    assert_eq!(job.export(&mut csv, 0, "csv", "").unwrap(), 1);
    assert!(
        String::from_utf8(csv)
            .unwrap()
            .contains("18446744073709551615")
    );
    let mut sql = vec![];
    job.export(&mut sql, 0, "sql", "copy").unwrap();
    let sql = String::from_utf8(sql).unwrap();
    assert!(sql.contains("from_hex('00ff')"));
    query(&engine,&config,&format!("CREATE TABLE copy(id UBIGINT, label VARCHAR, exact DECIMAL(38,18), payload BLOB); {sql}")).await;
    let copy = query(&engine, &config, "SELECT * FROM copy").await;
    assert_eq!(
        engine.job(&copy).unwrap().page(0, 0, 100).unwrap(),
        vec![expected]
    );
    query(
        &engine,
        &config,
        "BEGIN; UPDATE t SET label='uncommitted' WHERE id=18446744073709551615",
    )
    .await;
    let driver = engine.driver(&config.id).await.unwrap();
    assert_eq!(
        driver.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    // A read-only probe cannot change the shared database instance's access mode.
    // The rejected test must preserve the existing user transaction.
    assert!(
        engine
            .test_connection(config.clone(), None, None, None)
            .await
            .is_err()
    );
    assert_eq!(
        driver.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    assert!(
        engine
            .reconnect(&config.id, None, None, None, false)
            .await
            .is_err()
    );
    engine
        .reconnect(&config.id, None, None, None, true)
        .await
        .unwrap();
    assert!(driver.transaction_state().await.is_err());
    let fresh = query(&engine, &config, "SELECT label FROM t").await;
    assert_eq!(
        engine.job(&fresh).unwrap().page(0, 0, 100).unwrap(),
        vec![vec![Cell::Text("é;'\\next".into())]]
    );
    let mut retained = vec![];
    assert_eq!(job.export(&mut retained, 1, "csv", "").unwrap(), 1);
    assert_eq!(retained, b"next_set\n42\n");
    engine.disconnect(&config.id).await.unwrap();
    assert!(
        engine
            .test_connection(config.clone(), None, None, None)
            .await
            .is_ok()
    );
    config.read_only = true;
    engine.store.save(&config).unwrap();
    engine.connect(&config.id, None, None, None).await.unwrap();
    assert!(
        engine
            .start(config.id.clone(), "DELETE FROM t".into(), 100, 5, true)
            .await
            .is_err()
    );
    engine.disconnect(&config.id).await.unwrap();
}
