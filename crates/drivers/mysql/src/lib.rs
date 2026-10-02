mod edit;
use async_trait::async_trait;
use futures_util::StreamExt;
use klyndb_driver_api::*;
use mysql_async::{
    Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, SslOpts, Value,
    consts::{ColumnType, StatusFlags},
    prelude::Queryable,
};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub struct Mysql {
    connection: Mutex<Option<Conn>>,
    control: Pool,
    read_only: bool,
    maria: bool,
    explain_analyze: bool,
}
fn err(e: mysql_async::Error) -> Error {
    match e {
        mysql_async::Error::Server(e) => Error::new(format!("{} (MySQL {})", e.message, e.code)),
        _ => Error::new(
            "MySQL connection or protocol failed. Check host, credentials and TLS configuration.",
        ),
    }
}
impl Mysql {
    pub async fn connect(address: &str, password: Option<&str>, read_only: bool) -> Result<Self> {
        let mut url = url::Url::parse(address).map_err(|_| Error::new("Invalid MySQL URL"))?;
        if url.scheme() != "mysql" {
            return Err(Error::new("Expected mysql://user@host/database"));
        }
        let tls = url
            .query_pairs()
            .find(|(k, _)| k == "tls")
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| "required".into());
        if !["required", "disabled"].contains(&tls.as_str()) {
            return Err(Error::new("MySQL tls must be required or disabled"));
        }
        if url.query_pairs().any(|(k, _)| k != "tls") {
            return Err(Error::new("Unsupported MySQL URL option"));
        }
        url.set_query(None);
        let opts =
            Opts::from_url(url.as_str()).map_err(|_| Error::new("Invalid MySQL connection URL"))?;
        let mut builder = OptsBuilder::from_opts(opts)
            .prefer_socket(false)
            .client_found_rows(true)
            .ssl_opts(if tls == "required" {
                Some(SslOpts::default())
            } else {
                None
            })
            .pool_opts(
                PoolOpts::default().with_constraints(
                    PoolConstraints::new(0, 1)
                        .ok_or_else(|| Error::new("Invalid connection pool limit"))?,
                ),
            );
        if let Some(password) = password {
            builder = builder.pass(Some(password));
        }
        let opts: Opts = builder.into();
        let (connection, maria, version) =
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let mut connection = Conn::new(opts.clone()).await?;
                let version: Option<String> = connection.query_first("SELECT VERSION()").await?;
                let maria = version.is_some_and(|v| v.to_ascii_lowercase().contains("mariadb"));
                let version = connection.server_version();
                Ok::<_, mysql_async::Error>((connection, maria, version))
            })
            .await
            .map_err(|_| Error::new("MySQL connection timed out after 10 seconds"))?
            .map_err(err)?;
        // Only cancellation uses the lazy one-connection pool. User SQL retains a dedicated transaction-stable session.
        Ok(Self {
            connection: Mutex::new(Some(connection)),
            control: Pool::new(opts),
            read_only,
            maria,
            explain_analyze: !read_only && version >= if maria { (10, 1, 0) } else { (8, 0, 18) },
        })
    }
    async fn interrupt<F, T>(
        &self,
        id: u32,
        query: &mut Pin<Box<F>>,
        can_kill: impl Fn() -> bool + Send + Sync,
    ) -> (Result<T>, bool)
    where
        F: Future<Output = Result<T>> + Send,
    {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if can_kill()
                && !matches!(
                    tokio::time::timeout_at(deadline, self.kill(id)).await,
                    Ok(Ok(()))
                )
            {
                return (
                    Err(Error::new(
                        "Interruption could not be confirmed; connection closed. Disconnect and reconnect. Verify writes before retrying.",
                    )),
                    true,
                );
            }
            // Keep polling the same future: dropping a partially written command can desynchronize the session.
            match tokio::time::timeout(std::time::Duration::from_millis(50), query.as_mut()).await {
                Ok(result) => return (result, false),
                Err(_) if tokio::time::Instant::now() >= deadline => {
                    return (
                        Err(Error::new(
                            "Operation termination could not be confirmed; connection closed. Disconnect and reconnect. Verify writes before retrying.",
                        )),
                        true,
                    );
                }
                Err(_) => {}
            }
        }
    }
    async fn kill(&self, id: u32) -> Result<()> {
        let mut control = self.control.get_conn().await.map_err(err)?;
        control
            .query_drop(format!("KILL QUERY {id}"))
            .await
            .map_err(err)
    }
}

fn text(bytes: Vec<u8>) -> Cell {
    match String::from_utf8(bytes) {
        Ok(value) => Cell::Text(value),
        // Preserve bytes when a user switches the session away from UTF-8.
        Err(error) => Cell::Binary(hex::encode(error.into_bytes())),
    }
}

fn cell(value: Value, column: &mysql_async::Column) -> Cell {
    match value {
        Value::NULL => Cell::Null,
        Value::Int(n) => Cell::Number(n.to_string()),
        Value::UInt(n) => Cell::Number(n.to_string()),
        Value::Float(n) => Cell::Number(n.to_string()),
        Value::Double(n) => Cell::Number(n.to_string()),
        Value::Bytes(bytes) => {
            if column.column_type().is_numeric_type() {
                Cell::Number(String::from_utf8_lossy(&bytes).into())
            } else if column.column_type() == ColumnType::MYSQL_TYPE_JSON {
                serde_json::from_slice(&bytes)
                    .map(Cell::Json)
                    .unwrap_or_else(|_| text(bytes))
            } else if column.character_set() == 63
                && !matches!(
                    column.column_type(),
                    ColumnType::MYSQL_TYPE_DATE
                        | ColumnType::MYSQL_TYPE_DATETIME
                        | ColumnType::MYSQL_TYPE_TIMESTAMP
                        | ColumnType::MYSQL_TYPE_TIME
                        | ColumnType::MYSQL_TYPE_NEWDATE
                )
            {
                Cell::Binary(hex::encode(bytes))
            } else {
                text(bytes)
            }
        }
        Value::Date(y, m, d, h, i, s, u) => {
            Cell::Text(format!("{y:04}-{m:02}-{d:02} {h:02}:{i:02}:{s:02}.{u:06}"))
        }
        Value::Time(negative, days, h, m, s, u) => Cell::Text(format!(
            "{}{hours:02}:{m:02}:{s:02}.{u:06}",
            if negative { "-" } else { "" },
            hours = days * 24 + u32::from(h)
        )),
    }
}

async fn stream(
    conn: &mut Conn,
    sql: String,
    output: mpsc::Sender<Batch>,
    cancel: CancellationToken,
    limit: usize,
) -> Result<()> {
    let mut result = conn.query_iter(sql).await.map_err(err)?;
    let mut failure = None;
    while let Some(mut rows) = result.stream::<mysql_async::Row>().await.map_err(err)? {
        let columns = rows.columns();
        if output
            .send(Batch::Columns(
                columns.iter().map(|c| c.name_str().to_string()).collect(),
            ))
            .await
            .is_err()
        {
            failure = Some(Error::new("Result consumer closed"));
            cancel.cancel();
        }
        let mut count = 0;
        let mut truncated = false;
        let mut buffer = Vec::with_capacity(256);
        let mut bytes = 0;
        while let Some(row) = rows.next().await {
            let row = row.map_err(err)?;
            if failure.is_some() {
                continue;
            }
            if count >= limit {
                truncated = true;
                continue;
            }
            let values = row
                .unwrap()
                .into_iter()
                .zip(columns.iter())
                .map(|(v, c)| cell(v, c))
                .collect::<Row>();
            let row_bytes = values.iter().map(Cell::byte_len).sum::<usize>();
            if row_bytes > 8 * 1024 * 1024 {
                failure = Some(Error::new(
                    "A result row exceeds 8 MiB. Select smaller values.",
                ));
                cancel.cancel();
                continue;
            }
            bytes += row_bytes;
            buffer.push(values);
            count += 1;
            if buffer.len() == 256 || bytes >= 256 * 1024 {
                bytes = 0;
                if output
                    .send(Batch::Rows(std::mem::take(&mut buffer)))
                    .await
                    .is_err()
                {
                    failure = Some(Error::new("Result consumer closed"));
                    cancel.cancel();
                }
            }
        }
        if failure.is_none() {
            if !buffer.is_empty() && output.send(Batch::Rows(buffer)).await.is_err() {
                failure = Some(Error::new("Result consumer closed"));
                cancel.cancel();
            }
            if output
                .send(Batch::Complete {
                    affected: if columns.is_empty() {
                        rows.affected_rows()
                    } else {
                        0
                    },
                    truncated,
                })
                .await
                .is_err()
            {
                failure = Some(Error::new("Result consumer closed"));
                cancel.cancel();
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

#[async_trait]
impl Session for Mysql {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            transactions: true,
            schemas: true,
            explain: true,
            explain_analyze: self.explain_analyze,
            edit_rows: true,
            cancel: true,
            tls: true,
        }
    }
    fn explain_sql(&self, sql: &str, analyze: bool) -> Result<(String, PlanFormat)> {
        if analyze && !self.explain_analyze {
            return Err(Error::new(
                "Runtime ANALYZE is unavailable on this server or read-only connection",
            ));
        }
        if analyze && !self.maria && !klyndb_query::analyze(sql, "mysql")?.read_only {
            return Err(Error::new(
                "MySQL runtime plans currently support read-only SELECT queries. Use estimated Explain for writes.",
            ));
        }
        let (prefix, format) = match (self.maria, analyze) {
            (true, true) => ("ANALYZE FORMAT=JSON", PlanFormat::MariaJson),
            (true, false) => ("EXPLAIN FORMAT=JSON", PlanFormat::MariaJson),
            (false, true) => ("EXPLAIN ANALYZE FORMAT=TREE", PlanFormat::MysqlTree),
            (false, false) => ("EXPLAIN FORMAT=JSON", PlanFormat::MysqlJson),
        };
        Ok((format!("{prefix} {sql}; SHOW WARNINGS"), format))
    }
    async fn execute(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()> {
        let mut guard = tokio::select! {guard=self.connection.lock()=>guard,_=cancel.cancelled()=>return Err(Error::new("Query cancelled"))};
        let conn = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed. Disconnect and reconnect."))?;
        if cancel.is_cancelled() {
            return Err(Error::new("Query cancelled"));
        }
        if self.read_only {
            if !klyndb_query::analyze(&sql, "mysql")?.read_only {
                return Err(Error::new(
                    "This statement is not allowed on a read-only connection",
                ));
            }
            conn.query_drop("START TRANSACTION READ ONLY")
                .await
                .map_err(err)?;
        }
        let id = conn.id();
        let mut close = false;
        let result = {
            let mut query = Box::pin(stream(conn, sql, output, cancel.clone(), limit));
            tokio::select! {
                result=&mut query=>result,
                _=cancel.cancelled()=>{
                    let (result, closed)=self.interrupt(id,&mut query,||true).await;
                    close=closed;
                    if close {result} else {Err(Error::new("Query cancelled"))}
                }
            }
        };
        if close {
            guard.take();
            return result;
        }
        if self.read_only {
            conn.query_drop("ROLLBACK").await.map_err(err)?;
        }
        result
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        let mut guard = self.connection.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let rows:Vec<(String,String,String)>=conn.query("SELECT TABLE_SCHEMA,TABLE_NAME,LOWER(TABLE_TYPE) FROM information_schema.TABLES WHERE TABLE_SCHEMA NOT IN ('mysql','information_schema','performance_schema','sys') ORDER BY TABLE_SCHEMA,TABLE_NAME").await.map_err(err)?;
        Ok(rows
            .into_iter()
            .map(|(schema, name, kind)| Table { schema, name, kind })
            .collect())
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let mut guard = self.connection.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let columns = columns(conn, table).await?;
        let editable = editable(conn, table).await?;
        let indexes:Vec<(String,Option<String>,String,u64)>=conn.exec("SELECT INDEX_NAME,COLUMN_NAME,INDEX_TYPE,NON_UNIQUE FROM information_schema.STATISTICS WHERE TABLE_SCHEMA=? AND TABLE_NAME=? ORDER BY INDEX_NAME,SEQ_IN_INDEX",(&table.schema,&table.name)).await.map_err(err)?;
        let foreign:Vec<(String,String,String,String,String)>=conn.exec("SELECT CONSTRAINT_NAME,COLUMN_NAME,REFERENCED_TABLE_SCHEMA,REFERENCED_TABLE_NAME,REFERENCED_COLUMN_NAME FROM information_schema.KEY_COLUMN_USAGE WHERE TABLE_SCHEMA=? AND TABLE_NAME=? AND REFERENCED_TABLE_NAME IS NOT NULL",(&table.schema,&table.name)).await.map_err(err)?;
        let ddl: Option<mysql_async::Row> = conn
            .query_first(format!(
                "SHOW CREATE TABLE {}.{}",
                self.quote_identifier(&table.schema),
                self.quote_identifier(&table.name)
            ))
            .await
            .map_err(err)?;
        let ddl = ddl
            .and_then(|row| row.get_opt::<String, _>(1))
            .transpose()
            .map_err(|_| Error::new("Could not decode table DDL"))?;
        let indexes = indexes.into_iter().map(|(name,column,kind,non_unique)|serde_json::json!({"name":name,"column":column,"type":kind,"unique":non_unique==0})).collect();
        let foreign_keys = foreign.into_iter().map(|(name,column,schema,table,target)|serde_json::json!({"name":name,"column":column,"schema":schema,"table":table,"target":target})).collect();
        Ok(TableInfo {
            editable,
            columns,
            ddl,
            indexes,
            foreign_keys,
        })
    }
    fn quote_identifier(&self, name: &str) -> String {
        quote(name)
    }
    async fn transaction_state(&self) -> Result<TransactionState> {
        let mut guard = self.connection.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        // A harmless probe refreshes protocol status after errors; MySQL lacks MariaDB's @@in_transaction.
        conn.query_drop("SELECT 1").await.map_err(err)?;
        Ok(
            if conn.last_ok_packet().is_some_and(|p| {
                p.status_flags()
                    .contains(StatusFlags::SERVER_STATUS_IN_TRANS)
            }) {
                TransactionState::Active
            } else {
                TransactionState::Idle
            },
        )
    }
    async fn apply_changes(&self, table: Table, changes: Vec<Change>) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        validate_change_batch(&changes)?;
        let mut guard = self.connection.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed. Disconnect and reconnect."))?;
        let id = conn.id();
        let abort = CancellationToken::new();
        let committing = AtomicBool::new(false);
        let poison = AtomicBool::new(false);
        let (result, close) = {
            let mut operation = Box::pin(edit::apply(
                conn,
                &table,
                &changes,
                &abort,
                &committing,
                &poison,
            ));
            tokio::select! {
                result=&mut operation=>(result,false),
                _=tokio::time::sleep(std::time::Duration::from_secs(60))=>{
                    abort.cancel();
                    // Once COMMIT has started, drain its acknowledgement without interrupting it.
                    self.interrupt(id,&mut operation,||!committing.load(Ordering::Relaxed)).await
                }
            }
        };
        if close || poison.load(Ordering::Relaxed) {
            guard.take();
        }
        result
    }
    async fn disconnect(&self) -> Result<()> {
        if let Some(conn) = self.connection.lock().await.take() {
            conn.disconnect().await.map_err(err)?;
        }
        self.control.clone().disconnect().await.map_err(err)
    }
}

fn quote(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}
async fn columns(conn: &mut Conn, table: &Table) -> Result<Vec<Column>> {
    let rows:Vec<(String,String,String,Option<String>,String,String)>=conn.exec("SELECT COLUMN_NAME,COLUMN_TYPE,IS_NULLABLE,COLUMN_DEFAULT,COLUMN_KEY,EXTRA FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=? AND TABLE_NAME=? ORDER BY ORDINAL_POSITION",(&table.schema,&table.name)).await.map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|(name, data_type, nullable, default, key, extra)| Column {
            name,
            data_type,
            nullable: nullable == "YES",
            default,
            primary_key: key == "PRI",
            generated: [
                "VIRTUAL GENERATED",
                "STORED GENERATED",
                "PERSISTENT GENERATED",
            ]
            .iter()
            .any(|kind| extra.to_uppercase().contains(kind)),
        })
        .collect())
}
async fn editable(conn: &mut Conn, table: &Table) -> Result<bool> {
    let kind:Option<(String,Option<String>)>=conn.exec_first("SELECT TABLE_TYPE,ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?",(&table.schema,&table.name)).await.map_err(err)?;
    Ok(kind.is_some_and(|(kind, engine)| {
        kind == "BASE TABLE" && engine.is_some_and(|e| e.eq_ignore_ascii_case("InnoDB"))
    }))
}
