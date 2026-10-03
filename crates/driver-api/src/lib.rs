pub mod tls;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub message: String,
    /// Zero-based UTF-16 offset into the submitted SQL, when its source is known.
    pub sql_offset: Option<usize>,
}
impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            sql_offset: None,
        }
    }
}
pub fn sql_utf16_offset(sql: &str, byte_offset: usize) -> Option<usize> {
    sql.get(..byte_offset)
        .map(|prefix| prefix.encode_utf16().count())
}
pub type Result<T> = std::result::Result<T, Error>;

/// One shared bound for connection setup, including stalled protocol handshakes.
pub fn connect_timeout(
    values: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<std::time::Duration> {
    let mut values = values.into_iter();
    let seconds = match values.next() {
        Some(value) => {
            let value = value.as_ref();
            if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
                return Err(Error::new(
                    "Connection timeout must be an integer from 1 to 300 seconds",
                ));
            }
            value.parse::<u64>().map_err(|_| {
                Error::new("Connection timeout must be an integer from 1 to 300 seconds")
            })?
        }
        None => 10,
    };
    if !(1..=300).contains(&seconds) || values.next().is_some() {
        return Err(Error::new(
            "Choose one connection timeout from 1 to 300 seconds",
        ));
    }
    Ok(std::time::Duration::from_secs(seconds))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Null,
    Text(String),
    Number(String),
    Boolean(bool),
    Binary(String),
    Json(serde_json::Value),
}
impl Cell {
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Null => 0,
            Self::Text(v) | Self::Number(v) | Self::Binary(v) => v.len(),
            Self::Boolean(_) => 1,
            Self::Json(v) => v.to_string().len(),
        }
    }
    pub fn text(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Text(v) | Self::Number(v) | Self::Binary(v) => v.clone(),
            Self::Boolean(v) => v.to_string(),
            Self::Json(v) => v.to_string(),
        }
    }
}
/// Compare exact decimal/exponent values without converting through floating point.
pub fn normalized_number(s: &str) -> Result<(String, i64)> {
    let invalid = || Error::new("Enter a valid numeric literal");
    let s = serde_json::from_str::<serde_json::Number>(s)
        .map_err(|_| invalid())?
        .to_string();
    let (mantissa, exponent) = s.split_once(['e', 'E']).unwrap_or((&s, "0"));
    let exponent = exponent.parse::<i64>().map_err(|_| invalid())?;
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok(("0".into(), 0));
    }
    let significant = digits.trim_end_matches('0');
    let scale = (fraction.len() as i64)
        .checked_sub(exponent)
        .and_then(|n| n.checked_sub((digits.len() - significant.len()) as i64))
        .ok_or_else(invalid)?;
    Ok((
        format!("{}{significant}", if negative { "-" } else { "" }),
        scale,
    ))
}

pub type Row = Vec<Cell>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub key_value: bool,
    pub affected_rows: bool,
    pub table_browse: bool,
    pub routines: bool,
    pub diagrams: bool,
    pub transactions: bool,
    pub schemas: bool,
    pub explain: bool,
    pub explain_analyze: bool,
    pub edit_rows: bool,
    pub import_rows: bool,
    pub import_sql: bool,
    pub cancel: bool,
    pub tls: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum KeyValue {
    Cell(Cell),
    Array(Vec<KeyValue>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyEntry {
    pub key: Cell,
    pub data_type: String,
    pub ttl_ms: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyScan {
    pub cursor: String,
    pub keys: Vec<KeyEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyInspection {
    pub entry: KeyEntry,
    pub length: String,
    pub position: String,
    pub next: Option<String>,
    pub value: KeyValue,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyCommandInfo {
    pub command: String,
    pub writes: bool,
    pub arguments: usize,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanFormat {
    Sqlite,
    PostgresJson,
    MysqlJson,
    MysqlTree,
    MariaJson,
    DuckDbJson,
    ClickHouseJson,
    SqlServerTabular,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Routine {
    /// Opaque identifier scoped to this native connection.
    pub id: String,
    pub schema: String,
    pub name: String,
    pub kind: String,
    pub arguments: String,
    pub returns: Option<String>,
    pub language: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutinePage {
    pub routines: Vec<Routine>,
    pub has_more: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub default: Option<String>,
    pub generated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub target_schema: String,
    pub target_table: String,
    pub target_columns: Vec<Option<String>>,
}
/// Metadata rows must be ordered by native key-column ordinal.
pub fn group_foreign_keys(
    rows: impl IntoIterator<Item = (String, String, String, String, Option<String>)>,
) -> Vec<ForeignKey> {
    let mut keys = std::collections::BTreeMap::new();
    for (name, column, schema, table, target) in rows {
        let key = keys
            .entry((name.clone(), schema.clone(), table.clone()))
            .or_insert_with(|| ForeignKey {
                name,
                columns: vec![],
                target_schema: schema,
                target_table: table,
                target_columns: vec![],
            });
        key.columns.push(column);
        key.target_columns.push(target);
    }
    keys.into_values().collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Constraint {
    pub name: String,
    pub kind: String,
    pub definition: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    pub name: String,
    pub definition: String,
    pub state: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableInfo {
    pub editable: bool,
    pub columns: Vec<Column>,
    pub ddl: Option<String>,
    pub indexes: Vec<serde_json::Value>,
    pub foreign_keys: Vec<serde_json::Value>,
    /// None means definitions are available in DDL rather than a native constraint catalog.
    pub constraints: Option<Vec<Constraint>>,
    pub triggers: Vec<Trigger>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Contains,
    Like,
    IsNull,
    IsNotNull,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableFilter {
    pub column: String,
    pub op: FilterOp,
    pub value: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableSort {
    pub column: String,
    pub descending: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableQuery {
    pub filters: Vec<TableFilter>,
    pub sort: Vec<TableSort>,
    pub limit: usize,
    pub offset: u64,
}

/// Old values are compared as well as the primary key: a stale edit must affect no rows.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Insert {
        values: std::collections::BTreeMap<String, Cell>,
    },
    Update {
        old: Row,
        values: std::collections::BTreeMap<String, Cell>,
    },
    Delete {
        old: Row,
    },
}
impl Change {
    pub fn values(&self) -> Option<&std::collections::BTreeMap<String, Cell>> {
        match self {
            Self::Insert { values } | Self::Update { values, .. } => Some(values),
            Self::Delete { .. } => None,
        }
    }
    pub fn old(&self) -> Option<&Row> {
        match self {
            Self::Update { old, .. } | Self::Delete { old } => Some(old),
            Self::Insert { .. } => None,
        }
    }
    pub fn validate(&self, columns: &[Column]) -> Result<()> {
        if let Some(values) = self.values() {
            if matches!(self, Self::Update { .. }) && values.is_empty() {
                return Err(Error::new("No changed values"));
            }
            for name in values.keys() {
                let column = columns
                    .iter()
                    .find(|c| &c.name == name)
                    .ok_or_else(|| Error::new("Column no longer exists. Refresh the table."))?;
                if column.generated {
                    return Err(Error::new(format!(
                        "{} is generated by the database",
                        column.name
                    )));
                }
            }
        }
        if let Some(old) = self.old() {
            if old.len() != columns.len() {
                return Err(Error::new("Table columns changed. Refresh before editing."));
            }
            if !columns.iter().any(|c| c.primary_key)
                || columns
                    .iter()
                    .zip(old)
                    .any(|(c, v)| c.primary_key && matches!(v, Cell::Null))
            {
                return Err(Error::new("Update/delete requires a non-NULL primary key"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Idle,
    Active,
    Failed,
}
#[derive(Clone, Debug, Serialize)]
pub struct MutationResult {
    pub affected: u64,
    pub pending_transaction: bool,
}
pub fn validate_change_batch(changes: &[Change]) -> Result<()> {
    if changes.is_empty() || changes.len() > 1000 {
        return Err(Error::new("Apply 1–1000 changes per batch"));
    }
    let bytes = changes
        .iter()
        .map(|c| {
            c.old()
                .into_iter()
                .flatten()
                .chain(c.values().into_iter().flat_map(|v| v.values()))
                .map(Cell::byte_len)
                .sum::<usize>()
                + c.values()
                    .map_or(0, |values| values.keys().map(String::len).sum::<usize>())
        })
        .sum::<usize>();
    if bytes > 8 * 1024 * 1024 {
        return Err(Error::new("An editing batch is limited to 8 MiB"));
    }
    Ok(())
}

pub enum Batch {
    Columns(Vec<String>),
    Rows(Vec<Row>),
    Complete { affected: u64, truncated: bool },
}

/// A producer must explicitly finish; a dropped/failed producer must never commit.
pub enum InsertBatch {
    Rows(Vec<Change>),
    Complete,
}
pub enum ScriptBatch {
    Statement(String),
    Complete,
}
pub async fn next_script_statement(
    input: &mut mpsc::Receiver<Result<ScriptBatch>>,
    cancel: &CancellationToken,
) -> Result<Option<String>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Error::new("SQL import cancelled")),
        batch = input.recv() => match batch.ok_or_else(|| Error::new("SQL reader stopped before completing the file"))?? {
            ScriptBatch::Statement(sql) => Ok(Some(sql)),
            ScriptBatch::Complete => Ok(None),
        },
    }
}
pub async fn next_insert_batch(
    input: &mut mpsc::Receiver<Result<InsertBatch>>,
    cancel: &CancellationToken,
) -> Result<InsertBatch> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Error::new("Import cancelled")),
        batch = input.recv() => batch.ok_or_else(|| Error::new("Import reader stopped before completing the file"))?,
    }
}
pub fn validate_insert_batch(changes: &[Change]) -> Result<()> {
    validate_change_batch(changes)?;
    if changes
        .iter()
        .any(|change| !matches!(change, Change::Insert { .. }))
    {
        return Err(Error::new("Imports can only append rows"));
    }
    Ok(())
}

#[async_trait]
pub trait Session: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    /// The core validates a single original statement before calling this method.
    fn explain_sql(&self, _sql: &str, _analyze: bool) -> Result<(String, PlanFormat)> {
        Err(Error::new("This driver does not support execution plans"))
    }
    fn quote_identifier(&self, name: &str) -> String {
        quote_identifier(name)
    }
    fn quote_filter_value(&self, value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
    fn contains_filter_sql(&self, column: &str, value: &str) -> String {
        let value = format!(
            "%{}%",
            value
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        format!(
            "{column} LIKE {} ESCAPE '!'",
            self.quote_filter_value(&value)
        )
    }
    fn pagination_sql(&self, limit: usize, offset: u64, _ordered: bool) -> String {
        format!(" LIMIT {limit} OFFSET {offset};")
    }
    fn table_query_sql(
        &self,
        table: &Table,
        columns: &[Column],
        query: &TableQuery,
    ) -> Result<String> {
        if !self.capabilities().table_browse {
            return Err(Error::new("This driver does not support table browsing"));
        }
        if !(1..=500).contains(&query.limit) || query.offset > 1_000_000_000 {
            return Err(Error::new(
                "Use 1–500 rows per table page and an offset of at most one billion",
            ));
        }
        if query.filters.len() > 20 || query.sort.len() > 8 {
            return Err(Error::new("Use at most 20 filters and 8 sort columns"));
        }
        let column = |name: &str| {
            if columns.iter().any(|c| c.name == name) {
                Ok(self.quote_identifier(name))
            } else {
                Err(Error::new("Column no longer exists. Reopen the table."))
            }
        };
        let mut predicates = vec![];
        for filter in &query.filters {
            let name = column(&filter.column)?;
            if filter.value.len() > 16 * 1024 || filter.value.contains('\0') {
                return Err(Error::new(
                    "Filter values must be at most 16 KiB and cannot contain NUL",
                ));
            }
            let operator = match filter.op {
                FilterOp::Equal => "=",
                FilterOp::NotEqual => "<>",
                FilterOp::Less => "<",
                FilterOp::LessEqual => "<=",
                FilterOp::Greater => ">",
                FilterOp::GreaterEqual => ">=",
                FilterOp::Contains | FilterOp::Like => "LIKE",
                FilterOp::IsNull => {
                    predicates.push(format!("{name} IS NULL"));
                    continue;
                }
                FilterOp::IsNotNull => {
                    predicates.push(format!("{name} IS NOT NULL"));
                    continue;
                }
            };
            if matches!(filter.op, FilterOp::Contains) {
                predicates.push(self.contains_filter_sql(&name, &filter.value));
            } else {
                predicates.push(format!(
                    "{name} {operator} {}",
                    self.quote_filter_value(&filter.value)
                ));
            }
        }
        let mut order = vec![];
        let mut ordered = std::collections::HashSet::new();
        for sort in &query.sort {
            let name = column(&sort.column)?;
            if !ordered.insert(sort.column.as_str()) {
                return Err(Error::new("Sort each column only once"));
            }
            order.push(format!(
                "{name} {}",
                if sort.descending { "DESC" } else { "ASC" }
            ));
        }
        // Primary keys break sort ties and give unfiltered pages a repeatable order.
        for key in columns.iter().filter(|c| c.primary_key) {
            if ordered.insert(key.name.as_str()) {
                order.push(format!("{} ASC", self.quote_identifier(&key.name)));
            }
        }
        let mut sql = format!(
            "SELECT * FROM {}.{}",
            self.quote_identifier(&table.schema),
            self.quote_identifier(&table.name)
        );
        if !predicates.is_empty() {
            sql.push_str(&format!(" WHERE {}", predicates.join(" AND ")));
        }
        if !order.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
        // ponytail: OFFSET pages; use keyset paging if deep-page scans become a measured bottleneck.
        sql.push_str(&self.pagination_sql(query.limit, query.offset, !order.is_empty()));
        Ok(sql)
    }
    fn table_select_sql(&self, table: &Table, limit: usize) -> Result<String> {
        if !(1..=10_000_000).contains(&limit) {
            return Err(Error::new("Row limit must be 1–10,000,000"));
        }
        Ok(format!(
            "SELECT * FROM {}.{} LIMIT {limit};",
            self.quote_identifier(&table.schema),
            self.quote_identifier(&table.name)
        ))
    }
    async fn execute(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()>;
    /// Native plan execution can require session settings across separate requests.
    async fn execute_plan(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
        _analyze: bool,
    ) -> Result<()> {
        self.execute(sql, output, cancel, limit).await
    }
    async fn tables(&self) -> Result<Vec<Table>>;
    async fn scan_keys(&self, _pattern: &str, _cursor: &str) -> Result<KeyScan> {
        Err(Error::new("This driver does not support key browsing"))
    }
    async fn inspect_key(&self, _key: &Cell, _position: &str) -> Result<KeyInspection> {
        Err(Error::new("This driver does not support key inspection"))
    }
    fn key_command_info(&self, _text: &str) -> Result<KeyCommandInfo> {
        Err(Error::new(
            "This driver does not support native key commands",
        ))
    }
    async fn key_command(&self, _text: &str) -> Result<KeyValue> {
        Err(Error::new(
            "This driver does not support native key commands",
        ))
    }
    /// On-demand, fixed-size catalog pages. Definitions are fetched separately.
    async fn routines(&self, _search: &str, _offset: u32) -> Result<RoutinePage> {
        Err(Error::new("This driver does not support routine browsing"))
    }
    async fn routine_definition(&self, _id: &str) -> Result<String> {
        Err(Error::new(
            "This driver does not support routine definitions",
        ))
    }

    async fn inspect(&self, table: &Table) -> Result<TableInfo>;
    async fn relationships(&self, _table: &Table) -> Result<Vec<ForeignKey>> {
        Err(Error::new(
            "This driver does not support relationship diagrams",
        ))
    }
    async fn transaction_state(&self) -> Result<TransactionState>;
    async fn apply_changes(&self, table: Table, changes: Vec<Change>) -> Result<MutationResult>;
    /// Hold the session for the entire stream and roll back every batch on failure.
    async fn insert_stream(
        &self,
        _table: Table,
        _input: mpsc::Receiver<Result<InsertBatch>>,
        _cancel: CancellationToken,
    ) -> Result<MutationResult> {
        Err(Error::new("This driver does not support imports"))
    }
    async fn disconnect(&self) -> Result<()>;
    /// Execute original statements while owning the session across the entire file.
    /// Preserve script transactions as written; earlier statements may be committed on failure.
    async fn execute_script(
        &self,
        _input: mpsc::Receiver<Result<ScriptBatch>>,
        _output: mpsc::Sender<Batch>,
        _cancel: CancellationToken,
        _completed: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<()> {
        Err(Error::new("This driver does not support SQL file imports"))
    }
}

pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// ClickHouse quoted identifiers also interpret backslash escapes.
pub fn quote_clickhouse_identifier(name: &str) -> String {
    format!("`{}`", name.replace('\\', "\\\\").replace('`', "``"))
}
