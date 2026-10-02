use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{FilterOp, Row, TableFilter, TableQuery, TableSort};
use std::time::{Duration, Instant};

async fn run(engine: &Engine, connection: &Connection, sql: String, limit: usize) -> Vec<Row> {
    let id = engine
        .start(connection.id.clone(), sql.clone(), limit, 10, true)
        .await
        .unwrap();
    let job = engine.job(&id).unwrap();
    let began = Instant::now();
    while !job.status().unwrap().done {
        assert!(began.elapsed() < Duration::from_secs(15));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        job.status().unwrap().error.is_none(),
        "{sql}: {:?}",
        job.status().unwrap()
    );
    let rows = job.page(0, 0, 500).unwrap();
    engine.release(&id).unwrap();
    rows
}
#[tokio::test]
async fn real_server_filters_sort_and_pages() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut servers = vec![(
        "sqlite",
        directory
            .path()
            .join("data.db")
            .to_string_lossy()
            .into_owned(),
    )];
    for (variable, kind) in [
        ("KLYNDB_TEST_POSTGRES_URL", "postgres"),
        ("KLYNDB_TEST_MYSQL_URL", "mysql"),
    ] {
        if let Ok(url) = std::env::var(variable) {
            servers.push((kind, url));
        }
    }
    for (kind, address) in servers {
        let mut connection = Connection {
            id: String::new(),
            name: "Table browse contract".into(),
            engine: kind.into(),
            address,
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: kind == "sqlite",
        };
        connection.validate().unwrap();
        engine.store.save(&connection).unwrap();
        assert!(
            engine
                .connect(&connection.id, None, None)
                .await
                .unwrap()
                .table_browse
        );
        let driver = engine.driver(&connection.id).await.unwrap();
        let name = format!("browse_{}", uuid::Uuid::new_v4().simple());
        let quoted = driver.quote_identifier(&name);
        let values = (1..=1201)
            .map(|id| {
                format!(
                    "({id},'row {id}',{}, {})",
                    id % 5,
                    if id % 2 == 0 { "NULL" } else { "'odd'" }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        run(&engine, &connection, format!("CREATE TABLE {quoted} (id INTEGER PRIMARY KEY, label TEXT, score INTEGER, category TEXT); INSERT INTO {quoted} VALUES {values}"), 500).await;
        let table = driver
            .tables()
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap();
        let mut request = TableQuery {
            filters: vec![],
            sort: vec![],
            limit: 250,
            offset: 0,
        };
        let sql = engine
            .table_query_sql(&connection.id, &table, &request)
            .await
            .unwrap();
        let rows = run(&engine, &connection, sql, 250).await;
        assert_eq!(
            (rows[0][0].text(), rows[249][0].text()),
            ("1".into(), "250".into())
        );
        request.offset = 250;
        let rows = run(
            &engine,
            &connection,
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .unwrap(),
            250,
        )
        .await;
        assert_eq!(
            (rows[0][0].text(), rows[249][0].text()),
            ("251".into(), "500".into())
        );
        request.offset = 0;
        request.filters = vec![TableFilter {
            column: "id".into(),
            op: FilterOp::GreaterEqual,
            value: "1100".into(),
        }];
        request.sort = vec![TableSort {
            column: "id".into(),
            descending: true,
        }];
        let rows = run(
            &engine,
            &connection,
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .unwrap(),
            250,
        )
        .await;
        assert_eq!(rows.len(), 102);
        assert_eq!(
            (rows[0][0].text(), rows[101][0].text()),
            ("1201".into(), "1100".into())
        );
        request.filters.push(TableFilter {
            column: "category".into(),
            op: FilterOp::IsNull,
            value: String::new(),
        });
        let rows = run(
            &engine,
            &connection,
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .unwrap(),
            250,
        )
        .await;
        assert_eq!(rows.len(), 51);
        assert_eq!(rows[0][0].text(), "1200");
        request.filters.clear();
        request.sort = vec![TableSort {
            column: "score".into(),
            descending: true,
        }];
        request.limit = 3;
        let rows = run(
            &engine,
            &connection,
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .unwrap(),
            3,
        )
        .await;
        assert_eq!(
            rows.iter().map(|r| r[0].text()).collect::<Vec<_>>(),
            ["4", "9", "14"]
        );
        // Literal quoting is a trust boundary: filter input must never become executable SQL.
        let value = format!("O'Hara\\path %_! é'; DROP TABLE {quoted}; --");
        run(
            &engine,
            &connection,
            format!(
                "INSERT INTO {quoted} VALUES(1202,{},0,NULL)",
                driver.quote_filter_value(&value)
            ),
            10,
        )
        .await;
        request.filters = vec![TableFilter {
            column: "label".into(),
            op: FilterOp::Equal,
            value,
        }];
        let sql = engine
            .table_query_sql(&connection.id, &table, &request)
            .await
            .unwrap();
        let analysis = klyndb_query::analyze(&sql, kind).unwrap();
        assert!(analysis.read_only);
        assert_eq!(analysis.statements.len(), 1);
        let rows = run(&engine, &connection, sql, 3).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].text(), "1202");
        request.filters[0].op = FilterOp::Contains;
        request.filters[0].value = "%_!".into();
        let rows = run(
            &engine,
            &connection,
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .unwrap(),
            3,
        )
        .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].text(), "1202");
        request.filters[0].column = "missing".into();
        assert!(
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .is_err()
        );
        request.filters.clear();
        request.limit = 501;
        assert!(
            engine
                .table_query_sql(&connection.id, &table, &request)
                .await
                .is_err()
        );
        assert_eq!(
            run(
                &engine,
                &connection,
                format!("SELECT COUNT(*) FROM {quoted}"),
                10
            )
            .await[0][0]
                .text(),
            "1202"
        );
        run(&engine, &connection, format!("DROP TABLE {quoted}"), 10).await;
        engine.disconnect(&connection.id).await.unwrap();
    }
}
