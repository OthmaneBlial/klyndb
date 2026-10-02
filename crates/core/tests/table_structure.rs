use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use std::time::{Duration, Instant};

async fn run(engine: &Engine, id: &str, sql: String) {
    let job_id = engine
        .start(id.into(), format!("{sql};"), 500, 10, true)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    let job = engine.job(&job_id).unwrap();
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
    engine.release(&job_id).unwrap();
}

#[tokio::test]
async fn real_constraint_and_trigger_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
    let mut servers = vec![(
        "sqlite",
        dir.path().join("data.db").to_string_lossy().into_owned(),
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
        let mut c = Connection {
            id: String::new(),
            name: "Structure contract".into(),
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
        engine.connect(&c.id, None, None).await.unwrap();
        let driver = engine.driver(&c.id).await.unwrap();
        let base = format!("structure_{}", uuid::Uuid::new_v4().simple());
        let child = format!("{base}_child");
        let parent = format!("{base}_parent");
        let audit = format!("{base}_audit");
        let trigger = format!("{base}_trigger");
        let function = format!("{base}_fn");
        let q = |name: &str| driver.quote_identifier(name);
        run(
            &engine,
            &c.id,
            format!("CREATE TABLE {} (id INTEGER PRIMARY KEY)", q(&parent)),
        )
        .await;
        run(
            &engine,
            &c.id,
            format!("CREATE TABLE {} (id INTEGER)", q(&audit)),
        )
        .await;
        run(&engine, &c.id, format!("CREATE TABLE {} (id INTEGER PRIMARY KEY, label VARCHAR(40) UNIQUE, amount INTEGER, parent_id INTEGER, CHECK(amount>=0), CONSTRAINT {} FOREIGN KEY(parent_id) REFERENCES {}(id))",q(&child), q(&format!("{base}_fk")), q(&parent))).await;
        let body = format!("INSERT INTO {} VALUES (NEW.id)", q(&audit));
        let trigger_sql = match kind {
            "postgres" => {
                run(&engine, &c.id, format!("CREATE FUNCTION {}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN {body}; RETURN NEW; END $$", q(&function))).await;
                format!(
                    "CREATE TRIGGER {} AFTER INSERT ON {} FOR EACH ROW EXECUTE FUNCTION {}()",
                    q(&trigger),
                    q(&child),
                    q(&function)
                )
            }
            "sqlite" => format!(
                "CREATE TRIGGER {} AFTER INSERT ON {} BEGIN {body}; END",
                q(&trigger),
                q(&child)
            ),
            _ => format!(
                "CREATE TRIGGER {} AFTER INSERT ON {} FOR EACH ROW {body}",
                q(&trigger),
                q(&child)
            ),
        };
        run(&engine, &c.id, trigger_sql).await;
        let tables = driver.tables().await.unwrap();
        let table = tables.iter().find(|t| t.name == child).unwrap();
        let info = driver.inspect(table).await.unwrap();
        assert_eq!(info.columns.len(), 4);
        assert_eq!(info.triggers.len(), 1);
        assert_eq!(info.triggers[0].name, trigger);
        assert!(
            info.triggers[0]
                .definition
                .to_uppercase()
                .contains("AFTER INSERT")
        );
        if kind == "sqlite" {
            assert!(info.constraints.is_none());
            assert!(info.ddl.as_ref().unwrap().contains("CHECK(amount>=0)"));
            assert!(info.triggers[0].definition.contains(&audit));
        } else {
            let constraints = info.constraints.as_ref().unwrap();
            for expected in ["PRIMARY KEY", "UNIQUE", "FOREIGN KEY", "CHECK"] {
                assert!(
                    constraints.iter().any(|c| c.kind == expected),
                    "Missing {expected}: {constraints:?}"
                );
            }
            if kind == "postgres" {
                assert!(constraints.iter().all(|c| c.definition.is_some()));
                assert_eq!(info.triggers[0].state.as_deref(), Some("Origin/local"));
                run(
                    &engine,
                    &c.id,
                    format!("ALTER TABLE {} DISABLE TRIGGER {}", q(&child), q(&trigger)),
                )
                .await;
                assert_eq!(
                    driver.inspect(table).await.unwrap().triggers[0]
                        .state
                        .as_deref(),
                    Some("Disabled")
                );
                run(
                    &engine,
                    &c.id,
                    format!("ALTER TABLE {} ENABLE TRIGGER {}", q(&child), q(&trigger)),
                )
                .await;
            } else {
                assert!(info.triggers[0].definition.contains(&audit));
                assert!(info.ddl.as_ref().unwrap().contains("CHECK"));
            }
        }
        let other = tables.iter().find(|t| t.name == parent).unwrap();
        assert!(driver.inspect(other).await.unwrap().triggers.is_empty());
        // Inspection must not execute the displayed trigger body.
        run(
            &engine,
            &c.id,
            format!(
                "INSERT INTO {}(id,label,amount) VALUES(1,'demo',2)",
                q(&child)
            ),
        )
        .await;
        let id = engine
            .start(
                c.id.clone(),
                format!("SELECT COUNT(*) FROM {}", q(&audit)),
                10,
                10,
                true,
            )
            .await
            .unwrap();
        let job = engine.job(&id).unwrap();
        let began = Instant::now();
        while !job.status().unwrap().done {
            assert!(began.elapsed() < Duration::from_secs(15));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(job.status().unwrap().error.is_none());
        assert_eq!(job.page(0, 0, 10).unwrap()[0][0].text(), "1");
        engine.release(&id).unwrap();
        run(&engine, &c.id, format!("DROP TABLE {}", q(&child))).await;
        if kind == "postgres" {
            run(&engine, &c.id, format!("DROP FUNCTION {}()", q(&function))).await;
        }
        run(&engine, &c.id, format!("DROP TABLE {}", q(&audit))).await;
        run(&engine, &c.id, format!("DROP TABLE {}", q(&parent))).await;
        engine.disconnect(&c.id).await.unwrap();
    }
}
