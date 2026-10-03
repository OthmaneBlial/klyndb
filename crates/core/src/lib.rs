mod credentials;
pub mod diagram;
pub mod import;
use klyndb_connections::{Connection, Store};
use klyndb_driver_api::*;
use rusqlite::params;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Debug)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: usize,
    pub affected: Option<u64>,
    pub truncated: bool,
}
#[derive(Clone, Serialize, Debug)]
pub struct QueryStatus {
    pub id: String,
    pub sets: Vec<ResultSet>,
    pub done: bool,
    pub error: Option<String>,
    pub error_offset: Option<usize>,
    pub elapsed_ms: u64,
    pub connection_id: String,
    pub transaction: Option<TransactionState>,
    pub plan_format: Option<PlanFormat>,
    pub plan_analyze: bool,
}
pub struct Job {
    pub status: Mutex<QueryStatus>,
    pub db: Mutex<rusqlite::Connection>,
    pub cancel: CancellationToken,
    pub connection_id: String,
    _directory: tempfile::TempDir,
    engine: String,
}
impl Job {
    fn new(connection_id: String, engine: String) -> Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("klyndb-result-")
            .tempdir()
            .map_err(error)?;
        let db =
            rusqlite::Connection::open(directory.path().join("result.sqlite")).map_err(error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA cache_size=-2048; CREATE TABLE rows(result_set INTEGER,ordinal INTEGER,data TEXT NOT NULL,PRIMARY KEY(result_set,ordinal)) WITHOUT ROWID;").map_err(error)?;
        let id = uuid::Uuid::new_v4().to_string();
        Ok(Self {
            status: Mutex::new(QueryStatus {
                id,
                sets: vec![],
                done: false,
                error: None,
                error_offset: None,
                elapsed_ms: 0,
                connection_id: connection_id.clone(),
                transaction: None,
                plan_format: None,
                plan_analyze: false,
            }),
            db: Mutex::new(db),
            cancel: CancellationToken::new(),
            connection_id,
            _directory: directory,
            engine,
        })
    }
    pub fn status(&self) -> Result<QueryStatus> {
        Ok(self.status.lock().map_err(error)?.clone())
    }
    pub fn page(&self, set: usize, offset: usize, limit: usize) -> Result<Vec<Row>> {
        if limit > 1000 {
            return Err(Error::new("Result pages are limited to 1000 rows"));
        }
        let db = self.db.lock().map_err(error)?;
        let mut stmt = db
            .prepare(
                "SELECT data FROM rows WHERE result_set=? AND ordinal>=? ORDER BY ordinal LIMIT ?",
            )
            .map_err(error)?;
        let mut bytes = 0usize;
        stmt.query_map(params![set as i64, offset as i64, limit as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(error)?
        .map(|s| {
            let json = s.map_err(error)?;
            bytes += json.len();
            if bytes > 8 * 1024 * 1024 {
                return Err(Error::new(
                    "Result page exceeds 8 MiB. Select smaller values or request fewer rows.",
                ));
            }
            serde_json::from_str(&json).map_err(error)
        })
        .collect()
    }
    pub fn export(
        &self,
        out: impl std::io::Write,
        set: usize,
        format: &str,
        table: &str,
    ) -> Result<u64> {
        let status = self.status()?;
        if !status.done {
            return Err(Error::new("Wait for the query to finish before exporting"));
        }
        let result = status
            .sets
            .get(set)
            .ok_or_else(|| Error::new("Result set not found"))?;
        let db = self.db.lock().map_err(error)?;
        let mut stmt = db
            .prepare("SELECT data FROM rows WHERE result_set=? ORDER BY ordinal")
            .map_err(error)?;
        let rows = stmt
            .query_map([set as i64], |row| row.get::<_, String>(0))
            .map_err(error)?
            .map(|row| serde_json::from_str(&row.map_err(error)?).map_err(error));
        klyndb_export::export_for_engine(out, &result.columns, rows, format, table, &self.engine)
    }
    pub fn plan(&self) -> Result<klyndb_query::plan::Plan> {
        let status = self.status()?;
        self.decoded_plan().map_err(|error| {
            if status.plan_analyze && status.done && status.error.is_none() {
                Error::new(format!("Statement completed, but its plan could not be displayed. {} Verify writes before retrying.", error.message))
            } else { error }
        })
    }
    fn decoded_plan(&self) -> Result<klyndb_query::plan::Plan> {
        let status = self.status()?;
        let format = status
            .plan_format
            .ok_or_else(|| Error::new("This result is not an execution plan"))?;
        if !status.done || status.error.is_some() {
            return Err(Error::new(
                status
                    .error
                    .unwrap_or_else(|| "Wait for the plan to finish".into()),
            ));
        }
        let set_index = if format == PlanFormat::SqlServerTabular {
            status
                .sets
                .iter()
                .rposition(|s| {
                    [
                        "StmtText",
                        "StmtId",
                        "NodeId",
                        "Parent",
                        "PhysicalOp",
                        "EstimateRows",
                        "TotalSubtreeCost",
                    ]
                    .iter()
                    .all(|name| s.columns.iter().any(|c| c == name))
                })
                .ok_or_else(|| Error::new("SQL Server returned no native plan/profile result"))?
        } else {
            0
        };
        let first = status
            .sets
            .get(set_index)
            .ok_or_else(|| Error::new("The server returned no execution plan"))?;
        if first.truncated {
            return Err(Error::new(
                "The native plan was truncated. Narrow the query.",
            ));
        }
        let mut rows = vec![];
        let mut bytes = 0;
        for offset in (0..first.rows).step_by(500) {
            let page = self.page(set_index, offset, 500)?;
            bytes += page.iter().flatten().map(Cell::byte_len).sum::<usize>();
            if bytes > 4 * 1024 * 1024 {
                return Err(Error::new("Plan exceeds 4 MiB. Export the raw results."));
            }
            rows.extend(page);
        }
        let mut warnings = vec![];
        if let Some(set) = status
            .sets
            .get(1)
            .filter(|_| format != PlanFormat::SqlServerTabular)
        {
            for offset in (0..set.rows).step_by(500) {
                for row in self.page(1, offset, 500)? {
                    bytes += row.iter().map(Cell::byte_len).sum::<usize>();
                    if bytes > 4 * 1024 * 1024 {
                        return Err(Error::new(
                            "Plan and warnings exceed 4 MiB. Export the raw results.",
                        ));
                    }
                    warnings.push(row.iter().map(Cell::text).collect::<Vec<_>>().join(" · "));
                }
            }
            if set.truncated {
                warnings.push("Server warning output reached the row limit.".into());
            }
        }
        klyndb_query::plan::decode(format, &first.columns, &rows, warnings)
    }
    fn consume(&self, mut input: mpsc::Receiver<Batch>, affected_rows: bool) -> Result<()> {
        let mut total_bytes = 0usize;
        while let Some(batch) = input.blocking_recv() {
            if self.cancel.is_cancelled() {
                return Err(Error::new("Query cancelled"));
            }
            match batch {
                Batch::Columns(columns) => {
                    self.status.lock().map_err(error)?.sets.push(ResultSet {
                        columns,
                        rows: 0,
                        affected: None,
                        truncated: false,
                    });
                }
                Batch::Rows(rows) => {
                    let mut db = self.db.lock().map_err(error)?;
                    let mut status = self.status.lock().map_err(error)?;
                    let set_id = status
                        .sets
                        .len()
                        .checked_sub(1)
                        .ok_or_else(|| Error::new("Driver omitted result columns"))?;
                    let set = &mut status.sets[set_id];
                    let tx = db.transaction().map_err(error)?;
                    let mut committed_rows = set.rows;
                    for row in rows {
                        let json = serde_json::to_string(&row).map_err(error)?;
                        total_bytes += json.len();
                        if total_bytes > 512 * 1024 * 1024 {
                            self.cancel.cancel();
                            return Err(Error::new(
                                "Result exceeded the 512 MiB temporary storage limit. Narrow the query or export smaller ranges.",
                            ));
                        }
                        tx.execute(
                            "INSERT INTO rows VALUES(?,?,?)",
                            params![set_id as i64, committed_rows as i64, json],
                        )
                        .map_err(error)?;
                        committed_rows += 1;
                    }
                    tx.commit().map_err(error)?;
                    set.rows = committed_rows;
                }
                Batch::Complete {
                    affected,
                    truncated,
                } => {
                    if let Some(set) = self.status.lock().map_err(error)?.sets.last_mut() {
                        set.affected = affected_rows.then_some(affected);
                        set.truncated = truncated;
                    }
                }
            }
        }
        Ok(())
    }
}
fn error(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}

struct OpenConnection {
    driver: Arc<dyn Session>,
    config: Connection,
    tunnel: Option<klyndb_ssh::Tunnel>,
}
pub struct Engine {
    pub store: Arc<Store>,
    sessions: tokio::sync::Mutex<HashMap<String, OpenConnection>>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    pub imports: Arc<import::Imports>,
}
async fn open_session(
    config: &Connection,
    password: Option<&str>,
    identity_password: Option<&str>,
    ssh_password: Option<&str>,
) -> Result<(Arc<dyn Session>, Option<klyndb_ssh::Tunnel>)> {
    let timeout = config.connect_timeout()?;
    tokio::time::timeout(timeout, async {
        let mut address = config.address.clone();
        let tunnel = if let Some(ssh) = config.ssh()? {
            let mut url =
                url::Url::parse(&address).map_err(|_| Error::new("Invalid database URL"))?;
            let target = url
                .host_str()
                .ok_or_else(|| Error::new("Enter a database hostname"))?
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned();
            let port = url.port().unwrap_or(match config.engine.as_str() {
                "postgres" => 5432,
                "mssql" => 1433,
                "clickhouse"
                    if url
                        .query_pairs()
                        .any(|(k, v)| k == "tls" && v == "disabled") =>
                {
                    9000
                }
                "clickhouse" => 9440,
                _ => 3306,
            });
            let options: Vec<_> = url
                .query_pairs()
                .filter(|(k, _)| !klyndb_connections::ssh::OPTIONS.contains(&k.as_ref()))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut().clear().extend_pairs(options);
            address = url.to_string();
            Some(klyndb_ssh::Tunnel::open(ssh, target, port, ssh_password, timeout).await?)
        } else {
            None
        };
        let endpoint = tunnel.as_ref().map(|t| t.endpoint);
        let driver = match config.engine.as_str() {
            "sqlite" => Ok(Arc::new(
                klyndb_sqlite::Sqlite::connect(
                    config.address.clone(),
                    config.read_only,
                    config.create_file,
                )
                .await?,
            ) as Arc<dyn Session>),
            "duckdb" => Ok(Arc::new(
                klyndb_duckdb::DuckDb::connect(
                    config.address.clone(),
                    config.read_only,
                    config.create_file,
                )
                .await?,
            ) as Arc<dyn Session>),
            "postgres" => Ok(Arc::new(
                klyndb_postgres::Postgres::connect_via(
                    &address,
                    password,
                    config.read_only,
                    identity_password,
                    endpoint,
                )
                .await?,
            ) as Arc<dyn Session>),
            "mysql" => Ok(Arc::new(
                klyndb_mysql::Mysql::connect_via(
                    &address,
                    password,
                    config.read_only,
                    identity_password,
                    endpoint,
                )
                .await?,
            ) as Arc<dyn Session>),
            "clickhouse" => Ok(Arc::new(
                klyndb_clickhouse::ClickHouse::connect_via(
                    &address,
                    password,
                    config.read_only,
                    identity_password,
                    endpoint,
                )
                .await?,
            ) as Arc<dyn Session>),
            "mssql" => Ok(Arc::new(
                klyndb_mssql::SqlServer::connect_via(
                    &address,
                    password,
                    config.read_only,
                    endpoint,
                )
                .await?,
            ) as Arc<dyn Session>),
            _ => Err(Error::new("Database driver is not installed")),
        }?;
        Ok((driver, tunnel))
    })
    .await
    .map_err(|_| {
        Error::new(format!(
            "Connection timed out after {} seconds",
            timeout.as_secs()
        ))
    })?
}
impl Engine {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(store),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            imports: Arc::new(import::Imports::default()),
        }
    }
    pub async fn connect(
        &self,
        id: &str,
        password: Option<String>,
        identity_password: Option<String>,
        ssh_password: Option<String>,
    ) -> Result<Capabilities> {
        let config = self.store.connection(id)?;
        let timeout = config.connect_timeout()?;
        let deadline = tokio::time::Instant::now() + timeout;
        let password = if let Some(p) = password {
            Some(Zeroizing::new(p))
        } else if !config.is_local_file() {
            credentials::password(id, deadline).await?
        } else {
            None
        };
        let identity_password = if config.has_client_identity() {
            match identity_password {
                Some(p) => Some(Zeroizing::new(p)),
                None => {
                    credentials::password(&klyndb_connections::client_identity_key(id), deadline)
                        .await?
                }
            }
        } else {
            None
        };
        let ssh_password = self
            .ssh_password(&config, ssh_password, true, deadline)
            .await?;
        tokio::time::timeout_at(deadline, async {
            // ponytail: serialize session creation; use per-connection gates if overlapping opens become a bottleneck.
            let mut sessions = self.sessions.lock().await;
            if let Some(existing) = sessions.get(id) {
                return Ok(existing.driver.capabilities());
            }
            let (driver, tunnel) = open_session(
                &config,
                password.as_deref().map(|s| s.as_str()),
                identity_password.as_deref().map(|s| s.as_str()),
                ssh_password.as_deref().map(|s| s.as_str()),
            )
            .await?;
            let capabilities = driver.capabilities();
            sessions.insert(
                id.into(),
                OpenConnection {
                    driver,
                    config,
                    tunnel,
                },
            );
            tracing::info!(engine = %self.store.connection(id)?.engine, "connection opened");
            Ok(capabilities)
        })
        .await
        .map_err(|_| {
            Error::new(format!(
                "Connection timed out after {} seconds",
                timeout.as_secs()
            ))
        })?
    }

    pub async fn test_connection(
        &self,
        mut config: Connection,
        password: Option<String>,
        identity_password: Option<String>,
        ssh_password: Option<String>,
    ) -> Result<Capabilities> {
        let saved = !config.id.is_empty();
        let password = password.map(Zeroizing::new);
        if config.name.trim().is_empty() {
            config.name = "Connection test".into();
        }
        let embedded = config.validate()?;
        let password = password.or(embedded);
        let timeout = config.connect_timeout()?;
        let deadline = tokio::time::Instant::now() + timeout;
        let password = if password.is_none() && saved && !config.is_local_file() {
            credentials::password(&config.id, deadline).await?
        } else {
            password
        };
        let identity_password = if config.has_client_identity() {
            match identity_password {
                Some(p) => Some(Zeroizing::new(p)),
                None if saved => {
                    credentials::password(
                        &klyndb_connections::client_identity_key(&config.id),
                        deadline,
                    )
                    .await?
                }
                None => None,
            }
        } else {
            None
        };
        let ssh_password = self
            .ssh_password(&config, ssh_password, saved, deadline)
            .await?;
        // Testing opens an isolated read-only session and never creates a user database file.
        config.read_only = true;
        config.create_file = false;
        tokio::time::timeout_at(deadline, async {
            let (driver, mut tunnel) = open_session(
                &config,
                password.as_deref().map(|s| s.as_str()),
                identity_password.as_deref().map(|s| s.as_str()),
                ssh_password.as_deref().map(|s| s.as_str()),
            )
            .await?;
            let probe = driver.transaction_state().await;
            let closed = driver.disconnect().await;
            if let Some(tunnel) = &mut tunnel {
                tunnel.close().await;
            }
            probe?;
            closed?;
            Ok(driver.capabilities())
        })
        .await
        .map_err(|_| {
            Error::new(format!(
                "Connection test timed out after {} seconds",
                timeout.as_secs()
            ))
        })?
    }
    pub async fn disconnect(&self, id: &str) -> Result<()> {
        let connection = self.sessions.lock().await.remove(id);
        for job in self.jobs.lock().map_err(error)?.values() {
            if job.connection_id == id {
                job.cancel.cancel();
            }
        }
        self.imports.cancel_connection(id).await?;
        if let Some(mut connection) = connection {
            let closed = connection.driver.disconnect().await;
            if let Some(tunnel) = &mut connection.tunnel {
                tunnel.close().await;
            }
            closed?;
        }
        Ok(())
    }
    pub async fn reconnect(
        &self,
        id: &str,
        password: Option<String>,
        identity_password: Option<String>,
        ssh_password: Option<String>,
        confirmed: bool,
    ) -> Result<Capabilities> {
        if !confirmed {
            return Err(Error::new(
                "Confirmation required: reconnect closes the session and rolls back uncommitted changes",
            ));
        }
        // A fresh session intentionally resets transactions, temporary objects and session settings.
        // Reuse disconnect's query/import cancellation and native cleanup; never replay SQL.
        self.disconnect(id).await?;
        self.connect(id, password, identity_password, ssh_password)
            .await
    }
    async fn ssh_password(
        &self,
        config: &Connection,
        supplied: Option<String>,
        saved: bool,
        deadline: tokio::time::Instant,
    ) -> Result<Option<Zeroizing<String>>> {
        match config.ssh()? {
            Some(ssh) if ssh.auth != "agent" => {
                let password = match supplied {
                    Some(password) => Some(Zeroizing::new(password)),
                    None if saved => {
                        let previous = self
                            .store
                            .connections()?
                            .into_iter()
                            .find(|c| c.id == config.id);
                        if previous.map(|c| c.ssh()).transpose()?.flatten().as_ref() == Some(&ssh) {
                            credentials::password(
                                &klyndb_connections::ssh::credential_key(&config.id),
                                deadline,
                            )
                            .await?
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                if let Some(password) = &password {
                    klyndb_connections::ssh::validate_password(password)?;
                }
                Ok(password)
            }
            _ => Ok(None),
        }
    }
    pub async fn driver(&self, id: &str) -> Result<Arc<dyn Session>> {
        self.sessions
            .lock()
            .await
            .get(id)
            .map(|c| c.driver.clone())
            .ok_or_else(|| Error::new("Connect to this database first"))
    }
    pub async fn table_query_sql(
        &self,
        id: &str,
        table: &Table,
        query: &TableQuery,
    ) -> Result<String> {
        let driver = self.driver(id).await?;
        let info = tokio::time::timeout(std::time::Duration::from_secs(10), driver.inspect(table))
            .await
            .map_err(|_| Error::new("Table inspection timed out"))??;
        driver.table_query_sql(table, &info.columns, query)
    }
    pub async fn start(
        &self,
        connection: String,
        sql: String,
        limit: usize,
        timeout_seconds: u64,
        confirmed: bool,
    ) -> Result<String> {
        self.start_query(connection, sql, (limit, timeout_seconds, confirmed), None)
            .await
    }
    async fn start_query(
        &self,
        connection: String,
        mut sql: String,
        options: (usize, u64, bool),
        plan: Option<bool>,
    ) -> Result<String> {
        let (limit, timeout_seconds, confirmed) = options;
        if limit == 0 || limit > 10_000_000 {
            return Err(Error::new("Row limit must be 1–10,000,000"));
        }
        if !(1..=3600).contains(&timeout_seconds) {
            return Err(Error::new("Timeout must be 1–3600 seconds"));
        }
        let sessions = self.sessions.lock().await;
        let open = sessions
            .get(&connection)
            .ok_or_else(|| Error::new("Connect to the database first"))?;
        let plan = if let Some(analyze) = plan {
            let target = klyndb_query::explain_target(&sql, &open.config.engine)?;
            let (prepared, format) = open.driver.explain_sql(target, analyze)?;
            sql = prepared;
            if analyze && !confirmed {
                return Err(Error::new(
                    "Confirmation required: ANALYZE executes the statement, including writes and side effects",
                ));
            }
            Some((format, analyze))
        } else {
            None
        };
        let estimated = plan.is_some_and(|(_, analyze)| !analyze);
        let analysis = klyndb_query::analyze(&sql, &open.config.engine)?;
        if analysis.statements.len() > 100 {
            return Err(Error::new("Run at most 100 statements per batch"));
        }
        if open.config.read_only && !analysis.read_only && !estimated {
            return Err(Error::new(
                "This connection is read-only. Only SELECT and non-executing EXPLAIN are allowed.",
            ));
        }
        if !estimated && !confirmed && !analysis.warnings.is_empty() {
            return Err(Error::new(format!(
                "Confirmation required: {}",
                analysis.warnings.join("; ")
            )));
        }
        let driver = open.driver.clone();
        let dialect = open.config.engine.clone();
        let job = Arc::new(Job::new(connection.clone(), dialect)?);
        if let Some((format, analyze)) = plan {
            let mut status = job.status.lock().map_err(error)?;
            status.plan_format = Some(format);
            status.plan_analyze = analyze;
        }
        let id = job.status()?.id;
        {
            let mut jobs = self.jobs.lock().map_err(error)?;
            if jobs.len() >= 32 {
                return Err(Error::new(
                    "Close a result tab before running more queries (32 cached results maximum).",
                ));
            }
            jobs.insert(id.clone(), job.clone());
        }
        // Register while holding the session gate so disconnect/reconnect cannot miss this job.
        drop(sessions);
        let store = self.store.clone();
        tokio::spawn(async move {
            let began = Instant::now();
            let token = job.cancel.clone();
            let timer_token = token.clone();
            let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let timer_flag = timed_out.clone();
            let timer = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_seconds)).await;
                timer_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                timer_token.cancel();
            });
            let (output, input) = mpsc::channel(4);
            let spool = job.clone();
            let affected_rows = driver.capabilities().affected_rows;
            let consumer = tokio::task::spawn_blocking(move || spool.consume(input, affected_rows));
            let query = match plan {
                Some((_, analyze)) => {
                    driver
                        .execute_plan(sql.clone(), output, token, limit, analyze)
                        .await
                }
                None => driver.execute(sql.clone(), output, token, limit).await,
            };
            let consumed = consumer.await.map_err(error).and_then(|r| r);
            timer.abort();
            // Preserve an unconfirmed termination warning over a spool cancellation
            // or quota error: the user must know the session was closed.
            let outcome = match query {
                Err(e) if e.message.contains("connection closed") => Err(e),
                query => consumed.and(query),
            };
            let elapsed = began.elapsed().as_millis() as u64;
            let transaction = driver.transaction_state().await.ok();
            let error_offset = if plan.is_none()
                && !timed_out.load(std::sync::atomic::Ordering::Relaxed)
                && !job.cancel.is_cancelled()
            {
                outcome.as_ref().err().and_then(|error| error.sql_offset)
            } else {
                None
            };
            let message = outcome.err().map(|e| {
                if timed_out.load(std::sync::atomic::Ordering::Relaxed)
                    && !e.message.contains("connection closed")
                {
                    format!("Query timed out after {timeout_seconds} seconds")
                } else if job.cancel.is_cancelled()
                    && ["Query cancelled", "interrupted"].contains(&e.message.as_str())
                {
                    "Query cancelled".into()
                } else {
                    e.message
                }
            });
            if let Ok(mut status) = job.status.lock() {
                status.done = true;
                status.error = message.clone();
                status.error_offset = error_offset;
                status.elapsed_ms = elapsed;
                status.transaction = transaction;
            }
            let history_sql = if let Some((PlanFormat::SqlServerTabular, analyze)) = plan {
                let option = if analyze {
                    "STATISTICS PROFILE"
                } else {
                    "SHOWPLAN_ALL"
                };
                // Native batches, separated as in sqlcmd: history must not present an estimate as an executed write.
                format!("SET {option} ON;\nGO\n{sql}\nGO\nSET {option} OFF;")
            } else {
                sql
            };
            if let Err(e) =
                store.add_history(&connection, &history_sql, message.as_deref(), elapsed)
            {
                tracing::warn!(error=%e,"could not save query history");
            }
            tracing::info!(
                elapsed_ms = elapsed,
                failed = message.is_some(),
                "query finished"
            );
        });
        Ok(id)
    }
    pub async fn start_plan(
        &self,
        connection: String,
        sql: String,
        analyze: bool,
        timeout_seconds: u64,
        confirmed: bool,
    ) -> Result<String> {
        self.start_query(
            connection,
            sql,
            (5000, timeout_seconds, confirmed),
            Some(analyze),
        )
        .await
    }
    pub async fn apply_changes(
        &self,
        id: &str,
        table: Table,
        changes: Vec<Change>,
        confirmed: bool,
    ) -> Result<MutationResult> {
        validate_change_batch(&changes)?;
        let sessions = self.sessions.lock().await;
        let open = sessions
            .get(id)
            .ok_or_else(|| Error::new("Connect to the database first"))?;
        if open.config.read_only {
            return Err(Error::new("This connection is read-only"));
        }
        if !open.driver.capabilities().edit_rows {
            return Err(Error::new("This driver does not support table editing"));
        }
        if !confirmed
            && (open.config.environment == "production"
                || changes.iter().any(|c| matches!(c, Change::Delete { .. })))
        {
            return Err(Error::new(
                "Confirmation required for production writes or row deletion",
            ));
        }
        let driver = open.driver.clone();
        drop(sessions);
        driver.apply_changes(table, changes).await
    }
    pub fn job(&self, id: &str) -> Result<Arc<Job>> {
        self.jobs
            .lock()
            .map_err(error)?
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new("Result has expired. Run the query again."))
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        self.job(id)?.cancel.cancel();
        Ok(())
    }
    pub fn release(&self, id: &str) -> Result<()> {
        if let Some(job) = self.jobs.lock().map_err(error)?.remove(id) {
            job.cancel.cancel();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "writes and removes a synthetic, uniquely named OS keychain entry"]
    async fn ssh_keychain_credentials_are_scoped_to_the_saved_bastion() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
        let mut config = Connection {
            id: String::new(),
            name: "Synthetic SSH scope contract".into(),
            engine: "postgres".into(),
            address: format!(
                "postgresql://test@db.internal/test?ssh_host=bastion.example&ssh_user=test&ssh_auth=password&ssh_fingerprint=SHA256:{}",
                "A".repeat(43)
            ),
            environment: "development".into(),
            group: String::new(),
            color: "#79c7a4".into(),
            favorite: false,
            read_only: true,
            create_file: false,
        };
        config.validate().unwrap();
        engine.store.save(&config).unwrap();
        let key = klyndb_connections::ssh::credential_key(&config.id);
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = klyndb_connections::delete_password(&self.0);
            }
        }
        let _cleanup = Cleanup(key.clone());
        klyndb_connections::save_password(&key, "synthetic-scope-secret").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        assert_eq!(
            engine
                .ssh_password(&config, None, true, deadline)
                .await
                .unwrap()
                .as_deref()
                .map(|s| s.as_str()),
            Some("synthetic-scope-secret")
        );
        for (option, value) in [
            ("ssh_host", "other.example"),
            ("ssh_port", "23"),
            ("ssh_user", "other"),
            ("ssh_fingerprint", &format!("SHA256:{}", "B".repeat(43))),
            ("ssh_auth", "agent"),
        ] {
            let mut changed = config.clone();
            let mut url = url::Url::parse(&changed.address).unwrap();
            let options: Vec<_> = url
                .query_pairs()
                .filter(|(k, _)| k != option)
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(options)
                .append_pair(option, value);
            changed.address = url.to_string();
            assert!(
                engine
                    .ssh_password(&changed, None, true, deadline)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            engine
                .ssh_password(&config, None, false, deadline)
                .await
                .unwrap()
                .is_none()
        );
        klyndb_connections::delete_password(&key).unwrap();
        assert!(klyndb_connections::password(&key).unwrap().is_none());
    }
    #[tokio::test]
    async fn isolated_connection_test_and_handshake_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(Store::open(&dir.path().join("state.db")).unwrap());
        let mut config = Connection {
            id: String::new(),
            name: "existing".into(),
            engine: "sqlite".into(),
            address: dir.path().join("user.db").to_string_lossy().into(),
            environment: "development".into(),
            group: String::new(),
            color: "#78c6a3".into(),
            favorite: false,
            read_only: false,
            create_file: true,
        };
        config.validate().unwrap();
        engine.store.save(&config).unwrap();
        engine.connect(&config.id, None, None, None).await.unwrap();
        let original = engine.driver(&config.id).await.unwrap();
        let (tx, _rx) = mpsc::channel(4);
        original
            .execute("BEGIN".into(), tx, CancellationToken::new(), 100)
            .await
            .unwrap();
        let mut draft = config.clone();
        draft.id.clear();
        draft.name.clear();
        assert!(
            engine
                .test_connection(draft.clone(), None, None, None)
                .await
                .unwrap()
                .transactions
        );
        assert_eq!(engine.store.connections().unwrap().len(), 1);
        assert!(Arc::ptr_eq(
            &original,
            &engine.driver(&config.id).await.unwrap()
        ));
        assert_eq!(
            original.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        let missing = dir.path().join("not-created.db");
        draft.address = missing.to_string_lossy().into();
        assert!(
            engine
                .test_connection(draft.clone(), None, None, None)
                .await
                .is_err()
        );
        assert!(!missing.exists());
        std::fs::write(&missing, b"not a SQLite database").unwrap();
        assert!(
            engine
                .test_connection(draft.clone(), None, None, None)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&missing).unwrap(), b"not a SQLite database");
        for (name, engine_name) in [
            ("KLYNDB_TEST_POSTGRES_URL", "postgres"),
            ("KLYNDB_TEST_MYSQL_URL", "mysql"),
        ] {
            if let Ok(url) = std::env::var(name) {
                draft.address = url;
                draft.engine = engine_name.into();
                // No keychain entry or saved connection is created for successful server tests.
                assert!(
                    engine
                        .test_connection(draft.clone(), None, None, None)
                        .await
                        .unwrap()
                        .transactions
                );
                assert_eq!(engine.store.connections().unwrap().len(), 1);
            }
        }
        for (kind, tunneled) in [("postgres", false), ("mysql", false), ("postgres", true)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    tokio::spawn(async move {
                        // Consume client bytes without ever sending a greeting/auth response.
                        let _ = tokio::io::copy(&mut stream, &mut tokio::io::sink()).await;
                    });
                }
            });
            draft.id.clear();
            draft.engine = kind.into();
            draft.address = format!(
                "{kind}://test@{address}/test?{}&connect_timeout=1",
                if kind == "postgres" {
                    "sslmode=disable"
                } else {
                    "tls=disabled"
                }
            );
            if tunneled {
                draft.address = format!(
                    "postgresql://test@db.internal/test?sslmode=disable&connect_timeout=1&ssh_host=127.0.0.1&ssh_port={}&ssh_user=test&ssh_fingerprint=SHA256:{}",
                    address.port(),
                    "A".repeat(43)
                );
            }
            let began = std::time::Instant::now();
            let failure = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                engine.test_connection(draft.clone(), None, None, None),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(failure.message.contains("1 seconds"), "{}", failure.message);
            assert!(began.elapsed() >= std::time::Duration::from_millis(900));
            assert_eq!(engine.store.connections().unwrap().len(), 1);
            draft.name = "Stalled handshake".into();
            draft.validate().unwrap();
            engine.store.save(&draft).unwrap();
            let failure = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                engine.connect(&draft.id, Some(String::new()), None, None),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(failure.message.contains("1 seconds"), "{}", failure.message);
            assert!(engine.driver(&draft.id).await.is_err());
            engine.store.delete(&draft.id).unwrap();
            peer.abort();
        }
        assert_eq!(
            original.transaction_state().await.unwrap(),
            TransactionState::Active
        );
        engine.disconnect(&config.id).await.unwrap();
    }
    #[tokio::test]
    async fn end_to_end_disk_pages_and_safety() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state.db")).unwrap();
        let mut connection = Connection {
            id: String::new(),
            name: "local".into(),
            engine: "sqlite".into(),
            address: dir.path().join("user.db").to_string_lossy().into(),
            environment: "production".into(),
            group: String::new(),
            color: "#78c6a3".into(),
            favorite: false,
            read_only: false,
            create_file: true,
        };
        connection.validate().unwrap();
        store.save(&connection).unwrap();
        let engine = Engine::new(store);
        engine
            .connect(&connection.id, None, None, None)
            .await
            .unwrap();
        assert!(
            engine
                .start(connection.id.clone(), "DROP TABLE t".into(), 100, 5, false)
                .await
                .is_err()
        );
        assert!(
            engine
                .apply_changes(
                    &connection.id,
                    Table {
                        schema: "main".into(),
                        name: "t".into(),
                        kind: "table".into()
                    },
                    vec![Change::Insert {
                        values: std::collections::BTreeMap::new()
                    }],
                    false
                )
                .await
                .unwrap_err()
                .message
                .contains("Confirmation required")
        );
        let id=engine.start(connection.id.clone(),"CREATE TABLE t(id INTEGER PRIMARY KEY); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT x FROM n".into(),10_000,5,false).await.unwrap();
        let job = engine.job(&id).unwrap();
        while !job.status().unwrap().done {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let status = job.status().unwrap();
        assert!(status.error.is_none(), "{:?}", status.error);
        assert_eq!(status.sets[1].rows, 10_000);
        assert_eq!(
            job.page(1, 9999, 1).unwrap(),
            vec![vec![Cell::Number("10000".into())]]
        );
        assert!(job.page(1, 0, 1001).is_err());
        let wide = engine.start(connection.id.clone(), "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<40) SELECT zeroblob(128000) FROM n".into(), 40, 5, false).await.unwrap();
        let wide_job = engine.job(&wide).unwrap();
        while !wide_job.status().unwrap().done {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(wide_job.status().unwrap().error.is_none());
        assert_eq!(wide_job.page(0, 0, 10).unwrap().len(), 10);
        assert!(
            wide_job
                .page(0, 0, 40)
                .unwrap_err()
                .message
                .contains("8 MiB")
        );
        engine.release(&wide).unwrap();
        let cancelled=engine.start(connection.id.clone(), "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(),100,5,false).await.unwrap();
        engine.cancel(&cancelled).unwrap();
        let cancelled_job = engine.job(&cancelled).unwrap();
        while !cancelled_job.status().unwrap().done {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(
            cancelled_job.status().unwrap().error.as_deref(),
            Some("Query cancelled")
        );
        engine.release(&cancelled).unwrap();
        engine.disconnect(&connection.id).await.unwrap();
        engine.release(&id).unwrap();
    }
}
