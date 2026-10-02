use klyndb_driver_api::{Batch, Cell, Session};
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
    query(db.clone(), "DROP SCHEMA klyndb_test CASCADE").await;
    db.disconnect().await.unwrap();
}
