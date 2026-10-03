mod command;
mod inspect;
mod transport;
use async_trait::async_trait;
use klyndb_driver_api::*;
use redis::{Cmd, Value, aio::MultiplexedConnection};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub struct Redis {
    connection: Mutex<Option<State>>,
    read_only: bool,
}
struct State {
    connection: MultiplexedConnection,
    worker: JoinHandle<()>,
    remaining: Arc<AtomicUsize>,
    reusable: bool,
}
impl Drop for State {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl State {
    async fn open<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        stream: S,
        info: &redis::RedisConnectionInfo,
    ) -> Result<Self> {
        let remaining = Arc::new(AtomicUsize::new(transport::RESPONSE_BYTES));
        let config = redis::AsyncConnectionConfig::new()
            .set_response_timeout(None)
            .set_pipeline_buffer_size(4)
            .set_concurrency_limit(1);
        let (connection, worker) = MultiplexedConnection::new_with_config(info, transport::Bounded { stream, remaining: remaining.clone() }, config).await.map_err(|_| Error::new("Redis handshake failed. Check authentication, database number and server permissions."))?;
        Ok(Self {
            connection,
            worker: tokio::spawn(worker),
            remaining,
            reusable: true,
        })
    }
    async fn request(&mut self, command: &Cmd) -> Result<Value> {
        self.remaining
            .store(transport::RESPONSE_BYTES, Ordering::Relaxed);
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            command.query_async(&mut self.connection),
        )
        .await;
        self.finish(result)
    }
    async fn pipeline(&mut self, pipeline: &redis::Pipeline) -> Result<Vec<Value>> {
        self.remaining
            .store(transport::RESPONSE_BYTES, Ordering::Relaxed);
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            pipeline.query_async(&mut self.connection),
        )
        .await;
        self.finish(result)
    }
    fn finish<T>(
        &mut self,
        result: std::result::Result<redis::RedisResult<T>, tokio::time::error::Elapsed>,
    ) -> Result<T> {
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error))
                if !error.is_unrecoverable_error()
                    && !error.is_io_error()
                    && !error.is_timeout() =>
            {
                Err(Error::new(if error.is_cluster_error() {
                    "Redis Cluster redirection is not supported by this standalone connection"
                } else {
                    "Redis command failed. Check arguments, key type and server permissions."
                }))
            }
            error => {
                self.reusable = false;
                self.worker.abort();
                Err(Error::new(if error.is_err() {
                    "Redis request timed out after 10 seconds; connection closed. Reconnect."
                } else {
                    "Redis transport or 8 MiB response limit failed; connection closed. Reconnect."
                }))
            }
        }
    }
}
impl Redis {
    pub async fn connect(
        address: &str,
        password: Option<&str>,
        read_only: bool,
        identity_password: Option<&str>,
    ) -> Result<Self> {
        Self::connect_via(address, password, read_only, identity_password, None).await
    }
    pub async fn connect_via(
        address: &str,
        password: Option<&str>,
        read_only: bool,
        identity_password: Option<&str>,
        endpoint: Option<SocketAddr>,
    ) -> Result<Self> {
        let url = url::Url::parse(address).map_err(|_| Error::new("Enter a valid Redis URL"))?;
        if !matches!(url.scheme(), "redis" | "rediss")
            || url.host_str().is_none()
            || url.password().is_some()
        {
            return Err(Error::new(
                "Expected redis://user@host:6379/0; use the password field for credentials",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (key, value) in url.query_pairs() {
            if !seen.insert(key.to_string())
                || !matches!(
                    key.as_ref(),
                    "tls" | "sslrootcert" | "sslidentity" | "connect_timeout"
                )
                || (key == "tls" && !matches!(value.as_ref(), "required" | "disabled"))
            {
                return Err(Error::new(
                    "Unsupported or repeated Redis connection option",
                ));
            }
        }
        let timeout = connect_timeout(
            url.query_pairs()
                .filter(|(k, _)| k == "connect_timeout")
                .map(|(_, v)| v),
        )?;
        let tls = !url
            .query_pairs()
            .any(|(k, v)| k == "tls" && v == "disabled");
        if !tls
            && (url.scheme() == "rediss"
                || url
                    .query_pairs()
                    .any(|(k, _)| matches!(k.as_ref(), "sslrootcert" | "sslidentity")))
        {
            return Err(Error::new(
                "Redis certificate files and rediss URLs require verified TLS",
            ));
        }
        let database = if matches!(url.path(), "" | "/") {
            0
        } else {
            url.path()
                .trim_start_matches('/')
                .parse::<u32>()
                .map_err(|_| Error::new("Redis database must be a nonnegative integer"))?
        };
        let host = url
            .host_str()
            .ok_or_else(|| Error::new("Enter a Redis hostname"))?
            .trim_matches(['[', ']']);
        let port = url.port().unwrap_or(6379);
        let mut info = redis::RedisConnectionInfo::default()
            .set_db(i64::from(database))
            .set_skip_set_lib_name();
        if !url.username().is_empty() {
            let username = percent_encoding::percent_decode_str(url.username())
                .decode_utf8()
                .map_err(|_| Error::new("Invalid Redis username"))?;
            info = info.set_username(username);
        }
        if let Some(password) = password {
            if password.len() > 16384 {
                return Err(Error::new("Redis password exceeds 16 KiB"));
            }
            info = info.set_password(password);
        }
        tokio::time::timeout(timeout, async {
            let tcp = if let Some(endpoint) = endpoint { TcpStream::connect(endpoint).await } else { TcpStream::connect((host, port)).await }.map_err(|_| Error::new("Could not reach the Redis server"))?;
            tcp.set_nodelay(true).map_err(|_| Error::new("Could not configure the Redis connection"))?;
            let mut state = if tls {
                let mut builder = native_tls::TlsConnector::builder();
                for (_, path) in url.query_pairs().filter(|(k, _)| k == "sslrootcert") {
                    for certificate in tls::load_ca_certificates(&path).await? {
                        builder.add_root_certificate(native_tls::Certificate::from_der(&certificate).map_err(|_| Error::new("Could not decode Redis CA certificate"))?);
                    }
                }
                for (_, path) in url.query_pairs().filter(|(k, _)| k == "sslidentity") {
                    builder.identity(tls::load_client_identity(&path, identity_password).await?.1);
                }
                let connector = builder.build().map_err(|_| Error::new("Could not configure Redis TLS"))?;
                let stream = tokio_native_tls::TlsConnector::from(connector).connect(host, tcp).await.map_err(|_| Error::new("Redis TLS verification failed. Check hostname, CA and client certificate; plaintext fallback is disabled."))?;
                State::open(stream, &info).await?
            } else { State::open(tcp, &info).await? };
            state.request(&redis::cmd("PING")).await?;
            Ok(Self { connection: Mutex::new(Some(state)), read_only })
        }).await.map_err(|_| Error::new(format!("Redis connection timed out after {} seconds", timeout.as_secs())))?
    }
}

#[async_trait]
impl Session for Redis {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            key_value: true,
            document_queries: false,
            affected_rows: false,
            table_browse: false,
            routines: false,
            diagrams: false,
            transactions: false,
            schemas: false,
            explain: false,
            explain_analyze: false,
            edit_rows: false,
            import_rows: false,
            import_sql: false,
            cancel: false,
            tls: true,
        }
    }
    async fn scan_keys(&self, pattern: &str, cursor: &str) -> Result<KeyScan> {
        if pattern.len() > 1024 {
            return Err(Error::new("Key search patterns are limited to 1,024 bytes"));
        }
        let cursor = inspect::cursor(cursor)?;
        let mut guard = self.connection.lock().await;
        let mut state = guard
            .take()
            .ok_or_else(|| Error::new("Redis connection is closed; reconnect"))?;
        let result = inspect::scan(&mut state, pattern, cursor).await;
        if state.reusable {
            *guard = Some(state);
        }
        result
    }
    async fn inspect_key(&self, key: &Cell, position: &str) -> Result<KeyInspection> {
        let key = inspect::key_bytes(key)?;
        if position.len() > 64 {
            return Err(Error::new("Invalid value page position"));
        }
        let mut guard = self.connection.lock().await;
        let mut state = guard
            .take()
            .ok_or_else(|| Error::new("Redis connection is closed; reconnect"))?;
        let result = inspect::inspect(&mut state, key, position).await;
        if state.reusable {
            *guard = Some(state);
        }
        result
    }
    fn key_command_info(&self, text: &str) -> Result<KeyCommandInfo> {
        Ok(command::parse(text)?.1)
    }
    async fn key_command(&self, text: &str) -> Result<KeyValue> {
        let (arguments, info) = command::parse(text)?;
        if self.read_only && info.writes {
            return Err(Error::new("This Redis connection is read-only"));
        }
        let mut native = redis::cmd(&arguments[0]);
        native.arg(&arguments[1..]);
        let mut guard = self.connection.lock().await;
        let mut state = guard
            .take()
            .ok_or_else(|| Error::new("Redis connection is closed; reconnect"))?;
        let result = state.request(&native).await;
        let uncertain = !state.reusable && info.writes;
        if state.reusable {
            *guard = Some(state);
        }
        let result = result.and_then(|value| inspect::value(value).map_err(|error| {
            if info.writes { Error::new(format!("Command completed, but its reply could not be displayed: {} Verify the value before retrying.", error.message)) } else { error }
        }));
        result.map_err(|error| {
            if uncertain {
                Error::new(format!(
                    "{} The command may have completed. Verify writes before retrying.",
                    error.message
                ))
            } else {
                error
            }
        })
    }
    async fn transaction_state(&self) -> Result<TransactionState> {
        self.key_command("[\"PING\"]").await?;
        Ok(TransactionState::Idle)
    }
    async fn disconnect(&self) -> Result<()> {
        self.connection.lock().await.take();
        Ok(())
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        Err(Error::new("Redis uses the key explorer, not SQL tables"))
    }
    async fn inspect(&self, _: &Table) -> Result<TableInfo> {
        Err(Error::new("Redis uses native key inspection"))
    }
    async fn execute(
        &self,
        _: String,
        _: mpsc::Sender<Batch>,
        _: CancellationToken,
        _: usize,
    ) -> Result<()> {
        Err(Error::new("Use the Redis native command console, not SQL"))
    }
    async fn apply_changes(&self, _: Table, _: Vec<Change>) -> Result<MutationResult> {
        Err(Error::new("Use Redis native data commands to edit keys"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn stalled_native_request_closes_worker_without_replay() {
        let (client, mut server) = tokio::io::duplex(256);
        let info = redis::RedisConnectionInfo::default().set_skip_set_lib_name();
        let mut state = State::open(client, &info).await.unwrap();
        let receiver = tokio::spawn(async move {
            let mut buffer = [0; 256];
            let received = server.read(&mut buffer).await.unwrap();
            assert!(received > 0);
            assert_eq!(
                server.read(&mut buffer).await.unwrap(),
                0,
                "Timed-out worker must close transport without replaying the command"
            );
        });
        let started = std::time::Instant::now();
        let error = state.request(&redis::cmd("PING")).await.unwrap_err();
        assert!(error.message.contains("timed out"));
        assert!(!state.reusable);
        assert!(started.elapsed() < Duration::from_secs(12));
        tokio::time::timeout(Duration::from_secs(2), receiver)
            .await
            .unwrap()
            .unwrap();
    }
}
