use klyndb_driver_api::*;
use klyndb_mssql::SqlServer;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
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
#[ignore = "Requires the disposable tls=disabled SQL Server; fragments real TDS responses before cancellation"]
async fn real_sql_server_partial_token_cancellation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Notify;

    async fn packet(reader: &mut (impl AsyncRead + Unpin)) -> std::io::Result<Option<Vec<u8>>> {
        let first = match reader.read_u8().await {
            Ok(byte) => byte,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut header = [0u8; 8];
        header[0] = first;
        reader.read_exact(&mut header[1..]).await?;
        let length = u16::from_be_bytes([header[2], header[3]]) as usize;
        assert!(length >= 8, "invalid fixture TDS packet length");
        let mut frame = vec![0; length];
        frame[..8].copy_from_slice(&header);
        reader.read_exact(&mut frame[8..]).await?;
        Ok(Some(frame))
    }
    fn set_length(frame: &mut [u8]) {
        let length = u16::try_from(frame.len()).unwrap().to_be_bytes();
        frame[2..4].copy_from_slice(&length);
    }

    let address = std::env::var("KLYNDB_TEST_MSSQL_FRAGMENTED_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let url = url::Url::parse(&address).unwrap();
    assert!(
        url.query_pairs()
            .any(|(key, value)| key == "tls" && value == "disabled"),
        "packet-shaping uses only the disposable plaintext fixture; verified TLS has a separate contract"
    );
    let host = url.host_str().unwrap().to_owned();
    assert!(
        host == "localhost"
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()),
        "fragmentation fixture must stay on loopback"
    );
    let port = url.port().unwrap_or(1433);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let split_at = Arc::new(AtomicUsize::new(0));
    let prefix_sent = Arc::new(Notify::new());
    let attention = Arc::new(Notify::new());
    let proxy = {
        let split_at = split_at.clone();
        let prefix_sent = prefix_sent.clone();
        let attention = attention.clone();
        tokio::spawn(async move {
            let (client, _) = listener.accept().await?;
            let server = TcpStream::connect((host.as_str(), port)).await?;
            let (mut client_read, mut client_write) = client.into_split();
            let (mut server_read, mut server_write) = server.into_split();
            let requests = async {
                while let Some(frame) = packet(&mut client_read).await? {
                    server_write.write_all(&frame).await?;
                    if frame[0] == 0x06 {
                        attention.notify_one();
                    }
                }
                server_write.shutdown().await
            };
            let responses = async {
                let mut id_shift = 0u8;
                while let Some(mut frame) = packet(&mut server_read).await? {
                    let final_packet = frame[1] & 1 != 0;
                    frame[6] = frame[6].wrapping_add(id_shift);
                    let cut = split_at.swap(0, Ordering::SeqCst);
                    if cut != 0 {
                        assert_eq!(frame[0], 0x04, "shape only a native server response");
                        assert!(frame.len() > 8 + cut);
                        let mut prefix = frame[..8 + cut].to_vec();
                        prefix[1] &= !1; // The token continues in the next TDS packet.
                        set_length(&mut prefix);
                        client_write.write_all(&prefix).await?;
                        prefix_sent.notify_one();
                        attention.notified().await;
                        frame.drain(8..8 + cut);
                        set_length(&mut frame);
                        frame[6] = frame[6].wrapping_add(1);
                        id_shift = id_shift.wrapping_add(1);
                    }
                    client_write.write_all(&frame).await?;
                    if final_packet {
                        id_shift = 0;
                    }
                }
                client_write.shutdown().await
            };
            tokio::try_join!(requests, responses)?;
            Ok::<_, std::io::Error>(())
        })
    };
    let db = Arc::new(
        SqlServer::connect_via(&address, Some(&password), false, Some(endpoint))
            .await
            .unwrap(),
    );
    for cut in [1, 2, 5, 7] {
        eprintln!("fragmented response: prefix {cut}");
        split_at.store(cut, Ordering::SeqCst);
        let token = CancellationToken::new();
        let (sender, _receiver) = mpsc::channel(1);
        let worker = {
            let db = db.clone();
            let token = token.clone();
            tokio::spawn(async move {
                db.execute("SELECT 1 AS value".into(), sender, token, 10)
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(5), prefix_sent.notified())
            .await
            .unwrap();
        // The reader has a complete short packet and waits on the next field;
        // only the outgoing Attention unlocks the rest of that actual response.
        tokio::time::sleep(Duration::from_millis(20)).await;
        token.cancel();
        let error = tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.message, "Query cancelled", "partial token at {cut}");
        answer(db.clone()).await;
    }
    db.disconnect().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), proxy)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

async fn import_batches(
    db: Arc<SqlServer>,
    table: Table,
    batches: Vec<Result<InsertBatch>>,
) -> Result<MutationResult> {
    let (sender, receiver) = mpsc::channel(1);
    let producer = tokio::spawn(async move {
        for batch in batches {
            if sender.send(batch).await.is_err() {
                break;
            }
        }
    });
    let result = db
        .insert_stream(table, receiver, CancellationToken::new())
        .await;
    producer.await.unwrap();
    result
}

#[tokio::test]
#[ignore = "Requires a disposable SQL Server; checks complete-stream rollback and native cancellation"]
async fn real_sql_server_imports() {
    let address = std::env::var("KLYNDB_TEST_MSSQL_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let db = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let probe = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("klyndb_import_{suffix}");
    query(db.clone(),&format!("CREATE TABLE dbo.[{name}](id bigint PRIMARY KEY,label nvarchar(100),amount decimal(38,18),payload varbinary(32),flag bit,seq int IDENTITY)"),10).await.unwrap();
    let table = Table {
        schema: "dbo".into(),
        name: name.clone(),
        kind: "table".into(),
    };
    let insert = |id: i64| Change::Insert {
        values: BTreeMap::from([
            ("id".into(), Cell::Number(id.to_string())),
            ("label".into(), Cell::Text("é雪, \"quoted\"\nnext".into())),
            (
                "amount".into(),
                Cell::Number("12345678901234567890.123456789012345678".into()),
            ),
            ("payload".into(), Cell::Binary("00ff".into())),
            ("flag".into(), Cell::Boolean(true)),
        ]),
    };
    assert!(db.capabilities().import_rows);
    let result = import_batches(
        db.clone(),
        table.clone(),
        vec![
            Ok(InsertBatch::Rows((1..=256).map(insert).collect())),
            Ok(InsertBatch::Rows((257..=300).map(insert).collect())),
            Ok(InsertBatch::Complete),
        ],
    )
    .await
    .unwrap();
    assert!(result.affected == 300 && !result.pending_transaction);
    let count = format!("SELECT COUNT_BIG(*) FROM dbo.[{name}]");
    assert_eq!(
        rows(&query(db.clone(), &count, 10).await.unwrap())[0][0].text(),
        "300"
    );
    let first = rows(
        &query(
            db.clone(),
            &format!("SELECT * FROM dbo.[{name}] WHERE id=1"),
            10,
        )
        .await
        .unwrap(),
    )
    .remove(0);
    assert_eq!(
        &first[..5],
        &[
            Cell::Number("1".into()),
            Cell::Text("é雪, \"quoted\"\nnext".into()),
            Cell::Number("12345678901234567890.123456789012345678".into()),
            Cell::Binary("00ff".into()),
            Cell::Boolean(true)
        ]
    );
    for batches in [
        vec![
            Ok(InsertBatch::Rows(vec![insert(301)])),
            Err(Error::new("Fixture parser stopped")),
        ],
        vec![Ok(InsertBatch::Rows(vec![insert(301)]))], // EOF without explicit Complete must never commit.
        vec![
            Ok(InsertBatch::Rows(vec![insert(301)])),
            Ok(InsertBatch::Rows(vec![insert(1)])),
            Ok(InsertBatch::Complete),
        ],
        vec![
            Ok(InsertBatch::Rows(vec![insert(301)])),
            Ok(InsertBatch::Rows(vec![Change::Update {
                old: first.clone(),
                values: BTreeMap::from([("label".into(), Cell::Text("not an append".into()))]),
            }])),
            Ok(InsertBatch::Complete),
        ],
    ] {
        assert!(
            import_batches(db.clone(), table.clone(), batches)
                .await
                .is_err()
        );
        assert_eq!(
            rows(&query(db.clone(), &count, 10).await.unwrap())[0][0].text(),
            "300"
        );
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        answer(db.clone()).await;
    }
    query(
        db.clone(),
        &format!("BEGIN TRANSACTION; INSERT INTO dbo.[{name}](id) VALUES(400)"),
        10,
    )
    .await
    .unwrap();
    assert!(
        import_batches(
            db.clone(),
            table.clone(),
            vec![
                Ok(InsertBatch::Rows(vec![insert(401)])),
                Err(Error::new("late parse failure"))
            ]
        )
        .await
        .is_err()
    );
    assert_eq!(
        rows(&query(db.clone(), &count, 10).await.unwrap())[0][0].text(),
        "301"
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    let result = import_batches(
        db.clone(),
        table.clone(),
        vec![
            Ok(InsertBatch::Rows(vec![insert(401)])),
            Ok(InsertBatch::Complete),
        ],
    )
    .await
    .unwrap();
    assert!(result.pending_transaction && result.affected == 1);
    query(db.clone(), "ROLLBACK; SET IMPLICIT_TRANSACTIONS ON", 10)
        .await
        .unwrap();
    let result = import_batches(
        db.clone(),
        table.clone(),
        vec![
            Ok(InsertBatch::Rows(vec![insert(501)])),
            Ok(InsertBatch::Complete),
        ],
    )
    .await
    .unwrap();
    assert!(result.pending_transaction && result.affected == 1);
    assert_eq!(
        rows(&query(db.clone(), "SELECT @@TRANCOUNT", 10).await.unwrap())[0][0].text(),
        "1"
    );
    query(db.clone(), "ROLLBACK; SET IMPLICIT_TRANSACTIONS OFF", 10)
        .await
        .unwrap();
    query(db.clone(),&format!("EXEC(N'CREATE TRIGGER dbo.[tr_{suffix}] ON dbo.[{name}] AFTER INSERT AS BEGIN IF EXISTS(SELECT 1 FROM inserted WHERE id=999) WAITFOR DELAY ''00:02:00''; END')"),10).await.unwrap();
    // Observe actual uncommitted inserts through a separate dirty-read probe before cancelling.
    for active_request in [false, true] {
        let (sender, receiver) = mpsc::channel(1);
        let token = CancellationToken::new();
        let task = {
            let db = db.clone();
            let table = table.clone();
            let token = token.clone();
            tokio::spawn(async move { db.insert_stream(table, receiver, token).await })
        };
        sender
            .send(Ok(InsertBatch::Rows(if active_request {
                vec![insert(301), insert(999)]
            } else {
                vec![insert(301)]
            })))
            .await
            .unwrap();
        let observed = if active_request { 999 } else { 301 };
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if rows(
                    &query(
                        probe.clone(),
                        &format!("SELECT id FROM dbo.[{name}] WITH (NOLOCK) WHERE id={observed}"),
                        10,
                    )
                    .await
                    .unwrap(),
                )
                .len()
                    == 1
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut queued = {
            let db = db.clone();
            tokio::spawn(async move { query(db, "SELECT 42", 10).await })
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut queued)
                .await
                .is_err()
        );
        token.cancel();
        let error = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.message.contains("cancelled"), "{}", error.message);
        drop(sender);
        assert_eq!(rows(&queued.await.unwrap().unwrap())[0][0].text(), "42");
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        assert_eq!(
            rows(&query(db.clone(), &count, 10).await.unwrap())[0][0].text(),
            "300"
        );
    }
    let readonly = Arc::new(
        SqlServer::connect(&address, Some(&password), true)
            .await
            .unwrap(),
    );
    assert!(!readonly.capabilities().import_rows);
    assert!(
        import_batches(
            readonly.clone(),
            table,
            vec![
                Ok(InsertBatch::Rows(vec![insert(601)])),
                Ok(InsertBatch::Complete)
            ]
        )
        .await
        .unwrap_err()
        .message
        .contains("read-only")
    );
    readonly.disconnect().await.unwrap();
    query(db.clone(), &format!("DROP TABLE dbo.[{name}]"), 10)
        .await
        .unwrap();
    probe.disconnect().await.unwrap();
    db.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires a disposable KLYNDB_TEST_MSSQL_URL server and KLYNDB_TEST_MSSQL_PASSWORD"]
async fn real_sql_server_editing() {
    let address = std::env::var("KLYNDB_TEST_MSSQL_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let db = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("klyndb_edit_{suffix}");
    let audit = format!("klyndb_audit_{suffix}");
    // Dynamic DDL preserves SQL Server's computed-column/trigger syntax in the shared parser.
    query(db.clone(), &format!("EXEC(N'CREATE TABLE dbo.[{name}](id bigint PRIMARY KEY DEFAULT -1,label nvarchar(100) NULL DEFAULT N''default'',amount decimal(38,18) NULL,payload varbinary(32) NULL,flag bit NULL,seq int IDENTITY,computed AS LEN(label))'); CREATE TABLE dbo.[{audit}](id bigint); EXEC(N'CREATE TRIGGER dbo.[tr_{suffix}] ON dbo.[{name}] AFTER INSERT AS BEGIN INSERT INTO dbo.[{audit}] SELECT id FROM inserted; END')"), 10).await.unwrap();
    let table = Table {
        schema: "dbo".into(),
        name: name.clone(),
        kind: "table".into(),
    };
    let info = db.inspect(&table).await.unwrap();
    assert!(info.editable && db.capabilities().edit_rows);
    assert_eq!(info.columns[2].data_type, "decimal(38,18)");
    assert!(info.columns[5].generated && info.columns[6].generated);
    let select = format!("SELECT * FROM dbo.[{name}] WHERE id=9223372036854775807");
    let label = "é雪'; DELETE FROM unrelated; --";
    let insert = Change::Insert {
        values: BTreeMap::from([
            ("id".into(), Cell::Number("9223372036854775807".into())),
            ("label".into(), Cell::Text(label.into())),
            (
                "amount".into(),
                Cell::Number("12345678901234567890.123456789012345678".into()),
            ),
            ("payload".into(), Cell::Binary("00ff".into())),
            ("flag".into(), Cell::Boolean(true)),
        ]),
    };
    let result = db.apply_changes(table.clone(), vec![insert]).await.unwrap();
    assert_eq!(result.affected, 1); // The AFTER trigger also inserted; its count must be excluded.
    assert!(!result.pending_transaction);
    let original = rows(&query(db.clone(), &select, 10).await.unwrap()).remove(0);
    assert_eq!(
        &original[..5],
        &[
            Cell::Number("9223372036854775807".into()),
            Cell::Text(label.into()),
            Cell::Number("12345678901234567890.123456789012345678".into()),
            Cell::Binary("00ff".into()),
            Cell::Boolean(true),
        ]
    );
    assert_eq!(
        rows(
            &query(
                db.clone(),
                &format!("SELECT COUNT_BIG(*) FROM dbo.[{audit}]"),
                10
            )
            .await
            .unwrap()
        )[0][0]
            .text(),
        "1"
    );
    query(db.clone(), "SET ANSI_WARNINGS OFF; SET ARITHABORT OFF", 10)
        .await
        .unwrap();
    let options = rows(&query(db.clone(), "SELECT @@OPTIONS", 10).await.unwrap());
    db.apply_changes(
        table.clone(),
        vec![Change::Update {
            old: original.clone(),
            values: BTreeMap::from([
                ("label".into(), Cell::Null),
                ("amount".into(), Cell::Number("0.000000000000000001".into())),
            ]),
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        rows(&query(db.clone(), "SELECT @@OPTIONS", 10).await.unwrap()),
        options
    );
    let current = rows(&query(db.clone(), &select, 10).await.unwrap()).remove(0);
    assert_eq!(current[1], Cell::Null);
    assert_eq!(current[2], Cell::Number("0.000000000000000001".into()));
    let duplicate = db
        .apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: BTreeMap::from([("id".into(), current[0].clone())]),
            }],
        )
        .await
        .unwrap_err();
    assert!(duplicate.message.contains("2627"), "{}", duplicate.message);
    answer(db.clone()).await;
    let error = db
        .apply_changes(
            table.clone(),
            vec![
                Change::Insert {
                    values: BTreeMap::from([("id".into(), Cell::Number("7".into()))]),
                },
                Change::Delete {
                    old: original.clone(),
                },
            ],
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("Row changed"), "{}", error.message);
    assert!(
        rows(
            &query(
                db.clone(),
                &format!("SELECT id FROM dbo.[{name}] WHERE id=7"),
                10
            )
            .await
            .unwrap()
        )
        .is_empty()
    );
    assert_eq!(
        rows(
            &query(
                db.clone(),
                &format!("SELECT COUNT_BIG(*) FROM dbo.[{audit}]"),
                10
            )
            .await
            .unwrap()
        )[0][0]
            .text(),
        "1"
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    // Failed edits must preserve earlier caller work; successful edits remain pending until rollback.
    query(
        db.clone(),
        &format!("BEGIN TRANSACTION; INSERT INTO dbo.[{audit}] VALUES(99)"),
        10,
    )
    .await
    .unwrap();
    assert!(
        db.apply_changes(table.clone(), vec![Change::Delete { old: original }])
            .await
            .is_err()
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    assert_eq!(
        rows(
            &query(
                db.clone(),
                &format!("SELECT COUNT_BIG(*) FROM dbo.[{audit}] WHERE id=99"),
                10
            )
            .await
            .unwrap()
        )[0][0]
            .text(),
        "1"
    );
    let result = db
        .apply_changes(
            table.clone(),
            vec![Change::Update {
                old: current.clone(),
                values: BTreeMap::from([("label".into(), Cell::Text("pending".into()))]),
            }],
        )
        .await
        .unwrap();
    assert!(result.pending_transaction && result.affected == 1);
    query(db.clone(), "ROLLBACK", 10).await.unwrap();
    assert_eq!(
        rows(&query(db.clone(), &select, 10).await.unwrap())[0],
        current
    );
    for (column, value) in [
        ("seq", Cell::Number("42".into())),
        ("computed", Cell::Number("42".into())),
        ("amount", Cell::Number("0.1234567890123456789".into())),
        ("label", Cell::Text("x".repeat(101))),
        ("payload", Cell::Binary("ff".repeat(33))),
    ] {
        assert!(
            db.apply_changes(
                table.clone(),
                vec![Change::Update {
                    old: current.clone(),
                    values: BTreeMap::from([(column.into(), value)]),
                }]
            )
            .await
            .is_err(),
            "Unexpectedly accepted {column}"
        );
        assert_eq!(
            rows(&query(db.clone(), &select, 10).await.unwrap())[0],
            current
        );
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
    }
    // Native defaults and implicit-transaction mode also use the same review/apply path.
    query(db.clone(), "SET IMPLICIT_TRANSACTIONS ON", 10)
        .await
        .unwrap();
    let result = db
        .apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: BTreeMap::new(),
            }],
        )
        .await
        .unwrap();
    assert!(result.pending_transaction && result.affected == 1);
    assert_eq!(
        rows(&query(db.clone(), "SELECT @@TRANCOUNT", 10).await.unwrap())[0][0].text(),
        "1"
    );
    assert_eq!(
        rows(
            &query(
                db.clone(),
                &format!("SELECT label FROM dbo.[{name}] WHERE id=-1"),
                10
            )
            .await
            .unwrap()
        )[0][0]
            .text(),
        "default"
    );
    query(db.clone(), "ROLLBACK; SET IMPLICIT_TRANSACTIONS OFF", 10)
        .await
        .unwrap();
    let readonly = Arc::new(
        SqlServer::connect(&address, Some(&password), true)
            .await
            .unwrap(),
    );
    assert!(
        !readonly.capabilities().edit_rows && !readonly.inspect(&table).await.unwrap().editable
    );
    assert!(
        readonly
            .apply_changes(
                table.clone(),
                vec![Change::Delete {
                    old: current.clone()
                }]
            )
            .await
            .unwrap_err()
            .message
            .contains("read-only")
    );
    readonly.disconnect().await.unwrap();
    let result = db
        .apply_changes(table.clone(), vec![Change::Delete { old: current }])
        .await
        .unwrap();
    assert_eq!(result.affected, 1);
    assert!(rows(&query(db.clone(), &select, 10).await.unwrap()).is_empty());
    query(db.clone(), &format!("EXEC(N'CREATE TRIGGER dbo.[instead_{suffix}] ON dbo.[{name}] INSTEAD OF INSERT AS BEGIN SET NOCOUNT ON; END')"), 10).await.unwrap();
    assert!(!db.inspect(&table).await.unwrap().editable);
    assert!(
        db.apply_changes(
            table,
            vec![Change::Insert {
                values: BTreeMap::new()
            }]
        )
        .await
        .is_err()
    );
    answer(db.clone()).await;
    query(
        db.clone(),
        &format!("DROP TABLE dbo.[{name}]; DROP TABLE dbo.[{audit}]"),
        10,
    )
    .await
    .unwrap();
    db.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires a disposable SQL Server; verifies the actual 60-second edit deadline"]
async fn real_sql_server_editing_deadline() {
    let address = std::env::var("KLYNDB_TEST_MSSQL_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let db = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("klyndb_deadline_{suffix}");
    query(db.clone(), &format!("CREATE TABLE dbo.[{name}](id int PRIMARY KEY,label nvarchar(30)); INSERT INTO dbo.[{name}] VALUES(1,N'before'); EXEC(N'CREATE TRIGGER dbo.[tr_{suffix}] ON dbo.[{name}] AFTER UPDATE AS BEGIN WAITFOR DELAY ''00:02:00''; END')"), 10).await.unwrap();
    let table = Table {
        schema: "dbo".into(),
        name: name.clone(),
        kind: "table".into(),
    };
    let before = rows(
        &query(db.clone(), &format!("SELECT * FROM dbo.[{name}]"), 10)
            .await
            .unwrap(),
    )
    .remove(0);
    let start = std::time::Instant::now();
    let error = db
        .apply_changes(
            table,
            vec![
                Change::Insert {
                    values: BTreeMap::from([("id".into(), Cell::Number("2".into()))]),
                },
                Change::Update {
                    old: before.clone(),
                    values: BTreeMap::from([("label".into(), Cell::Text("after".into()))]),
                },
            ],
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("timed out"), "{}", error.message);
    assert!(
        start.elapsed() >= Duration::from_secs(59) && start.elapsed() < Duration::from_secs(68)
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    assert_eq!(
        rows(
            &query(
                db.clone(),
                &format!("SELECT * FROM dbo.[{name}] ORDER BY id"),
                10
            )
            .await
            .unwrap()
        ),
        vec![before]
    );
    answer(db.clone()).await;
    query(db.clone(), &format!("DROP TABLE dbo.[{name}]"), 10)
        .await
        .unwrap();
    db.disconnect().await.unwrap();
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
#[ignore = "Requires a disposable KLYNDB_TEST_MSSQL_URL server and KLYNDB_TEST_MSSQL_PASSWORD"]
async fn real_sql_server_catalog() {
    let address = std::env::var("KLYNDB_TEST_MSSQL_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let db = Arc::new(
        SqlServer::connect(&address, Some(&password), false)
            .await
            .unwrap(),
    );
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let parent = format!("klyndb_parent_{suffix}");
    let child = format!("klyndb_child_{suffix}");
    query(db.clone(), &format!("CREATE TABLE dbo.[{parent}](a int,b int,PRIMARY KEY(a,b)); CREATE TABLE dbo.[{child}](id int PRIMARY KEY,a int,b int,label nvarchar(30) DEFAULT N'hello',CONSTRAINT [fk_{suffix}] FOREIGN KEY(a,b) REFERENCES dbo.[{parent}](a,b),CONSTRAINT [ck_{suffix}] CHECK(id>0),CONSTRAINT [uq_{suffix}] UNIQUE(label))"), 10).await.unwrap();
    query(db.clone(), &format!("EXEC(N'CREATE TRIGGER dbo.[tr_{suffix}] ON dbo.[{child}] AFTER INSERT AS BEGIN SET NOCOUNT ON; END;')"), 10).await.unwrap();
    let table = Table {
        schema: "dbo".into(),
        name: child.clone(),
        kind: "table".into(),
    };
    let info = db.inspect(&table).await.unwrap();
    assert_eq!(info.foreign_keys.len(), 2);
    for (index, column) in ["a", "b"].iter().enumerate() {
        let key = &info.foreign_keys[index];
        assert_eq!(key["name"], format!("fk_{suffix}"));
        assert_eq!(key["column"], *column);
        assert_eq!(key["target"], *column);
        assert_eq!(key["table"], parent);
        assert_eq!(key["schema"], "dbo");
    }
    let constraints = info.constraints.unwrap();
    for kind in [
        "PRIMARY_KEY_CONSTRAINT",
        "UNIQUE_CONSTRAINT",
        "FOREIGN_KEY_CONSTRAINT",
        "CHECK_CONSTRAINT",
        "DEFAULT_CONSTRAINT",
    ] {
        assert!(constraints.iter().any(|c| c.kind == kind), "Missing {kind}");
    }
    assert!(
        constraints
            .iter()
            .find(|c| c.name == format!("ck_{suffix}"))
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .contains("id")
    );
    assert!(
        constraints
            .iter()
            .find(|c| c.kind == "DEFAULT_CONSTRAINT")
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .contains("hello")
    );
    assert_eq!(info.triggers.len(), 1);
    assert_eq!(info.triggers[0].name, format!("tr_{suffix}"));
    assert_eq!(info.triggers[0].state.as_deref(), Some("enabled · AFTER"));
    assert!(info.triggers[0].definition.contains("SET NOCOUNT ON"));
    query(
        db.clone(),
        &format!("EXEC(N'DISABLE TRIGGER dbo.[tr_{suffix}] ON dbo.[{child}]')"),
        10,
    )
    .await
    .unwrap();
    assert_eq!(
        db.inspect(&table).await.unwrap().triggers[0]
            .state
            .as_deref(),
        Some("disabled · AFTER")
    );
    query(
        db.clone(),
        &format!("DROP TABLE dbo.[{child}]; DROP TABLE dbo.[{parent}]"),
        10,
    )
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
