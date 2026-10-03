use async_trait::async_trait;
mod edit;
use duckdb::arrow::{
    array::{
        Array, BinaryArray, BooleanArray, Decimal128Array, FixedSizeBinaryArray, LargeBinaryArray,
    },
    datatypes::DataType,
    util::display::array_value_to_string,
};
use duckdb::core::{LogicalTypeHandle, LogicalTypeId};
use duckdb::{AccessMode, Config, Connection};
use klyndb_driver_api::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct DuckDb {
    connection: Arc<Mutex<Option<FileConnection>>>,
    read_only: bool,
}
struct Database {
    connection: Mutex<Connection>,
    read_only: bool,
}
static DATABASES: OnceLock<Mutex<HashMap<PathBuf, Weak<Database>>>> = OnceLock::new();
struct FileConnection {
    connection: Option<Connection>,
    database: Option<Arc<Database>>,
}
impl Drop for FileConnection {
    fn drop(&mut self) {
        // Finish native close/checkpoint before a new open can observe an expired weak entry.
        let _databases = DATABASES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.connection.take();
        self.database.take();
    }
}
fn err(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}
impl DuckDb {
    pub async fn connect(path: String, read_only: bool, create: bool) -> Result<Self> {
        let connection = tokio::task::spawn_blocking(move || {
            if path.is_empty() || path == ":memory:" || path.contains('\0') {
                return Err(Error::new("Choose a DuckDB database file"));
            }
            if !create && !std::path::Path::new(&path).is_file() {
                return Err(Error::new(
                    "DuckDB database file does not exist. Use + to create one.",
                ));
            }
            let file = Path::new(&path);
            let path = if file.exists() {
                file.canonicalize().map_err(err)?
            } else {
                file.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .canonicalize()
                    .map_err(err)?
                    .join(file.file_name().ok_or_else(|| Error::new("Choose a DuckDB database file"))?)
            };
            // DuckDB requires one database instance per file in a process. Native clones
            // share that instance while keeping independent sessions and transactions.
            // ponytail: serialize native open/close globally; use per-file locks if file setup becomes a bottleneck.
            let mut databases = DATABASES.get_or_init(Mutex::default).lock().map_err(err)?;
            databases.retain(|_, db| db.strong_count() > 0);
            let database = if let Some(database) = databases.get(&path).and_then(Weak::upgrade) {
                if database.read_only != read_only {
                    return Err(Error::new("This DuckDB file is already open with different read-only settings. Disconnect its existing connections before testing or changing access mode."));
                }
                database
            } else {
                let config = Config::default()
                .access_mode(if read_only {
                    AccessMode::ReadOnly
                } else {
                    AccessMode::ReadWrite
                })
                .and_then(|c| c.enable_autoload_extension(false))
                .and_then(|c| c.enable_external_access(false))
                // ponytail: conservative per-file budget; expose driver settings after measuring multi-connection workloads.
                .and_then(|c| c.max_memory("256MB"))
                .and_then(|c| c.threads(2))
                .map_err(err)?;
                let connection = Connection::open_with_flags(&path, config).map_err(err)?;
                let database = Arc::new(Database { connection: Mutex::new(connection), read_only });
                databases.insert(path, Arc::downgrade(&database));
                database
            };
            let conn = database.connection.lock().map_err(err)?.try_clone().map_err(err)?;
            conn.query_row("SELECT 1", [], |r| r.get::<_, i32>(0))
                .map_err(err)?;
            drop(databases);
            Ok::<_, Error>(FileConnection { connection: Some(conn), database: Some(database) })
        })
        .await
        .map_err(err)??;
        Ok(Self {
            connection: Arc::new(Mutex::new(Some(connection))),
            read_only,
        })
    }
    async fn write(
        &self,
        table: Table,
        source: edit::Source,
        cancel: CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = connection.lock().map_err(err)?;
            let conn = guard
                .as_ref()
                .and_then(|file| file.connection.as_ref())
                .ok_or_else(|| Error::new("Connection is closed"))?;
            let (result, poison) = edit::write(conn, &table, source, cancel, timeout);
            if poison {
                guard.take();
            }
            result
        })
        .await
        .map_err(err)?
    }

    async fn with<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let guard = connection.lock().map_err(err)?;
            f(guard
                .as_ref()
                .and_then(|file| file.connection.as_ref())
                .ok_or_else(|| Error::new("Connection is closed"))?)
        })
        .await
        .map_err(err)?
    }
}

fn transaction_state(conn: &Connection) -> Result<TransactionState> {
    // duckdb-rs is_autocommit() currently always returns true. Ask the native engine:
    // independent autocommit statements have different IDs; an explicit transaction keeps its ID.
    let id = || {
        conn.query_row("SELECT current_transaction_id()::VARCHAR", [], |r| {
            r.get::<_, String>(0)
        })
    };
    match (id(), id()) {
        (Ok(a), Ok(b)) => Ok(if a == b {
            TransactionState::Active
        } else {
            TransactionState::Idle
        }),
        (Err(e), _) | (_, Err(e)) if e.to_string().contains("Current transaction is aborted") => {
            Ok(TransactionState::Failed)
        }
        (Err(e), _) | (_, Err(e)) => Err(err(e)),
    }
}

fn foreign_keys(conn: &Connection, table: &Table) -> Result<Vec<ForeignKey>> {
    // DuckDB 1.5 rejects foreign keys across schemas/catalogs; targets share the source schema.
    let mut stmt = conn.prepare("SELECT constraint_name,unnest(constraint_column_names) AS source_column,schema_name,referenced_table,unnest(referenced_column_names) AS target_column,generate_subscripts(constraint_column_names,1) AS ordinal FROM duckdb_constraints() WHERE database_name=current_database() AND schema_name=? AND table_name=? AND constraint_type='FOREIGN KEY' ORDER BY constraint_index,ordinal").map_err(err)?;
    let rows = stmt
        .query_map([&table.schema, &table.name], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(err)?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(err)?;
    Ok(group_foreign_keys(rows))
}

fn inspect(conn: &Connection, table: &Table, read_only: bool) -> Result<TableInfo> {
    let args = [&table.schema, &table.name];
    let mut stmt = conn.prepare("SELECT column_name,data_type,is_nullable,column_default, EXISTS(SELECT 1 FROM duckdb_constraints() k WHERE k.database_name=c.database_name AND k.schema_name=c.schema_name AND k.table_name=c.table_name AND constraint_type='PRIMARY KEY' AND list_contains(constraint_column_names,c.column_name)) FROM duckdb_columns() c WHERE database_name=current_database() AND schema_name=? AND table_name=? ORDER BY column_index").map_err(err)?;
    let mut columns = stmt
        .query_map(args, |r| {
            Ok(Column {
                name: r.get(0)?,
                data_type: r.get(1)?,
                nullable: r.get(2)?,
                default: r.get(3)?,
                primary_key: r.get(4)?,
                generated: false,
            })
        })
        .map_err(err)?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(err)?;
    if columns.is_empty() {
        return Err(Error::new("Table no longer exists. Refresh the schema."));
    }
    let ddl: Option<String> = conn.query_row("SELECT sql FROM duckdb_tables() WHERE database_name=current_database() AND schema_name=? AND table_name=? UNION ALL SELECT sql FROM duckdb_views() WHERE database_name=current_database() AND schema_name=? AND view_name=?", [&table.schema, &table.name, &table.schema, &table.name], |r| r.get(0)).map_err(err)?;
    let mut stmt = conn.prepare("SELECT index_name,is_unique,sql FROM duckdb_indexes() WHERE database_name=current_database() AND schema_name=? AND table_name=? ORDER BY index_name").map_err(err)?;
    let indexes = stmt.query_map(args, |r| Ok(serde_json::json!({"name":r.get::<_, String>(0)?, "unique":r.get::<_, bool>(1)?, "definition":r.get::<_, String>(2)?}))).map_err(err)?.collect::<duckdb::Result<Vec<_>>>().map_err(err)?;
    let mut stmt = conn.prepare("SELECT constraint_name,constraint_type,constraint_text FROM duckdb_constraints() WHERE database_name=current_database() AND schema_name=? AND table_name=? ORDER BY constraint_index").map_err(err)?;
    let constraints = stmt
        .query_map(args, |r| {
            Ok(Constraint {
                name: r.get(0)?,
                kind: r.get(1)?,
                definition: r.get(2)?,
            })
        })
        .map_err(err)?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(err)?;
    let foreign_keys = foreign_keys(conn, table)?
        .into_iter()
        .map(serde_json::to_value)
        .collect::<serde_json::Result<Vec<_>>>()
        .map_err(err)?;
    let generated = ddl
        .as_deref()
        .and_then(|sql| klyndb_query::duckdb_generated_columns(sql).ok());
    let known_columns = generated.as_ref().is_some_and(|fields| {
        fields.len() == columns.len() && columns.iter().all(|c| fields.contains_key(&c.name))
    });
    if let Some(fields) = generated.as_ref().filter(|_| known_columns) {
        for column in &mut columns {
            column.generated = fields[&column.name];
            if column.generated {
                column.default = None;
            }
        }
    }
    let base: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM duckdb_tables() WHERE database_name=current_database() AND schema_name=? AND table_name=? AND NOT internal)", args, |r| r.get(0)).map_err(err)?;
    let editable = !read_only
        && base
        && known_columns
        && columns.iter().all(|c| edit::supported_type(&c.data_type));
    Ok(TableInfo {
        editable,
        columns,
        ddl,
        indexes,
        foreign_keys,
        constraints: Some(constraints),
        triggers: vec![],
    })
}

fn cell(array: &dyn Array, row: usize, logical: LogicalTypeId) -> Result<Cell> {
    if array.is_null(row) {
        return Ok(Cell::Null);
    }
    if matches!(logical, LogicalTypeId::Hugeint | LogicalTypeId::UHugeint) {
        let a = array
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .ok_or_else(|| {
                Error::new("Unsupported DuckDB 128-bit integer representation. CAST to VARCHAR.")
            })?;
        return Ok(Cell::Number(if logical == LogicalTypeId::UHugeint {
            (a.value(row) as u128).to_string()
        } else {
            a.value(row).to_string()
        }));
    }
    if logical == LogicalTypeId::Uuid
        && let Some(a) = array.as_any().downcast_ref::<FixedSizeBinaryArray>()
    {
        return Ok(Cell::Text(
            uuid::Uuid::from_slice(a.value(row))
                .map_err(err)?
                .to_string(),
        ));
    }
    let value = array_value_to_string(array, row).map_err(err)?;
    Ok(match array.data_type() {
        DataType::Boolean => Cell::Boolean(
            array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| Error::new("Invalid boolean array"))?
                .value(row),
        ),
        DataType::Binary => Cell::Binary(hex::encode(
            array
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| Error::new("Invalid binary array"))?
                .value(row),
        )),
        DataType::LargeBinary => Cell::Binary(hex::encode(
            array
                .as_any()
                .downcast_ref::<LargeBinaryArray>()
                .ok_or_else(|| Error::new("Invalid binary array"))?
                .value(row),
        )),
        DataType::FixedSizeBinary(_) if logical == LogicalTypeId::Blob => {
            Cell::Binary(hex::encode(
                array
                    .as_any()
                    .downcast_ref::<FixedSizeBinaryArray>()
                    .ok_or_else(|| Error::new("Invalid binary array"))?
                    .value(row),
            ))
        }
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Decimal128(_, _)
        | DataType::Decimal256(_, _) => Cell::Number(value),
        DataType::Float16 | DataType::Float32 | DataType::Float64
            if !["NaN", "inf", "-inf"].contains(&value.as_str()) =>
        {
            Cell::Number(value)
        }
        _ => Cell::Text(value),
    })
}
fn validate_type(logical: &LogicalTypeHandle, nested: bool) -> Result<()> {
    if matches!(
        logical.id(),
        LogicalTypeId::Bignum | LogicalTypeId::Bit | LogicalTypeId::TimeTZ
    ) || nested
        && matches!(
            logical.id(),
            LogicalTypeId::Hugeint | LogicalTypeId::UHugeint
        )
    {
        return Err(Error::new(
            "This DuckDB result type is not yet decoded losslessly. CAST it to VARCHAR in the query.",
        ));
    }
    for i in 0..logical.num_children() {
        validate_type(&logical.child(i), true)?;
    }
    Ok(())
}

fn stream(
    conn: &Connection,
    sql: String,
    output: mpsc::Sender<Batch>,
    cancel: CancellationToken,
    limit: usize,
) -> Result<()> {
    let runtime = tokio::runtime::Handle::current();
    let finished = CancellationToken::new();
    let done = finished.clone();
    let token = cancel.clone();
    let consumer = output.clone();
    let interrupt = conn.interrupt_handle();
    let watcher = runtime.spawn(async move {
        tokio::select! {
            biased;
            _ = done.cancelled() => return,
            _ = token.cancelled() => {},
            _ = consumer.closed() => token.cancel(),
        }
        // DuckDB resets its interrupt flag when starting a native statement.
        // Keep cancellation asserted until this query scope has actually ended.
        loop {
            interrupt.interrupt();
            tokio::select! {
                biased;
                _ = done.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
            }
        }
    });
    let send = |batch| {
        runtime.block_on(async {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::new("Query cancelled")),
                result = output.send(batch) => result.map_err(err),
            }
        })
    };
    let result = (|| {
        let mut reader = klyndb_import::SqlReader::new(sql.as_bytes(), "duckdb")?;
        reader.set_cancel(cancel.clone());
        while let Some(sql) = reader.next_statement()? {
            if cancel.is_cancelled() {
                return Err(Error::new("Query cancelled"));
            }
            let mut stmt = conn.prepare(&sql).map_err(err)?;
            if cancel.is_cancelled() {
                return Err(Error::new("Query cancelled"));
            }
            if klyndb_query::duckdb_count_result(&sql)? {
                let affected = stmt.execute([]).map_err(err)? as u64;
                send(Batch::Columns(vec![]))?;
                send(Batch::Complete {
                    affected,
                    truncated: false,
                })?;
                continue;
            }
            let schema = stmt.stream_arrow([]).map_err(err)?.get_schema();
            let types = (0..if limit == 0 { 0 } else { schema.fields().len() })
                .map(|i| {
                    let logical = stmt.column_logical_type(i);
                    validate_type(&logical, false)?;
                    Ok(logical.id())
                })
                .collect::<Result<Vec<_>>>()?;
            send(Batch::Columns(
                schema.fields().iter().map(|f| f.name().clone()).collect(),
            ))?;
            let mut count = 0;
            let mut truncated = false;
            let mut buffer = Vec::with_capacity(256);
            let mut bytes = 0;
            'chunks: while let Some(chunk) = stmt.step().map_err(err)? {
                for index in 0..chunk.len() {
                    if cancel.is_cancelled() {
                        return Err(Error::new("Query cancelled"));
                    }
                    if limit == 0 {
                        continue;
                    }
                    if count >= limit {
                        truncated = true;
                        break 'chunks;
                    }
                    let row = chunk
                        .columns()
                        .iter()
                        .zip(&types)
                        .map(|(a, logical)| cell(a.as_ref(), index, *logical))
                        .collect::<Result<Row>>()?;
                    let size = row.iter().map(Cell::byte_len).sum::<usize>();
                    if size > 8 * 1024 * 1024 {
                        return Err(Error::new(
                            "A result row exceeds the 8 MiB safety limit. Select smaller columns.",
                        ));
                    }
                    if !buffer.is_empty() && bytes + size > 256 * 1024 {
                        send(Batch::Rows(std::mem::take(&mut buffer)))?;
                        bytes = 0;
                    }
                    bytes += size;
                    buffer.push(row);
                    count += 1;
                    if buffer.len() == 256 {
                        send(Batch::Rows(std::mem::take(&mut buffer)))?;
                        bytes = 0;
                    }
                }
            }
            if !buffer.is_empty() {
                send(Batch::Rows(buffer))?;
            }
            send(Batch::Complete {
                affected: 0,
                truncated,
            })?;
        }
        Ok(())
    })();
    finished.cancel();
    // Join under the serialized session lock so a late interrupt cannot reach the next query.
    runtime.block_on(watcher).map_err(err)?;
    if cancel.is_cancelled() {
        Err(Error::new("Query cancelled"))
    } else {
        result
    }
}

#[async_trait]
impl Session for DuckDb {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            key_value: false,
            affected_rows: true,
            table_browse: true,
            routines: false,
            diagrams: true,
            transactions: true,
            schemas: true,
            explain: true,
            explain_analyze: !self.read_only,
            edit_rows: !self.read_only,
            import_rows: !self.read_only,
            import_sql: !self.read_only,
            cancel: true,
            tls: false,
        }
    }
    fn explain_sql(&self, sql: &str, analyze: bool) -> Result<(String, PlanFormat)> {
        if analyze && self.read_only {
            return Err(Error::new(
                "ANALYZE executes the statement and is disabled on read-only connections",
            ));
        }
        Ok((
            format!(
                "EXPLAIN ({}FORMAT JSON) {sql}",
                if analyze { "ANALYZE, " } else { "" }
            ),
            PlanFormat::DuckDbJson,
        ))
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
    async fn transaction_state(&self) -> Result<TransactionState> {
        self.with(transaction_state).await
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT schema_name,table_name,'table' FROM duckdb_tables() WHERE NOT internal AND database_name=current_database() UNION ALL SELECT schema_name,view_name,'view' FROM duckdb_views() WHERE NOT internal AND database_name=current_database() ORDER BY 1,2").map_err(err)?;
            stmt.query_map([], |r| Ok(Table { schema: r.get(0)?, name: r.get(1)?, kind: r.get(2)? })).map_err(err)?.collect::<duckdb::Result<Vec<_>>>().map_err(err)
        }).await
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let table = table.clone();
        let read_only = self.read_only;
        self.with(move |conn| inspect(conn, &table, read_only))
            .await
    }
    async fn relationships(&self, table: &Table) -> Result<Vec<ForeignKey>> {
        let table = table.clone();
        self.with(move |conn| foreign_keys(conn, &table)).await
    }

    async fn apply_changes(&self, table: Table, changes: Vec<Change>) -> Result<MutationResult> {
        validate_change_batch(&changes)?;
        self.write(
            table,
            edit::Source::Edits(Some(changes)),
            CancellationToken::new(),
            std::time::Duration::from_secs(60),
        )
        .await
    }
    async fn insert_stream(
        &self,
        table: Table,
        input: mpsc::Receiver<Result<InsertBatch>>,
        cancel: CancellationToken,
    ) -> Result<MutationResult> {
        self.write(
            table,
            edit::Source::Import(input),
            cancel,
            std::time::Duration::from_secs(3600),
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_transaction_probe() {
        let dir = tempfile::tempdir().unwrap();
        let db = DuckDb::connect(
            dir.path()
                .join("test.duckdb")
                .to_string_lossy()
                .into_owned(),
            false,
            true,
        )
        .await
        .unwrap();
        db.with(|conn| {
            let id = || {
                conn.query_row("SELECT current_transaction_id()::VARCHAR", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap()
            };
            assert_ne!(id(), id());
            conn.execute_batch("BEGIN; CREATE TABLE t(i INTEGER UNIQUE); INSERT INTO t VALUES(1)")
                .unwrap();
            assert_eq!(id(), id());
            assert!(conn.execute_batch("INSERT INTO t VALUES(1)").is_err());
            let failed = conn
                .query_row("SELECT current_transaction_id()", [], |r| {
                    r.get::<_, u64>(0)
                })
                .unwrap_err();
            eprintln!("native failed-transaction probe: {failed}");
            conn.execute_batch("ROLLBACK").unwrap();
            assert_ne!(id(), id());
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod contract {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn ordered_foreign_keys_in_quoted_schema_and_read_only_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("relations.duckdb")
            .to_string_lossy()
            .into_owned();
        let db = DuckDb::connect(path.clone(), false, true).await.unwrap();
        db.with(|conn| {
            conn.execute_batch(r#"
                CREATE SCHEMA "quoted schema";
                CREATE TABLE "quoted schema"."parent's & <table>" (
                    "first key" INTEGER, "second""key" INTEGER, u INTEGER UNIQUE,
                    PRIMARY KEY("first key", "second""key")
                );
                CREATE TABLE "quoted schema".child (
                    sb INTEGER, sa INTEGER, u INTEGER,
                    FOREIGN KEY(sa,sb) REFERENCES "quoted schema"."parent's & <table>"("first key","second""key"),
                    FOREIGN KEY(u) REFERENCES "quoted schema"."parent's & <table>"(u)
                );
                CREATE TABLE "quoted schema".implicit_child (
                    x INTEGER, y INTEGER,
                    FOREIGN KEY(x,y) REFERENCES "quoted schema"."parent's & <table>"
                );
                CREATE TABLE child (unrelated INTEGER);
            "#).map_err(err)?;
            Ok(())
        }).await.unwrap();
        let table = Table {
            schema: "quoted schema".into(),
            name: "child".into(),
            kind: "table".into(),
        };
        let keys = db.relationships(&table).await.unwrap();
        assert_eq!(keys.len(), 2);
        let composite = keys.iter().find(|k| k.columns.len() == 2).unwrap();
        assert_eq!(composite.columns, ["sa", "sb"]);
        assert_eq!(
            composite.target_columns,
            [Some("first key".into()), Some("second\"key".into())]
        );
        for key in &keys {
            assert_eq!(key.target_schema, "quoted schema");
            assert_eq!(key.target_table, "parent's & <table>");
        }
        let unique = keys.iter().find(|k| k.columns.len() == 1).unwrap();
        assert_eq!(unique.columns, ["u"]);
        assert_eq!(unique.target_columns, [Some("u".into())]);
        assert_eq!(
            db.inspect(&table).await.unwrap().foreign_keys,
            keys.iter()
                .map(|k| serde_json::to_value(k).unwrap())
                .collect::<Vec<_>>()
        );
        let implicit = Table {
            name: "implicit_child".into(),
            ..table.clone()
        };
        let inferred = db.relationships(&implicit).await.unwrap();
        assert_eq!(inferred.len(), 1);
        assert_eq!(inferred[0].columns, ["x", "y"]);
        assert_eq!(inferred[0].target_columns, composite.target_columns);
        assert!(
            db.relationships(&Table {
                schema: "main".into(),
                ..table.clone()
            })
            .await
            .unwrap()
            .is_empty()
        );
        db.disconnect().await.unwrap();
        let readonly = DuckDb::connect(path, true, false).await.unwrap();
        assert!(readonly.capabilities().diagrams);
        assert_eq!(
            serde_json::to_value(readonly.relationships(&table).await.unwrap()).unwrap(),
            serde_json::to_value(keys).unwrap()
        );
        assert_eq!(
            readonly.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        readonly.disconnect().await.unwrap();
    }

    pub(super) async fn query(db: &Arc<DuckDb>, sql: &str, limit: usize) -> Result<Vec<Batch>> {
        let (tx, mut rx) = mpsc::channel(2);
        let driver = db.clone();
        let sql = sql.to_owned();
        let task = tokio::spawn(async move {
            driver
                .execute(sql, tx, CancellationToken::new(), limit)
                .await
        });
        let mut batches = vec![];
        while let Some(b) = rx.recv().await {
            batches.push(b);
        }
        task.await.map_err(err)??;
        Ok(batches)
    }
    pub(super) fn rows(batches: &[Batch]) -> Vec<Row> {
        batches
            .iter()
            .flat_map(|b| {
                if let Batch::Rows(rows) = b {
                    rows.clone()
                } else {
                    vec![]
                }
            })
            .collect()
    }
    #[tokio::test]
    async fn real_file_query_metadata_precision_transactions_and_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("contract.duckdb")
            .to_string_lossy()
            .into_owned();
        assert!(DuckDb::connect(path.clone(), false, false).await.is_err());
        assert!(!std::path::Path::new(&path).exists());
        assert!(DuckDb::connect(path.clone(), true, false).await.is_err());
        let db = Arc::new(DuckDb::connect(path.clone(), false, true).await.unwrap());
        let setup = query(&db, "CREATE SCHEMA analytics; CREATE TABLE analytics.t(id BIGINT PRIMARY KEY, label VARCHAR, exact DECIMAL(38,18), payload BLOB); INSERT INTO analytics.t VALUES(9223372036854775807,'é;''\\next',12345678901234567890.123456789012345678,from_hex('00ff')), (2,NULL,NULL,NULL); CREATE INDEX label_idx ON analytics.t(label); CREATE VIEW analytics.v AS SELECT * FROM analytics.t", 100).await.unwrap();
        assert!(
            setup
                .iter()
                .any(|b| matches!(b, Batch::Complete { affected: 2, .. }))
        );
        let data = rows(&query(&db, "SELECT id,label,exact,payload FROM analytics.t ORDER BY id DESC; SELECT '340282366920938463463374607431768211455'::UHUGEINT, '-170141183460469231731687303715884105728'::HUGEINT, true, NULL, 'a1a2a3a4-b1b2-c1c2-d1d2-d3d4d5d6d7d8'::UUID; /* nested /* ; */ comment */ SELECT $$unchanged; 'é'$$", 100).await.unwrap());
        assert_eq!(
            data[0],
            vec![
                Cell::Number("9223372036854775807".into()),
                Cell::Text("é;'\\next".into()),
                Cell::Number("12345678901234567890.123456789012345678".into()),
                Cell::Binary("00ff".into())
            ]
        );
        assert_eq!(
            data[1],
            vec![Cell::Number("2".into()), Cell::Null, Cell::Null, Cell::Null]
        );
        assert_eq!(
            data[2],
            vec![
                Cell::Number(u128::MAX.to_string()),
                Cell::Number(i128::MIN.to_string()),
                Cell::Boolean(true),
                Cell::Null,
                Cell::Text("a1a2a3a4-b1b2-c1c2-d1d2-d3d4d5d6d7d8".into())
            ]
        );
        assert_eq!(data[3], vec![Cell::Text("unchanged; 'é'".into())]);
        let tables = db.tables().await.unwrap();
        assert_eq!(tables.len(), 2);
        let table = tables.iter().find(|t| t.kind == "table").unwrap();
        let info = db.inspect(table).await.unwrap();
        assert!(info.editable);
        assert!(info.columns[0].primary_key);
        assert!(!info.columns[0].nullable);
        assert!(info.ddl.unwrap().contains("CREATE TABLE"));
        assert_eq!(info.indexes[0]["name"], "label_idx");
        assert!(
            info.constraints
                .unwrap()
                .iter()
                .any(|c| c.kind == "PRIMARY KEY")
        );
        assert!(
            db.inspect(tables.iter().find(|t| t.kind == "view").unwrap())
                .await
                .is_ok()
        );
        let page = db
            .table_query_sql(
                table,
                &info.columns,
                &TableQuery {
                    filters: vec![TableFilter {
                        column: "id".into(),
                        op: FilterOp::Equal,
                        value: "2".into(),
                    }],
                    sort: vec![],
                    limit: 100,
                    offset: 0,
                },
            )
            .unwrap();
        assert_eq!(rows(&query(&db, &page, 100).await.unwrap()).len(), 1);
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        query(
            &db,
            "BEGIN; UPDATE analytics.t SET label='pending' WHERE id=2",
            100,
        )
        .await
        .unwrap();
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        assert!(
            DuckDb::connect(path.clone(), true, false)
                .await
                .err()
                .unwrap()
                .message
                .contains("read-only")
        );
        let probe = Arc::new(DuckDb::connect(path.clone(), false, false).await.unwrap());
        assert_eq!(
            rows(
                &query(&probe, "SELECT label FROM analytics.t WHERE id=2", 100)
                    .await
                    .unwrap()
            ),
            vec![vec![Cell::Null]]
        );
        probe.disconnect().await.unwrap();
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        assert!(
            query(&db, "INSERT INTO analytics.t(id) VALUES(2)", 100)
                .await
                .is_err()
        );
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Failed
        );
        query(&db, "ROLLBACK", 100).await.unwrap();
        assert_eq!(
            db.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        assert_eq!(
            rows(
                &query(&db, "SELECT label FROM analytics.t WHERE id=2", 100)
                    .await
                    .unwrap()
            ),
            vec![vec![Cell::Null]]
        );
        let limited = query(&db, "SELECT i FROM range(1000000) r(i)", 1003)
            .await
            .unwrap();
        assert_eq!(rows(&limited).len(), 1003);
        assert!(limited.iter().any(|b| matches!(
            b,
            Batch::Complete {
                truncated: true,
                ..
            }
        )));
        assert_eq!(rows(&query(&db, "SELECT current_setting('enable_external_access'), current_setting('autoinstall_known_extensions'), current_setting('autoload_known_extensions')", 100).await.unwrap()), vec![vec![Cell::Boolean(false),Cell::Boolean(false),Cell::Boolean(false)]]);
        let external = dir.path().join("external.txt");
        std::fs::write(&external, "owned external-file fixture").unwrap();
        let blocked = query(
            &db,
            &format!(
                "SELECT content FROM read_text({})",
                db.quote_filter_value(&external.to_string_lossy())
            ),
            100,
        )
        .await
        .err()
        .unwrap();
        assert!(blocked.message.contains("disabled"), "{}", blocked.message);
        assert!(
            query(
                &db,
                "SELECT ['340282366920938463463374607431768211455'::UHUGEINT]",
                100
            )
            .await
            .err()
            .unwrap()
            .message
            .contains("VARCHAR")
        );
        db.disconnect().await.unwrap();
        assert!(db.tables().await.is_err());
        let ro = Arc::new(DuckDb::connect(path, true, false).await.unwrap());
        assert_eq!(
            rows(
                &query(&ro, "SELECT count(*) FROM analytics.t", 100)
                    .await
                    .unwrap()
            ),
            vec![vec![Cell::Number("2".into())]]
        );
        assert!(query(&ro, "DELETE FROM analytics.t", 100).await.is_err());
        ro.disconnect().await.unwrap();
        let invalid = dir.path().join("invalid.duckdb");
        std::fs::write(&invalid, b"this is not a DuckDB file").unwrap();
        assert!(
            DuckDb::connect(invalid.to_string_lossy().into_owned(), false, false)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(invalid).unwrap(),
            b"this is not a DuckDB file"
        );
    }
    #[tokio::test]
    async fn native_cancellation_backpressure_and_session_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DuckDb::connect(
                dir.path()
                    .join("cancel.duckdb")
                    .to_string_lossy()
                    .into_owned(),
                false,
                true,
            )
            .await
            .unwrap(),
        );
        for sql in [
            "SELECT sum(a.i*b.i) FROM range(1000000000) a(i),range(1000000000) b(i)",
            "SELECT i FROM range(1000000000) r(i)",
        ] {
            let (tx, mut rx) = mpsc::channel(1);
            let token = CancellationToken::new();
            let driver = db.clone();
            let cancel = token.clone();
            let sql = sql.to_owned();
            let task =
                tokio::spawn(async move { driver.execute(sql, tx, cancel, usize::MAX).await });
            // Leave the receiver alive but unconsumed to exercise bounded-send cancellation.
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
            let result = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(result.message.contains("cancelled"), "{}", result.message);
            rx.close();
            assert_eq!(
                rows(&query(&db, "SELECT 42", 10).await.unwrap()),
                vec![vec![Cell::Number("42".into())]]
            );
        }
        let (tx, rx) = mpsc::channel(1);
        let driver = db.clone();
        let task = tokio::spawn(async move {
            driver
                .execute(
                    "SELECT i FROM range(1000000000) r(i)".into(),
                    tx,
                    CancellationToken::new(),
                    usize::MAX,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(rx);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(
            rows(&query(&db, "SELECT 43", 10).await.unwrap()),
            vec![vec![Cell::Number("43".into())]]
        );
        db.disconnect().await.unwrap();
    }
}
