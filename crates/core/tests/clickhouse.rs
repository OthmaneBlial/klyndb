use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{Cell, quote_clickhouse_identifier};
use std::time::Duration;

async fn query(engine: &Engine, config: &Connection, sql: &str) -> String {
    let id = engine
        .start(config.id.clone(), sql.into(), 10000, 5, true)
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
async fn clickhouse_saved_session_spool_export_isolation_and_reconnect() {
    let Ok(address) = std::env::var("KLYNDB_TEST_CLICKHOUSE_URL") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
    let mut config = Connection {
        id: String::new(),
        name: "ClickHouse core contract".into(),
        engine: "clickhouse".into(),
        address,
        environment: "development".into(),
        group: String::new(),
        color: "#79c7a4".into(),
        favorite: false,
        read_only: false,
        create_file: false,
    };
    engine
        .test_connection(config.clone(), None, None, None)
        .await
        .unwrap();
    assert!(engine.store.connections().unwrap().is_empty());
    config.validate().unwrap();
    engine.store.save(&config).unwrap();
    let caps = engine.connect(&config.id, None, None, None).await.unwrap();
    assert!(caps.table_browse && caps.cancel && caps.tls);
    assert!(!caps.affected_rows);
    assert!(!caps.transactions && !caps.edit_rows && !caps.import_rows && !caps.import_sql);
    let name = format!("klyndb_{}", uuid::Uuid::new_v4().simple());
    let copy = format!("{name}_copy\\`\"");
    let copy_quoted = quote_clickhouse_identifier(&copy);
    let writes = query(&engine,&config,&format!("CREATE TABLE `{name}`(id UInt64,label String,payload String,exact Decimal(38,18)) ENGINE=Memory; INSERT INTO `{name}` VALUES(18446744073709551615,'é;\\\'\\\\next',unhex('00ff'),12345678901234567890.123456789012345678)")).await;
    let written = engine.job(&writes).unwrap().status().unwrap();
    assert_eq!(written.sets.len(), 2);
    assert!(written.sets.iter().all(|set| set.affected.is_none()));
    let id = query(
        &engine,
        &config,
        &format!("SELECT * FROM `{name}`; SELECT 42 AS next_set"),
    )
    .await;
    let job = engine.job(&id).unwrap();
    assert_eq!(job.status().unwrap().sets.len(), 2);
    let expected = vec![
        Cell::Number("18446744073709551615".into()),
        Cell::Text("é;'\\next".into()),
        Cell::Binary("00ff".into()),
        Cell::Number("12345678901234567890.123456789012345678".into()),
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
    job.export(&mut sql, 0, "sql", &copy).unwrap();
    let sql = String::from_utf8(sql).unwrap();
    assert!(sql.contains("unhex('00ff')"));
    query(
        &engine,
        &config,
        &format!("CREATE TABLE {copy_quoted}(id UInt64,label String,payload String,exact Decimal(38,18)) ENGINE=Memory; {sql}"),
    )
    .await;
    let copied = query(&engine, &config, &format!("SELECT * FROM {copy_quoted}")).await;
    assert_eq!(
        engine.job(&copied).unwrap().page(0, 0, 100).unwrap(),
        vec![expected]
    );
    query(&engine, &config, "SET max_threads=1").await;
    engine
        .test_connection(config.clone(), None, None, None)
        .await
        .unwrap();
    let still = query(
        &engine,
        &config,
        "SELECT value FROM system.settings WHERE name='max_threads'",
    )
    .await;
    assert_eq!(
        engine.job(&still).unwrap().page(0, 0, 100).unwrap()[0][0].text(),
        "1"
    );
    engine
        .reconnect(&config.id, None, None, None, true)
        .await
        .unwrap();
    let fresh = query(
        &engine,
        &config,
        "SELECT value FROM system.settings WHERE name='max_threads'",
    )
    .await;
    assert_ne!(
        engine.job(&fresh).unwrap().page(0, 0, 100).unwrap()[0][0].text(),
        "1"
    );
    let mut retained = vec![];
    assert_eq!(job.export(&mut retained, 1, "csv", "").unwrap(), 1);
    assert_eq!(retained, b"next_set\n42\n");
    query(
        &engine,
        &config,
        &format!("DROP TABLE {copy_quoted}; DROP TABLE `{name}`"),
    )
    .await;
    engine.disconnect(&config.id).await.unwrap();
    config.read_only = true;
    engine.store.save(&config).unwrap();
    engine.connect(&config.id, None, None, None).await.unwrap();
    assert!(
        engine
            .start(
                config.id.clone(),
                "DELETE FROM missing".into(),
                100,
                5,
                true
            )
            .await
            .is_err()
    );
    engine.disconnect(&config.id).await.unwrap();
}
