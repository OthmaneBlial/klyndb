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
async fn backpressure_cancellation_and_consumer_close() {
    let url = std::env::var("KLYNDB_TEST_POSTGRES_URL").unwrap();
    for case in 0..3 {
        let db = Arc::new(Postgres::connect(&url, None, false).await.unwrap());
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
    let db = Postgres::connect(&url, None, true).await.unwrap();
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
    let db = Arc::new(Postgres::connect(&url, None, false).await.unwrap());
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
    let ro = Postgres::connect(&url, None, true).await.unwrap();
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
