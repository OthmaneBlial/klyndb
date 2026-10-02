use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub message: String,
}
impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

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
pub type Row = Vec<Cell>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub transactions: bool,
    pub schemas: bool,
    pub explain: bool,
    pub edit_rows: bool,
    pub cancel: bool,
    pub tls: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub default: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableInfo {
    pub columns: Vec<Column>,
    pub ddl: Option<String>,
    pub indexes: Vec<serde_json::Value>,
    pub foreign_keys: Vec<serde_json::Value>,
}

pub enum Batch {
    Columns(Vec<String>),
    Rows(Vec<Row>),
    Complete { affected: u64, truncated: bool },
}

#[async_trait]
pub trait Session: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    async fn execute(
        &self,
        sql: String,
        output: mpsc::Sender<Batch>,
        cancel: CancellationToken,
        limit: usize,
    ) -> Result<()>;
    async fn tables(&self) -> Result<Vec<Table>>;
    async fn inspect(&self, table: &Table) -> Result<TableInfo>;
    async fn disconnect(&self) -> Result<()>;
}

pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
