use klyndb_connections::{Connection, Store};
use klyndb_core::{Engine, Job};
use klyndb_driver_api::{PlanFormat, Row};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

async fn completed(engine: &Engine, id: &str) -> Arc<Job> {
    let job = engine.job(id).unwrap();
    let began = Instant::now();
    while !job.status().unwrap().done {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "query did not finish"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    job
}
async fn run(engine: &Engine, connection: &Connection, sql: &str) -> Vec<Row> {
    let id = engine
        .start(connection.id.clone(), sql.into(), 100, 5, true)
        .await
        .unwrap();
    let job = completed(engine, &id).await;
    assert!(
        job.status().unwrap().error.is_none(),
        "{:?}",
        job.status().unwrap()
    );
    let rows = job.page(0, 0, 100).unwrap();
    engine.release(&id).unwrap();
    rows
}
#[tokio::test]
async fn real_plan_workflow() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut servers = vec![
        (
            "sqlite",
            directory
                .path()
                .join("data.db")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "duckdb",
            directory
                .path()
                .join("plans.duckdb")
                .to_string_lossy()
                .into_owned(),
        ),
    ];
    for (name, engine) in [
        ("KLYNDB_TEST_POSTGRES_URL", "postgres"),
        ("KLYNDB_TEST_MYSQL_URL", "mysql"),
        ("KLYNDB_TEST_MARIADB_URL", "mysql"),
        ("KLYNDB_TEST_MSSQL_URL", "mssql"),
    ] {
        if let Ok(url) = std::env::var(name) {
            servers.push((engine, url));
        }
    }
    for (kind, address) in servers {
        let mut connection = Connection {
            id: String::new(),
            name: "Plan contract".into(),
            engine: kind.into(),
            address,
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: matches!(kind, "sqlite" | "duckdb"),
        };
        connection.validate().unwrap();
        engine.store.save(&connection).unwrap();
        let password =
            (kind == "mssql").then(|| std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap());
        let capabilities = engine
            .connect(&connection.id, password.clone(), None, None)
            .await
            .unwrap();
        let table = format!("klyndb_plan_{}", uuid::Uuid::new_v4().simple());
        run(&engine, &connection, &format!("CREATE TABLE {table}(id INTEGER PRIMARY KEY, value INTEGER); INSERT INTO {table} VALUES(1,10),(2,20)")).await;
        let select = format!(
            "SELECT * FROM {table} WHERE value > 1 /* inner comment */ ORDER BY id; -- trailing é\n"
        );
        let id = engine
            .start_plan(connection.id.clone(), select.clone(), false, 5, false)
            .await
            .unwrap();
        let job = completed(&engine, &id).await;
        let plan = job.plan().unwrap();
        assert!(!plan.nodes.is_empty());
        assert!(plan.raw.contains(&table), "{}", plan.raw);
        assert!(!job.status().unwrap().plan_analyze);
        let format = plan.format;
        if format == PlanFormat::MysqlJson {
            assert!(!plan.warnings.is_empty());
        }
        engine.release(&id).unwrap();
        for statement in [
            format!("UPDATE {table} SET value=99"),
            format!("DELETE FROM {table}"),
        ] {
            let id = engine
                .start_plan(connection.id.clone(), statement, false, 5, false)
                .await
                .unwrap();
            completed(&engine, &id).await.plan().unwrap();
            engine.release(&id).unwrap();
        }
        assert_eq!(
            run(
                &engine,
                &connection,
                &format!("SELECT value FROM {table} ORDER BY id")
            )
            .await
            .iter()
            .map(|r| r[0].text())
            .collect::<Vec<_>>(),
            ["10", "20"]
        );
        if kind == "postgres" {
            let error = engine
                .start(
                    connection.id.clone(),
                    format!("EXPLAIN (ANALYZE TRUE, FORMAT JSON) UPDATE {table} SET value=99"),
                    100,
                    5,
                    false,
                )
                .await
                .unwrap_err();
            assert!(error.message.contains("Confirmation required"));
        }
        for sql in [
            "SELECT 1; DELETE FROM missing",
            "EXPLAIN SELECT 1",
            "-- empty",
        ] {
            assert!(
                engine
                    .start_plan(connection.id.clone(), sql.into(), false, 5, true)
                    .await
                    .is_err()
            );
        }
        assert!(
            engine
                .start_plan(connection.id.clone(), select.clone(), true, 5, false)
                .await
                .is_err()
        );
        if capabilities.explain_analyze {
            let id = engine
                .start_plan(connection.id.clone(), select, true, 5, true)
                .await
                .unwrap();
            let job = completed(&engine, &id).await;
            let plan = job.plan().unwrap();
            if kind == "mssql" {
                assert_eq!(job.status().unwrap().sets[0].rows, 2);
                assert_eq!(job.page(0, 0, 10).unwrap()[0][1].text(), "10");
                assert!(has_attribute(&plan.nodes, "Rows", "2"));
            }
            let encoded = serde_json::to_string(&plan).unwrap();
            assert!(
                encoded.contains(match format {
                    PlanFormat::PostgresJson => "Actual Loops",
                    PlanFormat::MysqlJson => "loops=",
                    PlanFormat::MariaJson => "r_loops",
                    PlanFormat::DuckDbJson => "operator_timing",
                    PlanFormat::SqlServerTabular => "Executes",
                    _ => panic!("unexpected runtime format"),
                }),
                "{encoded}"
            );
            engine.release(&id).unwrap();
            // Runtime DML is a real write on engines that support it; a caller-owned transaction remains explicit.
            run(
                &engine,
                &connection,
                if kind == "mssql" {
                    "BEGIN TRANSACTION"
                } else {
                    "BEGIN"
                },
            )
            .await;
            let update = format!("UPDATE {table} SET value=value+1");
            if format == PlanFormat::MysqlJson {
                assert!(
                    engine
                        .start_plan(connection.id.clone(), update, true, 5, true)
                        .await
                        .is_err()
                );
            } else {
                let id = engine
                    .start_plan(connection.id.clone(), update, true, 5, true)
                    .await
                    .unwrap();
                let job = completed(&engine, &id).await;
                job.plan().unwrap();
                assert_eq!(
                    run(
                        &engine,
                        &connection,
                        &format!("SELECT value FROM {table} WHERE id=1")
                    )
                    .await[0][0]
                        .text(),
                    "11"
                );
                engine.release(&id).unwrap();
            }
            run(&engine, &connection, "ROLLBACK").await;
            assert_eq!(
                run(
                    &engine,
                    &connection,
                    &format!("SELECT value FROM {table} WHERE id=1")
                )
                .await[0][0]
                    .text(),
                "10"
            );
            let sleep = match kind {
                "postgres" => "SELECT pg_sleep(30)",
                "mssql" => {
                    "SELECT COUNT_BIG(*) FROM sys.all_objects a CROSS JOIN sys.all_objects b CROSS JOIN sys.all_objects c"
                }
                "duckdb" => {
                    "SELECT sum(a.i*b.i) FROM range(1000000000) a(i), range(1000000000) b(i)"
                }
                _ => "SELECT SLEEP(30)",
            };
            let id = engine
                .start_plan(connection.id.clone(), sleep.into(), true, 5, true)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let began = Instant::now();
            engine.cancel(&id).unwrap();
            let job = completed(&engine, &id).await;
            assert!(job.status().unwrap().error.is_some());
            assert!(job.plan().is_err());
            assert!(began.elapsed() < Duration::from_secs(4));
            engine.release(&id).unwrap();
            let id = engine
                .start_plan(connection.id.clone(), sleep.into(), true, 1, true)
                .await
                .unwrap();
            assert!(
                completed(&engine, &id)
                    .await
                    .status()
                    .unwrap()
                    .error
                    .unwrap()
                    .contains("timed out")
            );
            engine.release(&id).unwrap();
            assert_eq!(
                run(&engine, &connection, "SELECT 42").await[0][0].text(),
                "42"
            );
        } else {
            assert_eq!(format, PlanFormat::Sqlite);
            assert!(
                engine
                    .start_plan(connection.id.clone(), "SELECT 1".into(), true, 5, true)
                    .await
                    .is_err()
            );
        }
        if kind == "mssql" {
            run(&engine, &connection, "SET IMPLICIT_TRANSACTIONS ON").await;
            let estimate = engine
                .start_plan(
                    connection.id.clone(),
                    format!("UPDATE {table} SET value=99"),
                    false,
                    5,
                    false,
                )
                .await
                .unwrap();
            completed(&engine, &estimate).await.plan().unwrap();
            engine.release(&estimate).unwrap();
            assert_eq!(
                run(&engine, &connection, "SELECT @@TRANCOUNT").await[0][0].text(),
                "0"
            );
            run(&engine, &connection, "SET IMPLICIT_TRANSACTIONS OFF").await;
            let constant = engine
                .start_plan(connection.id.clone(), "SELECT 42".into(), true, 5, true)
                .await
                .unwrap();
            let job = completed(&engine, &constant).await;
            assert_eq!(job.page(0, 0, 10).unwrap()[0][0].text(), "42");
            assert!(
                job.plan()
                    .unwrap_err()
                    .message
                    .contains("Statement completed")
            );
            engine.release(&constant).unwrap();
            let sql = "SELECT TOP(6001) a.object_id AS id FROM sys.all_objects a CROSS JOIN sys.all_objects b";
            let id = engine
                .start_plan(connection.id.clone(), sql.into(), true, 10, true)
                .await
                .unwrap();
            let job = completed(&engine, &id).await;
            assert!(job.status().unwrap().sets[0].truncated);
            assert_eq!(job.status().unwrap().sets[0].rows, 5000);
            assert!(has_attribute(&job.plan().unwrap().nodes, "Rows", "6001"));
            engine.release(&id).unwrap();
            for option in ["SHOWPLAN_ALL", "STATISTICS PROFILE"] {
                run(&engine, &connection, &format!("SET {option} ON")).await;
                let id = engine
                    .start_plan(connection.id.clone(), "SELECT 42".into(), false, 5, false)
                    .await
                    .unwrap();
                assert!(
                    completed(&engine, &id)
                        .await
                        .status()
                        .unwrap()
                        .error
                        .unwrap()
                        .contains("Disable existing")
                );
                engine.release(&id).unwrap();
                // Rejection must preserve the caller's enabled mode, not silently turn it off.
                let id = engine
                    .start(
                        connection.id.clone(),
                        "SELECT COUNT_BIG(*) FROM sys.objects WHERE object_id=-1".into(),
                        100,
                        5,
                        true,
                    )
                    .await
                    .unwrap();
                let job = completed(&engine, &id).await;
                assert!(
                    job.status()
                        .unwrap()
                        .sets
                        .iter()
                        .any(|s| s.columns.iter().any(|c| c == "NodeId"))
                );
                engine.release(&id).unwrap();
                run(&engine, &connection, &format!("SET {option} OFF")).await;
            }
            let id = engine
                .start_plan(
                    connection.id.clone(),
                    format!("SELECT nonexistent_column FROM {table}"),
                    false,
                    5,
                    false,
                )
                .await
                .unwrap();
            assert!(
                completed(&engine, &id)
                    .await
                    .status()
                    .unwrap()
                    .error
                    .is_some()
            );
            engine.release(&id).unwrap();
            assert_eq!(
                run(&engine, &connection, "SELECT 42").await[0][0].text(),
                "42"
            );
            let history = engine.store.history().unwrap();
            assert!(history.iter().any(|entry| {
                entry["sql"]
                    .as_str()
                    .is_some_and(|sql| sql.starts_with("SET SHOWPLAN_ALL ON;\nGO\n"))
            }));
            assert!(history.iter().any(|entry| {
                entry["sql"]
                    .as_str()
                    .is_some_and(|sql| sql.starts_with("SET STATISTICS PROFILE ON;\nGO\n"))
            }));
            for unsafe_target in [
                "SET SHOWPLAN_ALL OFF",
                "EXEC(N'SELECT 1')",
                "BEGIN TRANSACTION",
            ] {
                assert!(
                    engine
                        .start_plan(connection.id.clone(), unsafe_target.into(), false, 5, true)
                        .await
                        .is_err()
                );
            }
        }
        if kind == "duckdb" {
            engine.disconnect(&connection.id).await.unwrap();
        }
        let mut readonly = connection.clone();
        readonly.id = String::new();
        readonly.read_only = true;
        readonly.create_file = false;
        readonly.validate().unwrap();
        engine.store.save(&readonly).unwrap();
        assert!(
            !engine
                .connect(&readonly.id, password.clone(), None, None)
                .await
                .unwrap()
                .explain_analyze
        );
        let id = engine
            .start_plan(
                readonly.id.clone(),
                format!("SELECT * FROM {table}"),
                false,
                5,
                false,
            )
            .await
            .unwrap();
        completed(&engine, &id).await.plan().unwrap();
        engine.release(&id).unwrap();
        let id = engine
            .start_plan(
                readonly.id.clone(),
                format!("UPDATE {table} SET value=99"),
                false,
                5,
                false,
            )
            .await
            .unwrap();
        let result = completed(&engine, &id).await.plan();
        // Native read-only modes can reject DML planning even without executing the write.
        match result {
            Err(error) if kind == "mysql" => assert!(error.message.contains("READ ONLY")),
            Err(error) if kind == "duckdb" => assert!(error.message.contains("read-only mode")),
            result => {
                result.unwrap();
            }
        }
        engine.release(&id).unwrap();
        assert!(
            engine
                .start_plan(readonly.id.clone(), "SELECT 1".into(), true, 5, true)
                .await
                .is_err()
        );
        assert_eq!(
            run(
                &engine,
                if kind == "duckdb" {
                    &readonly
                } else {
                    &connection
                },
                &format!("SELECT value FROM {table} WHERE id=1")
            )
            .await[0][0]
                .text(),
            "10"
        );
        engine.disconnect(&readonly.id).await.unwrap();
        if kind == "duckdb" {
            engine
                .connect(&connection.id, None, None, None)
                .await
                .unwrap();
        }
        run(&engine, &connection, &format!("DROP TABLE {table}")).await;
        engine.disconnect(&connection.id).await.unwrap();
    }
}

fn has_attribute(nodes: &[klyndb_query::plan::PlanNode], name: &str, value: &str) -> bool {
    nodes.iter().any(|node| {
        node.attributes.iter().any(|(n, v)| n == name && v == value)
            || has_attribute(&node.children, name, value)
    })
}
