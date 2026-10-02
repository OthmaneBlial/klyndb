mod edit;
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
impl Postgres {
    pub async fn connect(url: &str, password: Option<&str>, read_only: bool) -> Result<Self> {
        let mut config: tokio_postgres::Config = url
            .parse()
            .map_err(|_| Error::new("Invalid PostgreSQL connection URL"))?;
        if let Some(password) = password {
            config.password(password);
        }
        config.connect_timeout(std::time::Duration::from_secs(10));
        config.keepalives(true);
        let tls = MakeTlsConnector::new(
            native_tls::TlsConnector::new().map_err(|_| Error::new("Could not initialize TLS"))?,
        );
        let (client, connection) = config.connect(tls.clone()).await.map_err(err)?;
        let worker = tokio::spawn(async move {
            let _ = connection.await;
        });
        if read_only {
            client
                .batch_execute("SET default_transaction_read_only=on")
                .await
                .map_err(err)?;
        }
        Ok(Self {
            client,
            worker,
            serial: Mutex::new(()),
            tls,
            read_only,
        })
    }
    async fn stream(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()> {
        let stream = self.client.simple_query_raw(&sql).await.map_err(err)?;
        pin_mut!(stream);
        let mut count = 0;
        let mut has_columns = false;
        let mut buffer = Vec::with_capacity(256);
        let mut buffer_bytes = 0;
        loop {
            let item = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    self.client.cancel_token().cancel_query(self.tls.clone()).await.map_err(err)?;
                    // Drain ReadyForQuery before another query can use this session.
                    while stream.next().await.is_some() {}
                    return Err(Error::new("Query cancelled"));
                }
                item = stream.next() => item,
            };
            let Some(item) = item else {
                break;
            };
            match item.map_err(err)? {
                SimpleQueryMessage::RowDescription(columns) => {
                    count = 0;
                    has_columns = true;
                    output
                        .send(Batch::Columns(
                            columns.iter().map(|c| c.name().to_string()).collect(),
                        ))
                        .await
                        .map_err(|_| Error::new("Result consumer closed"))?;
                }
                SimpleQueryMessage::Row(row) => {
                    if count >= limit {
                        continue;
                    }
                    let row_bytes = (0..row.len())
                        .map(|i| row.get(i).map_or(0, str::len))
                        .sum::<usize>();
                    if row_bytes > 8 * 1024 * 1024 {
                        self.client
                            .cancel_token()
                            .cancel_query(self.tls.clone())
                            .await
                            .map_err(err)?;
                        while stream.next().await.is_some() {}
                        return Err(Error::new(
                            "A result row exceeds 8 MiB. Select smaller values or use database-native export.",
                        ));
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
                        output
                            .send(Batch::Rows(std::mem::take(&mut buffer)))
                            .await
                            .map_err(|_| Error::new("Result consumer closed"))?;
                    }
                }
                SimpleQueryMessage::CommandComplete(affected) => {
                    if !buffer.is_empty() {
                        output
                            .send(Batch::Rows(std::mem::take(&mut buffer)))
                            .await
                            .map_err(|_| Error::new("Result consumer closed"))?;
                    }
                    if !has_columns {
                        output
                            .send(Batch::Columns(vec![]))
                            .await
                            .map_err(|_| Error::new("Result consumer closed"))?;
                    }
                    output
                        .send(Batch::Complete {
                            affected: if has_columns { 0 } else { affected },
                            truncated: affected > limit as u64 && has_columns,
                        })
                        .await
                        .map_err(|_| Error::new("Result consumer closed"))?;
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
            transactions: true,
            schemas: true,
            explain: true,
            edit_rows: true,
            cancel: true,
            tls: true,
        }
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
        if self.read_only {
            self.client.batch_execute("ROLLBACK").await.map_err(err)?;
        }
        result
    }

    async fn tables(&self) -> Result<Vec<Table>> {
        let _guard = self.serial.lock().await;
        self.client.query("SELECT table_schema,table_name,lower(table_type) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema,table_name", &[]).await.map_err(err).map(|rows| rows.iter().map(|r| Table { schema:r.get(0), name:r.get(1), kind:r.get(2) }).collect())
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let _guard = self.serial.lock().await;
        let columns = self.client.query("SELECT c.column_name,c.data_type,c.is_nullable,c.column_default,(c.is_generated!='NEVER' OR c.identity_generation='ALWAYS'),EXISTS(SELECT 1 FROM information_schema.table_constraints t JOIN information_schema.key_column_usage k USING(constraint_catalog,constraint_schema,constraint_name) WHERE t.constraint_type='PRIMARY KEY' AND k.table_schema=c.table_schema AND k.table_name=c.table_name AND k.column_name=c.column_name) FROM information_schema.columns c WHERE table_schema=$1 AND table_name=$2 ORDER BY ordinal_position", &[&table.schema, &table.name]).await.map_err(err)?.iter().map(|r| Column { name:r.get(0), data_type:r.get(1), nullable:r.get::<_, String>(2)=="YES", default:r.get(3), primary_key:r.get(5), generated:r.get::<_, Option<bool>>(4).unwrap_or(false) }).collect();
        let indexes = self.client.query("SELECT indexname,indexdef FROM pg_indexes WHERE schemaname=$1 AND tablename=$2", &[&table.schema,&table.name]).await.map_err(err)?.iter().map(|r| serde_json::json!({"name":r.get::<_, String>(0), "definition":r.get::<_, String>(1)})).collect();
        let foreign_keys = self.client.query("SELECT c.conname,pg_get_constraintdef(c.oid) FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid JOIN pg_namespace n ON n.oid=t.relnamespace WHERE c.contype='f' AND n.nspname=$1 AND t.relname=$2", &[&table.schema,&table.name]).await.map_err(err)?.iter().map(|r| serde_json::json!({"name":r.get::<_, String>(0), "definition":r.get::<_, String>(1)})).collect();
        let editable=self.client.query_opt("SELECT c.relkind IN ('r','p') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2",&[&table.schema,&table.name]).await.map_err(err)?.is_some_and(|r|r.get(0));
        Ok(TableInfo {
            editable,
            columns,
            ddl: None,
            indexes,
            foreign_keys,
        })
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
        edit::apply(&self.client, &table, &changes).await
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
