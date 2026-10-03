mod edit;
use async_trait::async_trait;
use klyndb_driver_api::*;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Connection, OpenFlags, OptionalExtension, types::ValueRef};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct Sqlite {
    connection: Arc<Mutex<Option<Connection>>>,
    read_only: bool,
}
fn err(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}
fn query_error(e: rusqlite::Error, submitted: &str) -> Error {
    let offset = match &e {
        rusqlite::Error::SqlInputError { sql, offset, .. } if submitted.ends_with(sql) => {
            usize::try_from(*offset).ok().and_then(|offset| {
                if offset > sql.len() {
                    return None;
                }
                sql_utf16_offset(submitted, submitted.len() - sql.len() + offset)
            })
        }
        _ => None,
    };
    let mut error = err(e);
    error.sql_offset = offset;
    error
}
impl Sqlite {
    pub async fn connect(path: String, read_only: bool, create: bool) -> Result<Self> {
        let connection = tokio::task::spawn_blocking(move || {
            let flags = if read_only {
                OpenFlags::SQLITE_OPEN_READ_ONLY
            } else if create {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE
            };
            let conn = Connection::open_with_flags(path, flags).map_err(err)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .map_err(err)?;
            conn.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
                .map_err(err)?;
            conn.execute_batch("PRAGMA foreign_keys=ON;").map_err(err)?;
            if read_only {
                conn.execute_batch("PRAGMA query_only=ON;").map_err(err)?;
            }
            Ok::<_, Error>(conn)
        })
        .await
        .map_err(err)??;
        Ok(Self {
            connection: Arc::new(Mutex::new(Some(connection))),
            read_only,
        })
    }
    async fn with<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|_| Error::new("SQLite connection lock failed"))?;
            f(guard
                .as_ref()
                .ok_or_else(|| Error::new("Connection is closed"))?)
        })
        .await
        .map_err(err)?
    }
}
fn stream(
    conn: &Connection,
    sql: String,
    output: mpsc::Sender<Batch>,
    cancel: CancellationToken,
    limit: usize,
) -> Result<()> {
    let token = cancel.clone();
    conn.progress_handler(1000, Some(move || token.is_cancelled()))
        .map_err(err)?;
    let result = (|| {
        let mut statements = rusqlite::Batch::new(conn, &sql);
        while let Some(mut statement) = statements.next().map_err(|e| query_error(e, &sql))? {
            let before = conn.total_changes();
            if cancel.is_cancelled() {
                return Err(Error::new("Query cancelled"));
            }
            let columns = statement
                .column_names()
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>();
            let width = columns.len();
            output.blocking_send(Batch::Columns(columns)).map_err(err)?;
            let mut count = 0;
            let mut truncated = false;
            if width == 0 {
                statement.execute([]).map_err(err)?;
            } else {
                let mut rows = statement.query([]).map_err(err)?;
                let mut buffer = Vec::with_capacity(256);
                let mut buffer_bytes = 0;
                while let Some(row) = rows.next().map_err(err)? {
                    if cancel.is_cancelled() {
                        return Err(Error::new("Query cancelled"));
                    }
                    if limit == 0 {
                        continue;
                    }
                    if count >= limit {
                        truncated = true;
                        break;
                    }
                    let mut cells = Vec::with_capacity(width);
                    for i in 0..width {
                        cells.push(match row.get_ref(i).map_err(err)? {
                            ValueRef::Null => Cell::Null,
                            ValueRef::Integer(n) => Cell::Number(n.to_string()),
                            ValueRef::Real(n) => Cell::Number(n.to_string()),
                            ValueRef::Text(s) => Cell::Text(String::from_utf8_lossy(s).into()),
                            ValueRef::Blob(b) => Cell::Binary(hex::encode(b)),
                        });
                    }
                    let row_bytes = cells.iter().map(Cell::byte_len).sum::<usize>();
                    if row_bytes > 8 * 1024 * 1024 {
                        return Err(Error::new(
                            "A result row exceeds 8 MiB. Select smaller values or use database-native export.",
                        ));
                    }
                    buffer_bytes += row_bytes;
                    buffer.push(cells);
                    count += 1;
                    if buffer.len() == 256 || buffer_bytes >= 256 * 1024 {
                        buffer_bytes = 0;
                        output
                            .blocking_send(Batch::Rows(std::mem::take(&mut buffer)))
                            .map_err(err)?;
                    }
                }
                if !buffer.is_empty() {
                    output.blocking_send(Batch::Rows(buffer)).map_err(err)?;
                }
            }
            output
                .blocking_send(Batch::Complete {
                    affected: if limit == 0 {
                        conn.total_changes() - before
                    } else if width == 0 {
                        conn.changes()
                    } else {
                        0
                    },
                    truncated,
                })
                .map_err(err)?;
        }
        Ok(())
    })();
    conn.progress_handler(0, None::<fn() -> bool>)
        .map_err(err)?;
    result
}

#[async_trait]
impl Session for Sqlite {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            affected_rows: true,
            table_browse: true,
            diagrams: true,
            transactions: true,
            schemas: false,
            explain: true,
            explain_analyze: false,
            edit_rows: true,
            import_rows: !self.read_only,
            import_sql: !self.read_only,
            cancel: true,
            tls: false,
        }
    }
    fn explain_sql(&self, sql: &str, analyze: bool) -> Result<(String, PlanFormat)> {
        if analyze {
            return Err(Error::new(
                "SQLite provides estimated QUERY PLAN output, without runtime ANALYZE metrics",
            ));
        }
        Ok((format!("EXPLAIN QUERY PLAN {sql}"), PlanFormat::Sqlite))
    }
    async fn execute(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()> {
        self.with(move |conn| stream(conn, sql, output, cancel, limit))
            .await
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT name,type FROM sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(err)?;
            stmt.query_map([], |row| Ok(Table { schema: "main".into(), name: row.get(0)?, kind: row.get(1)? })).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
        }).await
    }
    async fn execute_script(
        &self,
        mut input: mpsc::Receiver<Result<ScriptBatch>>,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        completed: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<()> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        let runtime = tokio::runtime::Handle::current();
        self.with(move |conn| {
            while let Some(sql) = runtime.block_on(next_script_statement(&mut input, &cancel))? {
                stream(conn, sql, output.clone(), cancel.clone(), 0)?;
                completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(())
        })
        .await
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let table = table.clone();
        self.with(move |conn| inspect(conn, &table)).await
    }
    async fn relationships(&self, table: &Table) -> Result<Vec<ForeignKey>> {
        let table = table.clone();
        self.with(move |conn| {
            if table.schema!="main" {return Err(Error::new("Only the main SQLite schema is supported"));}
            let mut stmt=conn.prepare("SELECT id,seq,\"from\",\"table\",\"to\" FROM pragma_foreign_key_list(?) ORDER BY id,seq").map_err(err)?;
            let rows=stmt.query_map([&table.name], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
            let mut result=vec![];
            for (id,seq,column,target_table,mut target_column) in rows {
                if target_column.is_none() {
                    target_column=conn.query_row("SELECT name FROM pragma_table_info(?) WHERE pk>0 ORDER BY pk LIMIT 1 OFFSET ?", rusqlite::params![&target_table,seq], |r| r.get(0)).optional().map_err(err)?;
                }
                result.push((format!("FK #{id}"),column,"main".into(),target_table,target_column));
            }
            Ok(group_foreign_keys(result))
        }).await
    }
    async fn transaction_state(&self) -> Result<TransactionState> {
        self.with(|conn| {
            Ok(if conn.is_autocommit() {
                TransactionState::Idle
            } else {
                TransactionState::Active
            })
        })
        .await
    }
    async fn apply_changes(&self, table: Table, changes: Vec<Change>) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        validate_change_batch(&changes)?;
        self.with(move |conn| edit::apply(conn, &table, &changes, CancellationToken::new()))
            .await
    }
    async fn insert_stream(
        &self,
        table: Table,
        mut input: mpsc::Receiver<Result<InsertBatch>>,
        cancel: CancellationToken,
    ) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        let connection = self.connection.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let mut guard = connection.lock().map_err(err)?;
            let conn = guard.as_ref().ok_or_else(|| Error::new("Connection is closed"))?;
            if cancel.is_cancelled() { return Err(Error::new("Import cancelled")); }
            let savepoint = format!("klyndb_import_{}", uuid::Uuid::new_v4().simple());
            let pending = !conn.is_autocommit();
            conn.execute_batch(&format!("SAVEPOINT {savepoint}")).map_err(err)?;
            let result = (|| {
                let mut affected = 0;
                loop {
                    if cancel.is_cancelled() { return Err(Error::new("Import cancelled")); }
                    match runtime.block_on(next_insert_batch(&mut input, &cancel))? {
                        InsertBatch::Rows(changes) => {
                            validate_insert_batch(&changes)?;
                            affected += edit::apply(conn, &table, &changes, cancel.clone())?.affected;
                        },
                        InsertBatch::Complete => break,
                    }
                }
                if cancel.is_cancelled() { return Err(Error::new("Import cancelled")); }
                // No cancellation once releasing the outer savepoint can commit.
                conn.execute_batch(&format!("RELEASE {savepoint}")).map_err(err)?;
                Ok(MutationResult { affected, pending_transaction: pending })
            })();
            if result.is_err() && conn.execute_batch(&format!("ROLLBACK TO {savepoint}; RELEASE {savepoint}")).is_err() {
                let ended_outer = pending && conn.is_autocommit();
                guard.take();
                return Err(Error::new(if ended_outer {
                    "SQLite ended the outer transaction; earlier uncommitted changes may have been rolled back. Import rollback could not be confirmed; connection closed. Verify data before retrying."
                } else {
                    "Import rollback could not be confirmed; connection closed. Verify data before retrying."
                }));
            }
            result
        }).await.map_err(err)?
    }
    async fn disconnect(&self) -> Result<()> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            connection.lock().map_err(err)?.take();
            Ok(())
        })
        .await
        .map_err(err)?
    }
}

fn inspect(conn: &Connection, table: &Table) -> Result<TableInfo> {
    if table.schema != "main" {
        return Err(Error::new("Only the main SQLite schema is supported"));
    }
    let name = table.name.clone();
    let mut stmt = conn.prepare("SELECT name,type,\"notnull\",dflt_value,pk,hidden FROM pragma_table_xinfo(?) WHERE hidden!=1").map_err(err)?;
    let columns = stmt
        .query_map([&name], |row| {
            Ok(Column {
                name: row.get(0)?,
                data_type: row.get(1)?,
                nullable: row.get::<_, i64>(2)? == 0,
                default: row.get(3)?,
                primary_key: row.get::<_, i64>(4)? > 0,
                generated: row.get::<_, i64>(5)? > 0,
            })
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    let ddl = conn
        .query_row("SELECT sql FROM sqlite_schema WHERE name=?", [&name], |r| {
            r.get(0)
        })
        .ok();
    let mut stmt = conn
        .prepare("SELECT name,\"unique\",origin,partial FROM pragma_index_list(?)")
        .map_err(err)?;
    let indexes = stmt.query_map([&name], |r| Ok(serde_json::json!({"name": r.get::<_, String>(0)?, "unique":r.get::<_, bool>(1)?, "origin":r.get::<_, String>(2)?, "partial":r.get::<_, bool>(3)?}))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT \"table\",\"from\",\"to\",on_update,on_delete FROM pragma_foreign_key_list(?)",
        )
        .map_err(err)?;
    let foreign_keys = stmt.query_map([&name], |r| Ok(serde_json::json!({"table":r.get::<_, String>(0)?, "from":r.get::<_, String>(1)?, "to":r.get::<_, Option<String>>(2)?, "on_update":r.get::<_, String>(3)?, "on_delete":r.get::<_, String>(4)?}))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT name,sql FROM sqlite_schema WHERE type='trigger' AND tbl_name=? ORDER BY name",
        )
        .map_err(err)?;
    let triggers = stmt
        .query_map([&name], |r| {
            Ok(Trigger {
                name: r.get(0)?,
                definition: r.get(1)?,
                state: None,
            })
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    let editable = conn
        .query_row(
            "SELECT type='table' FROM sqlite_schema WHERE name=?",
            [&name],
            |r| r.get::<_, bool>(0),
        )
        .map_err(err)?;
    Ok(TableInfo {
        editable,
        columns,
        ddl,
        indexes,
        foreign_keys,
        constraints: None,
        triggers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_file_preserves_types_limits_and_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite").to_string_lossy().to_string();
        let db = Arc::new(Sqlite::connect(path.clone(), false, true).await.unwrap());
        let (tx, mut rx) = mpsc::channel(2);
        let driver = db.clone();
        let task = tokio::spawn(async move {
            driver.execute("CREATE TABLE t(id INTEGER PRIMARY KEY, v BLOB); INSERT INTO t VALUES(9223372036854775807,x'ff'); SELECT id,v,NULL FROM t; SELECT 1 WHERE 0".into(), tx, CancellationToken::new(), 10).await
        });
        let mut sets = 0;
        while let Some(batch) = rx.recv().await {
            match batch {
                Batch::Columns(_) => sets += 1,
                Batch::Rows(rows) => assert_eq!(
                    rows[0],
                    vec![
                        Cell::Number("9223372036854775807".into()),
                        Cell::Binary("ff".into()),
                        Cell::Null
                    ]
                ),
                _ => {}
            }
        }
        task.await.unwrap().unwrap();
        assert_eq!(sets, 4);
        assert!(
            db.inspect(&db.tables().await.unwrap()[0])
                .await
                .unwrap()
                .columns[0]
                .primary_key
        );
        let ro = Sqlite::connect(path, true, false).await.unwrap();
        let (tx, mut rx) = mpsc::channel(16);
        assert!(
            ro.execute("DELETE FROM t".into(), tx, CancellationToken::new(), 10)
                .await
                .is_err()
        );
        rx.close();
        let token = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(2);
        let d = db.clone();
        let t = token.clone();
        let task = tokio::spawn(async move {
            d.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(), tx, t, 10).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        token.cancel();
        while rx.recv().await.is_some() {}
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
    }
}
