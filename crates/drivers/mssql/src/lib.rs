use async_trait::async_trait;
mod edit;
mod plan;
mod routines;
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
fn object_id(table: &Table) -> String {
    format!(
        "OBJECT_ID({})",
        literal(&format!(
            "{}.{}",
            identifier(&table.schema),
            identifier(&table.name)
        ))
    )
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
    drain_truncated: bool,
) -> Result<bool> {
    // Preserve one original T-SQL batch: DECLARE variables do not survive separate requests.
    let mut stream = client.simple_query(sql).await.map_err(err)?;
    let mut found = false;
    let mut count = 0;
    let mut buffer = vec![];
    let mut bytes = 0;
    let mut truncated = false;
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
                            truncated,
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
                truncated = false;
            }
            QueryItem::Row(native) => {
                if !found {
                    return Err(Error::new("SQL Server omitted result metadata"));
                }
                if count == limit {
                    if drain_truncated {
                        truncated = true;
                        continue;
                    }
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
    Ok(truncated)
}
async fn execute_batch(
    client: &mut NativeClient,
    sql: &str,
    output: &mpsc::Sender<Batch>,
    cancel: &CancellationToken,
    limit: usize,
    discard: bool,
) -> (Result<bool>, bool) {
    let result = tokio::select! {biased;
        result=stream(client,sql,output,limit,discard)=>result,
        _=cancel.cancelled()=>Err(Error::new("Query cancelled")),
        _=output.closed()=>Err(Error::new("Result consumer closed")),
    };
    if (result.is_err() || !discard && matches!(result, Ok(true)))
        && !matches!(
            tokio::time::timeout(Duration::from_secs(3), client.cancel_query()).await,
            Ok(Ok(()))
        )
    {
        let cause = result
            .as_ref()
            .err()
            .map_or("Row limit reached", |e| e.message.as_str());
        return (
            Err(Error::new(format!(
                "{cause}. Interruption could not be confirmed; connection closed. Reconnect and verify writes before retrying."
            ))),
            false,
        );
    }
    let result = match result {
        Err(error) => Err(error),
        Ok(truncated) => tokio::select! {biased;
            result=send(output,Batch::Complete {affected:0,truncated:truncated && !discard})=>result.map(|()|truncated),
            _=cancel.cancelled()=>Err(Error::new("Query cancelled")),
        },
    };
    (result, true)
}
async fn catalog(client: &mut NativeClient, sql: String) -> Result<Vec<Row>> {
    catalog_rows(client.simple_query(sql).await.map_err(err)?).await
}
async fn catalog_rows(mut stream: tiberius::QueryStream<'_>) -> Result<Vec<Row>> {
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
        // Klyndb owns deadlines and drains Attention before reuse. The SDK timer
        // poisons the protocol at 30 seconds before our edit/query deadline fires.
        config.command_timeout(None);
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
        // If a caller drops this future (for example the inspector deadline),
        // the owned client closes instead of leaving an unread response reusable.
        let mut client = guard
            .take()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let result = tokio::time::timeout(Duration::from_secs(30), catalog(&mut client, sql))
            .await
            .unwrap_or_else(|_| Err(Error::new("SQL Server metadata timed out after 30 seconds")));
        if result.is_ok() {
            *guard = Some(client);
        }
        result.map_err(|e| {
            Error::new(format!(
                "{} Metadata connection closed; reconnect.",
                e.message
            ))
        })
    }
    async fn write(
        &self,
        table: Table,
        source: edit::Source<'_>,
        cancel: CancellationToken,
    ) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This SQL Server connection is read-only"));
        }
        let mut guard = tokio::select! {biased; _=cancel.cancelled()=>return Err(Error::new("Import cancelled")), guard=self.connection.lock()=>guard};
        // Own the client while writing: a dropped caller cannot leave a half-read response reusable.
        let mut client = guard
            .take()
            .ok_or_else(|| Error::new("Connection is closed; reconnect."))?;
        let mut poison = false;
        let result = edit::apply(&mut client, &table, source, &cancel, &mut poison).await;
        if poison {
            let cause = result
                .err()
                .map(|e| e.message)
                .unwrap_or_else(|| "Session cleanup failed".into());
            return Err(Error::new(format!(
                "{cause} Table writes could not be safely completed or rolled back. Connection closed; reconnect and verify writes and the original transaction before retrying."
            )));
        }
        *guard = Some(client);
        result
    }
    async fn foreign_key_rows(&self, table: &Table) -> Result<Vec<Row>> {
        self.metadata(format!("SELECT TOP (50001) fk.name,pc.name,SCHEMA_NAME(t.schema_id),t.name,rc.name FROM sys.foreign_keys fk JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id=fk.object_id JOIN sys.columns pc ON pc.object_id=fkc.parent_object_id AND pc.column_id=fkc.parent_column_id JOIN sys.tables t ON t.object_id=fkc.referenced_object_id JOIN sys.columns rc ON rc.object_id=fkc.referenced_object_id AND rc.column_id=fkc.referenced_column_id WHERE fk.parent_object_id={} ORDER BY fk.name,fkc.constraint_column_id",object_id(table))).await
    }
}
#[async_trait]
impl Session for SqlServer {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            affected_rows: false,
            table_browse: true,
            routines: true,
            diagrams: true,
            transactions: true,
            schemas: true,
            explain: true,
            explain_analyze: !self.read_only,
            edit_rows: !self.read_only,
            import_rows: !self.read_only,
            import_sql: !self.read_only,
            cancel: true,
            tls: true,
        }
    }
    fn quote_identifier(&self, name: &str) -> String {
        identifier(name)
    }
    fn explain_sql(&self, sql: &str, analyze: bool) -> Result<(String, PlanFormat)> {
        klyndb_query::sql_server_plan_target(sql)?;
        if analyze && self.read_only {
            return Err(Error::new(
                "ANALYZE executes the statement and is disabled on read-only connections",
            ));
        }
        Ok((sql.into(), PlanFormat::SqlServerTabular))
    }
    async fn execute_plan(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
        analyze: bool,
    ) -> Result<()> {
        self.explain_sql(&sql, analyze)?;
        let mut guard = tokio::select! {biased; _=cancel.cancelled()=>return Err(Error::new("Query cancelled")), guard=self.connection.lock()=>guard};
        let mut client = guard
            .take()
            .ok_or_else(|| Error::new("Connection is closed; reconnect."))?;
        let (result, reusable) =
            plan::execute(&mut client, &sql, &output, &cancel, limit, analyze).await;
        if reusable {
            *guard = Some(client);
        }
        result
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
        let mut client = guard
            .take()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let mut reusable = true;
        let result = async {
            let mut reader = klyndb_query::MssqlReader::new(sql.as_bytes());
            while let Some(batch) = reader.next_batch(|| cancel.is_cancelled())? {
                let (result, synchronized) =
                    execute_batch(&mut client, &batch, &output, &cancel, limit, false).await;
                reusable = synchronized;
                if result? {
                    break;
                } // A row cap stops later GO batches too.
            }
            Ok(())
        }
        .await;
        if reusable {
            *guard = Some(client);
        }
        result
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
        let mut guard = tokio::select! {biased;
            _=cancel.cancelled()=>return Err(Error::new("SQL import cancelled while waiting for the session")),
            guard=self.connection.lock()=>guard,
        };
        let mut client = guard
            .take()
            .ok_or_else(|| Error::new("Connection is closed"))?;
        let mut reusable = true;
        let result = async {
            while let Some(sql) = next_script_statement(&mut input,&cancel).await? {
                klyndb_query::analyze_script(&sql,"mssql")?;
                // Constant session-property reads do not open an implicit table transaction.
                let mode = tokio::select! {biased;
                    _=cancel.cancelled()=>None,
                    result=tokio::time::timeout(Duration::from_secs(3),catalog(&mut client,"SELECT CAST(SESSIONPROPERTY('QUOTED_IDENTIFIER') AS int)".into()))=>result.ok(),
                };
                let Some(mode) = mode else {
                    reusable = false;
                    return Err(Error::new("SQL import lexical check interrupted; connection closed. Reconnect and verify earlier writes."));
                };
                let mode = match mode { Ok(mode)=>mode, Err(error)=> {
                    reusable=false;
                    return Err(Error::new(format!("{} SQL import lexical check failed; connection closed. Reconnect and verify earlier writes.",error.message)));
                } };
                if mode.len()!=1 || mode[0].len()!=1 || mode[0][0].text()!="1" {
                    return Err(Error::new("SQL Server imports require QUOTED_IDENTIFIER ON and SHOWPLAN disabled"));
                }
                // Zero retained rows: discard SELECT data while fully draining every native batch.
                let (result,synchronized)=execute_batch(&mut client,&sql,&output,&cancel,0,true).await;
                reusable=synchronized;
                result?;
                completed.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
            }
            Ok(())
        }.await;
        if reusable {
            *guard = Some(client);
        }
        result
    }
    async fn routines(&self, search: &str, offset: u32) -> Result<RoutinePage> {
        routines::list(self, search, offset).await
    }
    async fn routine_definition(&self, id: &str) -> Result<String> {
        routines::definition(self, id).await
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        Ok(self.metadata("SELECT TOP (50001) SCHEMA_NAME(schema_id),name,type FROM sys.objects WHERE type IN ('U','V') AND is_ms_shipped=0 ORDER BY 1,2".into()).await?.into_iter().map(|r|Table {schema:r[0].text(),name:r[1].text(),kind:if r[2].text().trim()=="V" {"view"} else {"table"}.into()}).collect())
    }
    async fn inspect(&self, table: &Table) -> Result<TableInfo> {
        let object = object_id(table);
        let columns = edit::decode_columns(self.metadata(edit::column_sql(table)).await?);
        let editable = !self.read_only
            && edit::decode_editable(self.metadata(edit::editable_sql(table)).await?);
        let ddl = self
            .metadata(format!("SELECT OBJECT_DEFINITION({object})"))
            .await?
            .first()
            .and_then(|r| r.first())
            .filter(|c| !matches!(c, Cell::Null))
            .map(Cell::text);
        let indexes=self.metadata(format!("SELECT TOP (50001) name,type_desc,is_unique,is_primary_key,filter_definition FROM sys.indexes WHERE object_id={object} AND index_id>0 ORDER BY index_id")).await?.into_iter().map(|r|serde_json::json!({"name":r[0].text(),"type":r[1].text(),"unique":r[2].text()=="true","primary":r[3].text()=="true","filter":r[4]})).collect();
        let foreign_keys = self.foreign_key_rows(table).await?.into_iter().map(|r|serde_json::json!({"name":r[0].text(),"column":r[1].text(),"schema":r[2].text(),"table":r[3].text(),"target":r[4].text()})).collect();
        let constraints = self.metadata(format!("SELECT TOP (50001) name,type_desc,OBJECT_DEFINITION(object_id) FROM sys.objects WHERE parent_object_id={object} AND type IN ('PK','UQ','F','C','D') ORDER BY name")).await?.into_iter().map(|r|Constraint {
            name:r[0].text(),kind:r[1].text(),definition:if matches!(r[2],Cell::Null) {None} else {Some(r[2].text())}
        }).collect();
        let triggers = self.metadata(format!("SELECT TOP (50001) name,OBJECT_DEFINITION(object_id),CASE WHEN is_disabled=1 THEN N'disabled' ELSE N'enabled' END,CASE WHEN is_instead_of_trigger=1 THEN N'INSTEAD OF' ELSE N'AFTER' END FROM sys.triggers WHERE parent_id={object} AND parent_class=1 AND is_ms_shipped=0 ORDER BY name")).await?.into_iter().map(|r|Trigger {
            name:r[0].text(),definition:if matches!(r[1],Cell::Null) {"Definition unavailable to this connection".into()} else {r[1].text()},state:Some(format!("{} · {}",r[2].text(),r[3].text()))
        }).collect();
        Ok(TableInfo {
            editable,
            columns,
            ddl,
            indexes,
            foreign_keys,
            constraints: Some(constraints),
            triggers,
        })
    }
    async fn relationships(&self, table: &Table) -> Result<Vec<ForeignKey>> {
        Ok(group_foreign_keys(
            self.foreign_key_rows(table).await?.into_iter().map(|r| {
                (
                    r[0].text(),
                    r[1].text(),
                    r[2].text(),
                    r[3].text(),
                    Some(r[4].text()),
                )
            }),
        ))
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
    async fn apply_changes(&self, table: Table, changes: Vec<Change>) -> Result<MutationResult> {
        validate_change_batch(&changes)?;
        self.write(
            table,
            edit::Source::Edits(Some(&changes)),
            CancellationToken::new(),
        )
        .await
    }
    async fn insert_stream(
        &self,
        table: Table,
        input: mpsc::Receiver<Result<InsertBatch>>,
        cancel: CancellationToken,
    ) -> Result<MutationResult> {
        self.write(table, edit::Source::Import(input), cancel).await
    }
    async fn disconnect(&self) -> Result<()> {
        self.connection.lock().await.take();
        Ok(())
    }
}
