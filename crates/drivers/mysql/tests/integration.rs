use klyndb_driver_api::{Batch, Cell, Session, TransactionState};
use klyndb_mysql::Mysql;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn query(db: Arc<Mysql>, sql: &str, limit: usize) -> Vec<Batch> {
    let (tx, mut rx) = mpsc::channel(2);
    let sql = sql.to_owned();
    let task =
        tokio::spawn(async move { db.execute(sql, tx, CancellationToken::new(), limit).await });
    let mut batches = vec![];
    while let Some(batch) = rx.recv().await {
        batches.push(batch);
    }
    task.await.unwrap().unwrap();
    batches
}
#[tokio::test]
#[ignore = "requires a disposable MySQL or MariaDB server and KLYNDB_TEST_MYSQL_URL"]
async fn real_mysql_workflow_and_cancellation() {
    let url = std::env::var("KLYNDB_TEST_MYSQL_URL").expect("set a disposable test server URL");
    let db = Arc::new(Mysql::connect(&url, None, false).await.unwrap());
    query(db.clone(),"DROP TABLE IF EXISTS klyndb_child; DROP TABLE IF EXISTS klyndb_items; CREATE TABLE klyndb_items(id BIGINT UNSIGNED PRIMARY KEY, value TEXT, amount DECIMAL(30,8), payload BLOB, doubled DECIMAL(40,8) GENERATED ALWAYS AS (amount*2) STORED); INSERT INTO klyndb_items(id,value,amount,payload) VALUES(18446744073709551615,NULL,1234567890123456789012.12345678,X'00FF')",100).await;
    let tables = db.tables().await.unwrap();
    let table = tables.iter().find(|t| t.name == "klyndb_items").unwrap();
    let info = db.inspect(table).await.unwrap();
    assert!(info.columns[0].primary_key);
    assert!(info.columns[4].generated);
    assert!(!info.indexes.is_empty());
    let encoded=query(db.clone(),"SET character_set_results=latin1; SELECT _latin1 X'FF' AS encoded; SET character_set_results=utf8mb4",100).await;
    assert!(
        encoded
            .iter()
            .any(|b| matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Binary("ff".into())]))
    );
    query(db.clone(), "CREATE TABLE klyndb_child(id INT PRIMARY KEY, parent_id BIGINT UNSIGNED, CONSTRAINT klyndb_parent_fk FOREIGN KEY(parent_id) REFERENCES klyndb_items(id))",100).await;
    let child = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb_child")
        .unwrap();
    assert_eq!(
        db.inspect(&child).await.unwrap().foreign_keys[0]["target"],
        "id"
    );
    let result = query(
        db.clone(),
        &format!(
            "{} SELECT 1 WHERE FALSE; SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3;",
            db.table_select_sql(table, 100).unwrap()
        ),
        2,
    )
    .await;
    assert_eq!(
        result
            .iter()
            .filter(|b| matches!(b, Batch::Columns(_)))
            .count(),
        3
    );
    assert!(result.iter().any(|b|matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Number("18446744073709551615".into()),Cell::Null,Cell::Number("1234567890123456789012.12345678".into()),Cell::Binary("00ff".into()),Cell::Number("2469135780246913578024.24691356".into())])));
    assert!(result.iter().any(|b| matches!(
        b,
        Batch::Complete {
            truncated: true,
            ..
        }
    )));
    query(
        db.clone(),
        "DROP VIEW IF EXISTS klyndb_view; CREATE VIEW klyndb_view AS SELECT id FROM klyndb_items",
        100,
    )
    .await;
    let view = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb_view")
        .unwrap();
    assert!(
        db.inspect(&view)
            .await
            .unwrap()
            .ddl
            .unwrap()
            .contains("VIEW")
    );
    query(db.clone(), "DROP TABLE IF EXISTS `klyndb``quoted`; CREATE TABLE `klyndb``quoted`(id INT); INSERT INTO `klyndb``quoted` VALUES(7)", 100).await;
    let quoted = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb`quoted")
        .unwrap();
    assert!(
        query(db.clone(), &db.table_select_sql(&quoted, 100).unwrap(), 100)
            .await
            .iter()
            .any(|b| matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Number("7".into())]))
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    query(
        db.clone(),
        "BEGIN; UPDATE klyndb_items SET value='pending'",
        100,
    )
    .await;
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    query(db.clone(), "ROLLBACK", 100).await;
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    // Repeated cancellation must leave protocol framing and the user session reusable.
    for _ in 0..3 {
        let token = CancellationToken::new();
        let task_token = token.clone();
        let driver = db.clone();
        let (tx, mut rx) = mpsc::channel(2);
        let task = tokio::spawn(async move {
            driver
                .execute("SELECT SLEEP(30)".into(), tx, task_token, 100)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let began = std::time::Instant::now();
        token.cancel();
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(4), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        drain.await.unwrap();
        assert!(began.elapsed() < std::time::Duration::from_secs(4));
        assert!(
            query(db.clone(), "SELECT 42", 100)
                .await
                .iter()
                .any(|b| matches!(b,Batch::Rows(rows) if rows[0]==vec![Cell::Number("42".into())]))
        );
    }
    let (tx, _rx) = mpsc::channel(16);
    assert!(
        db.execute(
            "SELECT 1; SELECT missing_klyndb_column; SELECT 2".into(),
            tx,
            CancellationToken::new(),
            100
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    assert!(
        query(db.clone(), "SELECT 1", 100)
            .await
            .iter()
            .any(|b| matches!(b, Batch::Rows(_)))
    );
    let ro = Arc::new(Mysql::connect(&url, None, true).await.unwrap());
    query(ro.clone(), "SELECT * FROM klyndb_items", 100).await;
    for sql in [
        "UPDATE klyndb_items SET value='bad'",
        "DROP TABLE klyndb_items",
        "COMMIT; INSERT INTO klyndb_items(id) VALUES(1)",
    ] {
        let (tx, _rx) = mpsc::channel(16);
        assert!(
            ro.execute(sql.into(), tx, CancellationToken::new(), 100)
                .await
                .is_err()
        );
    }
    ro.disconnect().await.unwrap();
    // Default TLS must fail against this explicitly plaintext-only local fixture.
    if url.ends_with("?tls=disabled") {
        assert!(
            Mysql::connect(url.trim_end_matches("?tls=disabled"), None, false)
                .await
                .is_err()
        );
    }
    query(
        db.clone(),
        "DROP VIEW klyndb_view; DROP TABLE klyndb_child; DROP TABLE klyndb_items; DROP TABLE `klyndb``quoted`",
        100,
    )
    .await;
    db.disconnect().await.unwrap();
}
