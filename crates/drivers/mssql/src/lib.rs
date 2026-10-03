use async_trait::async_trait;
use futures_util::TryStreamExt;
use klyndb_driver_api::*;
use std::{net::SocketAddr, time::Duration};
use tiberius::{AuthMethod, Client, ColumnData, ColumnType, Config, EncryptionLevel, QueryItem};
use tokio::{
    net::TcpStream,
    sync::{Mutex, mpsc},
};
use tokio_util::{
    compat::{Compat, TokioAsyncWriteCompatExt},
    sync::CancellationToken,
};

type NativeClient = Client<Compat<TcpStream>>;
pub struct SqlServer {
    connection: Mutex<Option<NativeClient>>,
    read_only: bool,
}
fn err(e: tiberius::error::Error) -> Error {
    // Server text may echo submitted SQL and credentials; retain only the native code.
    match e.code() {
        Some(code) => Error::new(format!(
            "SQL Server error {code}. Check the SQL and database permissions."
        )),
        None => Error::new(
            "SQL Server transport or decoding failed. Check hostname, CA and native column types; unsupported types can be cast in the SELECT.",
        ),
    }
}
fn identifier(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}
fn literal(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}
fn temporal<'a, T: tiberius::FromSql<'a> + std::fmt::Display>(
    v: &'a ColumnData<'static>,
) -> Result<Cell> {
    Ok(T::from_sql(v)
        .map_err(err)?
        .map_or(Cell::Null, |v| Cell::Text(v.to_string())))
}
fn cell(ty: ColumnType, v: &ColumnData<'static>) -> Result<Cell> {
    if matches!(ty, ColumnType::Money | ColumnType::Money4) {
        return Err(Error::new(
            "Cast SQL Server money/smallmoney to DECIMAL in the SELECT to retain exact values.",
        ));
    }
    macro_rules! number {
        ($v:expr) => {
            $v.map_or(Cell::Null, |v| Cell::Number(v.to_string()))
        };
    }
    Ok(match v {
        ColumnData::U8(v) => number!(v),
        ColumnData::I16(v) => number!(v),
        ColumnData::I32(v) => number!(v),
        ColumnData::I64(v) => number!(v),
        ColumnData::F32(v) => number!(v),
        ColumnData::F64(v) => number!(v),
        ColumnData::Bit(v) => v.map_or(Cell::Null, Cell::Boolean),
        ColumnData::String(v) => v.as_ref().map_or(Cell::Null, |v| Cell::Text(v.to_string())),
        ColumnData::Guid(v) => v.map_or(Cell::Null, |v| Cell::Text(v.to_string())),
        ColumnData::Binary(v) => v
            .as_ref()
            .map_or(Cell::Null, |v| Cell::Binary(hex::encode(v))),
        ColumnData::Numeric(v) => v.map_or(Cell::Null, |v| {
            Cell::Number(if v.scale() == 0 {
                v.value().to_string()
            } else {
                v.to_string()
            })
        }),
        ColumnData::Xml(v) => v.as_ref().map_or(Cell::Null, |v| Cell::Text(v.to_string())),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            return temporal::<chrono::NaiveDateTime>(v);
        }
        ColumnData::Date(_) => return temporal::<chrono::NaiveDate>(v),
        ColumnData::Time(_) => return temporal::<chrono::NaiveTime>(v),
        ColumnData::DateTimeOffset(_) => {
            return temporal::<chrono::DateTime<chrono::FixedOffset>>(v);
        }
    })
}
fn row(row: &tiberius::Row) -> Result<Row> {
    let row = row
        .cells()
        .map(|(c, v)| cell(c.column_type(), v))
        .collect::<Result<Row>>()?;
    if row.iter().map(Cell::byte_len).sum::<usize>() > 8 * 1024 * 1024 {
        return Err(Error::new(
            "A result row exceeds 8 MiB. Select smaller values.",
        ));
    }
    Ok(row)
}
async fn send(out: &mpsc::Sender<Batch>, batch: Batch) -> Result<()> {
    out.send(batch)
        .await
        .map_err(|_| Error::new("Result consumer closed"))
}
async fn stream(
    client: &mut NativeClient,
    sql: &str,
    out: &mpsc::Sender<Batch>,
    limit: usize,
) -> Result<bool> {
    // Preserve one original T-SQL batch: DECLARE variables do not survive separate requests.
    let mut stream = client.simple_query(sql).await.map_err(err)?;
    let mut found = false;
    let mut count = 0;
    let mut buffer = vec![];
    let mut bytes = 0;
    while let Some(item) = stream.try_next().await.map_err(err)? {
        match item {
            QueryItem::Metadata(meta) => {
                if found {
                    if !buffer.is_empty() {
                        send(out, Batch::Rows(std::mem::take(&mut buffer))).await?;
                    }
                    send(
                        out,
                        Batch::Complete {
                            affected: 0,
                            truncated: false,
                        },
                    )
                    .await?;
                }
                send(
                    out,
                    Batch::Columns(
                        meta.columns()
                            .iter()
                            .map(|c| c.name().to_string())
                            .collect(),
                    ),
                )
                .await?;
                found = true;
                count = 0;
                bytes = 0;
            }
            QueryItem::Row(native) => {
                if !found {
                    return Err(Error::new("SQL Server omitted result metadata"));
                }
                if count == limit {
                    if !buffer.is_empty() {
                        send(out, Batch::Rows(buffer)).await?;
                    }
                    return Ok(true);
                }
                let row = row(&native)?;
                bytes += row.iter().map(Cell::byte_len).sum::<usize>();
                buffer.push(row);
                count += 1;
                if buffer.len() == 256 || bytes >= 256 * 1024 {
                    send(out, Batch::Rows(std::mem::take(&mut buffer))).await?;
                    bytes = 0;
                }
            }
        }
    }
    if !found {
        send(out, Batch::Columns(vec![])).await?;
    }
    if !buffer.is_empty() {
        send(out, Batch::Rows(buffer)).await?;
    }
    Ok(false)
}
async fn catalog(client: &mut NativeClient, sql: String) -> Result<Vec<Row>> {
    let mut stream = client.simple_query(sql).await.map_err(err)?;
    let mut rows = vec![];
    let mut bytes = 0;
    while let Some(item) = stream.try_next().await.map_err(err)? {
        if let QueryItem::Row(native) = item {
            let row = row(&native)?;
            bytes += row.iter().map(Cell::byte_len).sum::<usize>();
            if rows.len() == 50_000 || bytes > 16 * 1024 * 1024 {
                return Err(Error::new(
                    "Catalog exceeds 50,000 rows or 16 MiB. Use a narrower database.",
                ));
            }
            rows.push(row);
        }
    }
    Ok(rows)
}
impl SqlServer {
    pub async fn connect(address: &str, password: Option<&str>, read_only: bool) -> Result<Self> {
        Self::connect_via(address, password, read_only, None).await
    }
    pub async fn connect_via(
        address: &str,
        password: Option<&str>,
        read_only: bool,
        endpoint: Option<SocketAddr>,
    ) -> Result<Self> {
        let url = url::Url::parse(address).map_err(|_| Error::new("Invalid SQL Server URL"))?;
        if url.scheme() != "mssql" || url.host_str().is_none() || url.fragment().is_some() {
            return Err(Error::new("Expected mssql://user@host:1433/database"));
        }
        let mut seen = std::collections::HashSet::new();
        if url.query_pairs().any(|(k, _)| {
            !matches!(k.as_ref(), "tls" | "sslrootcert" | "connect_timeout")
                || !seen.insert(k.into_owned())
        }) {
            return Err(Error::new("Unsupported or repeated SQL Server URL option"));
        }
        let option = |key| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let timeout = connect_timeout(
            url.query_pairs()
                .filter(|(k, _)| k == "connect_timeout")
                .map(|(_, v)| v),
        )?;
        let mode = option("tls").unwrap_or_else(|| "required".into());
        if !matches!(mode.as_str(), "required" | "disabled") {
            return Err(Error::new("SQL Server tls must be required or disabled"));
        }
        if endpoint.is_some_and(|e| !e.ip().is_loopback()) {
            return Err(Error::new("Invalid tunnel endpoint"));
        }
        let decode = |s: &str| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8()
                .map(|s| s.into_owned())
                .map_err(|_| Error::new("SQL Server URL values must use UTF-8"))
        };
        let user = decode(url.username())?;
        let password = match password {
            Some(p) => p.to_string(),
            None => decode(url.password().unwrap_or_default())?,
        };
        let database = decode(url.path().trim_start_matches('/'))?;
        if [&user, &password, &database]
            .iter()
            .any(|s| s.len() > 16384 || s.contains('\0'))
        {
            return Err(Error::new(
                "Invalid SQL Server credentials or database name",
            ));
        }
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let mut config = Config::new();
        config.host(host);
        config.port(url.port().unwrap_or(1433));
        config.authentication(AuthMethod::sql_server(user, password));
        config.application_name("Klyndb");
        if !database.is_empty() {
            config.database(database);
        }
        config.encryption(if mode == "required" {
            EncryptionLevel::Required
        } else {
            EncryptionLevel::NotSupported
        });
        if let Some(path) = option("sslrootcert") {
            if mode != "required" {
                return Err(Error::new("CA certificates require verified TLS"));
            }
            for cert in tls::load_ca_certificates(&path).await? {
                config.trust_cert_ca_bundle(cert);
            }
        }
        let client = tokio::time::timeout(timeout, async {
            let tcp = match endpoint {
                Some(e) => TcpStream::connect(e).await,
                None => TcpStream::connect(config.get_addr()).await,
            }
            .map_err(|_| Error::new("Could not reach SQL Server TCP endpoint"))?;
            tcp.set_nodelay(true)
                .map_err(|_| Error::new("Could not configure SQL Server transport"))?;
            let mut client = Client::connect(config, tcp.compat_write())
                .await
                .map_err(err)?;
            catalog(&mut client, "SELECT 1".into()).await?;
            Ok::<_, Error>(client)
        })
        .await
        .map_err(|_| {
            Error::new(format!(
                "SQL Server connection timed out after {} seconds",
                timeout.as_secs()
            ))
        })??;
        Ok(Self {
            connection: Mutex::new(Some(client)),
            read_only,
        })
    }
    async fn metadata(&self, sql: String) -> Result<Vec<Row>> {
        let mut guard = self.connection.lock().await;
        let client = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let result = catalog(client, sql).await;
        if result.is_err() {
            guard.take();
        }
        result.map_err(|e| {
            Error::new(format!(
                "{} Metadata connection closed; reconnect.",
                e.message
            ))
        })
    }
}
#[async_trait]
impl Session for SqlServer {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            affected_rows: false,
            table_browse: true,
            diagrams: false,
            transactions: true,
            schemas: true,
            explain: false,
            explain_analyze: false,
            edit_rows: false,
            import_rows: false,
            import_sql: false,
            cancel: true,
            tls: true,
        }
    }
    fn quote_identifier(&self, name: &str) -> String {
        identifier(name)
    }
    fn quote_filter_value(&self, value: &str) -> String {
        literal(value)
    }
    fn contains_filter_sql(&self, column: &str, value: &str) -> String {
        format!("CHARINDEX({}, {column}) > 0", literal(value))
    }
    fn pagination_sql(&self, limit: usize, offset: u64, ordered: bool) -> String {
        format!(
            "{} OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY;",
            if ordered {
                ""
            } else {
                " ORDER BY (SELECT NULL)"
            }
        )
    }
    fn table_select_sql(&self, table: &Table, limit: usize) -> Result<String> {
        if !(1..=10_000_000).contains(&limit) {
            return Err(Error::new("Row limit must be 1–10,000,000"));
        }
        Ok(format!(
            "SELECT TOP ({limit}) * FROM {}.{};",
            identifier(&table.schema),
            identifier(&table.name)
        ))
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
        let analysis = klyndb_query::analyze(&sql, "mssql")?;
        if self.read_only && !analysis.read_only {
            return Err(Error::new("This SQL Server connection is read-only"));
        }
        let mut guard = tokio::select! {biased;_ = cancel.cancelled()=>return Err(Error::new("Query cancelled")),guard=self.connection.lock()=>guard};
        let client = guard
            .as_mut()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let result = {
            let request = stream(client, &sql, &output, limit);
            tokio::pin!(request);
            tokio::select! {biased; result=&mut request=>result,_=cancel.cancelled()=>Err(Error::new("Query cancelled")),_=output.closed()=>Err(Error::new("Result consumer closed"))}
        };
        if result.as_ref().is_err() || matches!(result, Ok(true)) {
            let attention =
                tokio::time::timeout(Duration::from_secs(3), client.cancel_query()).await;
            if !matches!(attention, Ok(Ok(()))) {
                guard.take();
                let cause = result
                    .as_ref()
                    .err()
                    .map_or("Row limit reached", |e| e.message.as_str());
                return Err(Error::new(format!(
                    "{cause}. Interruption could not be confirmed; connection closed. Reconnect and verify writes before retrying."
                )));
            }
        }
        let truncated = result?;
        // The response is fully consumed: cancellation here must not send late Attention.
        tokio::select! {
            biased;
            result = send(&output, Batch::Complete { affected: 0, truncated }) => result,
            _ = cancel.cancelled() => Err(Error::new("Query cancelled")),
            _ = output.closed() => Err(Error::new("Result consumer closed")),
        }
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        Ok(self.metadata("SELECT TOP (50001) SCHEMA_NAME(schema_id),name,type FROM sys.objects WHERE type IN ('U','V') AND is_ms_shipped=0 ORDER BY 1,2".into()).await?.into_iter().map(|r|Table {schema:r[0].text(),name:r[1].text(),kind:if r[2].text().trim()=="V" {"view"} else {"table"}.into()}).collect())
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let object = format!(
            "OBJECT_ID({})",
            literal(&format!(
                "{}.{}",
                identifier(&table.schema),
                identifier(&table.name)
            ))
        );
        let columns=self.metadata(format!("SELECT TOP (50001) c.name,t.name,CAST(c.is_nullable AS bit),CAST(CASE WHEN EXISTS(SELECT 1 FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id=i.object_id AND ic.index_id=i.index_id WHERE i.object_id=c.object_id AND i.is_primary_key=1 AND ic.column_id=c.column_id) THEN 1 ELSE 0 END AS bit),OBJECT_DEFINITION(c.default_object_id),CAST(CASE WHEN c.is_identity=1 OR c.is_computed=1 OR c.generated_always_type<>0 OR t.name='timestamp' THEN 1 ELSE 0 END AS bit) FROM sys.columns c JOIN sys.types t ON t.user_type_id=c.user_type_id WHERE c.object_id={object} ORDER BY c.column_id")).await?.into_iter().map(|r|Column {name:r[0].text(),data_type:r[1].text(),nullable:r[2].text()=="true",primary_key:r[3].text()=="true",default:if matches!(r[4],Cell::Null) {None} else {Some(r[4].text())},generated:r[5].text()=="true"}).collect();
        let ddl = self
            .metadata(format!("SELECT OBJECT_DEFINITION({object})"))
            .await?
            .first()
            .and_then(|r| r.first())
            .filter(|c| !matches!(c, Cell::Null))
            .map(Cell::text);
        let indexes=self.metadata(format!("SELECT TOP (50001) name,type_desc,is_unique,is_primary_key,filter_definition FROM sys.indexes WHERE object_id={object} AND index_id>0 ORDER BY index_id")).await?.into_iter().map(|r|serde_json::json!({"name":r[0].text(),"type":r[1].text(),"unique":r[2].text()=="true","primary":r[3].text()=="true","filter":r[4]})).collect();
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
        let rows = self.metadata("SELECT XACT_STATE()".into()).await?;
        match rows
            .first()
            .and_then(|r| r.first())
            .map(Cell::text)
            .as_deref()
        {
            Some("0") => Ok(TransactionState::Idle),
            Some("1") => Ok(TransactionState::Active),
            Some("-1") => Ok(TransactionState::Failed),
            _ => Err(Error::new("SQL Server transaction state is unavailable")),
        }
    }
    async fn apply_changes(&self, _: Table, _: Vec<Change>) -> Result<MutationResult> {
        Err(Error::new(
            "SQL Server grid editing is unavailable; use explicit SQL transactions",
        ))
    }
    async fn disconnect(&self) -> Result<()> {
        self.connection.lock().await.take();
        Ok(())
    }
}
