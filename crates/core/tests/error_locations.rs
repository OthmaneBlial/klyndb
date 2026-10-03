use klyndb_connections::{Connection, Store};
use klyndb_core::{Engine, QueryStatus};
use klyndb_driver_api::sql_utf16_offset;
use std::time::Duration;

async fn finished(engine: &Engine, id: &str) -> QueryStatus {
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
    .unwrap()
}
async fn run(engine: &Engine, connection: &str, sql: &str) -> QueryStatus {
    let id = engine
        .start(connection.into(), sql.into(), 10, 5, true)
        .await
        .unwrap();
    let status = finished(engine, &id).await;
    engine.release(&id).unwrap();
    status
}

#[tokio::test]
async fn native_query_error_offsets_unicode_batches_and_reuse() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.sqlite")).unwrap());
    let mut servers = vec![(
        "sqlite",
        directory
            .path()
            .join("database.sqlite")
            .to_string_lossy()
            .into_owned(),
    )];
    if let Ok(url) = std::env::var("KLYNDB_TEST_POSTGRES_URL") {
        servers.push(("postgres", url));
    }
    for (kind, address) in servers {
        let mut connection = Connection {
            id: String::new(),
            name: "Error location fixture".into(),
            engine: kind.into(),
            address,
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: kind == "sqlite",
        };
        let password = connection
            .validate()
            .unwrap()
            .map(|value| value.to_string());
        engine.store.save(&connection).unwrap();
        engine
            .connect(&connection.id, password, None, None)
            .await
            .unwrap();
        if kind == "postgres" {
            let id = engine
                .start(
                    connection.id.clone(),
                    "SHOW server_encoding".into(),
                    10,
                    5,
                    true,
                )
                .await
                .unwrap();
            assert!(finished(&engine, &id).await.error.is_none());
            eprintln!(
                "Error location fixture server encoding: {}",
                engine.job(&id).unwrap().page(0, 0, 1).unwrap()[0][0].text()
            );
            engine.release(&id).unwrap();
        }
        for sql in [
            "SELECT missing_sql_column",
            "SELECT 'é😀' AS first;\nSELECT missing_sql_column AS second",
        ] {
            let result = run(&engine, &connection.id, sql).await;
            if sql.contains('😀')
                && let Ok(directory) = std::env::var("KLYNDB_ERROR_LOCATION_EVIDENCE_DIR")
            {
                let path = std::path::Path::new(&directory);
                std::fs::create_dir_all(path).unwrap();
                std::fs::write(
                    path.join(format!("{kind}.json")),
                    serde_json::to_vec_pretty(&serde_json::json!({"query":sql,"status":result}))
                        .unwrap(),
                )
                .unwrap();
            }
            assert!(
                result
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("missing_sql_column")
            );
            assert_eq!(
                result.error_offset,
                sql_utf16_offset(sql, sql.find("missing_sql_column").unwrap()),
                "{kind}: {:?}",
                result.error
            );
        }
        let invalid = "SELECT 'é😀';\nSELECT (1 + );";
        let error = engine
            .start(connection.id.clone(), invalid.into(), 10, 5, true)
            .await
            .unwrap_err();
        assert_eq!(
            error.sql_offset,
            sql_utf16_offset(invalid, invalid.find(')').unwrap())
        );
        let table = format!("klyndb_error_{}", uuid::Uuid::new_v4().simple());
        assert!(
            run(
                &engine,
                &connection.id,
                &format!(
                    "CREATE TABLE {table}(id INTEGER PRIMARY KEY); INSERT INTO {table} VALUES(1)"
                )
            )
            .await
            .error
            .is_none()
        );
        let constraint = run(
            &engine,
            &connection.id,
            &format!("INSERT INTO {table} VALUES(1)"),
        )
        .await;
        assert!(constraint.error.is_some());
        assert!(constraint.error_offset.is_none());
        if kind == "postgres" {
            let function = format!("klyndb_error_fn_{}", uuid::Uuid::new_v4().simple());
            assert!(run(&engine,&connection.id,&format!("CREATE FUNCTION {function}() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN EXECUTE 'SELECT missing_internal_column'; RETURN 1; END $$")).await.error.is_none());
            let internal = run(&engine, &connection.id, &format!("SELECT {function}()")).await;
            assert!(
                internal
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("missing_internal_column")
            );
            assert!(internal.error_offset.is_none());
            assert!(
                run(
                    &engine,
                    &connection.id,
                    &format!("DROP FUNCTION {function}()")
                )
                .await
                .error
                .is_none()
            );
        }
        assert!(
            run(&engine, &connection.id, "SELECT 42 AS reused")
                .await
                .error
                .is_none()
        );
        assert!(
            run(&engine, &connection.id, &format!("DROP TABLE {table}"))
                .await
                .error
                .is_none()
        );
        engine.disconnect(&connection.id).await.unwrap();
    }
}
