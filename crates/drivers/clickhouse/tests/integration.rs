use klyndb_clickhouse::ClickHouse;
use klyndb_driver_api::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn query(
    session: Arc<ClickHouse>,
    sql: &str,
    limit: usize,
) -> Result<(Vec<String>, Vec<Row>, bool)> {
    let (sender, mut receiver) = mpsc::channel(2);
    let sql = sql.to_owned();
    let worker = tokio::spawn(async move {
        session
            .execute(sql, sender, CancellationToken::new(), limit)
            .await
    });
    let mut columns = vec![];
    let mut rows = vec![];
    let mut truncated = false;
    while let Some(batch) = receiver.recv().await {
        match batch {
            Batch::Columns(c) => columns = c,
            Batch::Rows(r) => rows.extend(r),
            Batch::Complete { truncated: t, .. } => truncated = t,
        }
    }
    worker.await.unwrap()?;
    Ok((columns, rows, truncated))
}
#[tokio::test]
#[ignore = "Requires a disposable KLYNDB_TEST_CLICKHOUSE_URL native TCP server"]
async fn real_clickhouse_workflow() {
    let address = std::env::var("KLYNDB_TEST_CLICKHOUSE_URL").unwrap();
    let db = Arc::new(
        ClickHouse::connect(&address, None, false, None)
            .await
            .unwrap(),
    );
    let (_,data,_)=query(db.clone(),"SELECT toUInt64('18446744073709551615') AS max, toDecimal128('0.000000000000000001',18) AS tiny, toDecimal128('-0.123456789012345678',18) AS negative, CAST(NULL AS Nullable(String)) AS missing, unhex('00ff') AS binary, 'é\\\'quoted' AS label",100).await.unwrap();
    assert_eq!(
        data[0],
        vec![
            Cell::Number("18446744073709551615".into()),
            Cell::Number("0.000000000000000001".into()),
            Cell::Number("-0.123456789012345678".into()),
            Cell::Null,
            Cell::Binary("00ff".into()),
            Cell::Text("é'quoted".into())
        ]
    );
    let (_,data,_)=query(db.clone(),"SELECT toUInt128('340282366920938463463374607431768211455') AS u128, toInt256('-1') AS i256, toUInt256('115792089237316195423570985008687907853269984665640564039457584007913129639935') AS u256, toDecimal256('-0.000001',6) AS d256, toDate('2026-10-03') AS date, toDateTime64('2026-10-03 12:34:56.123456789',9,'UTC') AS ts",100).await.unwrap();
    assert_eq!(data[0][0].text(), "340282366920938463463374607431768211455");
    assert_eq!(data[0][1].text(), "-1");
    assert_eq!(
        data[0][2].text(),
        "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    );
    assert_eq!(data[0][3].text(), "-0.000001");
    assert_eq!(data[0][4].text(), "2026-10-03");
    assert!(data[0][5].text().contains("12:34:56.123456789"));
    assert!(
        query(db.clone(), "SELECT [1,2] AS unsupported", 100)
            .await
            .unwrap_err()
            .message
            .contains("Cast")
    );
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 1).await.unwrap().1[0][0].text(),
        "42"
    );
    let name = format!("klyndb_{}", uuid::Uuid::new_v4().simple());
    query(db.clone(),&format!("CREATE TABLE `{name}` (id UInt64, label Nullable(String)) ENGINE=MergeTree ORDER BY id; INSERT INTO `{name}` VALUES (1,'a'),(2,NULL);"),100).await.unwrap();
    let table = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
    let info = db.inspect(&table).await.unwrap();
    assert!(!info.editable);
    assert!(!info.columns[0].primary_key);
    assert!(info.columns[1].nullable);
    assert!(info.ddl.unwrap().contains("MergeTree"));
    let (_, data, truncated) = query(db.clone(), "SELECT number FROM numbers(1000000)", 7)
        .await
        .unwrap();
    assert_eq!(data.len(), 7);
    assert!(truncated);
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 1).await.unwrap().1[0][0].text(),
        "42"
    );
    let browse = db
        .table_query_sql(
            &table,
            &info.columns,
            &TableQuery {
                filters: vec![TableFilter {
                    column: "label".into(),
                    op: FilterOp::Contains,
                    value: "a".into(),
                }],
                sort: vec![],
                limit: 10,
                offset: 0,
            },
        )
        .unwrap();
    assert_eq!(query(db.clone(), &browse, 10).await.unwrap().1.len(), 1);
    // Cancel while the producer is blocked on a full result channel.
    let token = CancellationToken::new();
    let (sender, receiver) = mpsc::channel(1);
    let blocked = {
        let db = db.clone();
        let token = token.clone();
        tokio::spawn(async move {
            db.execute(
                "SELECT number FROM numbers(1000000000000000000)".into(),
                sender,
                token,
                10_000_000,
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    token.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), blocked)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    drop(receiver);
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 1).await.unwrap().1[0][0].text(),
        "42"
    );
    // Losing a consumer also stops the owned server query without a user cancellation token.
    let (sender, receiver) = mpsc::channel(1);
    let lost = {
        let db = db.clone();
        tokio::spawn(async move {
            db.execute(
                "SELECT sum(number) AS total FROM numbers(1000000000000000000)".into(),
                sender,
                CancellationToken::new(),
                100,
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    drop(receiver);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), lost)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 1).await.unwrap().1[0][0].text(),
        "42"
    );
    let token = CancellationToken::new();
    let (sender, mut receiver) = mpsc::channel(2);
    let worker = {
        let db = db.clone();
        let token = token.clone();
        tokio::spawn(async move {
            db.execute(
                "SELECT sum(number) AS total FROM numbers(1000000000000000000)".into(),
                sender,
                token,
                100,
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    token.cancel();
    while receiver.recv().await.is_some() {}
    assert!(
        worker
            .await
            .unwrap()
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 1).await.unwrap().1[0][0].text(),
        "42"
    );
    let readonly = Arc::new(
        ClickHouse::connect(&address, None, true, None)
            .await
            .unwrap(),
    );
    assert_eq!(
        query(readonly.clone(), "SELECT 42 AS answer", 1)
            .await
            .unwrap()
            .1[0][0]
            .text(),
        "42"
    );
    assert!(query(readonly.clone(), "SET readonly=0", 1).await.is_err());
    assert!(
        query(
            readonly.clone(),
            &format!("INSERT INTO `{name}` VALUES (3,'blocked')"),
            1
        )
        .await
        .is_err()
    );
    readonly.disconnect().await.unwrap();
    query(db.clone(), &format!("DROP TABLE `{name}`"), 1)
        .await
        .unwrap();
    // The pinned client guard must fail rather than overwrite duplicate result columns.
    assert!(
        query(db.clone(), "SELECT 1 AS duplicate, 1 AS duplicate", 10)
            .await
            .is_err()
    );
    assert!(db.transaction_state().await.is_err());
    db.disconnect().await.unwrap();
    let fresh = Arc::new(
        ClickHouse::connect(&address, None, false, None)
            .await
            .unwrap(),
    );
    assert_eq!(
        query(fresh.clone(), "SELECT 42 AS answer", 1)
            .await
            .unwrap()
            .1[0][0]
            .text(),
        "42"
    );
    fresh.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires a disposable native TLS server and KLYNDB_TEST_TLS_CERT_DIR"]
async fn real_clickhouse_verified_tls_and_native_readonly_profile() {
    let address = std::env::var("KLYNDB_TEST_TLS_CLICKHOUSE_URL").unwrap();
    let directory = std::env::var("KLYNDB_TEST_TLS_CERT_DIR").unwrap();
    let db = Arc::new(
        ClickHouse::connect(&address, None, false, None)
            .await
            .unwrap(),
    );
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 100)
            .await
            .unwrap()
            .1[0][0]
            .text(),
        "42"
    );
    db.tables().await.unwrap();
    db.disconnect().await.unwrap();
    let mut wrong = url::Url::parse(&address).unwrap();
    wrong.set_host(Some("127.0.0.1")).unwrap();
    assert!(
        ClickHouse::connect(wrong.as_str(), None, false, None)
            .await
            .is_err()
    );
    let mut wrong = url::Url::parse(&address).unwrap();
    wrong
        .query_pairs_mut()
        .clear()
        .append_pair("tls", "required")
        .append_pair("sslrootcert", &format!("{directory}/other-ca.pem"));
    assert!(
        ClickHouse::connect(wrong.as_str(), None, false, None)
            .await
            .is_err()
    );
    let mut untrusted = url::Url::parse(&address).unwrap();
    untrusted.set_query(None);
    assert!(
        ClickHouse::connect(untrusted.as_str(), None, false, None)
            .await
            .is_err()
    );
    let mut readonly = url::Url::parse(&address).unwrap();
    readonly.set_username("readonly_fixture").unwrap();
    let db = Arc::new(
        ClickHouse::connect(readonly.as_str(), None, true, None)
            .await
            .unwrap(),
    );
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 100)
            .await
            .unwrap()
            .1[0][0]
            .text(),
        "42"
    );
    let (sql, format) = db.explain_sql("SELECT 42 AS answer", false).unwrap();
    assert_eq!(format, PlanFormat::ClickHouseJson);
    assert!(db.capabilities().explain && !db.capabilities().explain_analyze);
    assert!(db.explain_sql("SELECT 42", true).is_err());
    let (_, native, truncated) = query(db.clone(), &sql, 100).await.unwrap();
    assert!(!truncated);
    assert_eq!(native.len(), 1);
    let plan: serde_json::Value = serde_json::from_str(&native[0][0].text()).unwrap();
    assert!(plan[0]["Plan"].is_object());
    assert!(query(db.clone(), "SET readonly=0", 1).await.is_err());
    let token = CancellationToken::new();
    let (sender, mut receiver) = mpsc::channel(2);
    let task = {
        let db = db.clone();
        let token = token.clone();
        tokio::spawn(async move {
            db.execute(
                "SELECT sum(number) AS total FROM numbers(1000000000000000000)".into(),
                sender,
                token,
                100,
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    token.cancel();
    while receiver.recv().await.is_some() {}
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    assert_eq!(
        query(db.clone(), "SELECT 42 AS answer", 100)
            .await
            .unwrap()
            .1[0][0]
            .text(),
        "42"
    );
    db.disconnect().await.unwrap();
}
