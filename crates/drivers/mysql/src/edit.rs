use super::{columns, editable, err, quote};
use klyndb_driver_api::*;
use mysql_async::{Conn, Value, consts::StatusFlags, prelude::Queryable};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

fn value(cell: &Cell) -> Result<Value> {
    Ok(match cell {
        Cell::Null => Value::NULL,
        Cell::Text(v) => Value::Bytes(v.as_bytes().to_vec()),
        Cell::Number(v) => {
            if let Ok(n) = v.parse::<i64>() {
                Value::Int(n)
            } else if let Ok(n) = v.parse::<u64>() {
                Value::UInt(n)
            } else {
                if !v.parse::<f64>().is_ok_and(f64::is_finite) {
                    return Err(Error::new("Number must be a finite numeric literal"));
                }
                // Bind exact decimal/exponent text; floating-point parsing validates, never stores, the value.
                Value::Bytes(v.as_bytes().to_vec())
            }
        }
        Cell::Boolean(v) => Value::Int(i64::from(*v)),
        Cell::Binary(v) => Value::Bytes(
            hex::decode(v).map_err(|_| Error::new("Binary values must be hexadecimal"))?,
        ),
        Cell::Json(v) => Value::Bytes(v.to_string().into_bytes()),
    })
}
fn check_abort(abort: &CancellationToken) -> Result<()> {
    if abort.is_cancelled() {
        Err(Error::new(
            "Table editing timed out; the batch was rolled back",
        ))
    } else {
        Ok(())
    }
}

pub(super) async fn apply(
    conn: &mut Conn,
    table: &Table,
    changes: &[Change],
    abort: &CancellationToken,
    committing: &AtomicBool,
    poison: &AtomicBool,
) -> Result<MutationResult> {
    let charset: Option<(String, String, Option<String>)> = conn
        .query_first(
            "SELECT @@character_set_client, @@character_set_connection, @@character_set_results",
        )
        .await
        .map_err(err)?;
    if !charset.is_some_and(|(client, connection, results)| {
        client == "utf8mb4" && connection == "utf8mb4" && results.as_deref() == Some("utf8mb4")
    }) {
        return Err(Error::new(
            "Staged editing requires a UTF-8 session. Run SET NAMES utf8mb4 and refresh the table before editing.",
        ));
    }
    conn.query_drop("SELECT 1").await.map_err(err)?;
    let status = conn
        .last_ok_packet()
        .map(|p| p.status_flags())
        .unwrap_or_default();
    let active = status.contains(StatusFlags::SERVER_STATUS_IN_TRANS);
    let own = !active && status.contains(StatusFlags::SERVER_STATUS_AUTOCOMMIT);
    if !active {
        conn.query_drop("START TRANSACTION").await.map_err(err)?;
    }
    let savepoint = format!("klyndb_edit_{}", uuid::Uuid::new_v4().simple());
    let mut has_savepoint = false;
    let batch = async {
        conn.query_drop(format!("SAVEPOINT {savepoint}")).await.map_err(err)?;
        has_savepoint = true;
        let qualified = format!("{}.{}", quote(&table.schema), quote(&table.name));
        // Hold the metadata lock before checking the engine/columns against concurrent ALTER.
        conn.query_drop(format!("SELECT * FROM {qualified} LIMIT 0 FOR UPDATE")).await.map_err(err)?;
        if !editable(conn, table).await? {
            return Err(Error::new(
                "Staged editing requires an InnoDB base table. Use the SQL editor for other engines or views.",
            ));
        }
        let columns = columns(conn, table).await?;
        for change in changes {
            change.validate(&columns)?;
        }
        let mut affected = 0;
        for change in changes {
            check_abort(abort)?;
            let mut params = vec![];
            let mut names = vec![];
            if let Some(values) = change.values() {
                for (name, cell) in values {
                    names.push(quote(name));
                    params.push(value(cell)?);
                }
            }
            let sql = match change {
                Change::Insert { .. } => format!("INSERT INTO {qualified} ({}) VALUES ({})", names.join(","), vec!["?"; names.len()].join(",")),
                Change::Update { .. } => format!("UPDATE {qualified} SET {}", names.iter().map(|n| format!("{n}=?")).collect::<Vec<_>>().join(",")),
                Change::Delete { .. } => format!("DELETE FROM {qualified}"),
            };
            let sql = if let Some(old) = change.old() {
                let mut predicates = vec![];
                // Native PK comparisons use the index; binary old values catch collation-equivalent changes.
                for (column, old) in columns.iter().zip(old).filter(|(c, _)| c.primary_key) {
                    predicates.push(format!("{}=?", quote(&column.name)));
                    params.push(value(old)?);
                }
                for (column, old) in columns.iter().zip(old) {
                    let name = quote(&column.name);
                    let predicate = match old {
                        Cell::Binary(_) => format!("CAST({name} AS BINARY) <=> CAST(? AS BINARY)"),
                        Cell::Json(_) => format!("CAST(JSON_EXTRACT({name},'$') AS BINARY) <=> CAST(JSON_EXTRACT(?,'$') AS BINARY)"),
                        _ => format!("CAST(CONVERT({name} USING utf8mb4) AS BINARY) <=> CAST(CONVERT(? USING utf8mb4) AS BINARY)"),
                    };
                    predicates.push(predicate);
                    params.push(value(old)?);
                }
                format!("{sql} WHERE {}", predicates.join(" AND "))
            } else {
                sql
            };
            conn.exec_drop(sql, params).await.map_err(err)?;
            if conn.get_warnings() != 0 {
                return Err(Error::new("Database reported a value-conversion warning; the batch was rolled back. Check the column types and values."));
            }
            if conn.affected_rows() != 1 {
                return Err(Error::new("Row changed or was removed. Refresh the table; this batch was rolled back."));
            }
            affected += 1;
        }
        check_abort(abort)?;
        conn.query_drop(format!("RELEASE SAVEPOINT {savepoint}")).await.map_err(err)?;
        if own {
            check_abort(abort)?;
            committing.store(true, Ordering::Relaxed);
            conn.query_drop("COMMIT AND NO CHAIN NO RELEASE").await.map_err(err)?;
        }
        Ok(MutationResult { affected, pending_transaction: !own })
    }.await;
    if batch.is_err() {
        let rollback = if own {
            "ROLLBACK AND NO CHAIN NO RELEASE".into()
        } else if has_savepoint {
            format!("ROLLBACK TO SAVEPOINT {savepoint}")
        } else {
            return batch;
        };
        let rollback_ok = conn.query_drop(rollback).await.is_ok() && conn.get_warnings() == 0;
        let release_ok = own
            || (rollback_ok
                && conn
                    .query_drop(format!("RELEASE SAVEPOINT {savepoint}"))
                    .await
                    .is_ok());
        if !rollback_ok || !release_ok {
            poison.store(true, Ordering::Relaxed);
            return Err(Error::new(
                "Editing failed and rollback could not be confirmed. Connection closed; disconnect and reconnect. Verify data before retrying; the server may have rolled back the existing transaction or retained nontransactional trigger effects.",
            ));
        }
    }
    batch
}
