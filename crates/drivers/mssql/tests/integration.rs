use klyndb_driver_api::*;
use klyndb_mssql::SqlServer;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn query(db: Arc<SqlServer>, sql: &str, limit: usize) -> Result<Vec<Batch>> {
    let (sender, mut receiver) = mpsc::channel(2);
    let sql = sql.to_owned();
    let task = tokio::spawn(async move {
        db.execute(sql, sender, CancellationToken::new(), limit)
            .await
    });
    let mut batches = vec![];
    while let Some(batch) = receiver.recv().await {
        batches.push(batch);
    }
    task.await.unwrap()?;
    Ok(batches)
}
fn rows(batches: &[Batch]) -> Vec<Row> {
    batches
        .iter()
        .filter_map(|b| {
            if let Batch::Rows(r) = b {
                Some(r.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect()
}
async fn answer(db: Arc<SqlServer>) {
    assert_eq!(
        rows(&query(db, "SELECT 42 AS answer", 10).await.unwrap())[0][0].text(),
        "42"
    );
}
#[tokio::test]
#[ignore = "Requires a disposable KLYNDB_TEST_MSSQL_URL server and KLYNDB_TEST_MSSQL_PASSWORD"]
async fn real_sql_server_workflow() {
    let address = std::env::var("KLYNDB_TEST_MSSQL_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let db = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let batches=query(db.clone(),"DECLARE @n bigint = 9223372036854775807; SELECT @n AS maximum, CAST('-0.000000000000000001' AS decimal(38,18)) AS tiny, CAST(NULL AS nvarchar(20)) AS missing, N'é雪''quoted' AS label, 0x00ff AS binary; SELECT CAST('2026-10-03T12:34:56.1234567+02:00' AS datetimeoffset(7)) AS moment; SELECT 1 AS empty WHERE 1=0;",100).await.unwrap();
    assert_eq!(
        batches
            .iter()
            .filter(|b| matches!(b, Batch::Columns(_)))
            .count(),
        3
    );
    assert_eq!(
        batches
            .iter()
            .filter(|b| matches!(b, Batch::Complete { .. }))
            .count(),
        3
    );
    let data = rows(&batches);
    assert_eq!(
        data[0],
        vec![
            Cell::Number("9223372036854775807".into()),
            Cell::Number("-0.000000000000000001".into()),
            Cell::Null,
            Cell::Text("é雪'quoted".into()),
            Cell::Binary("00ff".into())
        ]
    );
    assert!(data[1][0].text().contains("12:34:56.123456700 +02:00"));
    assert!(!db.capabilities().affected_rows);
    let name = format!("klyndb_{}", uuid::Uuid::new_v4().simple());
    query(db.clone(),&format!("CREATE TABLE dbo.[{name}] (id int PRIMARY KEY, label nvarchar(100) NULL, generated int IDENTITY(1,1)); INSERT INTO dbo.[{name}] (id,label) VALUES (1,N'a%_雪'),(2,NULL),(3,N'z');"),100).await.unwrap();
    let table = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
    let info = db.inspect(&table).await.unwrap();
    assert!(info.columns[0].primary_key);
    assert!(info.columns[1].nullable);
    assert!(info.columns[2].generated);
    let sql = db
        .table_query_sql(
            &table,
            &info.columns,
            &TableQuery {
                filters: vec![TableFilter {
                    column: "label".into(),
                    op: FilterOp::Contains,
                    value: "%_雪".into(),
                }],
                sort: vec![],
                limit: 2,
                offset: 0,
            },
        )
        .unwrap();
    assert_eq!(rows(&query(db.clone(), &sql, 10).await.unwrap()).len(), 1);
    let sql = db
        .table_query_sql(
            &table,
            &info.columns,
            &TableQuery {
                filters: vec![],
                sort: vec![],
                limit: 2,
                offset: 1,
            },
        )
        .unwrap();
    assert_eq!(
        rows(&query(db.clone(), &sql, 10).await.unwrap())[0][0].text(),
        "2"
    );
    assert_eq!(
        rows(
            &query(db.clone(), &db.table_select_sql(&table, 2).unwrap(), 10)
                .await
                .unwrap()
        )
        .len(),
        2
    );
    query(db.clone(), "BEGIN TRANSACTION", 10).await.unwrap();
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    query(
        db.clone(),
        &format!("UPDATE dbo.[{name}] SET label=N'changed' WHERE id=1"),
        10,
    )
    .await
    .unwrap();
    query(db.clone(), "ROLLBACK", 10).await.unwrap();
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    let large = "SELECT a.object_id FROM sys.all_objects a CROSS JOIN sys.all_objects b CROSS JOIN sys.all_objects c";
    let batches = query(db.clone(), large, 7).await.unwrap();
    assert_eq!(rows(&batches).len(), 7);
    assert!(batches.iter().any(|b| matches!(
        b,
        Batch::Complete {
            truncated: true,
            ..
        }
    )));
    answer(db.clone()).await;
    assert!(
        query(db.clone(), "SELECT 1/0 AS failure", 10)
            .await
            .is_err()
    );
    answer(db.clone()).await;
    // Cancel an executing native batch, then cancel a producer blocked by backpressure.
    for sql in ["WAITFOR DELAY '00:01:00'; SELECT 1", large] {
        let token = CancellationToken::new();
        let (sender, receiver) = mpsc::channel(1);
        let worker = {
            let db = db.clone();
            let token = token.clone();
            let sql = sql.to_owned();
            tokio::spawn(async move { db.execute(sql, sender, token, 10_000_000).await })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        token.cancel();
        let error = tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.message.contains("cancelled"), "{}", error.message);
        drop(receiver);
        answer(db.clone()).await;
    }
    let (sender, receiver) = mpsc::channel(1);
    let worker = {
        let db = db.clone();
        tokio::spawn(async move {
            db.execute(
                "WAITFOR DELAY '00:01:00'; SELECT 1".into(),
                sender,
                CancellationToken::new(),
                10,
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(receiver);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    answer(db.clone()).await;
    let readonly = Arc::new(
        SqlServer::connect(&address, Some(&password), true)
            .await
            .unwrap(),
    );
    answer(readonly.clone()).await;
    assert!(
        query(readonly.clone(), &format!("DELETE FROM dbo.[{name}]"), 10)
            .await
            .err()
            .unwrap()
            .message
            .contains("read-only")
    );
    readonly.disconnect().await.unwrap();
    query(db.clone(), &format!("DROP TABLE dbo.[{name}]"), 10)
        .await
        .unwrap();
    db.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires KLYNDB_TEST_MSSQL_URL with verified TLS, password and KLYNDB_TEST_TLS_CERT_DIR"]
async fn real_sql_server_verified_tls() {
    let address = std::env::var("KLYNDB_TEST_MSSQL_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let db = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let data = rows(
        &query(
            db.clone(),
            "SELECT encrypt_option FROM sys.dm_exec_connections WHERE session_id=@@SPID",
            10,
        )
        .await
        .unwrap(),
    );
    assert_eq!(data[0][0].text(), "TRUE");
    db.disconnect().await.unwrap();
    let mut url = url::Url::parse(&address).unwrap();
    url.set_host(Some("127.0.0.1")).unwrap();
    assert!(
        SqlServer::connect(url.as_str(), Some(&password), false)
            .await
            .is_err()
    );
    let mut url = url::Url::parse(&address).unwrap();
    let mut options: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k != "sslrootcert")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let dir = std::env::var("KLYNDB_TEST_TLS_CERT_DIR").unwrap();
    options.push(("sslrootcert".into(), format!("{dir}/other-ca.pem")));
    url.query_pairs_mut().clear().extend_pairs(options);
    assert!(
        SqlServer::connect(url.as_str(), Some(&password), false)
            .await
            .is_err()
    );
    url.set_query(None);
    assert!(
        SqlServer::connect(url.as_str(), Some(&password), false)
            .await
            .is_err()
    );
    url.set_query(Some("tls=disabled"));
    let db = Arc::new(
        SqlServer::connect(url.as_str(), Some(&password), false)
            .await
            .unwrap(),
    );
    answer(db.clone()).await;
    let data = rows(
        &query(
            db.clone(),
            "SELECT encrypt_option FROM sys.dm_exec_connections WHERE session_id=@@SPID",
            10,
        )
        .await
        .unwrap(),
    );
    assert_eq!(data[0][0].text(), "FALSE");
    db.disconnect().await.unwrap();
}
