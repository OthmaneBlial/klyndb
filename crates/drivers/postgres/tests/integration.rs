use klyndb_driver_api::{Batch, Cell, Change, Session, TransactionState};
use klyndb_postgres::Postgres;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn query(db: Arc<Postgres>, sql: &str) -> Vec<Batch> {
    let (tx, mut rx) = mpsc::channel(2);
    let sql = sql.to_owned();
    let task =
        tokio::spawn(async move { db.execute(sql, tx, CancellationToken::new(), 100).await });
    let mut batches = vec![];
    while let Some(batch) = rx.recv().await {
        batches.push(batch);
    }
    task.await.unwrap().unwrap();
    batches
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server and KLYNDB_TEST_POSTGRES_URL"]
async fn postgres_structure_ddl_roundtrip_views_and_materialized_catalog() {
    use klyndb_driver_api::Table;
    let url = std::env::var("KLYNDB_TEST_POSTGRES_URL").unwrap();
    let db = Arc::new(Postgres::connect(&url, None, false, None).await.unwrap());
    let schema = format!("klyndb_ddl_{}", uuid::Uuid::new_v4().simple());
    let table = Table {
        schema: schema.clone(),
        name: "odd \" customers".into(),
        kind: "base table".into(),
    };
    let qualified = format!("{schema}.\"odd \"\" customers\"");
    query(db.clone(), &format!(r#"
        CREATE SCHEMA {schema};
        CREATE TYPE {schema}."order type" AS ENUM ('new', 'ready');
        CREATE TABLE {schema}.parents(id bigint PRIMARY KEY);
        CREATE UNLOGGED TABLE {qualified} (
            id bigint GENERATED ALWAYS AS IDENTITY (START WITH 7 INCREMENT BY 3 MINVALUE 1 MAXVALUE 10000 CACHE 4),
            "label "" text" varchar(40) COLLATE "C" DEFAULT 'quoted '' value' NOT NULL,
            amount numeric(30,18) DEFAULT 0.000000000000000001 NOT NULL,
            tags text[] DEFAULT ARRAY['local','native'],
            state {schema}."order type" DEFAULT 'new',
            doubled numeric GENERATED ALWAYS AS (amount * 2) STORED,
            parent_id bigint REFERENCES {schema}.parents(id) DEFERRABLE INITIALLY DEFERRED,
            obsolete text,
            PRIMARY KEY(id), UNIQUE("label "" text"), CHECK (amount >= 0)
        );
        ALTER TABLE {qualified} DROP COLUMN obsolete;
        ALTER TABLE {qualified} ENABLE ROW LEVEL SECURITY;
        ALTER TABLE {qualified} FORCE ROW LEVEL SECURITY;
    "#)).await;
    let original = db.inspect(&table).await.unwrap();
    assert!(original.editable);
    assert_eq!(original.columns.len(), 7);
    assert_eq!(original.columns[2].data_type, "numeric(30,18)");
    assert!(original.columns[0].primary_key && original.columns[0].generated);
    assert!(original.columns[5].generated);
    let ddl = original.ddl.as_ref().unwrap();
    assert!(ddl.contains("CREATE UNLOGGED TABLE"));
    assert!(ddl.contains("START WITH 7 INCREMENT BY 3 MINVALUE 1 MAXVALUE 10000 CACHE 4 NO CYCLE"));
    assert!(ddl.contains("COLLATE pg_catalog.\"C\""));
    assert!(ddl.contains("GENERATED ALWAYS AS") && ddl.contains("STORED"));
    assert!(ddl.contains("ENABLE ROW LEVEL SECURITY") && ddl.contains("FORCE ROW LEVEL SECURITY"));
    assert!(!ddl.contains("obsolete"));
    assert!(ddl.contains("REFERENCES") && ddl.contains("DEFERRABLE INITIALLY DEFERRED"));
    query(db.clone(), &format!("DROP TABLE {qualified}; {ddl}")).await;
    let recreated = db.inspect(&table).await.unwrap();
    assert_eq!(
        serde_json::to_value(&original.columns).unwrap(),
        serde_json::to_value(&recreated.columns).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&original.constraints).unwrap(),
        serde_json::to_value(&recreated.constraints).unwrap()
    );
    assert_eq!(recreated.ddl.as_ref(), Some(ddl));
    query(db.clone(), &format!(r#"
        CREATE VIEW {schema}.visible (customer_id, balance) WITH (security_barrier=true, security_invoker=true) AS SELECT id, amount FROM {qualified} WHERE amount > 0 WITH LOCAL CHECK OPTION;
        CREATE MATERIALIZED VIEW {schema}.cached (customer_id, balance) AS SELECT id, amount FROM {qualified} WITH NO DATA;
        CREATE TABLE {schema}.partitioned(id bigint) PARTITION BY RANGE(id);
        CREATE TABLE {schema}.child PARTITION OF {schema}.partitioned FOR VALUES FROM (0) TO (100);
    "#)).await;
    let ro = Postgres::connect(&url, None, true, None).await.unwrap();
    let tables = ro.tables().await.unwrap();
    for (name, kind) in [("visible", "view"), ("cached", "materialized view")] {
        let target = tables
            .iter()
            .find(|t| t.schema == schema && t.name == name)
            .unwrap();
        assert_eq!(target.kind, kind);
        let info = ro.inspect(target).await.unwrap();
        assert_eq!(info.columns.len(), 2);
        assert!(!info.editable);
        let definition = info.ddl.unwrap();
        assert!(definition.contains("customer_id, balance"));
        if name == "visible" {
            assert!(
                definition.contains("security_barrier=true")
                    && definition.contains("security_invoker=true")
                    && definition.contains("check_option=local")
            );
        } else {
            assert!(
                definition.contains("CREATE MATERIALIZED VIEW")
                    && definition.contains("WITH NO DATA")
            );
        }
        query(
            db.clone(),
            &format!(
                "DROP {} {schema}.{name}; {definition}",
                if name == "cached" {
                    "MATERIALIZED VIEW"
                } else {
                    "VIEW"
                }
            ),
        )
        .await;
        assert_eq!(ro.inspect(target).await.unwrap().ddl, Some(definition));
    }
    for name in ["partitioned", "child"] {
        let target = tables
            .iter()
            .find(|t| t.schema == schema && t.name == name)
            .unwrap();
        let info = ro.inspect(target).await.unwrap();
        assert_eq!(info.columns.len(), 1);
        assert!(info.ddl.unwrap().contains("unavailable for partitioned"));
    }
    // Reading definitions must not execute even volatile view expressions.
    query(db.clone(), &format!("CREATE SEQUENCE {schema}.inspect_probe; CREATE VIEW {schema}.volatile_view AS SELECT nextval('{schema}.inspect_probe') AS value; CREATE MATERIALIZED VIEW {schema}.volatile_cached AS SELECT nextval('{schema}.inspect_probe') AS value WITH NO DATA")).await;
    for name in ["volatile_view", "volatile_cached"] {
        let info = ro
            .inspect(&Table {
                schema: schema.clone(),
                name: name.into(),
                kind: "view".into(),
            })
            .await
            .unwrap();
        assert!(info.ddl.unwrap().contains("nextval"));
    }
    assert!(
        query(
            db.clone(),
            &format!("SELECT is_called::text FROM {schema}.inspect_probe")
        )
        .await
        .iter()
        .any(|b| matches!(b, Batch::Rows(rows) if rows[0] == vec![Cell::Text("false".into())]))
    );
    query(
        db.clone(),
        &format!(
            "CREATE VIEW {schema}.oversized AS SELECT '{}'::text AS value",
            "x".repeat(2 * 1024 * 1024)
        ),
    )
    .await;
    let oversized = ro
        .inspect(&Table {
            schema: schema.clone(),
            name: "oversized".into(),
            kind: "view".into(),
        })
        .await
        .unwrap();
    assert_eq!(oversized.columns.len(), 1);
    assert!(oversized.ddl.unwrap().contains("2 MiB viewer limit"));
    // Inspection leaves the caller's transaction, search path and uncommitted work intact.
    query(db.clone(), "BEGIN; SET LOCAL search_path = pg_catalog; CREATE TEMP TABLE ddl_session_guard(v int); INSERT INTO ddl_session_guard VALUES(42)").await;
    assert!(
        db.inspect(&table)
            .await
            .unwrap()
            .ddl
            .unwrap()
            .contains("CREATE UNLOGGED TABLE")
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    let session_rows = query(
        db.clone(),
        "SHOW search_path; SELECT * FROM ddl_session_guard",
    )
    .await;
    for value in ["pg_catalog", "42"] {
        assert!(
            session_rows.iter().any(
                |b| matches!(b, Batch::Rows(rows) if rows[0] == vec![Cell::Text(value.into())])
            )
        );
    }
    query(db.clone(), "ROLLBACK").await;
    ro.disconnect().await.unwrap();
    query(db.clone(), &format!("DROP SCHEMA {schema} CASCADE")).await;
    db.disconnect().await.unwrap();
}
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server and KLYNDB_TEST_POSTGRES_URL"]
async fn backpressure_cancellation_and_consumer_close() {
    let url = std::env::var("KLYNDB_TEST_POSTGRES_URL").unwrap();
    for case in 0..3 {
        let db = Arc::new(Postgres::connect(&url, None, false, None).await.unwrap());
        let (tx, mut rx) = mpsc::channel(1);
        let token = CancellationToken::new();
        let cancel = token.clone();
        let driver = db.clone();
        let mut task = tokio::spawn(async move {
            let sql = if case == 2 {
                "SELECT pg_sleep(30)"
            } else {
                "SELECT generate_series(1,100000); SELECT pg_sleep(30)"
            };
            driver.execute(sql.into(), tx, cancel, 10000).await
        });
        // Leave the receiver alive and full: cancellation must wake a blocked send.
        if case != 2 {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while rx.is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if case != 0 {
            rx.close();
        } else {
            token.cancel();
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(4), &mut task).await;
        if result.is_err() {
            task.abort();
            db.disconnect().await.unwrap();
            panic!("query did not terminate with a blocked or closed consumer");
        }
        assert!(result.unwrap().unwrap().is_err());
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                query(db.clone(), "SELECT 42")
            )
            .await
            .unwrap()
            .iter()
            .any(|b| matches!(b, Batch::Rows(rows) if rows[0] == vec![Cell::Text("42".into())]))
        );
        db.disconnect().await.unwrap();
    }
}
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server and KLYNDB_TEST_POSTGRES_URL"]
async fn failed_read_only_cleanup_reports_closed_session() {
    let url = std::env::var("KLYNDB_TEST_POSTGRES_URL").unwrap();
    let db = Postgres::connect(&url, None, true, None).await.unwrap();
    let (tx, _rx) = mpsc::channel(16);
    // End only this disposable test connection, making protective ROLLBACK fail.
    let failure = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        db.execute(
            "SELECT pg_terminate_backend(pg_backend_pid())".into(),
            tx,
            CancellationToken::new(),
            100,
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        failure.message.contains("connection closed"),
        "{}",
        failure.message
    );
    assert!(db.transaction_state().await.is_err());
    db.disconnect().await.unwrap();
}
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server and KLYNDB_TEST_POSTGRES_URL"]
async fn real_postgres_workflow_and_cancellation() {
    let url = std::env::var("KLYNDB_TEST_POSTGRES_URL").expect("set a disposable test server URL");
    let db = Arc::new(Postgres::connect(&url, None, false, None).await.unwrap());
    query(db.clone(),"CREATE SCHEMA IF NOT EXISTS klyndb_test; DROP TABLE IF EXISTS klyndb_test.items; CREATE TABLE klyndb_test.items(id BIGINT PRIMARY KEY, value TEXT); INSERT INTO klyndb_test.items VALUES(9223372036854775807,NULL)").await;
    let tables = db.tables().await.unwrap();
    let table = tables
        .iter()
        .find(|t| t.schema == "klyndb_test" && t.name == "items")
        .unwrap();
    assert!(db.inspect(table).await.unwrap().columns[0].primary_key);
    let result = query(
        db.clone(),
        "SELECT * FROM klyndb_test.items; SELECT 1 WHERE false; SELECT generate_series(1,200)",
    )
    .await;
    assert_eq!(
        result
            .iter()
            .filter(|b| matches!(b, Batch::Columns(_)))
            .count(),
        3
    );
    assert!(result.iter().any(|b| matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Text("9223372036854775807".into()),Cell::Null])));
    assert!(result.iter().any(|b| matches!(
        b,
        Batch::Complete {
            truncated: true,
            ..
        }
    )));
    let token = CancellationToken::new();
    let t = token.clone();
    let driver = db.clone();
    let (tx, mut rx) = mpsc::channel(2);
    let task = tokio::spawn(async move {
        driver
            .execute("SELECT pg_sleep(30)".into(), tx, t, 100)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let began = std::time::Instant::now();
    token.cancel();
    while rx.recv().await.is_some() {}
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(began.elapsed() < std::time::Duration::from_secs(2));
    assert!(
        query(db.clone(), "SELECT 1")
            .await
            .iter()
            .any(|b| matches!(b, Batch::Rows(_)))
    );
    let old = vec![Cell::Text("9223372036854775807".into()), Cell::Null];
    let literal = "x'; DROP SCHEMA klyndb_test CASCADE; --";
    let changes = vec![Change::Update {
        old: old.clone(),
        values: std::collections::BTreeMap::from([("value".into(), Cell::Text(literal.into()))]),
    }];
    assert!(
        !db.apply_changes(table.clone(), changes.clone())
            .await
            .unwrap()
            .pending_transaction
    );
    let insert = Change::Insert {
        values: std::collections::BTreeMap::from([("id".into(), Cell::Number("1".into()))]),
    };
    assert!(
        db.apply_changes(table.clone(), vec![insert, changes[0].clone()])
            .await
            .unwrap_err()
            .message
            .contains("Row changed")
    );
    assert!(
        query(db.clone(), "SELECT count(*) FROM klyndb_test.items")
            .await
            .iter()
            .any(|b| matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Text("1".into())]))
    );
    let current = vec![old[0].clone(), Cell::Text(literal.into())];
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    query(db.clone(), "BEGIN").await;
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    assert!(
        db.apply_changes(
            table.clone(),
            vec![Change::Update {
                old: current.clone(),
                values: std::collections::BTreeMap::from([(
                    "value".into(),
                    Cell::Text("pending".into())
                )])
            }]
        )
        .await
        .unwrap()
        .pending_transaction
    );
    // A stale edit must roll back only its savepoint, keeping the prior user's uncommitted write.
    assert!(db.apply_changes(table.clone(), changes).await.is_err());
    assert!(
        query(db.clone(), "SELECT value FROM klyndb_test.items")
            .await
            .iter()
            .any(|b| matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Text("pending".into())]))
    );
    query(db.clone(), "ROLLBACK").await;
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    let (failed_tx, mut failed_rx) = mpsc::channel(16);
    assert!(
        db.execute(
            "BEGIN; SELECT klyndb_missing_column".into(),
            failed_tx,
            CancellationToken::new(),
            100
        )
        .await
        .is_err()
    );
    while failed_rx.recv().await.is_some() {}
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Failed
    );
    query(db.clone(), "ROLLBACK").await;
    let ro = Postgres::connect(&url, None, true, None).await.unwrap();
    assert!(
        ro.apply_changes(
            table.clone(),
            vec![Change::Delete {
                old: current.clone()
            }]
        )
        .await
        .is_err()
    );
    ro.disconnect().await.unwrap();
    assert_eq!(
        db.apply_changes(table.clone(), vec![Change::Delete { old: current }])
            .await
            .unwrap()
            .affected,
        1
    );
    query(db.clone(), "DROP SCHEMA klyndb_test CASCADE").await;
    db.disconnect().await.unwrap();
}
