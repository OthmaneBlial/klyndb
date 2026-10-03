use super::{err, inspect, transaction_state};
use duckdb::{Connection, params_from_iter, types::Value};
use klyndb_driver_api::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(super) enum Source {
    Edits(Option<Vec<Change>>),
    Import(mpsc::Receiver<Result<InsertBatch>>),
}
fn numeric(ty: &str) -> bool {
    matches!(
        ty,
        "TINYINT"
            | "SMALLINT"
            | "INTEGER"
            | "BIGINT"
            | "HUGEINT"
            | "UTINYINT"
            | "USMALLINT"
            | "UINTEGER"
            | "UBIGINT"
            | "UHUGEINT"
            | "FLOAT"
            | "DOUBLE"
    ) || ty.starts_with("DECIMAL(")
}
pub(super) fn supported_type(ty: &str) -> bool {
    // ponytail: scalar writes only; add container/interval writers with lossless native roundtrip checks.
    numeric(ty)
        || matches!(
            ty,
            "VARCHAR"
                | "BOOLEAN"
                | "BLOB"
                | "JSON"
                | "UUID"
                | "DATE"
                | "TIME"
                | "TIME_NS"
                | "TIMESTAMP"
                | "TIMESTAMP_S"
                | "TIMESTAMP_MS"
                | "TIMESTAMP_NS"
                | "TIMESTAMP WITH TIME ZONE"
        )
        || ty.starts_with("ENUM(")
}
fn value(cell: &Cell) -> Result<Value> {
    Ok(match cell {
        Cell::Null => Value::Null,
        Cell::Boolean(v) => Value::Boolean(*v),
        Cell::Binary(v) => Value::Blob(
            hex::decode(v).map_err(|_| Error::new("Binary values must be hexadecimal"))?,
        ),
        Cell::Number(v) => {
            normalized_number(v)?;
            Value::Text(v.clone())
        }
        _ => Value::Text(cell.text()),
    })
}
fn converted(conn: &Connection, cell: &Cell, column: &Column) -> Result<Value> {
    let parameter = value(cell)?;
    if matches!(cell, Cell::Null) {
        return Ok(parameter);
    }
    let ty = &column.data_type;
    let expression = if ty == "BLOB" {
        format!("hex(CAST(? AS {ty}))")
    } else {
        format!("CAST(CAST(? AS {ty}) AS VARCHAR)")
    };
    let actual: String = conn
        .prepare_cached(&format!("SELECT {expression}"))
        .map_err(err)?
        .query_row([&parameter], |r| r.get(0))
        .map_err(err)?;
    let original = cell.text();
    let exact = if numeric(ty) {
        let original = if let Cell::Boolean(b) = cell {
            if *b { "1" } else { "0" }
        } else {
            original.trim()
        };
        normalized_number(original)? == normalized_number(&actual)?
    } else if ty == "BLOB" {
        matches!(cell, Cell::Binary(_)) && actual.eq_ignore_ascii_case(&original)
    } else if ty == "BOOLEAN" {
        let original = match original.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => "true",
            "false" | "0" => "false",
            _ => "",
        };
        actual == original
    } else if ty == "UUID" {
        actual.eq_ignore_ascii_case(original.trim())
    } else if ty == "DATE" {
        actual == original.trim()
    } else if ty.starts_with("TIME") {
        // Native casts normalize time zones/formats; reject loss of fractional precision.
        let precision = match ty.as_str() {
            "TIMESTAMP_S" => 0,
            "TIMESTAMP_MS" => 3,
            "TIMESTAMP_NS" | "TIME_NS" => 9,
            _ => 6,
        };
        original.split_once('.').is_none_or(|(_, fraction)| {
            fraction
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .trim_end_matches('0')
                .len()
                <= precision
        })
    } else if ty == "JSON" {
        serde_json::from_str::<serde_json::Value>(&original).map_err(err)?
            == serde_json::from_str::<serde_json::Value>(&actual).map_err(err)?
    } else {
        actual == original
    };
    if !exact {
        return Err(Error::new(format!(
            "{} cannot store this value without conversion loss",
            column.name
        )));
    }
    Ok(parameter)
}
fn apply(conn: &Connection, table: &str, columns: &[Column], change: &Change) -> Result<u64> {
    change.validate(columns)?;
    let mut parameters = vec![];
    let mut names = vec![];
    let mut bindings = vec![];
    if let Some(values) = change.values() {
        for (name, cell) in values {
            let column = columns
                .iter()
                .find(|c| &c.name == name)
                .ok_or_else(|| Error::new("Column changed"))?;
            parameters.push(converted(conn, cell, column)?);
            names.push(quote_identifier(name));
            bindings.push(format!("CAST(? AS {})", column.data_type));
        }
    }
    let mut sql = match change {
        Change::Insert { values } if values.is_empty() => {
            format!("INSERT INTO {table} DEFAULT VALUES")
        }
        Change::Insert { .. } => format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            names.join(","),
            bindings.join(",")
        ),
        Change::Update { .. } => format!(
            "UPDATE {table} SET {}",
            names
                .iter()
                .zip(&bindings)
                .map(|(n, b)| format!("{n}={b}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Change::Delete { .. } => format!("DELETE FROM {table}"),
    };
    if let Some(old) = change.old() {
        let mut predicates = vec![];
        for (column, cell) in columns.iter().zip(old) {
            parameters.push(match cell {
                Cell::Binary(v) => Value::Blob(hex::decode(v).map_err(err)?),
                Cell::Null => Value::Null,
                _ => Value::Text(cell.text()),
            });
            let column_name = quote_identifier(&column.name);
            predicates.push(if column.data_type == "VARCHAR" || column.data_type.starts_with("ENUM(") {
                format!("encode(CAST({column_name} AS VARCHAR)) IS NOT DISTINCT FROM encode(CAST(CAST(? AS {}) AS VARCHAR))", column.data_type)
            } else { format!("{column_name} IS NOT DISTINCT FROM CAST(? AS {})", column.data_type) });
        }
        sql.push_str(&format!(" WHERE {}", predicates.join(" AND ")));
    }
    let affected = conn
        .prepare_cached(&sql)
        .map_err(err)?
        .execute(params_from_iter(parameters))
        .map_err(err)? as u64;
    if affected != 1 {
        return Err(Error::new(
            "Row changed or was removed. Refresh the table; the batch was aborted.",
        ));
    }
    Ok(affected)
}

/// Returns whether uncertain cleanup requires closing the native session under its lock.
pub(super) fn write(
    conn: &Connection,
    table: &Table,
    mut source: Source,
    cancel: CancellationToken,
    timeout: Duration,
) -> (Result<MutationResult>, bool) {
    let state = match transaction_state(conn) {
        Ok(state) => state,
        Err(e) => return (Err(e), false),
    };
    if state != TransactionState::Idle {
        return (
            Err(Error::new(
                "Finish the current transaction before DuckDB row edits/imports. DuckDB has no savepoints; COMMIT or ROLLBACK in the SQL editor first.",
            )),
            false,
        );
    }
    if let Err(e) = conn.execute_batch("BEGIN") {
        return (Err(err(e)), false);
    }
    let runtime = tokio::runtime::Handle::current();
    let finished = CancellationToken::new();
    let done = finished.clone();
    let token = cancel.clone();
    let timed_out = Arc::new(AtomicBool::new(false));
    let expired = timed_out.clone();
    let interrupt = conn.interrupt_handle();
    let watcher = runtime.spawn(async move {
        tokio::select! { biased; _=done.cancelled()=>return, _=token.cancelled()=>{}, _=tokio::time::sleep(timeout)=>{expired.store(true, Ordering::Relaxed);token.cancel();} }
        loop {
            interrupt.interrupt();
            tokio::select! { biased; _=done.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(10))=>{} }
        }
    });
    let batch = (|| {
        let info = inspect(conn, table, false)?;
        if !info.editable {
            return Err(Error::new(
                "Choose a supported DuckDB base table for row writes",
            ));
        }
        let qualified = format!(
            "{}.{}",
            quote_identifier(&table.schema),
            quote_identifier(&table.name)
        );
        let mut affected = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(Error::new("Table writes cancelled"));
            }
            let importing = matches!(source, Source::Import(_));
            let batch = match &mut source {
                Source::Edits(changes) => changes
                    .take()
                    .map(InsertBatch::Rows)
                    .unwrap_or(InsertBatch::Complete),
                Source::Import(input) => runtime.block_on(next_insert_batch(input, &cancel))?,
            };
            let InsertBatch::Rows(changes) = batch else {
                break;
            };
            if importing {
                validate_insert_batch(&changes)?;
            } else {
                validate_change_batch(&changes)?;
            }
            for change in &changes {
                if cancel.is_cancelled() {
                    return Err(Error::new("Table writes cancelled"));
                }
                affected += apply(conn, &qualified, &info.columns, change)?;
            }
        }
        Ok(affected)
    })();
    finished.cancel();
    if let Err(e) = runtime.block_on(watcher) {
        return (
            Err(Error::new(format!(
                "Native write interruption could not be joined: {e}. Connection closed; verify writes."
            ))),
            true,
        );
    }
    let batch = if timed_out.load(Ordering::Relaxed) {
        Err(Error::new("Table writes timed out"))
    } else if cancel.is_cancelled() {
        Err(Error::new("Table writes cancelled"))
    } else {
        batch
    };
    let mut committing = false;
    let result = batch.and_then(|affected| {
        // Do not deliberately interrupt a submitted final COMMIT or rollback.
        committing = true;
        conn.execute_batch("COMMIT").map_err(err)?;
        Ok(MutationResult {
            affected,
            pending_transaction: false,
        })
    });
    if let Err(error) = result {
        let rolled_back = conn.execute_batch("ROLLBACK").is_ok()
            || transaction_state(conn).is_ok_and(|state| state == TransactionState::Idle);
        if !rolled_back || committing {
            return (
                Err(Error::new(format!(
                    "{error}. Write completion/rollback could not be verified; connection closed. Verify data before retrying."
                ))),
                true,
            );
        }
        return (Err(error), false);
    }
    (result, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DuckDb,
        contract::{query, rows},
    };
    use std::collections::BTreeMap;

    fn table(name: &str) -> Table {
        Table {
            schema: "main".into(),
            name: name.into(),
            kind: "table".into(),
        }
    }
    fn insert(id: &str, name: &str) -> Change {
        Change::Insert {
            values: BTreeMap::from([
                ("id".into(), Cell::Number(id.into())),
                ("name".into(), Cell::Text(name.into())),
            ]),
        }
    }
    async fn data(db: &Arc<DuckDb>, sql: &str) -> Vec<Row> {
        rows(&query(db, sql, 100).await.unwrap())
    }

    #[tokio::test]
    async fn native_reviewed_writes_exact_values_conflicts_and_transaction_safety() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("writes.duckdb")
            .to_string_lossy()
            .into_owned();
        let db = Arc::new(DuckDb::connect(path, false, true).await.unwrap());
        query(&db, "CREATE TABLE t(id UHUGEINT PRIMARY KEY, name VARCHAR COLLATE NOCASE DEFAULT 'default', exact DECIMAL(38,18), payload BLOB, moment TIMESTAMP_NS, derived INTEGER GENERATED ALWAYS AS(length(name)))", 100).await.unwrap();
        let info = db.inspect(&table("t")).await.unwrap();
        assert!(info.editable, "{:?}", info.ddl);
        assert!(info.columns[5].generated);
        assert!(info.columns[5].default.is_none());
        assert_eq!(info.columns[1].default.as_deref(), Some("'default'"));
        let exact = "12345678901234567890.123456789012345678";
        let moment = "2026-10-03T12:34:56.123456789";
        let mut change = insert(&u128::MAX.to_string(), "é'; DROP TABLE t; --\nnext");
        let Change::Insert { values } = &mut change else {
            unreachable!()
        };
        values.extend([
            ("exact".into(), Cell::Number(exact.into())),
            ("payload".into(), Cell::Binary("00ff".into())),
            ("moment".into(), Cell::Text(moment.into())),
        ]);
        let applied = db.apply_changes(table("t"), vec![change]).await.unwrap();
        assert_eq!(applied.affected, 1);
        assert!(!applied.pending_transaction);
        let old = data(&db, "SELECT * FROM t").await.remove(0);
        assert_eq!(old[0], Cell::Number(u128::MAX.to_string()));
        assert_eq!(old[2], Cell::Number(exact.into()));
        assert_eq!(old[3], Cell::Binary("00ff".into()));
        assert_eq!(old[4], Cell::Text(moment.into()));
        db.apply_changes(
            table("t"),
            vec![Change::Update {
                old: old.clone(),
                values: BTreeMap::from([("name".into(), Cell::Text("Updated".into()))]),
            }],
        )
        .await
        .unwrap();
        assert!(
            db.apply_changes(
                table("t"),
                vec![insert("2", "prefix"), Change::Delete { old }]
            )
            .await
            .is_err()
        );
        assert_eq!(data(&db, "SELECT count(*) FROM t").await[0][0].text(), "1");
        for (column, cell) in [
            ("exact", Cell::Number("0.1234567890123456789".into())),
            ("id", Cell::Number("1.5".into())),
            ("payload", Cell::Text("not binary".into())),
            ("derived", Cell::Number("5".into())),
        ] {
            let invalid = Change::Insert {
                values: BTreeMap::from([
                    ("id".into(), Cell::Number("3".into())),
                    (column.into(), cell),
                ]),
            };
            assert!(
                db.apply_changes(table("t"), vec![insert("2", "prefix"), invalid])
                    .await
                    .is_err(),
                "{column}"
            );
            assert_eq!(data(&db, "SELECT count(*) FROM t").await[0][0].text(), "1");
        }
        let stale = data(&db, "SELECT * FROM t").await.remove(0);
        query(&db, "UPDATE t SET name='updated'", 100)
            .await
            .unwrap();
        assert!(
            db.apply_changes(table("t"), vec![Change::Delete { old: stale }])
                .await
                .is_err()
        );
        query(
            &db,
            "BEGIN; INSERT INTO t(id,name) VALUES(2,'caller work')",
            100,
        )
        .await
        .unwrap();
        let refused = db
            .apply_changes(table("t"), vec![insert("3", "new")])
            .await
            .unwrap_err();
        assert!(refused.message.contains("no savepoints"));
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        assert_eq!(data(&db, "SELECT count(*) FROM t").await[0][0].text(), "2");
        query(&db, "ROLLBACK", 100).await.unwrap();
        let old = data(&db, "SELECT * FROM t").await.remove(0);
        assert_eq!(
            db.apply_changes(table("t"), vec![Change::Delete { old }])
                .await
                .unwrap()
                .affected,
            1
        );
        query(&db, "CREATE TABLE defaults(id INTEGER DEFAULT 42, name VARCHAR DEFAULT 'default'); CREATE TABLE nested(id INTEGER, items INTEGER[]); CREATE VIEW v AS SELECT * FROM defaults; CREATE TYPE mood AS ENUM('happy','sad'); CREATE TABLE moods(id INTEGER PRIMARY KEY, mood mood)", 100).await.unwrap();
        assert!(!db.inspect(&table("nested")).await.unwrap().editable);
        assert!(!db.inspect(&table("v")).await.unwrap().editable);
        db.apply_changes(
            table("defaults"),
            vec![Change::Insert {
                values: BTreeMap::new(),
            }],
        )
        .await
        .unwrap();
        let old = data(&db, "SELECT * FROM defaults").await.remove(0);
        assert_eq!(old[0].text(), "42");
        assert!(
            db.apply_changes(table("defaults"), vec![Change::Delete { old }])
                .await
                .is_err()
        );
        db.apply_changes(
            table("moods"),
            vec![Change::Insert {
                values: BTreeMap::from([
                    ("id".into(), Cell::Number("1".into())),
                    ("mood".into(), Cell::Text("happy".into())),
                ]),
            }],
        )
        .await
        .unwrap();
        let old = data(&db, "SELECT * FROM moods").await.remove(0);
        db.apply_changes(table("moods"), vec![Change::Delete { old }])
            .await
            .unwrap();
        db.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn native_stream_eof_rollback_cancel_deadline_and_session_reuse() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("writes.duckdb")
            .to_string_lossy()
            .into_owned();
        let db = Arc::new(DuckDb::connect(path, false, true).await.unwrap());
        query(
            &db,
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name VARCHAR)",
            100,
        )
        .await
        .unwrap();
        for batches in [
            vec![
                Ok(InsertBatch::Rows(vec![insert("1", "first")])),
                Ok(InsertBatch::Rows(vec![insert("2", "second")])),
                Ok(InsertBatch::Complete),
            ],
            vec![
                Ok(InsertBatch::Rows(vec![insert("3", "prefix")])),
                Ok(InsertBatch::Rows(vec![insert("1", "duplicate")])),
                Ok(InsertBatch::Complete),
            ],
            vec![Ok(InsertBatch::Rows(vec![insert("3", "missing EOF")]))],
        ] {
            let success = batches.len() == 3
                && matches!(&batches[1], Ok(InsertBatch::Rows(r)) if r[0].values().unwrap()["id"].text()=="2");
            let (tx, rx) = mpsc::channel(4);
            for batch in batches {
                tx.send(batch).await.unwrap();
            }
            drop(tx);
            let result = db
                .insert_stream(table("t"), rx, CancellationToken::new())
                .await;
            assert_eq!(result.is_ok(), success);
            assert_eq!(data(&db, "SELECT count(*) FROM t").await[0][0].text(), "2");
            assert_eq!(
                db.transaction_state().await.unwrap(),
                TransactionState::Idle
            );
        }
        let (tx, rx) = mpsc::channel(1);
        tx.send(Ok(InsertBatch::Rows(vec![insert("3", "cancelled")])))
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let writer = db.clone();
        let token = cancel.clone();
        let task = tokio::spawn(async move { writer.insert_stream(table("t"), rx, token).await });
        // A concurrent COMMIT must wait for the whole stream; it cannot commit its prefix.
        while tx.capacity() == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let reader = db.clone();
        let commit = tokio::spawn(async move { query(&reader, "COMMIT", 100).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!commit.is_finished());
        cancel.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        drop(tx);
        assert!(commit.await.unwrap().is_err());
        assert_eq!(data(&db, "SELECT count(*) FROM t").await[0][0].text(), "2");
        query(
            &db,
            "CREATE TABLE slow(name VARCHAR DEFAULT sha256(repeat(uuid()::VARCHAR,1000000)))",
            100,
        )
        .await
        .unwrap();
        let failure = db
            .write(
                table("slow"),
                Source::Edits(Some(vec![Change::Insert {
                    values: BTreeMap::new(),
                }])),
                CancellationToken::new(),
                Duration::from_millis(10),
            )
            .await
            .unwrap_err();
        assert!(failure.message.contains("timed out"), "{failure}");
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        assert_eq!(
            data(&db, "SELECT count(*) FROM slow").await[0][0].text(),
            "0"
        );
        assert_eq!(data(&db, "SELECT 42").await[0][0].text(), "42");
        db.disconnect().await.unwrap();
    }
}
