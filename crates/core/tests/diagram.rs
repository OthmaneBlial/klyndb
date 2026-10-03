use klyndb_connections::{Connection, Store};
use klyndb_core::{
    Engine,
    diagram::{Point, Positions, key},
};
use std::time::{Duration, Instant};

async fn run(engine: &Engine, id: &str, sql: String) {
    let job_id = engine
        .start(id.into(), format!("{sql};"), 500, 10, true)
        .await
        .unwrap();
    let job = engine.job(&job_id).unwrap();
    let start = Instant::now();
    while !job.status().unwrap().done {
        assert!(start.elapsed() < Duration::from_secs(15));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        job.status().unwrap().error.is_none(),
        "{sql}: {:?}",
        job.status().unwrap()
    );
    engine.release(&job_id).unwrap();
}
#[tokio::test]
async fn real_relationships_layout_and_safe_svg() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
    let mut servers = vec![(
        "sqlite",
        dir.path().join("data.db").to_string_lossy().into_owned(),
    )];
    for (variable, kind) in [
        ("KLYNDB_TEST_POSTGRES_URL", "postgres"),
        ("KLYNDB_TEST_MYSQL_URL", "mysql"),
        ("KLYNDB_TEST_MSSQL_URL", "mssql"),
    ] {
        if let Ok(url) = std::env::var(variable) {
            servers.push((kind, url));
        }
    }
    for (kind, address) in servers {
        let mut c = Connection {
            id: String::new(),
            name: "Diagram contract".into(),
            engine: kind.into(),
            address,
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: kind == "sqlite",
        };
        c.validate().unwrap();
        engine.store.save(&c).unwrap();
        let password =
            (kind == "mssql").then(|| std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap());
        assert!(
            engine
                .connect(&c.id, password.clone(), None, None)
                .await
                .unwrap()
                .diagrams
        );
        let driver = engine.driver(&c.id).await.unwrap();
        let q = |n: &str| driver.quote_identifier(n);
        let base = format!("diagram_{}", uuid::Uuid::new_v4().simple());
        let parent = format!("{base}_p");
        let child = format!("{base}_c");
        let parent_schema = format!("{base}_schema");
        let parent_name = if kind == "mssql" {
            run(
                &engine,
                &c.id,
                format!("CREATE SCHEMA {}", q(&parent_schema)),
            )
            .await;
            format!("{}.{}", q(&parent_schema), q(&parent))
        } else {
            q(&parent)
        };
        run(
            &engine,
            &c.id,
            format!(
                "CREATE TABLE {} (a INTEGER,b INTEGER,PRIMARY KEY(a,b))",
                parent_name
            ),
        )
        .await;
        let target = if kind == "sqlite" {
            String::new()
        } else {
            "(a,b)".into()
        };
        run(&engine,&c.id,format!("CREATE TABLE {} (id INTEGER PRIMARY KEY,a INTEGER,b INTEGER,parent_id INTEGER,CONSTRAINT {} FOREIGN KEY(a,b) REFERENCES {}{target},CONSTRAINT {} FOREIGN KEY(parent_id) REFERENCES {}(id))",q(&child),q(&format!("{base}_fk")),parent_name,q(&format!("{base}_self")),q(&child))).await;
        let tables: Vec<_> = driver
            .tables()
            .await
            .unwrap()
            .into_iter()
            .filter(|t| t.name == parent || t.name == child)
            .collect();
        assert_eq!(tables.len(), 2);
        let model = engine.diagram(&c.id, &tables).await.unwrap();
        let source = model.tables.iter().find(|t| t.table.name == child).unwrap();
        assert_eq!(source.relationships.len(), 2);
        assert_eq!(source.columns.len(), 4);
        let fk = source
            .relationships
            .iter()
            .find(|r| r.target_table == parent)
            .unwrap();
        assert_eq!(fk.columns, vec!["a", "b"]);
        assert_eq!(fk.target_columns, vec![Some("a".into()), Some("b".into())]);
        assert_eq!(
            &fk.target_schema,
            if kind == "mssql" {
                &parent_schema
            } else {
                &source.table.schema
            }
        );
        let self_fk = source
            .relationships
            .iter()
            .find(|r| r.target_table == child)
            .unwrap();
        assert_eq!(self_fk.columns, vec!["parent_id"]);
        assert_eq!(self_fk.target_columns, vec![Some("id".into())]);
        let positions: Positions = model
            .tables
            .iter()
            .enumerate()
            .map(|(i, t)| {
                (
                    key(&t.table.schema, &t.table.name),
                    Point {
                        x: i as f64 * 400.0,
                        y: 40.0,
                    },
                )
            })
            .collect();
        let svg = model.svg(&positions).unwrap();
        assert_eq!(svg.matches("marker-end").count(), 3);
        assert!(svg.contains("◆ "));
        assert!(svg.contains("↗ "));
        assert!(engine.diagram(&c.id, &[]).await.is_err());
        assert!(
            engine
                .diagram(&c.id, &[tables[0].clone(), tables[0].clone()])
                .await
                .is_err()
        );
        assert!(model.svg(&Positions::new()).is_err());
        let mut invalid = positions.clone();
        invalid.values_mut().next().unwrap().x = f64::NAN;
        assert!(model.svg(&invalid).is_err());
        let mut hostile = model.clone();
        hostile.tables[0].columns[0].name = "<script>&bad".into();
        let safe = hostile.svg(&positions).unwrap();
        assert!(safe.contains("&lt;script&gt;&amp;bad"));
        assert!(!safe.contains("<script>"));
        let layout = serde_json::json!({"tables":tables,"positions":positions,"zoom":1.2,"pan":{"x":-20,"y":40}});
        let doc_id = format!("diagram-{}", c.id);
        engine.store.save_document(&doc_id, &layout).unwrap();
        assert_eq!(engine.store.document(&doc_id).unwrap().unwrap(), layout);
        if kind == "mssql" {
            let mut readonly = c.clone();
            readonly.id.clear();
            readonly.read_only = true;
            readonly.validate().unwrap();
            engine.store.save(&readonly).unwrap();
            let caps = engine
                .connect(&readonly.id, password, None, None)
                .await
                .unwrap();
            assert!(caps.diagrams && !caps.edit_rows && !caps.import_rows);
            let readonly_model = engine.diagram(&readonly.id, &tables).await.unwrap();
            assert_eq!(
                serde_json::to_value(readonly_model).unwrap(),
                serde_json::to_value(&model).unwrap()
            );
            engine.disconnect(&readonly.id).await.unwrap();
        }
        run(&engine, &c.id, format!("DROP TABLE {}", q(&child))).await;
        run(&engine, &c.id, format!("DROP TABLE {parent_name}")).await;
        if kind == "mssql" {
            run(&engine, &c.id, format!("DROP SCHEMA {}", q(&parent_schema))).await;
        }
        engine.disconnect(&c.id).await.unwrap();
    }
}
