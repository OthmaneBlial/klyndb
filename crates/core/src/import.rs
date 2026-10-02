use super::{Engine, error};
use klyndb_driver_api::*;
use klyndb_import::{FILE_LIMIT, Snapshot, validate_mapping};
pub use klyndb_import::{ImportFormat, ImportOptions, Mapping, Preview};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
pub struct ImportSource {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub preview: Preview,
}
#[derive(Deserialize)]
pub struct ImportRequest {
    pub source: String,
    pub connection: String,
    pub table: Table,
    pub options: ImportOptions,
    pub mapping: Vec<Mapping>,
    pub timeout_seconds: u64,
    pub confirmed: bool,
}
#[derive(Clone, Serialize)]
pub struct ImportStatus {
    pub id: String,
    pub connection_id: String,
    pub done: bool,
    pub read_rows: u64,
    pub elapsed_ms: u64,
    pub result: Option<MutationResult>,
    pub transaction: Option<TransactionState>,
    pub error: Option<String>,
}
struct ImportJob {
    status: Mutex<ImportStatus>,
    read_rows: AtomicU64,
    began: Instant,
    cancel: CancellationToken,
    // A sticky completion signal: disconnect must wait for writer + reader cleanup.
    finished: CancellationToken,
    connection: String,
}
struct Source {
    snapshot: Arc<Snapshot>,
    job: Option<Arc<ImportJob>>,
}
#[derive(Default)]
pub struct Imports {
    sources: Mutex<HashMap<String, Source>>,
    gate: Arc<tokio::sync::Mutex<()>>,
    closing: AtomicBool,
}
impl Imports {
    fn snapshot(&self, id: &str) -> Result<Arc<Snapshot>> {
        let sources = self.sources.lock().map_err(error)?;
        let source = sources
            .get(id)
            .ok_or_else(|| Error::new("Selected import file has expired"))?;
        if source.job.is_some() {
            return Err(Error::new(
                "This file has already started importing; choose it again to run another import",
            ));
        }
        Ok(source.snapshot.clone())
    }
    fn check_capacity(sources: &HashMap<String, Source>, bytes: u64) -> Result<()> {
        if sources.len() >= 4
            || sources.values().map(|s| s.snapshot.bytes).sum::<u64>() + bytes > FILE_LIMIT
        {
            return Err(Error::new(
                "Close an import first (four files and 512 MiB total maximum)",
            ));
        }
        Ok(())
    }
    // Called only with a path from a native dialog, never with a frontend path.
    pub async fn prepare(
        self: &Arc<Self>,
        path: PathBuf,
        options: ImportOptions,
    ) -> Result<ImportSource> {
        let gate = self.gate.clone().lock_owned().await;
        let imports = self.clone();
        tokio::task::spawn_blocking(move || {
            let _gate = gate;
            options.validate()?;
            {
                let sources = imports.sources.lock().map_err(error)?;
                Self::check_capacity(&sources, 0)?;
            }
            let snapshot = Arc::new(Snapshot::copy(&path)?);
            let preview = snapshot.preview(&options)?;
            let mut sources = imports.sources.lock().map_err(error)?;
            Self::check_capacity(&sources, snapshot.bytes)?;
            let id = uuid::Uuid::new_v4().to_string();
            let response = ImportSource {
                id: id.clone(),
                name: snapshot.name.clone(),
                bytes: snapshot.bytes,
                preview,
            };
            sources.insert(
                id,
                Source {
                    snapshot,
                    job: None,
                },
            );
            Ok(response)
        })
        .await
        .map_err(error)?
    }
    pub async fn preview(self: &Arc<Self>, id: &str, options: ImportOptions) -> Result<Preview> {
        let gate = self.gate.clone().lock_owned().await;
        let snapshot = self.snapshot(id)?;
        tokio::task::spawn_blocking(move || {
            let _gate = gate;
            options.validate()?;
            snapshot.preview(&options)
        })
        .await
        .map_err(error)?
    }
    fn job(&self, id: &str) -> Result<Arc<ImportJob>> {
        self.sources
            .lock()
            .map_err(error)?
            .get(id)
            .and_then(|s| s.job.clone())
            .ok_or_else(|| Error::new("Import job not found"))
    }
    pub fn status(&self, id: &str) -> Result<ImportStatus> {
        let job = self.job(id)?;
        let mut status = job.status.lock().map_err(error)?.clone();
        status.read_rows = job.read_rows.load(Ordering::Relaxed);
        if !status.done {
            status.elapsed_ms = job.began.elapsed().as_millis() as u64;
        }
        Ok(status)
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        self.job(id)?.cancel.cancel();
        Ok(())
    }
    pub fn release(&self, id: &str) -> Result<()> {
        let mut sources = self.sources.lock().map_err(error)?;
        if sources
            .get(id)
            .and_then(|s| s.job.as_ref())
            .is_some_and(|j| !j.finished.is_cancelled())
        {
            return Err(Error::new(
                "Cancel the import and wait for it to finish before closing",
            ));
        }
        sources.remove(id);
        Ok(())
    }
    pub async fn cancel_connection(&self, connection: &str) -> Result<()> {
        let jobs: Vec<_> = self
            .sources
            .lock()
            .map_err(error)?
            .values()
            .filter_map(|s| s.job.as_ref())
            .filter(|j| j.connection == connection)
            .cloned()
            .collect();
        for job in &jobs {
            job.cancel.cancel();
        }
        for job in jobs {
            job.finished.cancelled().await;
        }
        Ok(())
    }
    pub fn begin_shutdown(&self) -> bool {
        self.closing.store(true, Ordering::Relaxed);
        self.sources.lock().map_or(true, |sources| {
            sources
                .values()
                .filter_map(|s| s.job.as_ref())
                .any(|j| !j.finished.is_cancelled())
        })
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.closing.store(true, Ordering::Relaxed);
        let jobs: Vec<_> = self
            .sources
            .lock()
            .map_err(error)?
            .values()
            .filter_map(|s| s.job.as_ref())
            .cloned()
            .collect();
        for job in &jobs {
            job.cancel.cancel();
        }
        for job in jobs {
            job.finished.cancelled().await;
        }
        Ok(())
    }
}

impl Engine {
    pub async fn start_import(&self, request: ImportRequest) -> Result<String> {
        let ImportRequest {
            source,
            connection,
            table,
            options,
            mapping,
            timeout_seconds,
            confirmed,
        } = request;
        if !(1..=3600).contains(&timeout_seconds) {
            return Err(Error::new("Timeout must be 1–3600 seconds"));
        }
        options.validate()?;
        let (driver, config) = {
            let sessions = self.sessions.lock().await;
            let open = sessions
                .get(&connection)
                .ok_or_else(|| Error::new("Connect to the database first"))?;
            (open.driver.clone(), open.config.clone())
        };
        if config.read_only || !driver.capabilities().import_rows {
            return Err(Error::new("This connection does not allow imports"));
        }
        if config.environment == "production" && !confirmed {
            return Err(Error::new(
                "Confirmation required for importing into production",
            ));
        }
        let snapshot = self.imports.snapshot(&source)?;
        let gate = self.imports.gate.clone().lock_owned().await;
        let header_source = snapshot.clone();
        let header_options = options.clone();
        let width = tokio::task::spawn_blocking(move || {
            let _gate = gate;
            header_source
                .reader(&header_options)?
                .headers()
                .map(|h| h.len())
        })
        .await
        .map_err(error)??;
        let info = tokio::time::timeout(Duration::from_secs(10), driver.inspect(&table))
            .await
            .map_err(|_| Error::new("Table inspection timed out; no import started"))??;
        if !info.editable || !["table", "base table"].contains(&table.kind.as_str()) {
            return Err(Error::new("Choose an editable base table for import"));
        }
        validate_mapping(&mapping, width, &info.columns)?;
        // Register before releasing the session registry, so disconnect sees and cancels this job.
        let sessions = self.sessions.lock().await;
        if !sessions
            .get(&connection)
            .is_some_and(|open| Arc::ptr_eq(&open.driver, &driver))
        {
            return Err(Error::new(
                "Connection changed; reconnect and review the import again",
            ));
        }
        let job = Arc::new(ImportJob {
            status: Mutex::new(ImportStatus {
                id: source.clone(),
                connection_id: connection.clone(),
                done: false,
                read_rows: 0,
                elapsed_ms: 0,
                result: None,
                transaction: None,
                error: None,
            }),
            read_rows: AtomicU64::new(0),
            began: Instant::now(),
            cancel: CancellationToken::new(),
            finished: CancellationToken::new(),
            connection: connection.clone(),
        });
        {
            let mut sources = self.imports.sources.lock().map_err(error)?;
            if self.imports.closing.load(Ordering::Relaxed) {
                return Err(Error::new("The application is closing; no import started"));
            }
            if sources
                .values()
                .filter_map(|s| s.job.as_ref())
                .any(|j| j.connection == connection && !j.finished.is_cancelled())
            {
                return Err(Error::new(
                    "Wait for the current import on this connection to finish",
                ));
            }
            let entry = sources
                .get_mut(&source)
                .ok_or_else(|| Error::new("Selected file has expired"))?;
            if entry.job.is_some() {
                return Err(Error::new("This file has already started importing"));
            }
            entry.job = Some(job.clone());
        }
        tokio::spawn(async move {
            let timed_out = Arc::new(AtomicBool::new(false));
            let timer_flag = timed_out.clone();
            let timeout_cancel = job.cancel.clone();
            let timer = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(timeout_seconds)).await;
                timer_flag.store(true, Ordering::Relaxed);
                timeout_cancel.cancel();
            });
            let (output, input) = mpsc::channel(2);
            let reader_job = job.clone();
            let producer = tokio::task::spawn_blocking(move || {
                snapshot.produce(
                    &options,
                    &mapping,
                    output,
                    reader_job.cancel.clone(),
                    &reader_job.read_rows,
                )
            });
            let written = driver.insert_stream(table, input, job.cancel.clone()).await;
            if written.is_err() {
                job.cancel.cancel();
            }
            let parsed = producer.await.map_err(error).and_then(|r| r);
            timer.abort();
            let result = match (written, parsed) {
                (Ok(written), Ok(count)) if written.affected == count => Ok(written),
                (Ok(_), _) => Err(Error::new(
                    "Database acknowledged the import, but reader completion could not be verified. Verify data before retrying.",
                )),
                (Err(e), _) => Err(e),
            };
            let transaction = match &result {
                Ok(r) => Some(if r.pending_transaction {
                    TransactionState::Active
                } else {
                    TransactionState::Idle
                }),
                Err(_) => tokio::time::timeout(Duration::from_secs(3), driver.transaction_state())
                    .await
                    .ok()
                    .and_then(std::result::Result::ok),
            };
            let message = result.as_ref().err().map(|e| {
                if timed_out.load(Ordering::Relaxed) {
                    format!(
                        "Import timed out after {timeout_seconds} seconds. {}",
                        e.message
                    )
                } else {
                    e.message.clone()
                }
            });
            if let Ok(mut status) = job.status.lock() {
                status.done = true;
                status.elapsed_ms = job.began.elapsed().as_millis() as u64;
                status.error = message;
                status.result = result.ok();
                status.transaction = transaction;
            }
            job.finished.cancel();
        });
        drop(sessions);
        Ok(source)
    }
}
