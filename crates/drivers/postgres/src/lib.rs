mod edit;
mod import;
use async_trait::async_trait;
use futures_util::{StreamExt, pin_mut};
use klyndb_driver_api::*;
use postgres_native_tls::MakeTlsConnector;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use tokio_postgres::{Client, SimpleQueryMessage};
use tokio_util::sync::CancellationToken;

pub struct Postgres {
    client: Client,
    worker: JoinHandle<()>,
    serial: Mutex<()>,
    tls: MakeTlsConnector,
    read_only: bool,
    server_sql_ascii: bool,
}
// Server error text can contain SQL literals. Never include submitted URLs/passwords in client errors.
fn err(e: tokio_postgres::Error) -> Error {
    if let Some(db) = e.as_db_error() {
        Error::new(format!("{} (SQLSTATE {})", db.message(), db.code().code()))
    } else {
        Error::new(
            "PostgreSQL connection or protocol failed. Check host, credentials and TLS configuration.",
        )
    }
}
fn query_error(e: tokio_postgres::Error, sql: &str, server_sql_ascii: bool) -> Error {
    let offset = e.as_db_error().and_then(|db| match db.position() {
        Some(tokio_postgres::error::ErrorPosition::Original(position)) => {
            original_error_offset(sql, *position, server_sql_ascii)
        }
        _ => None, // Internal queries have a different source; do not point into the editor.
    });
    let mut error = err(e);
    error.sql_offset = offset;
    error
}
fn original_error_offset(sql: &str, position: u32, server_sql_ascii: bool) -> Option<usize> {
    let original = position.checked_sub(1)? as usize;
    // SQL_ASCII treats each input byte as a character, even for UTF-8 client SQL.
    let byte = if server_sql_ascii {
        original
    } else {
        sql.char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(sql.len()))
            .nth(original)?
    };
    sql_utf16_offset(sql, byte)
}
impl Postgres {
    async fn request_cancel(&self) -> Result<()> {
        if !matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                self.client.cancel_token().cancel_query(self.tls.clone())
            )
            .await,
            Ok(Ok(()))
        ) {
            self.worker.abort();
            return Err(Error::new(
                "Cancellation request could not be sent; connection closed. Verify writes before retrying.",
            ));
        }
        Ok(())
    }
    async fn interrupt_stream<S>(&self, stream: &mut S, message: &str) -> Result<()>
    where
        S: futures_util::Stream<
                Item = std::result::Result<SimpleQueryMessage, tokio_postgres::Error>,
            > + Unpin,
    {
        // If the response is already complete, discard it without sending a late
        // cancel packet that could strike the next statement on this session.
        if tokio::time::timeout(std::time::Duration::from_millis(10), async {
            let mut messages = 0;
            while stream.next().await.is_some() {
                messages += 1;
                if messages % 64 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        })
        .await
        .is_ok()
        {
            return Err(Error::new(message));
        }
        self.request_cancel().await?;
        let drained = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut acknowledged = false;
            while let Some(item) = stream.next().await {
                if let Err(e) = item {
                    acknowledged |= e.code()
                        == Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
                        && e.as_db_error().is_some_and(|db| {
                            db.message() == "canceling statement due to user request"
                        });
                }
            }
            acknowledged
        })
        .await;
        if matches!(drained, Ok(true)) {
            return Err(Error::new(message));
        }
        self.worker.abort();
        Err(Error::new(format!(
            "{message}. Cancellation could not be synchronized; connection closed. Verify writes before retrying."
        )))
    }
    async fn deliver<S>(
        &self,
        output: &mpsc::Sender<Batch>,
        batch: Batch,
        stream: &mut S,
        cancel: &CancellationToken,
    ) -> Result<()>
    where
        S: futures_util::Stream<
                Item = std::result::Result<SimpleQueryMessage, tokio_postgres::Error>,
            > + Unpin,
    {
        let sent = tokio::select! {
            biased;
            _ = cancel.cancelled() => false,
            sent = output.send(batch) => sent.is_ok(),
        };
        if sent {
            Ok(())
        } else {
            self.interrupt_stream(
                stream,
                if cancel.is_cancelled() {
                    "Query cancelled"
                } else {
                    "Result consumer closed"
                },
            )
            .await
        }
    }
    pub async fn connect(
        url: &str,
        password: Option<&str>,
        read_only: bool,
        identity_password: Option<&str>,
    ) -> Result<Self> {
        Self::connect_via(url, password, read_only, identity_password, None).await
    }
    pub async fn connect_via(
        url: &str,
        password: Option<&str>,
        read_only: bool,
        identity_password: Option<&str>,
        endpoint: Option<std::net::SocketAddr>,
    ) -> Result<Self> {
        let mut address =
            url::Url::parse(url).map_err(|_| Error::new("Invalid PostgreSQL connection URL"))?;
        let timeout = connect_timeout(
            address
                .query_pairs()
                .filter(|(k, _)| k == "connect_timeout")
                .map(|(_, v)| v),
        )?;
        let roots: Vec<_> = address
            .query_pairs()
            .filter(|(k, _)| k == "sslrootcert")
            .map(|(_, v)| v.into_owned())
            .collect();
        if roots.len() > 1 {
            return Err(Error::new("Choose one CA certificate file"));
        }
        let identities: Vec<_> = address
            .query_pairs()
            .filter(|(k, _)| k == "sslidentity")
            .map(|(_, v)| v.into_owned())
            .collect();
        if identities.len() > 1 {
            return Err(Error::new("Choose one client identity file"));
        }
        let remaining: Vec<_> = address
            .query_pairs()
            .filter(|(k, _)| !["sslrootcert", "sslidentity"].contains(&k.as_ref()))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        address.set_query(None);
        if !remaining.is_empty() {
            address.query_pairs_mut().extend_pairs(remaining);
        }
        if let Some(endpoint) = endpoint {
            if !endpoint.ip().is_loopback() {
                return Err(Error::new("Invalid tunnel endpoint"));
            }
            // Config::port appends a port. Replace the URL port before parsing so one host has one port.
            address
                .set_port(Some(endpoint.port()))
                .map_err(|_| Error::new("Invalid tunnel endpoint"))?;
        }
        let mut config: tokio_postgres::Config = address
            .as_str()
            .parse()
            .map_err(|_| Error::new("Invalid PostgreSQL connection URL"))?;
        if let Some(password) = password {
            config.password(password);
        }
        config.connect_timeout(timeout);
        if let Some(endpoint) = endpoint {
            config.hostaddr(endpoint.ip());
        }
        config.keepalives(true);
        let mut builder = native_tls::TlsConnector::builder();
        if let Some(path) = roots.first() {
            if config.get_ssl_mode() != tokio_postgres::config::SslMode::Require {
                return Err(Error::new(
                    "A custom CA certificate requires sslmode=require",
                ));
            }
            for der in klyndb_driver_api::tls::load_ca_certificates(path).await? {
                builder.add_root_certificate(
                    native_tls::Certificate::from_der(&der)
                        .map_err(|_| Error::new("Could not decode the CA certificate"))?,
                );
            }
        }
        if let Some(path) = identities.first() {
            if config.get_ssl_mode() != tokio_postgres::config::SslMode::Require {
                return Err(Error::new("A client identity requires sslmode=require"));
            }
            let (_, identity) =
                klyndb_driver_api::tls::load_client_identity(path, identity_password).await?;
            builder.identity(identity);
        }
        let tls = MakeTlsConnector::new(
            builder
                .build()
                .map_err(|_| Error::new("Could not initialize TLS"))?,
        );
        tokio::time::timeout(timeout, async {
            let (client, connection) = config.connect(tls.clone()).await.map_err(err)?;
            let server_sql_ascii = connection.parameter("server_encoding") == Some("SQL_ASCII");
            let worker = tokio::spawn(async move {
                let _ = connection.await;
            });
            let session = Self {
                client,
                worker,
                serial: Mutex::new(()),
                tls,
                read_only,
                server_sql_ascii,
            };
            if read_only {
                session
                    .client
                    .batch_execute("SET default_transaction_read_only=on")
                    .await
                    .map_err(err)?;
            }
            Ok(session)
        })
        .await
        .map_err(|_| {
            Error::new(format!(
                "PostgreSQL connection timed out after {} seconds",
                timeout.as_secs()
            ))
        })?
    }
    async fn stream(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()> {
        let stream = self
            .client
            .simple_query_raw(&sql)
            .await
            .map_err(|e| query_error(e, &sql, self.server_sql_ascii))?;
        pin_mut!(stream);
        let mut count = 0;
        let mut has_columns = false;
        let mut buffer = Vec::with_capacity(256);
        let mut buffer_bytes = 0;
        loop {
            let item = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    return self.interrupt_stream(&mut stream, "Query cancelled").await;
                }
                _ = output.closed() => {
                    return self.interrupt_stream(&mut stream, "Result consumer closed").await;
                }
                item = stream.next() => item,
            };
            let Some(item) = item else {
                break;
            };
            match item.map_err(|e| query_error(e, &sql, self.server_sql_ascii))? {
                SimpleQueryMessage::RowDescription(columns) => {
                    count = 0;
                    has_columns = true;
                    self.deliver(
                        &output,
                        Batch::Columns(columns.iter().map(|c| c.name().to_string()).collect()),
                        &mut stream,
                        &cancel,
                    )
                    .await?;
                }
                SimpleQueryMessage::Row(row) => {
                    if count >= limit {
                        continue;
                    }
                    let row_bytes = (0..row.len())
                        .map(|i| row.get(i).map_or(0, str::len))
                        .sum::<usize>();
                    if row_bytes > 8 * 1024 * 1024 {
                        return self.interrupt_stream(&mut stream,
                            "A result row exceeds 8 MiB. Select smaller values or use database-native export.").await;
                    }
                    buffer_bytes += row_bytes;
                    buffer.push(
                        (0..row.len())
                            .map(|i| {
                                row.get(i)
                                    .map(|s| Cell::Text(s.into()))
                                    .unwrap_or(Cell::Null)
                            })
                            .collect(),
                    );
                    count += 1;
                    if buffer.len() == 256 || buffer_bytes >= 256 * 1024 {
                        buffer_bytes = 0;
                        self.deliver(
                            &output,
                            Batch::Rows(std::mem::take(&mut buffer)),
                            &mut stream,
                            &cancel,
                        )
                        .await?;
                    }
                }
                SimpleQueryMessage::CommandComplete(affected) => {
                    if !buffer.is_empty() {
                        self.deliver(
                            &output,
                            Batch::Rows(std::mem::take(&mut buffer)),
                            &mut stream,
                            &cancel,
                        )
                        .await?;
                    }
                    if !has_columns {
                        self.deliver(&output, Batch::Columns(vec![]), &mut stream, &cancel)
                            .await?;
                    }
                    self.deliver(
                        &output,
                        Batch::Complete {
                            affected: if has_columns { 0 } else { affected },
                            truncated: affected > limit as u64 && has_columns,
                        },
                        &mut stream,
                        &cancel,
                    )
                    .await?;
                    has_columns = false;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
#[async_trait]
impl Session for Postgres {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            key_value: false,
            affected_rows: true,
            table_browse: true,
            routines: true,
            diagrams: true,
            transactions: true,
            schemas: true,
            explain: true,
            explain_analyze: !self.read_only,
            edit_rows: true,
            import_rows: !self.read_only,
            import_sql: !self.read_only,
            cancel: true,
            tls: true,
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
                "EXPLAIN (FORMAT JSON{}) {sql}",
                if analyze { ", ANALYZE, BUFFERS" } else { "" }
            ),
            PlanFormat::PostgresJson,
        ))
    }
    fn quote_filter_value(&self, value: &str) -> String {
        format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
    }
    async fn execute(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()> {
        let _guard = tokio::select! {
            guard = self.serial.lock() => guard,
            _ = cancel.cancelled() => return Err(Error::new("Query cancelled while waiting for the session")),
        };
        if cancel.is_cancelled() {
            return Err(Error::new("Query cancelled"));
        }
        if self.read_only {
            self.client
                .batch_execute("BEGIN READ ONLY")
                .await
                .map_err(err)?;
        }
        let result = self.stream(sql, output, cancel, limit).await;
        if self.read_only && self.client.batch_execute("ROLLBACK").await.is_err() {
            self.worker.abort();
            let cause = result
                .err()
                .map_or(String::new(), |e| format!("{} ", e.message));
            return Err(Error::new(format!(
                "{cause}Read-only transaction cleanup failed; connection closed."
            )));
        }
        result
    }

    async fn tables(&self) -> Result<Vec<Table>> {
        let _guard = self.serial.lock().await;
        self.client.query("SELECT table_schema,table_name,lower(table_type) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema,table_name", &[]).await.map_err(err).map(|rows| rows.iter().map(|r| Table { schema:r.get(0), name:r.get(1), kind:r.get(2) }).collect())
    }
    async fn routines(&self, search: &str, offset: u32) -> Result<RoutinePage> {
        if search.len() > 1024 || offset > 1_000_000 {
            return Err(Error::new(
                "Routine search is limited to 1024 bytes and 1,000,000 rows of paging",
            ));
        }
        let _guard = self.serial.lock().await;
        let pattern = format!(
            "%{}%",
            search
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        let rows = self.client.query(
            "SELECT p.oid,n.nspname,p.proname,CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END,pg_catalog.pg_get_function_identity_arguments(p.oid),pg_catalog.pg_get_function_result(p.oid),l.lanname FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace JOIN pg_catalog.pg_language l ON l.oid=p.prolang WHERE p.prokind IN ('f','p','w') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND (n.nspname || '.' || p.proname) ILIKE $1 ESCAPE '!' ORDER BY n.nspname,p.proname,p.oid LIMIT 101 OFFSET $2",
            &[&pattern, &(offset as i64)],
        ).await.map_err(err)?;
        Ok(RoutinePage {
            has_more: rows.len() > 100,
            routines: rows
                .into_iter()
                .take(100)
                .map(|row| Routine {
                    id: row.get::<_, u32>(0).to_string(),
                    schema: row.get(1),
                    name: row.get(2),
                    kind: row.get(3),
                    arguments: row.get(4),
                    returns: row.get(5),
                    language: row.get(6),
                })
                .collect(),
        })
    }
    async fn routine_definition(&self, id: &str) -> Result<String> {
        let oid = id
            .parse::<u32>()
            .map_err(|_| Error::new("Invalid routine identifier"))?;
        let _guard = self.serial.lock().await;
        let row = self.client.query_opt(
            "SELECT pg_catalog.pg_get_functiondef(p.oid) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE p.oid=$1 AND p.prokind IN ('f','p','w') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND pg_catalog.has_schema_privilege(n.oid,'USAGE')",
            &[&oid],
        ).await.map_err(err)?.ok_or_else(|| Error::new("Routine is no longer visible. Refresh the catalog."))?;
        let definition: String = row.get(0);
        if definition.len() > 2 * 1024 * 1024 {
            return Err(Error::new(
                "Routine definition exceeds the 2 MiB viewer limit",
            ));
        }
        Ok(definition)
    }
    async fn execute_script(
        &self,
        mut input: mpsc::Receiver<Result<ScriptBatch>>,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        completed: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<()> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        let _guard = tokio::select! {
            guard = self.serial.lock() => guard,
            _ = cancel.cancelled() => return Err(Error::new("SQL import cancelled while waiting for the session")),
        };
        while let Some(sql) = next_script_statement(&mut input, &cancel).await? {
            // Recheck after every statement: functions can change lexical settings too.
            let mode = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                mode = tokio::time::timeout(std::time::Duration::from_secs(3), self.client.query_one("SHOW standard_conforming_strings", &[])) => mode.ok(),
            };
            let Some(mode) = mode else {
                self.worker.abort();
                return Err(Error::new(
                    "SQL import lexical check interrupted; connection closed. Reconnect and verify earlier writes.",
                ));
            };
            let mode: String = mode.map_err(err)?.get(0);
            if mode != "on" {
                return Err(Error::new(
                    "SQL file imports require standard_conforming_strings=on",
                ));
            }
            self.stream(sql, output.clone(), cancel.clone(), 0).await?;
            completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let _guard = self.serial.lock().await;
        let columns = self.client.query("SELECT c.column_name,c.data_type,c.is_nullable,c.column_default,(c.is_generated!='NEVER' OR c.identity_generation='ALWAYS'),EXISTS(SELECT 1 FROM information_schema.table_constraints t JOIN information_schema.key_column_usage k USING(constraint_catalog,constraint_schema,constraint_name) WHERE t.constraint_type='PRIMARY KEY' AND k.table_schema=c.table_schema AND k.table_name=c.table_name AND k.column_name=c.column_name) FROM information_schema.columns c WHERE table_schema=$1 AND table_name=$2 ORDER BY ordinal_position", &[&table.schema, &table.name]).await.map_err(err)?.iter().map(|r| Column { name:r.get(0), data_type:r.get(1), nullable:r.get::<_, String>(2)=="YES", default:r.get(3), primary_key:r.get(5), generated:r.get::<_, Option<bool>>(4).unwrap_or(false) }).collect();
        let indexes = self.client.query("SELECT indexname,indexdef FROM pg_indexes WHERE schemaname=$1 AND tablename=$2", &[&table.schema,&table.name]).await.map_err(err)?.iter().map(|r| serde_json::json!({"name":r.get::<_, String>(0), "definition":r.get::<_, String>(1)})).collect();
        let foreign_keys = self.client.query("SELECT c.conname,pg_get_constraintdef(c.oid) FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid JOIN pg_namespace n ON n.oid=t.relnamespace WHERE c.contype='f' AND n.nspname=$1 AND t.relname=$2", &[&table.schema,&table.name]).await.map_err(err)?.iter().map(|r| serde_json::json!({"name":r.get::<_, String>(0), "definition":r.get::<_, String>(1)})).collect();
        let constraints = self.client.query("SELECT c.conname,CASE c.contype WHEN 'p' THEN 'PRIMARY KEY' WHEN 'u' THEN 'UNIQUE' WHEN 'f' THEN 'FOREIGN KEY' WHEN 'c' THEN 'CHECK' WHEN 'x' THEN 'EXCLUSION' WHEN 't' THEN 'CONSTRAINT TRIGGER' ELSE c.contype::text END,pg_get_constraintdef(c.oid) FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid JOIN pg_namespace n ON n.oid=t.relnamespace WHERE n.nspname=$1 AND t.relname=$2 ORDER BY c.conname", &[&table.schema,&table.name]).await.map_err(err)?.iter().map(|r| Constraint { name:r.get(0), kind:r.get(1), definition:Some(r.get(2)) }).collect();
        let triggers = self.client.query("SELECT g.tgname,pg_get_triggerdef(g.oid),CASE g.tgenabled WHEN 'D' THEN 'Disabled' WHEN 'O' THEN 'Origin/local' WHEN 'R' THEN 'Replica' WHEN 'A' THEN 'Always' ELSE g.tgenabled::text END FROM pg_trigger g JOIN pg_class t ON t.oid=g.tgrelid JOIN pg_namespace n ON n.oid=t.relnamespace WHERE NOT g.tgisinternal AND n.nspname=$1 AND t.relname=$2 ORDER BY g.tgname", &[&table.schema,&table.name]).await.map_err(err)?.iter().map(|r| Trigger { name:r.get(0), definition:r.get(1), state:Some(r.get(2)) }).collect();
        let editable=self.client.query_opt("SELECT c.relkind IN ('r','p') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2",&[&table.schema,&table.name]).await.map_err(err)?.is_some_and(|r|r.get(0));
        Ok(TableInfo {
            editable,
            columns,
            ddl: None,
            indexes,
            foreign_keys,
            constraints: Some(constraints),
            triggers,
        })
    }
    async fn relationships(&self, table: &Table) -> Result<Vec<ForeignKey>> {
        let _guard = self.serial.lock().await;
        let rows=self.client.query("SELECT c.conname,a.attname,tn.nspname,tt.relname,b.attname FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_class tt ON tt.oid=c.confrelid JOIN pg_namespace tn ON tn.oid=tt.relnamespace CROSS JOIN LATERAL unnest(c.conkey,c.confkey) WITH ORDINALITY AS k(local_col,target_col,ordinal) JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.local_col JOIN pg_attribute b ON b.attrelid=c.confrelid AND b.attnum=k.target_col WHERE c.contype='f' AND n.nspname=$1 AND t.relname=$2 ORDER BY c.conname,k.ordinal", &[&table.schema,&table.name]).await.map_err(err)?;
        Ok(group_foreign_keys(rows.iter().map(|r| {
            (r.get(0), r.get(1), r.get(2), r.get(3), Some(r.get(4)))
        })))
    }
    async fn transaction_state(&self) -> Result<TransactionState> {
        let _guard = self.serial.lock().await;
        match self
            .client
            .batch_execute("SAVEPOINT klyndb_state; RELEASE SAVEPOINT klyndb_state")
            .await
        {
            Ok(()) => Ok(TransactionState::Active),
            Err(e)
                if e.code()
                    == Some(&tokio_postgres::error::SqlState::NO_ACTIVE_SQL_TRANSACTION) =>
            {
                Ok(TransactionState::Idle)
            }
            Err(e)
                if e.code()
                    == Some(&tokio_postgres::error::SqlState::IN_FAILED_SQL_TRANSACTION) =>
            {
                Ok(TransactionState::Failed)
            }
            Err(e) => Err(err(e)),
        }
    }
    async fn apply_changes(&self, table: Table, changes: Vec<Change>) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        validate_change_batch(&changes)?;
        let _guard = self.serial.lock().await;
        edit::apply(
            &self.client,
            &table,
            &changes,
            &CancellationToken::new(),
            None,
        )
        .await
    }
    async fn insert_stream(
        &self,
        table: Table,
        input: mpsc::Receiver<Result<InsertBatch>>,
        cancel: CancellationToken,
    ) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        let _guard = tokio::select! { biased; _=cancel.cancelled()=>return Err(Error::new("Import cancelled")), guard=self.serial.lock()=>guard };
        let committing = std::sync::atomic::AtomicBool::new(false);
        let poison = std::sync::atomic::AtomicBool::new(false);
        let interruptible = std::sync::atomic::AtomicBool::new(true);
        let mut operation = Box::pin(import::apply(
            &self.client,
            &table,
            input,
            &cancel,
            &committing,
            &poison,
            &interruptible,
        ));
        let result = tokio::select! {
            biased;
            result=&mut operation=>result,
            _=cancel.cancelled()=>{
                let requested = interruptible.load(std::sync::atomic::Ordering::Relaxed) && !committing.load(std::sync::atomic::Ordering::Relaxed);
                if requested { self.request_cancel().await?; }
                match tokio::time::timeout(std::time::Duration::from_secs(3), operation.as_mut()).await {
                    Ok(result)=>{
                        if requested && !poison.load(std::sync::atomic::Ordering::Relaxed)
                            && result.as_ref().err().is_none_or(|e| e.message != "canceling statement due to user request (SQLSTATE 57014)") {
                            self.worker.abort();
                            return Err(Error::new("Import cancellation could not be synchronized; connection closed. Verify data before retrying."));
                        }
                        result
                    },
                    Err(_)=>{ self.worker.abort(); return Err(Error::new("Import termination could not be confirmed; connection closed. Verify data before retrying.")); }
                }
            }
        };
        if poison.load(std::sync::atomic::Ordering::Relaxed) {
            self.worker.abort();
        }
        result
    }
    async fn disconnect(&self) -> Result<()> {
        self.worker.abort();
        Ok(())
    }
}
impl Drop for Postgres {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
#[cfg(test)]
mod error_position_tests {
    use super::*;
    #[test]
    fn original_positions_use_database_encoding_and_utf16_editor_units() {
        let sql = "SELECT 'é😀'; SELECT missing";
        let byte = sql.find("missing").unwrap();
        let character = sql[..byte].chars().count();
        let expected = sql_utf16_offset(sql, byte);
        assert_eq!(
            original_error_offset(sql, character as u32 + 1, false),
            expected
        );
        assert_eq!(original_error_offset(sql, byte as u32 + 1, true), expected);
        assert_eq!(original_error_offset(sql, 0, false), None);
        assert_eq!(original_error_offset(sql, sql.len() as u32 + 2, true), None);
    }
}
