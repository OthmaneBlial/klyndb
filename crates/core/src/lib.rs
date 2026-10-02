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
    pub affected: u64,
    pub truncated: bool,
}
#[derive(Clone, Serialize, Debug)]
pub struct QueryStatus {
    pub id: String,
    pub sets: Vec<ResultSet>,
    pub done: bool,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}
pub struct Job {
    pub status: Mutex<QueryStatus>,
    pub db: Mutex<rusqlite::Connection>,
    pub cancel: CancellationToken,
    pub connection_id: String,
    _directory: tempfile::TempDir,
}
impl Job {
    fn new(connection_id: String) -> Result<Self> {
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
                elapsed_ms: 0,
            }),
            db: Mutex::new(db),
            cancel: CancellationToken::new(),
            connection_id,
            _directory: directory,
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
    fn consume(&self, mut input: mpsc::Receiver<Batch>) -> Result<()> {
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
                        affected: 0,
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
                        set.affected = affected;
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
}
pub struct Engine {
    pub store: Arc<Store>,
    sessions: tokio::sync::Mutex<HashMap<String, OpenConnection>>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}
impl Engine {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(store),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
        }
    }
    pub async fn connect(&self, id: &str, password: Option<String>) -> Result<Capabilities> {
        let config = self.store.connection(id)?;
        let password = if let Some(p) = password {
            Some(Zeroizing::new(p))
        } else if config.engine != "sqlite" {
            klyndb_connections::password(id)?
        } else {
            None
        };
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(id) {
            return Ok(existing.driver.capabilities());
        }
        let driver: Arc<dyn Session> = match config.engine.as_str() {
            "sqlite" => Arc::new(
                klyndb_sqlite::Sqlite::connect(
                    config.address.clone(),
                    config.read_only,
                    config.create_file,
                )
                .await?,
            ),
            "postgres" => Arc::new(
                klyndb_postgres::Postgres::connect(
                    &config.address,
                    password.as_deref().map(|s| s.as_str()),
                    config.read_only,
                )
                .await?,
            ),
            _ => return Err(Error::new("Database driver is not installed")),
        };
        let capabilities = driver.capabilities();
        sessions.insert(id.into(), OpenConnection { driver, config });
        tracing::info!(engine = %self.store.connection(id)?.engine, "connection opened");
        Ok(capabilities)
    }
    pub async fn disconnect(&self, id: &str) -> Result<()> {
        for job in self.jobs.lock().map_err(error)?.values() {
            if job.connection_id == id {
                job.cancel.cancel();
            }
        }
        if let Some(connection) = self.sessions.lock().await.remove(id) {
            connection.driver.disconnect().await?;
        }
        Ok(())
    }
    pub async fn driver(&self, id: &str) -> Result<Arc<dyn Session>> {
        self.sessions
            .lock()
            .await
            .get(id)
            .map(|c| c.driver.clone())
            .ok_or_else(|| Error::new("Connect to this database first"))
    }
    pub async fn start(
        &self,
        connection: String,
        sql: String,
        limit: usize,
        timeout_seconds: u64,
        confirmed: bool,
    ) -> Result<String> {
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
        let analysis = klyndb_query::analyze(&sql, &open.config.engine)?;
        if analysis.statements.len() > 100 {
            return Err(Error::new("Run at most 100 statements per batch"));
        }
        if open.config.read_only && !analysis.read_only {
            return Err(Error::new(
                "This connection is read-only. Only SELECT and non-executing EXPLAIN are allowed.",
            ));
        }
        if !confirmed && !analysis.warnings.is_empty() {
            return Err(Error::new(format!(
                "Confirmation required: {}",
                analysis.warnings.join("; ")
            )));
        }
        let driver = open.driver.clone();
        drop(sessions);
        let job = Arc::new(Job::new(connection.clone())?);
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
            let consumer = tokio::task::spawn_blocking(move || spool.consume(input));
            let query = driver.execute(sql.clone(), output, token, limit).await;
            let consumed = consumer.await.map_err(error).and_then(|r| r);
            timer.abort();
            let outcome = consumed.and(query);
            let elapsed = began.elapsed().as_millis() as u64;
            let message = outcome.err().map(|e| {
                if timed_out.load(std::sync::atomic::Ordering::Relaxed) {
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
                status.elapsed_ms = elapsed;
            }
            if let Err(e) = store.add_history(&connection, &sql, message.as_deref(), elapsed) {
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
        engine.connect(&connection.id, None).await.unwrap();
        assert!(
            engine
                .start(connection.id.clone(), "DROP TABLE t".into(), 100, 5, false)
                .await
                .is_err()
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
