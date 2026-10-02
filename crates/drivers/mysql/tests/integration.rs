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
        "SELECT 1; /*! COMMIT */; /*! UPDATE klyndb_items SET value='hidden write' */",
        "SELECT 1; /*M! COMMIT */; /*M! UPDATE klyndb_items SET value='hidden write' */",
    ] {
        let (tx, _rx) = mpsc::channel(16);
        assert!(
            ro.execute(sql.into(), tx, CancellationToken::new(), 100)
                .await
                .is_err(),
            "read-only execution accepted: {sql}"
        );
    }
    assert_eq!(
        rows(db.clone(), "SELECT value FROM klyndb_items").await,
        vec![vec![Cell::Null]]
    );
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

async fn rows(db: Arc<Mysql>, sql: &str) -> Vec<klyndb_driver_api::Row> {
    query(db, sql, 100)
        .await
        .into_iter()
        .filter_map(|b| match b {
            Batch::Rows(rows) => Some(rows),
            _ => None,
        })
        .flatten()
        .collect()
}
fn values(items: &[(&str, Cell)]) -> std::collections::BTreeMap<String, Cell> {
    items
        .iter()
        .map(|(name, cell)| ((*name).into(), cell.clone()))
        .collect()
}
fn text(v: &str) -> Cell {
    Cell::Text(v.into())
}
fn number(v: &str) -> Cell {
    Cell::Number(v.into())
}
#[tokio::test]
#[ignore = "requires a disposable MySQL or MariaDB server and KLYNDB_TEST_MYSQL_URL"]
async fn real_atomic_editing() {
    use klyndb_driver_api::Change;
    let url = std::env::var("KLYNDB_TEST_MYSQL_URL").unwrap();
    let db = Arc::new(Mysql::connect(&url, None, false).await.unwrap());
    query(db.clone(), "DROP TABLE IF EXISTS klyndb_edits; CREATE TABLE klyndb_edits(id BIGINT UNSIGNED PRIMARY KEY, value VARCHAR(100) COLLATE utf8mb4_general_ci DEFAULT 'default', amount DECIMAL(30,8), payload BLOB, doc JSON, legacy VARCHAR(100) CHARACTER SET latin1, happened DATETIME(6), ratio DOUBLE, doubled DECIMAL(40,8) GENERATED ALWAYS AS (amount*2) STORED) ENGINE=InnoDB",100).await;
    let table = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb_edits")
        .unwrap();
    assert!(db.inspect(&table).await.unwrap().editable);
    let insert = values(&[
        ("id", number("18446744073709551615")),
        ("value", text("O'Reilly; DROP TABLE x --")),
        ("amount", number("1234567890123456789012.12345678")),
        ("payload", Cell::Binary("00ff".into())),
        ("doc", Cell::Json(serde_json::json!({"b":2,"a":1}))),
        ("legacy", text("café")),
        ("happened", text("2026-10-02 12:34:56.123456")),
        ("ratio", number("1.25")),
    ]);
    let result = db
        .apply_changes(table.clone(), vec![Change::Insert { values: insert }])
        .await
        .unwrap();
    assert_eq!(result.affected, 1);
    assert!(!result.pending_transaction);
    let old = rows(
        db.clone(),
        "SELECT * FROM klyndb_edits WHERE id=18446744073709551615",
    )
    .await
    .remove(0);
    assert_eq!(old[2], number("1234567890123456789012.12345678"));
    assert_eq!(old[3], Cell::Binary("00ff".into()));
    assert_eq!(old[5], text("café"));
    // No-op updates still match exactly one row, including exact numeric, JSON, generated and encoded values.
    db.apply_changes(
        table.clone(),
        vec![Change::Update {
            old: old.clone(),
            values: values(&[("value", old[1].clone())]),
        }],
    )
    .await
    .unwrap();
    db.apply_changes(
        table.clone(),
        vec![Change::Insert {
            values: values(&[("id", number("18446744073709551614"))]),
        }],
    )
    .await
    .unwrap();
    db.apply_changes(
        table.clone(),
        vec![Change::Update {
            old: old.clone(),
            values: values(&[("value", Cell::Null)]),
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        rows(db.clone(), "SELECT value FROM klyndb_edits ORDER BY id").await,
        vec![vec![text("default")], vec![Cell::Null]]
    );
    // Stale old values roll back earlier successful statements in the same batch.
    assert!(
        db.apply_changes(
            table.clone(),
            vec![
                Change::Insert {
                    values: values(&[("id", number("3"))])
                },
                Change::Delete { old: old.clone() }
            ]
        )
        .await
        .is_err()
    );
    assert!(
        rows(db.clone(), "SELECT id FROM klyndb_edits WHERE id=3")
            .await
            .is_empty()
    );
    assert!(
        db.apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: values(&[("id", number("3")), ("doubled", number("1"))])
            }]
        )
        .await
        .is_err()
    );
    // An external change equal under the column's collation must still cause an optimistic conflict.
    query(
        db.clone(),
        "UPDATE klyndb_edits SET value='Case' WHERE id=18446744073709551615",
        100,
    )
    .await;
    let stale = rows(
        db.clone(),
        "SELECT * FROM klyndb_edits WHERE id=18446744073709551615",
    )
    .await
    .remove(0);
    let other = Arc::new(Mysql::connect(&url, None, false).await.unwrap());
    query(
        other.clone(),
        "UPDATE klyndb_edits SET value='case ' WHERE id=18446744073709551615",
        100,
    )
    .await;
    assert!(
        db.apply_changes(table.clone(), vec![Change::Delete { old: stale }])
            .await
            .is_err()
    );
    assert_eq!(
        rows(
            db.clone(),
            "SELECT value FROM klyndb_edits WHERE id=18446744073709551615"
        )
        .await,
        vec![vec![text("case ")]]
    );
    // Savepoint rollback preserves the caller's earlier uncommitted work.
    query(
        db.clone(),
        "BEGIN; INSERT INTO klyndb_edits(id) VALUES(10)",
        100,
    )
    .await;
    assert!(
        db.apply_changes(
            table.clone(),
            vec![
                Change::Insert {
                    values: values(&[("id", number("11"))])
                },
                Change::Delete { old: old.clone() }
            ]
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Active
    );
    assert_eq!(
        rows(db.clone(), "SELECT id FROM klyndb_edits WHERE id IN(10,11)").await,
        vec![vec![number("10")]]
    );
    let result = db
        .apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: values(&[("id", number("12"))]),
            }],
        )
        .await
        .unwrap();
    assert!(result.pending_transaction);
    assert!(
        rows(
            other.clone(),
            "SELECT id FROM klyndb_edits WHERE id IN(10,12)"
        )
        .await
        .is_empty()
    );
    query(db.clone(), "ROLLBACK", 100).await;
    assert!(
        rows(db.clone(), "SELECT id FROM klyndb_edits WHERE id IN(10,12)")
            .await
            .is_empty()
    );
    query(db.clone(), "SET autocommit=0", 100).await;
    assert!(
        db.apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: values(&[("id", number("13"))])
            }]
        )
        .await
        .unwrap()
        .pending_transaction
    );
    query(db.clone(), "ROLLBACK; SET autocommit=1", 100).await;
    assert!(
        rows(db.clone(), "SELECT id FROM klyndb_edits WHERE id=13")
            .await
            .is_empty()
    );
    query(db.clone(), "SET NAMES latin1", 100).await;
    assert!(
        db.apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: values(&[("id", number("15")), ("legacy", text("café"))])
            }]
        )
        .await
        .unwrap_err()
        .message
        .contains("UTF-8")
    );
    query(db.clone(), "SET NAMES utf8mb4", 100).await;
    assert!(
        rows(db.clone(), "SELECT id FROM klyndb_edits WHERE id=15")
            .await
            .is_empty()
    );
    // Non-strict servers must not silently truncate staged values.
    query(db.clone(), "SET SESSION sql_mode=''", 100).await;
    assert!(
        db.apply_changes(
            table.clone(),
            vec![Change::Insert {
                values: values(&[("id", number("14")), ("value", text(&"x".repeat(101)))])
            }]
        )
        .await
        .is_err()
    );
    assert!(
        rows(db.clone(), "SELECT id FROM klyndb_edits WHERE id=14")
            .await
            .is_empty()
    );
    let current = rows(
        db.clone(),
        "SELECT * FROM klyndb_edits WHERE id=18446744073709551615",
    )
    .await
    .remove(0);
    db.apply_changes(table.clone(), vec![Change::Delete { old: current }])
        .await
        .unwrap();
    assert_eq!(
        rows(db.clone(), "SELECT id FROM klyndb_edits").await,
        vec![vec![number("18446744073709551614")]]
    );
    query(db.clone(),"DROP VIEW IF EXISTS klyndb_edit_view; CREATE VIEW klyndb_edit_view AS SELECT * FROM klyndb_edits; DROP TABLE IF EXISTS klyndb_myisam; CREATE TABLE klyndb_myisam(id INT PRIMARY KEY) ENGINE=MyISAM; DROP TABLE IF EXISTS `klyndb``edit`; CREATE TABLE `klyndb``edit`(id INT AUTO_INCREMENT PRIMARY KEY, `val``ue` TEXT DEFAULT NULL, created TIMESTAMP DEFAULT CURRENT_TIMESTAMP) ENGINE=InnoDB",100).await;
    for name in ["klyndb_edit_view", "klyndb_myisam"] {
        let t = db
            .tables()
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap();
        assert!(!db.inspect(&t).await.unwrap().editable);
        assert!(
            db.apply_changes(
                t,
                vec![Change::Insert {
                    values: values(&[("id", number("1"))])
                }]
            )
            .await
            .is_err()
        );
    }
    let quoted = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb`edit")
        .unwrap();
    assert!(!db.inspect(&quoted).await.unwrap().columns[2].generated);
    db.apply_changes(
        quoted.clone(),
        vec![Change::Insert {
            values: values(&[]),
        }],
    )
    .await
    .unwrap();
    let old = rows(db.clone(), "SELECT * FROM `klyndb``edit`")
        .await
        .remove(0);
    db.apply_changes(
        quoted.clone(),
        vec![Change::Update {
            old,
            values: values(&[("val`ue", text("bound literal"))]),
        }],
    )
    .await
    .unwrap();
    let ro = Mysql::connect(&url, None, true).await.unwrap();
    assert!(
        ro.apply_changes(
            table,
            vec![Change::Insert {
                values: values(&[("id", number("20"))])
            }]
        )
        .await
        .is_err()
    );
    ro.disconnect().await.unwrap();
    other.disconnect().await.unwrap();
    query(db.clone(),"DROP VIEW klyndb_edit_view; DROP TABLE klyndb_edits; DROP TABLE klyndb_myisam; DROP TABLE `klyndb``edit`",100).await;
    db.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable MySQL/MariaDB server; deliberately tests the 60-second edit deadline"]
async fn real_edit_timeout_and_incomplete_rollback() {
    use klyndb_driver_api::Change;
    let url = std::env::var("KLYNDB_TEST_MYSQL_URL").unwrap();
    let db = Arc::new(Mysql::connect(&url, None, false).await.unwrap());
    let blocker = Arc::new(Mysql::connect(&url, None, false).await.unwrap());
    query(db.clone(),"DROP TABLE IF EXISTS klyndb_edit_deadline; CREATE TABLE klyndb_edit_deadline(id INT PRIMARY KEY, value TEXT) ENGINE=InnoDB; INSERT INTO klyndb_edit_deadline VALUES(1,'original'); SET SESSION innodb_lock_wait_timeout=120",100).await;
    let table = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb_edit_deadline")
        .unwrap();
    let old = rows(db.clone(), "SELECT * FROM klyndb_edit_deadline")
        .await
        .remove(0);
    query(
        blocker.clone(),
        "BEGIN; UPDATE klyndb_edit_deadline SET value='locked' WHERE id=1",
        100,
    )
    .await;
    let began = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(65),
        db.apply_changes(
            table,
            vec![
                Change::Insert {
                    values: values(&[("id", number("2"))]),
                },
                Change::Update {
                    old,
                    values: values(&[("value", text("timeout"))]),
                },
            ],
        ),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(began.elapsed() >= std::time::Duration::from_secs(59));
    query(blocker.clone(), "ROLLBACK", 100).await;
    assert_eq!(
        db.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    assert_eq!(
        rows(db.clone(), "SELECT * FROM klyndb_edit_deadline").await,
        vec![vec![number("1"), text("original")]]
    );
    query(db.clone(),"DROP TABLE klyndb_edit_deadline; DROP TABLE IF EXISTS klyndb_edit_unsafe; DROP TABLE IF EXISTS klyndb_edit_audit; CREATE TABLE klyndb_edit_unsafe(id INT PRIMARY KEY) ENGINE=InnoDB; CREATE TABLE klyndb_edit_audit(id INT) ENGINE=MyISAM; CREATE TRIGGER klyndb_edit_audit_trigger AFTER INSERT ON klyndb_edit_unsafe FOR EACH ROW INSERT INTO klyndb_edit_audit VALUES(NEW.id)",100).await;
    let table = db
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "klyndb_edit_unsafe")
        .unwrap();
    query(db.clone(), "BEGIN", 100).await;
    let error = db
        .apply_changes(
            table,
            vec![
                Change::Insert {
                    values: values(&[("id", number("1"))]),
                },
                Change::Delete {
                    old: vec![number("999")],
                },
            ],
        )
        .await
        .unwrap_err();
    assert!(
        error.message.contains("rollback could not be confirmed"),
        "{error}"
    );
    assert!(db.transaction_state().await.is_err());
    // The warning is honest: the external MyISAM trigger side effect survives the InnoDB rollback.
    assert!(
        rows(blocker.clone(), "SELECT * FROM klyndb_edit_unsafe")
            .await
            .is_empty()
    );
    assert_eq!(
        rows(blocker.clone(), "SELECT * FROM klyndb_edit_audit").await,
        vec![vec![number("1")]]
    );
    query(
        blocker.clone(),
        "DROP TABLE klyndb_edit_unsafe; DROP TABLE klyndb_edit_audit",
        100,
    )
    .await;
    db.disconnect().await.unwrap();
    blocker.disconnect().await.unwrap();
}
