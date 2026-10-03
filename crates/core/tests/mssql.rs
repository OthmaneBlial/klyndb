use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{Cell, Change, TransactionState};
use std::collections::BTreeMap;
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
async fn sql_server_saved_session_spool_export_and_reconnect() {
    let Ok(address) = std::env::var("KLYNDB_TEST_MSSQL_URL") else {
        return;
    };
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
    let mut config = Connection {
        id: String::new(),
        name: "SQL Server core contract".into(),
        engine: "mssql".into(),
        address,
        environment: "production".into(),
        group: String::new(),
        color: "#79c7a4".into(),
        favorite: false,
        read_only: false,
        create_file: false,
    };
    engine
        .test_connection(config.clone(), Some(password.clone()), None, None)
        .await
        .unwrap();
    assert!(engine.store.connections().unwrap().is_empty());
    config.validate().unwrap();
    engine.store.save(&config).unwrap();
    let caps = engine
        .connect(&config.id, Some(password.clone()), None, None)
        .await
        .unwrap();
    assert!(caps.transactions && caps.table_browse && caps.cancel && caps.tls && caps.edit_rows);
    assert!(!caps.affected_rows && !caps.import_rows && !caps.import_sql);
    let name = format!("klyndb_{}", uuid::Uuid::new_v4().simple());
    let copy = format!("{name}_copy]雪");
    let quoted = format!("[{}]", copy.replace(']', "]]"));
    let writes=query(&engine,&config,&format!("CREATE TABLE dbo.[{name}](id bigint PRIMARY KEY,label nvarchar(100),payload varbinary(100),exact decimal(38,18),flag bit); INSERT INTO dbo.[{name}] VALUES(9223372036854775807,N'é;''雪',0x00ff,12345678901234567890.123456789012345678,1)")).await;
    assert!(
        engine
            .job(&writes)
            .unwrap()
            .status()
            .unwrap()
            .sets
            .iter()
            .all(|set| set.affected.is_none())
    );
    let id = query(
        &engine,
        &config,
        &format!("DECLARE @n int=42; SELECT * FROM dbo.[{name}]; SELECT @n AS answer"),
    )
    .await;
    let job = engine.job(&id).unwrap();
    assert_eq!(job.status().unwrap().sets.len(), 2);
    let expected = vec![
        Cell::Number("9223372036854775807".into()),
        Cell::Text("é;'雪".into()),
        Cell::Binary("00ff".into()),
        Cell::Number("12345678901234567890.123456789012345678".into()),
        Cell::Boolean(true),
    ];
    assert_eq!(job.page(0, 0, 100).unwrap(), vec![expected.clone()]);
    let driver = engine.driver(&config.id).await.unwrap();
    let table = driver
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
    assert!(driver.inspect(&table).await.unwrap().editable);
    let update = Change::Update {
        old: expected.clone(),
        values: BTreeMap::from([("label".into(), Cell::Text("reviewed".into()))]),
    };
    assert!(
        engine
            .apply_changes(&config.id, table.clone(), vec![update.clone()], false)
            .await
            .unwrap_err()
            .message
            .contains("Confirmation")
    );
    let applied = engine
        .apply_changes(&config.id, table.clone(), vec![update], true)
        .await
        .unwrap();
    assert_eq!(applied.affected, 1);
    assert!(!applied.pending_transaction);
    let mut edited = expected.clone();
    edited[1] = Cell::Text("reviewed".into());
    engine
        .apply_changes(
            &config.id,
            table,
            vec![Change::Update {
                old: edited,
                values: BTreeMap::from([("label".into(), expected[1].clone())]),
            }],
            true,
        )
        .await
        .unwrap();
    let mut csv = vec![];
    assert_eq!(job.export(&mut csv, 0, "csv", "").unwrap(), 1);
    assert!(
        String::from_utf8(csv)
            .unwrap()
            .contains("9223372036854775807")
    );
    let mut sql = vec![];
    job.export(&mut sql, 0, "sql", &copy).unwrap();
    let sql = String::from_utf8(sql).unwrap();
    assert!(sql.contains("0x00ff") && sql.contains("N'é;''雪'"));
    query(&engine,&config,&format!("CREATE TABLE {quoted}(id bigint,label nvarchar(100),payload varbinary(100),exact decimal(38,18),flag bit); {sql}")).await;
    let copied = query(&engine, &config, &format!("SELECT * FROM {quoted}")).await;
    assert_eq!(
        engine.job(&copied).unwrap().page(0, 0, 100).unwrap(),
        vec![expected]
    );
    query(
        &engine,
        &config,
        "BEGIN TRANSACTION; CREATE TABLE #temporary(id int)",
    )
    .await;
    assert_eq!(
        engine
            .driver(&config.id)
            .await
            .unwrap()
            .transaction_state()
            .await
            .unwrap(),
        TransactionState::Active
    );
    engine
        .test_connection(config.clone(), Some(password.clone()), None, None)
        .await
        .unwrap();
    assert_eq!(
        engine
            .driver(&config.id)
            .await
            .unwrap()
            .transaction_state()
            .await
            .unwrap(),
        TransactionState::Active
    );
    engine
        .reconnect(&config.id, Some(password.clone()), None, None, true)
        .await
        .unwrap();
    assert_eq!(
        engine
            .driver(&config.id)
            .await
            .unwrap()
            .transaction_state()
            .await
            .unwrap(),
        TransactionState::Idle
    );
    let temporary = query(
        &engine,
        &config,
        "SELECT OBJECT_ID('tempdb..#temporary') AS removed",
    )
    .await;
    assert_eq!(
        engine.job(&temporary).unwrap().page(0, 0, 100).unwrap()[0][0],
        Cell::Null
    );
    assert_eq!(job.page(1, 0, 100).unwrap()[0][0].text(), "42");
    query(
        &engine,
        &config,
        &format!("DROP TABLE dbo.[{name}]; DROP TABLE {quoted}"),
    )
    .await;
    engine.disconnect(&config.id).await.unwrap();
}
