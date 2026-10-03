use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_core::import::{ImportRequest, ImportStatus};
use klyndb_driver_api::{Cell, Change, TransactionState};
use klyndb_import::{ImportFormat, ImportOptions, Mapping, ValueKind};
use std::collections::BTreeMap;
use std::time::Duration;

async fn imported(engine: &Engine, id: &str) -> ImportStatus {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let status = engine.imports.status(id).unwrap();
            if status.done {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn sql_server_csv_json_import_jobs_rollback_and_deadline() {
    let Ok(address) = std::env::var("KLYNDB_TEST_MSSQL_URL") else {
        return;
    };
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&dir.path().join("imports.db")).unwrap());
    let mut config = Connection {
        id: String::new(),
        name: "SQL Server import contract".into(),
        engine: "mssql".into(),
        address,
        environment: "production".into(),
        group: String::new(),
        color: "#79c7a4".into(),
        favorite: false,
        read_only: false,
        create_file: false,
    };
    config.validate().unwrap();
    engine.store.save(&config).unwrap();
    assert!(
        engine
            .connect(&config.id, Some(password), None, None)
            .await
            .unwrap()
            .import_rows
    );
    let name = format!("klyndb_file_import_{}", uuid::Uuid::new_v4().simple());
    let table = klyndb_driver_api::Table {
        schema: "dbo".into(),
        name: name.clone(),
        kind: "table".into(),
    };
    query(&engine,&config,&format!("CREATE TABLE dbo.[{name}](id bigint PRIMARY KEY,label nvarchar(100),amount decimal(38,18),payload varbinary(32),flag bit,seq int IDENTITY)" )).await;
    let file = dir.path().join("rows.csv");
    let csv = |offset: i64| {
        let mut csv = "id,label,amount,payload,flag\n".to_owned();
        for i in 1..=600 {
            csv.push_str(&format!(
                "{},\"{}\",12345678901234567890.123456789012345678,00ff,true\n",
                offset + i,
                if i == 1 {
                    "\\N"
                } else {
                    "é雪, \"\"quoted\"\"\nnext"
                }
            ));
        }
        csv
    };
    let options = ImportOptions {
        null_value: Some("\\N".into()),
        ..Default::default()
    };
    let mapping = |headers: &[String]| {
        headers
            .iter()
            .map(|name| Mapping {
                column: Some(name.clone()),
                kind: match name.as_str() {
                    "id" | "amount" => ValueKind::Number,
                    "payload" => ValueKind::Binary,
                    "flag" => ValueKind::Boolean,
                    _ => ValueKind::Text,
                },
            })
            .collect::<Vec<_>>()
    };
    let request = |source: String,
                   options: ImportOptions,
                   mapping: Vec<Mapping>,
                   confirmed: bool,
                   timeout_seconds: u64| ImportRequest {
        source,
        connection: config.id.clone(),
        table: table.clone(),
        options,
        mapping,
        confirmed,
        timeout_seconds,
    };
    std::fs::write(&file, csv(0)).unwrap();
    let source = engine
        .imports
        .prepare(file.clone(), options.clone())
        .await
        .unwrap();
    assert_eq!(source.preview.rows[0][1], Some("\\N".into())); // CSV preview shows the raw token; mapping applies NULL semantics.
    let mapped = mapping(&source.preview.headers);
    assert!(
        engine
            .start_import(request(
                source.id.clone(),
                options.clone(),
                mapped.clone(),
                false,
                60
            ))
            .await
            .unwrap_err()
            .message
            .contains("Confirmation")
    );
    let mut generated = mapped.clone();
    generated[0].column = Some("seq".into());
    assert!(
        engine
            .start_import(request(
                source.id.clone(),
                options.clone(),
                generated,
                true,
                60
            ))
            .await
            .is_err()
    );
    // Preparing captures a private immutable file; replacing the selected file cannot change execution.
    std::fs::write(&file, b"changed after selection").unwrap();
    let id = engine
        .start_import(request(
            source.id.clone(),
            options.clone(),
            mapped.clone(),
            true,
            60,
        ))
        .await
        .unwrap();
    let status = imported(&engine, &id).await;
    assert!(status.error.is_none(), "{:?}", status.error);
    let result = status.result.unwrap();
    assert!(result.affected == 600 && !result.pending_transaction);
    assert_eq!(status.transaction, Some(TransactionState::Idle));
    let selected = format!("SELECT id,label,amount,payload,flag FROM dbo.[{name}] ORDER BY id");
    let original = engine
        .job(&query(&engine, &config, &selected).await)
        .unwrap();
    let expected = original.page(0, 0, 10).unwrap();
    assert_eq!(
        &expected[0],
        &vec![
            Cell::Number("1".into()),
            Cell::Null,
            Cell::Number("12345678901234567890.123456789012345678".into()),
            Cell::Binary("00ff".into()),
            Cell::Boolean(true)
        ]
    );
    engine.imports.release(&id).unwrap();
    let count = |job: &str| engine.job(job).unwrap().page(0, 0, 10).unwrap()[0][0].text();
    let count_sql = format!("SELECT COUNT_BIG(*) FROM dbo.[{name}]");
    assert_eq!(count(&query(&engine, &config, &count_sql).await), "600");
    std::fs::write(&file, format!("{}broken-width\n", csv(1000))).unwrap();
    let source = engine
        .imports
        .prepare(file.clone(), options.clone())
        .await
        .unwrap();
    let id = engine
        .start_import(request(
            source.id,
            options.clone(),
            mapped.clone(),
            true,
            60,
        ))
        .await
        .unwrap();
    let status = imported(&engine, &id).await;
    assert!(status.error.is_some() && status.read_rows >= 256);
    assert_eq!(status.transaction, Some(TransactionState::Idle));
    assert_eq!(count(&query(&engine, &config, &count_sql).await), "600");
    engine.imports.release(&id).unwrap();
    let mut json = vec![];
    original.export(&mut json, 0, "json", "").unwrap();
    let typed = dir.path().join("native.json");
    std::fs::write(&typed, json).unwrap();
    let typed_options = ImportOptions {
        format: ImportFormat::KlyndbJson,
        ..Default::default()
    };
    let source = engine
        .imports
        .prepare(typed, typed_options.clone())
        .await
        .unwrap();
    let typed_mapping = mapping(&source.preview.headers);
    query(
        &engine,
        &config,
        &format!("BEGIN TRANSACTION; DELETE FROM dbo.[{name}] WHERE id>0"),
    )
    .await;
    let id = engine
        .start_import(request(source.id, typed_options, typed_mapping, true, 60))
        .await
        .unwrap();
    let status = imported(&engine, &id).await;
    assert!(status.error.is_none(), "{:?}", status.error);
    let result = status.result.unwrap();
    assert!(result.pending_transaction && result.affected == 600);
    assert_eq!(status.transaction, Some(TransactionState::Active));
    let roundtrip = engine
        .job(&query(&engine, &config, &selected).await)
        .unwrap();
    assert_eq!(roundtrip.page(0, 0, 10).unwrap(), expected);
    query(&engine, &config, "ROLLBACK").await;
    assert_eq!(count(&query(&engine, &config, &count_sql).await), "600");
    engine.imports.release(&id).unwrap();
    let standard = dir.path().join("standard.json");
    std::fs::write(&standard,r#"[{"id":9223372036854775807,"label":"é雪","amount":12345678901234567890.123456789012345678,"payload":"00ff","flag":true},{"id":-1,"label":null,"amount":null,"payload":null,"flag":false},{"id":0,"label":"","amount":0.000000000000000001,"payload":"","flag":null}]"#).unwrap();
    let json_options = ImportOptions {
        format: ImportFormat::Json,
        ..Default::default()
    };
    let source = engine
        .imports
        .prepare(standard, json_options.clone())
        .await
        .unwrap();
    let json_mapping = mapping(&source.preview.headers);
    let id = engine
        .start_import(request(source.id, json_options, json_mapping, true, 60))
        .await
        .unwrap();
    let status = imported(&engine, &id).await;
    assert!(status.error.is_none(), "{:?}", status.error);
    assert_eq!(status.result.unwrap().affected, 3);
    let maximum=engine.job(&query(&engine,&config,&format!("SELECT id,label,amount,payload,flag FROM dbo.[{name}] WHERE id=9223372036854775807")).await).unwrap();
    assert_eq!(
        maximum.page(0, 0, 10).unwrap()[0],
        vec![
            Cell::Number("9223372036854775807".into()),
            Cell::Text("é雪".into()),
            Cell::Number("12345678901234567890.123456789012345678".into()),
            Cell::Binary("00ff".into()),
            Cell::Boolean(true)
        ]
    );
    assert_eq!(count(&query(&engine, &config, &count_sql).await), "603");
    engine.imports.release(&id).unwrap();
    query(&engine,&config,&format!("EXEC(N'CREATE TRIGGER dbo.[tr_{name}] ON dbo.[{name}] AFTER INSERT AS BEGIN IF EXISTS(SELECT 1 FROM inserted WHERE id=700) WAITFOR DELAY ''00:02:00''; END')")).await;
    std::fs::write(
        &file,
        "id,label,amount,payload,flag\n701,first,1,00ff,true\n700,slow,1,00ff,true\n",
    )
    .unwrap();
    let source = engine.imports.prepare(file, options.clone()).await.unwrap();
    let id = engine
        .start_import(request(source.id, options, mapped, true, 1))
        .await
        .unwrap();
    let status = imported(&engine, &id).await;
    assert!(
        status.error.as_ref().unwrap().contains("timed out"),
        "{:?}",
        status.error
    );
    assert_eq!(status.transaction, Some(TransactionState::Idle));
    assert_eq!(count(&query(&engine, &config, &count_sql).await), "603");
    engine.imports.release(&id).unwrap();
    query(&engine, &config, &format!("DROP TABLE dbo.[{name}]")).await;
    engine.disconnect(&config.id).await.unwrap();
}

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
    assert!(caps.import_rows && !caps.affected_rows && !caps.import_sql);
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
