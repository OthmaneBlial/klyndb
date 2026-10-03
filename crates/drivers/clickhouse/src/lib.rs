use async_trait::async_trait;
use futures_util::StreamExt;
use klickhouse::{Client, ClientOptions, KlickhouseError, Type, Value};
use klyndb_driver_api::*;
use std::{net::SocketAddr, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

pub struct ClickHouse {
    connection: Mutex<Option<Client>>,
    transport: Transport,
}
struct Transport {
    host: String,
    port: u16,
    endpoint: Option<SocketAddr>,
    tls: Option<native_tls::TlsConnector>,
    options: ClientOptions,
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.options.password.zeroize();
    }
}
fn err(e: KlickhouseError) -> Error {
    // Server messages/stack traces may echo SQL containing credentials. Keep codes only.
    match e {
        KlickhouseError::ServerException { code, name, .. } => Error::new(format!(
            "ClickHouse {code}: {name}. Check the SQL and database permissions."
        )),
        _ => Error::new(
            "ClickHouse connection or result decoding failed. Check host, credentials, TLS and column types; unsupported types can be cast to String.",
        ),
    }
}
impl Transport {
    async fn open(&self) -> Result<Client> {
        let stream = match self.endpoint {
            Some(endpoint) => TcpStream::connect(endpoint).await,
            None => TcpStream::connect((self.host.as_str(), self.port)).await,
        }
        .map_err(|_| Error::new("Could not reach the ClickHouse native TCP server"))?;
        stream
            .set_nodelay(true)
            .map_err(|_| Error::new("Could not configure ClickHouse transport"))?;
        let options = self.options.clone();
        if let Some(tls) = &self.tls {
            let stream = tokio_native_tls::TlsConnector::from(tls.clone()).connect(&self.host, stream).await
                .map_err(|_| Error::new("ClickHouse TLS verification failed. Check the hostname, CA and client identity."))?;
            let (reader, writer) = tokio::io::split(stream);
            Client::connect_stream(reader, writer, options)
                .await
                .map_err(err)
        } else {
            let (reader, writer) = stream.into_split();
            Client::connect_stream(reader, writer, options)
                .await
                .map_err(err)
        }
    }
}
impl ClickHouse {
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
        let url = url::Url::parse(address).map_err(|_| Error::new("Invalid ClickHouse URL"))?;
        if url.scheme() != "clickhouse" || url.host_str().is_none() {
            return Err(Error::new(
                "Expected clickhouse://user@host:9000/database?tls=disabled or verified TLS on port 9440",
            ));
        }
        let timeout = connect_timeout(
            url.query_pairs()
                .filter(|(k, _)| k == "connect_timeout")
                .map(|(_, v)| v),
        )?;
        let mut seen = std::collections::HashSet::new();
        if url.query_pairs().any(|(k, _)| {
            !["tls", "sslrootcert", "sslidentity", "connect_timeout"].contains(&k.as_ref())
                || !seen.insert(k.into_owned())
        }) {
            return Err(Error::new("Unsupported or repeated ClickHouse URL option"));
        }
        let option = |key| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let mode = option("tls").unwrap_or_else(|| "required".into());
        if !["required", "disabled"].contains(&mode.as_str()) {
            return Err(Error::new("ClickHouse tls must be required or disabled"));
        }
        if endpoint.is_some_and(|e| !e.ip().is_loopback()) {
            return Err(Error::new("Invalid tunnel endpoint"));
        }
        let tls = if mode == "required" {
            let mut builder = native_tls::TlsConnector::builder();
            if let Some(path) = option("sslrootcert") {
                for cert in tls::load_ca_certificates(&path).await? {
                    builder.add_root_certificate(
                        native_tls::Certificate::from_der(&cert)
                            .map_err(|_| Error::new("Invalid CA certificate"))?,
                    );
                }
            }
            if let Some(path) = option("sslidentity") {
                builder.identity(tls::load_client_identity(&path, identity_password).await?.1);
            }
            Some(
                builder
                    .build()
                    .map_err(|_| Error::new("Could not configure ClickHouse TLS"))?,
            )
        } else {
            if option("sslrootcert").is_some() || option("sslidentity").is_some() {
                return Err(Error::new("Certificate files require verified TLS"));
            }
            None
        };
        let decode = |s: &str| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8()
                .map(|s| s.into_owned())
                .map_err(|_| Error::new("ClickHouse URL values must use UTF-8"))
        };
        let username = if url.username().is_empty() {
            "default".into()
        } else {
            decode(url.username())?
        };
        let password = match password {
            Some(p) => p.to_owned(),
            None => decode(url.password().unwrap_or_default())?,
        };
        let database = decode(url.path().trim_start_matches('/'))?;
        if [&username, &password, &database]
            .iter()
            .any(|s| s.len() > 16384 || s.contains('\0'))
        {
            return Err(Error::new(
                "Invalid ClickHouse credentials or database name",
            ));
        }
        let transport = Transport {
            host: url
                .host_str()
                .unwrap_or_default()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .into(),
            port: url
                .port()
                .unwrap_or(if mode == "required" { 9440 } else { 9000 }),
            endpoint,
            tls,
            options: ClientOptions {
                username,
                password,
                default_database: database,
                tcp_nodelay: true,
            },
        };
        let client = tokio::time::timeout(timeout, async {
            let client = transport.open().await?;
            match client
                .execute("SET max_block_size=256, preferred_block_size_bytes=262144")
                .await
            {
                Ok(()) | Err(KlickhouseError::ServerException { code: 164, .. }) => {}
                Err(e) => return Err(err(e)),
            }
            if read_only && let Err(e) = client.execute("SET readonly=1").await {
                // A server-enforced readonly profile cannot change settings, even to the same value.
                let native = rows(
                    &client,
                    "SELECT value FROM system.settings WHERE name='readonly'".into(),
                )
                .await?;
                if !matches!(e, KlickhouseError::ServerException { code: 164, .. })
                    || native
                        .first()
                        .and_then(|r| r.first())
                        .is_none_or(|v| v.text() != "1")
                {
                    return Err(err(e));
                }
            }
            client.execute("SELECT 1").await.map_err(err)?;
            Ok::<_, Error>(client)
        })
        .await
        .map_err(|_| {
            Error::new(format!(
                "ClickHouse connection timed out after {} seconds",
                timeout.as_secs()
            ))
        })??;
        Ok(Self {
            connection: Mutex::new(Some(client)),
            transport,
        })
    }
    async fn kill(&self, marker: &str) -> Result<()> {
        // Only this session's UUID-prefixed statement and authenticated user can match.
        let control = self.transport.open().await?;
        control
            .execute(format!(
                "KILL QUERY WHERE startsWith(query, {}) AND user=currentUser() ASYNC",
                literal(marker)
            ))
            .await
            .map_err(err)
    }
}
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}
fn decimal(raw: String, scale: usize) -> Result<String> {
    if scale > 76 {
        return Err(Error::new("Unsupported ClickHouse decimal scale"));
    }
    let (sign, digits) = raw
        .strip_prefix('-')
        .map_or(("", raw.as_str()), |v| ("-", v));
    if scale == 0 {
        return Ok(raw);
    }
    let digits = format!("{:0>width$}", digits, width = scale + 1);
    let split = digits.len() - scale;
    Ok(format!("{sign}{}.{}", &digits[..split], &digits[split..]))
}
fn cell(value: &Value, ty: &Type) -> Result<Cell> {
    if let Type::Nullable(inner) | Type::LowCardinality(inner) = ty {
        return cell(value, inner);
    }
    use Value::*;
    let number = |s| Ok(Cell::Number(s));
    match value {
        Null => Ok(Cell::Null),
        Int8(v) => number(v.to_string()),
        Int16(v) => number(v.to_string()),
        Int32(v) => number(v.to_string()),
        Int64(v) => number(v.to_string()),
        Int128(v) => number(v.to_string()),
        UInt8(v) => number(v.to_string()),
        UInt16(v) => number(v.to_string()),
        UInt32(v) => number(v.to_string()),
        UInt64(v) => number(v.to_string()),
        UInt128(v) => number(v.to_string()),
        Int256(v) => number(num_bigint::BigInt::from_signed_bytes_be(&v.0).to_string()),
        UInt256(v) => number(num_bigint::BigUint::from_bytes_be(&v.0).to_string()),
        Float32(v) => number(v.to_string()),
        Float64(v) => number(v.to_string()),
        BFloat16(v) => number(v.to_string()),
        Decimal32(s, v) => number(decimal(v.to_string(), *s)?),
        Decimal64(s, v) => number(decimal(v.to_string(), *s)?),
        Decimal128(s, v) => number(decimal(v.to_string(), *s)?),
        Decimal256(s, v) => number(decimal(
            num_bigint::BigInt::from_signed_bytes_be(&v.0).to_string(),
            *s,
        )?),
        String(bytes) => Ok(match std::str::from_utf8(bytes) {
            Ok(s) => Cell::Text(s.into()),
            Err(_) => Cell::Binary(hex::encode(bytes)),
        }),
        Uuid(v) => Ok(Cell::Text(v.to_string())),
        Ipv4(v) => Ok(Cell::Text(v.to_string())),
        Ipv6(v) => Ok(Cell::Text(v.to_string())),
        Date(v) => Ok(Cell::Text(chrono::NaiveDate::from(*v).to_string())),
        DateTime(_) | DateTime64(_) => {
            let date: chrono::DateTime<chrono_tz::Tz> =
                klickhouse::FromSql::from_sql(ty, value.clone()).map_err(err)?;
            Ok(Cell::Text(date.to_rfc3339()))
        }
        Enum8(v) => enum_label(i16::from(*v), ty),
        Enum16(v) => enum_label(*v, ty),
        // ponytail: nested native types need their own typed UI/export mapping; cast to String for now.
        _ => Err(Error::new(
            "This ClickHouse column type is not mapped yet. Cast it to String in the SELECT.",
        )),
    }
}
fn enum_label(v: i16, ty: &Type) -> Result<Cell> {
    match ty {
        Type::Nullable(inner) | Type::LowCardinality(inner) => return enum_label(v, inner),
        Type::Enum8(items) => items
            .iter()
            .find(|(_, n)| i16::from(*n) == v)
            .map(|(s, _)| Cell::Text(s.clone())),
        Type::Enum16(items) => items
            .iter()
            .find(|(_, n)| *n == v)
            .map(|(s, _)| Cell::Text(s.clone())),
        _ => None,
    }
    .ok_or_else(|| Error::new("Unknown ClickHouse enum value"))
}
async fn send(output: &mpsc::Sender<Batch>, batch: Batch, stop: &CancellationToken) -> Result<()> {
    tokio::select! {biased; _=stop.cancelled()=>Err(Error::new("Query cancelled")), result=output.send(batch)=>result.map_err(|_| {stop.cancel(); Error::new("Result consumer closed")})}
}
async fn stream(
    client: &Client,
    sql: String,
    output: &mpsc::Sender<Batch>,
    stop: &CancellationToken,
    limit: usize,
) -> Result<(bool, Vec<Row>)> {
    let mut stream = client.query_raw(sql).await.map_err(err)?;
    let mut columns = None;
    let mut count = 0;
    let mut truncated = false;
    let mut buffer = vec![];
    let mut bytes = 0;
    let mut failure = None;
    while let Some(block) = stream.next().await {
        let block = match block {
            Ok(block) => block,
            Err(KlickhouseError::ServerException { code: 394, .. }) if stop.is_cancelled() => break,
            Err(e) => return Err(err(e)),
        };
        if stop.is_cancelled() {
            continue;
        }
        if columns.is_none() {
            let names = block.column_types.keys().cloned().collect::<Vec<_>>();
            if let Err(e) = send(output, Batch::Columns(names.clone()), stop).await {
                failure = Some(e);
                continue;
            }
            columns = Some(names);
        }
        for row in block.iter_rows() {
            if stop.is_cancelled() {
                break;
            }
            if count == limit {
                truncated = true;
                stop.cancel();
                break;
            }
            let values = row
                .into_iter()
                .map(|(name, v)| cell(v, &block.column_types[name]))
                .collect::<Result<Row>>();
            let values = match values {
                Ok(v) => v,
                Err(e) => {
                    failure = Some(e);
                    stop.cancel();
                    break;
                }
            };
            let size = values.iter().map(Cell::byte_len).sum::<usize>();
            if size > 8 * 1024 * 1024 {
                failure = Some(Error::new(
                    "A result row exceeds 8 MiB. Select smaller values.",
                ));
                stop.cancel();
                break;
            }
            bytes += size;
            buffer.push(values);
            count += 1;
            if buffer.len() == 256 || bytes >= 256 * 1024 {
                if let Err(e) = send(output, Batch::Rows(std::mem::take(&mut buffer)), stop).await {
                    failure = Some(e);
                    break;
                }
                bytes = 0;
            }
        }
    }
    if client.is_closed() {
        return Err(Error::new(
            "ClickHouse connection closed while reading results. Reconnect; use distinct column aliases and cast unsupported types to String.",
        ));
    }
    if let Some(e) = failure {
        return Err(e);
    }
    if columns.is_none() && !stop.is_cancelled() {
        send(output, Batch::Columns(vec![]), stop).await?;
    }
    Ok((truncated, buffer))
}
async fn rows(client: &Client, sql: String) -> Result<Vec<Row>> {
    let mut stream = client.query_raw(sql).await.map_err(err)?;
    let mut rows = vec![];
    while let Some(block) = stream.next().await {
        let block = block.map_err(err)?;
        for row in block.iter_rows() {
            if rows.len() == 50_000 {
                return Err(Error::new(
                    "Catalog exceeds 50,000 rows. Use a narrower database.",
                ));
            }
            rows.push(
                row.into_iter()
                    .map(|(name, v)| cell(v, &block.column_types[name]))
                    .collect::<Result<Row>>()?,
            );
        }
    }
    if client.is_closed() {
        return Err(Error::new(
            "ClickHouse connection closed while reading metadata",
        ));
    }
    Ok(rows)
}
#[async_trait]
impl Session for ClickHouse {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            affected_rows: false,
            table_browse: true,
            routines: false,
            diagrams: false,
            transactions: false,
            schemas: true,
            explain: true,
            explain_analyze: false,
            edit_rows: false,
            import_rows: false,
            import_sql: false,
            cancel: true,
            tls: true,
        }
    }
    fn explain_sql(&self, sql: &str, analyze: bool) -> Result<(String, PlanFormat)> {
        if analyze {
            return Err(Error::new(
                "ClickHouse runtime ANALYZE is not supported by this driver",
            ));
        }
        klyndb_query::clickhouse_plan_target(sql)?;
        Ok((
            format!("EXPLAIN PLAN json=1, indexes=1 {sql}"),
            PlanFormat::ClickHouseJson,
        ))
    }
    fn quote_identifier(&self, name: &str) -> String {
        quote_clickhouse_identifier(name)
    }
    fn quote_filter_value(&self, value: &str) -> String {
        literal(value)
    }
    fn contains_filter_sql(&self, column: &str, value: &str) -> String {
        format!("position({column}, {}) > 0", literal(value))
    }
    async fn execute(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()> {
        if !(1..=10_000_000).contains(&limit) {
            return Err(Error::new("Row limit must be 1–10,000,000"));
        }
        let mut reader = klyndb_import::SqlReader::new(sql.as_bytes(), "clickhouse")?;
        let mut guard = self.connection.lock().await;
        while let Some(sql) = reader.next_statement()? {
            if cancel.is_cancelled() {
                return Err(Error::new("Query cancelled"));
            }
            let client = guard
                .as_ref()
                .ok_or_else(|| Error::new("Connection is closed"))?;
            let marker = format!("/* klyndb-query-{} */", uuid::Uuid::new_v4());
            let stop = CancellationToken::new();
            let mut query = Box::pin(stream(
                client,
                format!("{marker} {sql}"),
                &output,
                &stop,
                limit,
            ));
            let result = tokio::select! {biased;
                result=&mut query=>result,
                _=cancel.cancelled()=>{stop.cancel(); self.interrupt(&marker,&mut query).await},
                _=output.closed()=>{stop.cancel(); self.interrupt(&marker,&mut query).await},
                _=stop.cancelled()=>self.interrupt(&marker,&mut query).await,
            };
            drop(query);
            if client.is_closed()
                || result
                    .as_ref()
                    .is_err_and(|e| e.message.contains("Interruption could not be confirmed"))
            {
                guard.take();
            }
            let (truncated, buffer) = result?;
            if cancel.is_cancelled() {
                return Err(Error::new("Query cancelled"));
            }
            if !buffer.is_empty() {
                send(&output, Batch::Rows(buffer), &cancel).await?;
            }
            send(
                &output,
                Batch::Complete {
                    affected: 0,
                    truncated,
                },
                &cancel,
            )
            .await?;
        }
        Ok(())
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        let guard = self.connection.lock().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        Ok(rows(client,"SELECT database,name,engine FROM system.tables WHERE database=currentDatabase() AND is_temporary=0 ORDER BY name".into()).await?.into_iter().map(|r| Table {schema:r[0].text(),name:r[1].text(),kind:if r[2].text().contains("View") {"view"} else {"table"}.into()}).collect())
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let guard = self.connection.lock().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let predicate = format!(
            "database={} AND table={}",
            literal(&table.schema),
            literal(&table.name)
        );
        let columns=rows(client,format!("SELECT name,type,default_kind,default_expression FROM system.columns WHERE {predicate} ORDER BY position")).await?.into_iter().map(|r|Column {name:r[0].text(),data_type:r[1].text(),nullable:r[1].text().starts_with("Nullable("),primary_key:false,default:if r[3].text().is_empty() {None} else {Some(r[3].text())},generated:["MATERIALIZED","ALIAS"].contains(&r[2].text().as_str())}).collect();
        let ddl = rows(
            client,
            format!(
                "SHOW CREATE TABLE {}.{}",
                self.quote_identifier(&table.schema),
                self.quote_identifier(&table.name)
            ),
        )
        .await?
        .first()
        .and_then(|r| r.first())
        .map(Cell::text);
        // MergeTree primary/sorting keys are not relational uniqueness constraints.
        let indexes=rows(client,format!("SELECT name,type,expr,granularity FROM system.data_skipping_indices WHERE {predicate} ORDER BY name")).await?.into_iter().map(|r|serde_json::json!({"name":r[0].text(),"type":r[1].text(),"expression":r[2].text(),"granularity":r[3].text(),"unique":false})).collect();
        Ok(TableInfo {
            editable: false,
            columns,
            ddl,
            indexes,
            foreign_keys: vec![],
            constraints: None,
            triggers: vec![],
        })
    }
    async fn transaction_state(&self) -> Result<TransactionState> {
        if self
            .connection
            .lock()
            .await
            .as_ref()
            .is_none_or(Client::is_closed)
        {
            return Err(Error::new("Connection is closed"));
        }
        Ok(TransactionState::Idle)
    }
    async fn apply_changes(&self, _: Table, _: Vec<Change>) -> Result<MutationResult> {
        Err(Error::new(
            "ClickHouse grid editing is unavailable; use SQL with explicit mutation semantics",
        ))
    }
    async fn disconnect(&self) -> Result<()> {
        self.connection.lock().await.take();
        Ok(())
    }
}
impl ClickHouse {
    async fn interrupt<F>(
        &self,
        marker: &str,
        query: &mut std::pin::Pin<Box<F>>,
    ) -> Result<(bool, Vec<Row>)>
    where
        F: std::future::Future<Output = Result<(bool, Vec<Row>)>> + Send,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let kill = self.kill(marker);
            tokio::pin!(kill);
            tokio::select! {biased;
                result=query.as_mut()=>return result,
                _=tokio::time::sleep_until(deadline)=>break,
                result=&mut kill=>if result.is_err(){break;},
            }
            tokio::select! {biased;
                result=query.as_mut()=>return result,
                _=tokio::time::sleep_until(deadline)=>break,
                _=tokio::time::sleep(Duration::from_millis(50))=>{},
            }
        }
        Err(Error::new(
            "Interruption could not be confirmed; connection closed. Disconnect and reconnect. Verify writes before retrying.",
        ))
    }
}
