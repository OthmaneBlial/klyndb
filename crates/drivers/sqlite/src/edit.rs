use super::{err, inspect};
use klyndb_driver_api::*;
use rusqlite::{Connection, params_from_iter, types::Value};

fn value(cell: &Cell) -> Result<Value> {
    Ok(match cell {
        Cell::Null => Value::Null,
        Cell::Text(v) => Value::Text(v.clone()),
        Cell::Number(v) => match v.parse::<i64>() {
            Ok(n) => Value::Integer(n),
            Err(_) => {
                if v.trim_start_matches(['-', '+'])
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                {
                    return Err(Error::new(
                        "SQLite integers must fit signed 64-bit range; use Text for larger exact values",
                    ));
                }
                let n = v
                    .parse::<f64>()
                    .map_err(|_| Error::new("Invalid SQLite number"))?;
                if !n.is_finite() {
                    return Err(Error::new("SQLite numbers must be finite"));
                }
                Value::Real(n)
            }
        },
        Cell::Boolean(v) => Value::Integer(i64::from(*v)),
        Cell::Binary(v) => Value::Blob(
            hex::decode(v).map_err(|_| Error::new("Binary values must be hexadecimal"))?,
        ),
        Cell::Json(v) => Value::Text(v.to_string()),
    })
}

pub(super) fn apply(
    conn: &Connection,
    table: &Table,
    changes: &[Change],
    cancel: tokio_util::sync::CancellationToken,
) -> Result<MutationResult> {
    let table_name = format!(
        "{}.{}",
        quote_identifier(&table.schema),
        quote_identifier(&table.name)
    );
    let began = std::time::Instant::now();
    conn.progress_handler(
        1000,
        Some({
            let token = cancel.clone();
            move || token.is_cancelled() || began.elapsed() > std::time::Duration::from_secs(60)
        }),
    )
    .map_err(err)?;
    let result = (|| {
        conn.execute_batch("SAVEPOINT klyndb_edit").map_err(err)?;
        let batch = (|| {
            let kind: String = conn
                .query_row(
                    "SELECT type FROM sqlite_schema WHERE name=?",
                    [&table.name],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if table.schema != "main" || kind != "table" {
                return Err(Error::new("Only base tables can be edited"));
            }
            let columns = inspect(conn, table)?.columns;
            for change in changes {
                change.validate(&columns)?;
            }
            let mut affected = 0;
            for change in changes {
                if cancel.is_cancelled() {
                    return Err(Error::new("Import cancelled"));
                }
                let mut params = vec![];
                let sql = match change {
                    Change::Insert { values } if values.is_empty() => {
                        format!("INSERT INTO {table_name} DEFAULT VALUES")
                    }
                    Change::Insert { values } => {
                        for cell in values.values() {
                            params.push(value(cell)?);
                        }
                        format!(
                            "INSERT INTO {table_name} ({}) VALUES ({})",
                            values
                                .keys()
                                .map(|n| quote_identifier(n))
                                .collect::<Vec<_>>()
                                .join(","),
                            vec!["?"; values.len()].join(",")
                        )
                    }
                    Change::Update { values, .. } => {
                        for cell in values.values() {
                            params.push(value(cell)?);
                        }
                        format!(
                            "UPDATE {table_name} SET {}",
                            values
                                .keys()
                                .map(|n| format!("{}=?", quote_identifier(n)))
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    }
                    Change::Delete { .. } => format!("DELETE FROM {table_name}"),
                };
                let sql = if let Some(old) = change.old() {
                    for cell in old {
                        params.push(value(cell)?);
                    }
                    format!(
                        "{sql} WHERE {}",
                        columns
                            .iter()
                            .map(|c| format!("({} COLLATE BINARY) IS ?", quote_identifier(&c.name)))
                            .collect::<Vec<_>>()
                            .join(" AND ")
                    )
                } else {
                    sql
                };
                let count = conn.execute(&sql, params_from_iter(params)).map_err(err)?;
                if count != 1 {
                    return Err(Error::new(
                        "Row changed or was removed. Refresh the table; no changes in this batch were applied.",
                    ));
                }
                affected += count as u64;
            }
            conn.execute_batch("RELEASE klyndb_edit").map_err(err)?;
            Ok(MutationResult {
                affected,
                pending_transaction: !conn.is_autocommit(),
            })
        })();
        if batch.is_err() {
            conn.progress_handler(0, None::<fn() -> bool>)
                .map_err(err)?;
            // A constraint or deferred COMMIT failure must undo the whole batch, preserving any user's outer transaction.
            if let Err(rollback) =
                conn.execute_batch("ROLLBACK TO klyndb_edit; RELEASE klyndb_edit")
            {
                return Err(Error::new(format!(
                    "Editing failed and rollback could not be verified: {rollback}. Disconnect before further writes."
                )));
            }
        }
        batch
    })();
    conn.progress_handler(0, None::<fn() -> bool>)
        .map_err(err)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sqlite;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn edits_are_bound_atomic_and_preserve_outer_transaction() {
        let db = Sqlite::connect(":memory:".into(), false, true)
            .await
            .unwrap();
        db.with(|c|c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, txt TEXT, n REAL, b BLOB, g TEXT GENERATED ALWAYS AS (txt||id) STORED); CREATE TABLE no_pk(v TEXT);").map_err(err)).await.unwrap();
        let table = Table {
            schema: "main".into(),
            name: "t".into(),
            kind: "table".into(),
        };
        let literal = "x'; DROP TABLE t; --";
        let values = BTreeMap::from([
            ("id".into(), Cell::Number("9223372036854775807".into())),
            ("txt".into(), Cell::Text(literal.into())),
            ("n".into(), Cell::Number("0.1".into())),
            ("b".into(), Cell::Binary("00ff".into())),
        ]);
        assert_eq!(
            db.apply_changes(table.clone(), vec![Change::Insert { values }])
                .await
                .unwrap()
                .affected,
            1
        );
        let old = vec![
            Cell::Number("9223372036854775807".into()),
            Cell::Text(literal.into()),
            Cell::Number("0.1".into()),
            Cell::Binary("00ff".into()),
            Cell::Text(format!("{literal}9223372036854775807")),
        ];
        let update = Change::Update {
            old: old.clone(),
            values: BTreeMap::from([("txt".into(), Cell::Text("updated".into()))]),
        };
        assert!(
            !db.apply_changes(table.clone(), vec![update.clone()])
                .await
                .unwrap()
                .pending_transaction
        );
        let insert = Change::Insert {
            values: BTreeMap::from([("id".into(), Cell::Number("1".into()))]),
        };
        assert!(
            db.apply_changes(table.clone(), vec![insert, update])
                .await
                .unwrap_err()
                .message
                .contains("Row changed")
        );
        db.with(|c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                c.query_row("SELECT txt FROM t", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "updated"
            );
            c.execute_batch("BEGIN").map_err(err)
        })
        .await
        .unwrap();
        let mut current = old;
        current[1] = Cell::Text("updated".into());
        current[4] = Cell::Text("updated9223372036854775807".into());
        assert!(
            db.apply_changes(
                table.clone(),
                vec![Change::Update {
                    old: current.clone(),
                    values: BTreeMap::from([("txt".into(), Cell::Null)])
                }]
            )
            .await
            .unwrap()
            .pending_transaction
        );
        db.with(|c| {
            c.execute_batch("ROLLBACK").unwrap();
            assert_eq!(
                c.query_row("SELECT txt FROM t", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "updated"
            );
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            db.apply_changes(
                table.clone(),
                vec![Change::Update {
                    old: current.clone(),
                    values: BTreeMap::from([("g".into(), Cell::Text("bad".into()))])
                }]
            )
            .await
            .is_err()
        );
        assert!(
            db.apply_changes(
                Table {
                    schema: "main".into(),
                    name: "no_pk".into(),
                    kind: "table".into()
                },
                vec![Change::Delete {
                    old: vec![Cell::Text("anything".into())]
                }]
            )
            .await
            .is_err()
        );
        assert!(value(&Cell::Number("9223372036854775808".into())).is_err());
        assert!(value(&Cell::Number("NaN".into())).is_err());
        assert_eq!(
            db.apply_changes(table, vec![Change::Delete { old: current }])
                .await
                .unwrap()
                .affected,
            1
        );
    }
}
