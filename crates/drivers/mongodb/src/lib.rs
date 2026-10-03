mod document;
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use bson::{Bson, Document, doc};
use futures_util::TryStreamExt;
use klyndb_driver_api::*;
use mongodb::{
    Client,
    options::{Acknowledgment, ClientOptions, Tls, TlsOptions, WriteConcern},
    results::CollectionType,
};
use std::{future::Future, io::Write, time::Duration};
use tempfile::NamedTempFile;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

// ponytail: one request per session; allow parallel reads only if queueing becomes a measured bottleneck.
pub struct MongoDb {
    state: Mutex<Option<State>>,
    read_only: bool,
}
struct State {
    client: Client,
    _certificates: Vec<NamedTempFile>,
}
const DEADLINE: Duration = Duration::from_secs(10);
const MAX_REPLY: usize = 8 * 1024 * 1024;
fn native_error(error: mongodb::error::Error) -> Error {
    // Native errors may contain hosts, filters, documents or credentials. Never return their Debug/Display.
    if matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Authentication { .. }
    ) {
        return Error::new(
            "MongoDB authentication failed. Check the username, password, authSource and authMechanism.",
        );
    }
    Error::new(
        "MongoDB request failed. Check permissions, syntax, duplicate _id/index values, TLS and server availability. A submitted write may have completed; refresh before retrying.",
    )
}
async fn bounded<T>(
    client: &Client,
    future: impl Future<Output = mongodb::error::Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(DEADLINE, future).await {
        Ok(result) => result.map_err(native_error),
        Err(_) => {
            let _ = tokio::time::timeout(DEADLINE, client.clone().shutdown().immediate(true)).await;
            Err(Error::new(
                "MongoDB request timed out after 10 seconds; connection closed. A submitted write may have completed. Reconnect and refresh before retrying.",
            ))
        }
    }
}
fn snapshot_file(bytes: &[u8]) -> Result<NamedTempFile> {
    let mut file =
        NamedTempFile::new().map_err(|_| Error::new("Could not create private TLS snapshot"))?;
    file.write_all(bytes)
        .map_err(|_| Error::new("Could not write private TLS snapshot"))?;
    Ok(file)
}
impl MongoDb {
    pub async fn connect(
        address: &str,
        password: Option<&str>,
        read_only: bool,
        identity_password: Option<&str>,
    ) -> Result<Self> {
        let mut url = url::Url::parse(address)
            .map_err(|_| Error::new("Enter a single-seed mongodb:// or mongodb+srv:// URL"))?;
        if !matches!(url.scheme(), "mongodb" | "mongodb+srv")
            || url.host_str().is_none()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::new(
                "Expected mongodb://user@host/database; enter credentials in the password field",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (key, value) in url.query_pairs() {
            if !seen.insert(key.to_string())
                || !matches!(
                    key.as_ref(),
                    "tls"
                        | "sslrootcert"
                        | "sslidentity"
                        | "connect_timeout"
                        | "authSource"
                        | "authMechanism"
                        | "replicaSet"
                        | "directConnection"
                )
                || (key == "tls" && !matches!(value.as_ref(), "required" | "disabled"))
                || (key == "authMechanism"
                    && !matches!(value.as_ref(), "SCRAM-SHA-1" | "SCRAM-SHA-256"))
                || (key == "directConnection" && !matches!(value.as_ref(), "true" | "false"))
            {
                return Err(Error::new(
                    "Unsupported, unsafe or repeated MongoDB connection option",
                ));
            }
        }
        let timeout = connect_timeout(
            url.query_pairs()
                .filter(|(k, _)| k == "connect_timeout")
                .map(|(_, v)| v),
        )?;
        let enabled = !url
            .query_pairs()
            .any(|(k, v)| k == "tls" && v == "disabled");
        if !enabled
            && (url.scheme() == "mongodb+srv"
                || url
                    .query_pairs()
                    .any(|(k, _)| matches!(k.as_ref(), "sslrootcert" | "sslidentity")))
        {
            return Err(Error::new(
                "MongoDB SRV URLs and certificate files require verified TLS",
            ));
        }
        if password.is_some_and(|p| p.len() > 16384) {
            return Err(Error::new("MongoDB password exceeds 16 KiB"));
        }
        tls::validate_identity_password(identity_password.unwrap_or_default())?;
        let mut certificates = vec![];
        let mut tls_options = TlsOptions::default();
        for (key, path) in url.query_pairs() {
            if key == "sslrootcert" {
                let mut pem = String::new();
                for der in tls::load_ca_certificates(&path).await? {
                    let encoded = STANDARD.encode(der);
                    pem.push_str("-----BEGIN CERTIFICATE-----\n");
                    for chunk in encoded.as_bytes().chunks(64) {
                        pem.push_str(
                            std::str::from_utf8(chunk)
                                .map_err(|_| Error::new("Could not encode CA"))?,
                        );
                        pem.push('\n');
                    }
                    pem.push_str("-----END CERTIFICATE-----\n");
                }
                let file = snapshot_file(pem.as_bytes())?;
                tls_options.ca_file_path = Some(file.path().into());
                certificates.push(file);
            } else if key == "sslidentity" {
                let bytes =
                    tls::read_security_file(&path, "MongoDB PEM certificate and private key")
                        .await?;
                let file = snapshot_file(&bytes)?;
                tls_options.cert_key_file_path = Some(file.path().into());
                tls_options.tls_certificate_key_file_password = identity_password
                    .filter(|p| !p.is_empty())
                    .map(|p| p.as_bytes().to_vec());
                certificates.push(file);
            }
        }
        let options: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| {
                matches!(
                    k.as_ref(),
                    "authSource" | "authMechanism" | "replicaSet" | "directConnection"
                )
            })
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut().clear().extend_pairs(options);
        tokio::time::timeout(timeout, async {
            let mut options = ClientOptions::parse(url.as_str())
                .await
                .map_err(native_error)?;
            if let Some(password) = password {
                let credential = options
                    .credential
                    .as_mut()
                    .ok_or_else(|| Error::new("Enter a MongoDB username to use a password"))?;
                credential.password = Some(password.to_owned());
            }
            options.tls = Some(if enabled {
                Tls::Enabled(tls_options)
            } else {
                Tls::Disabled
            });
            options.connect_timeout = Some(timeout);
            options.server_selection_timeout = Some(timeout);
            options.max_pool_size = Some(2);
            options.retry_reads = Some(false);
            options.retry_writes = Some(false);
            options.write_concern =
                Some(WriteConcern::builder().w(Acknowledgment::Majority).build());
            options.app_name = Some("Klyndb".into());
            let client = Client::with_options(options).map_err(native_error)?;
            if let Err(error) = client
                .database("admin")
                .run_command(doc! { "ping": 1 })
                .await
            {
                client.shutdown().immediate(true).await;
                return Err(native_error(error));
            }
            Ok(Self {
                state: Mutex::new(Some(State {
                    client,
                    _certificates: certificates,
                })),
                read_only,
            })
        })
        .await
        .map_err(|_| {
            Error::new(format!(
                "MongoDB connection timed out after {} seconds",
                timeout.as_secs()
            ))
        })?
    }
}
fn connected(state: &Option<State>) -> Result<&State> {
    state
        .as_ref()
        .ok_or_else(|| Error::new("MongoDB connection is closed. Reconnect."))
}
#[async_trait]
impl Session for MongoDb {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            key_value: false,
            document_queries: true,
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
    async fn document_databases(&self) -> Result<Vec<String>> {
        let state = self.state.lock().await;
        let client = &connected(&state)?.client;
        let names = bounded(client, async {
            client
                .list_database_names()
                .authorized_databases(true)
                .await
        })
        .await?;
        if names.len() > 10_000 || names.iter().map(String::len).sum::<usize>() > MAX_REPLY {
            return Err(Error::new("Database catalog exceeds workspace limit"));
        }
        Ok(names)
    }
    async fn document_collections(&self, database: &str) -> Result<Vec<Table>> {
        document::name(database, true)?;
        let state = self.state.lock().await;
        let client = &connected(&state)?.client;
        let db = client.database(database);
        let tables = bounded(client, async {
            let mut cursor = db.list_collections().batch_size(100).await?;
            let mut tables = vec![];
            // ponytail: bounded catalog; add searchable catalog paging for >10k collections.
            while let Some(spec) = cursor.try_next().await? {
                if tables.len() == 10_001 {
                    break;
                }
                tables.push(Table {
                    schema: database.into(),
                    name: spec.name,
                    kind: match spec.collection_type {
                        CollectionType::View => "view",
                        CollectionType::Timeseries => "timeseries",
                        _ => "collection",
                    }
                    .into(),
                });
            }
            Ok(tables)
        })
        .await?;
        if tables.len() > 10_000
            || tables
                .iter()
                .map(|t| t.name.len() + t.schema.len() + 64)
                .sum::<usize>()
                > MAX_REPLY
        {
            return Err(Error::new(
                "Collection catalog exceeds workspace limit; select a smaller database",
            ));
        }
        Ok(tables)
    }
    async fn document_indexes(&self, database: &str, collection: &str) -> Result<Vec<String>> {
        document::name(database, true)?;
        document::name(collection, false)?;
        let state = self.state.lock().await;
        let client = &connected(&state)?.client;
        let collection = client.database(database).collection::<Document>(collection);
        let indexes = bounded(client, async {
            let mut cursor = collection.list_indexes().batch_size(10).await?;
            let mut indexes = vec![];
            while let Some(index) = cursor.try_next().await? {
                if indexes.len() == 100 {
                    break;
                }
                indexes.push(index);
            }
            Ok(indexes)
        })
        .await?;
        let reply: Vec<String> = indexes
            .iter()
            .map(|index| {
                let d = bson::serialize_to_document(index)
                    .map_err(|_| Error::new("Could not decode index metadata"))?;
                Ok(document::record(d, false)?.json)
            })
            .collect::<Result<_>>()?;
        if serde_json::to_vec(&reply)
            .map_err(|_| Error::new("Could not encode index metadata"))?
            .len()
            > MAX_REPLY
        {
            return Err(Error::new(
                "Index metadata exceeds the 8 MiB workspace limit",
            ));
        }
        Ok(reply)
    }
    async fn document_query(&self, query: DocumentQuery) -> Result<DocumentPage> {
        document::name(&query.database, true)?;
        document::name(&query.collection, false)?;
        if query.offset > 1_000_000 {
            return Err(Error::new(
                "Document page offset is limited to one million; narrow the filter for deep pages",
            ));
        }
        let pipeline = if query.aggregate {
            Some(document::pipeline(&query.text)?)
        } else {
            None
        };
        let filter = if query.aggregate {
            Document::new()
        } else {
            let filter = document::object(&query.text)?;
            document::read_only(&Bson::Document(filter.clone()))?;
            filter
        };
        let mut sort = document::object(&query.sort)?;
        if sort.len() > 8
            || sort
                .values()
                .any(|v| !matches!(v, Bson::Int32(1 | -1) | Bson::Int64(1 | -1)))
        {
            return Err(Error::new(
                "Sort is a JSON object with up to eight fields set to 1 or -1",
            ));
        }
        if !sort.contains_key("_id") {
            sort.insert("_id", 1);
        }
        let state = self.state.lock().await;
        let client = &connected(&state)?.client;
        let collection = client
            .database(&query.database)
            .collection::<Document>(&query.collection);
        let page_deadline = tokio::time::Instant::now() + DEADLINE;
        let cursor = bounded(client, async {
            if let Some(mut stages) = pipeline {
                stages.push(doc! { "$skip": i64::from(query.offset) });
                stages.push(doc! { "$limit": (document::PAGE + 1) as i64 });
                collection
                    .aggregate(stages)
                    .batch_size(1)
                    .max_time(DEADLINE)
                    .allow_disk_use(false)
                    .await
            } else {
                // ponytail: native skip pages; use keyset paging if deep-page scans become a bottleneck.
                collection
                    .find(filter)
                    .sort(sort)
                    .skip(u64::from(query.offset))
                    .limit((document::PAGE + 1) as i64)
                    .batch_size(1)
                    .max_time(DEADLINE)
                    .await
            }
        })
        .await?;
        let mut cursor = cursor;
        let mut documents = vec![];
        let mut bytes = 64;
        let mut has_more = false;
        // The same deadline must cover every getMore, not reset per document.
        let result = tokio::time::timeout_at(page_deadline, async {
            while let Some(doc) = cursor.try_next().await.map_err(native_error)? {
                if documents.len() == document::PAGE { has_more = true; break; }
                let row = document::record(doc, !query.aggregate)?;
                bytes += serde_json::to_vec(&row).map_err(|_| Error::new("Could not encode document page"))?.len() + 1;
                if bytes > MAX_REPLY { return Err(Error::new("Document page exceeds 8 MiB; narrow your filter or use a projected read-only aggregation")); }
                documents.push(row);
            }
            Ok(DocumentPage { documents, has_more })
        }).await;
        drop(cursor);
        match result {
            Ok(result) => result,
            Err(_) => {
                let _ =
                    tokio::time::timeout(DEADLINE, client.clone().shutdown().immediate(true)).await;
                Err(Error::new(
                    "MongoDB document page timed out; connection closed. Reconnect.",
                ))
            }
        }
    }
    async fn document_change(
        &self,
        database: &str,
        collection: &str,
        change: DocumentChange,
    ) -> Result<MutationResult> {
        if self.read_only {
            return Err(Error::new("This MongoDB connection is read-only"));
        }
        document::name(database, true)?;
        document::name(collection, false)?;
        document::validate_change(&change)?;
        let state = self.state.lock().await;
        let client = &connected(&state)?.client;
        let db = client.database(database);
        let collection_name = collection;
        let collection = db.collection::<Document>(collection);
        bounded(client, async {
            let mut cursor = db.list_collections().filter(doc! { "name": collection_name }).await?;
            let spec = cursor.try_next().await?;
            Ok(spec)
        }).await?.filter(|s| matches!(s.collection_type, CollectionType::Collection) && !s.info.read_only)
            .ok_or_else(|| Error::new("Editing requires an existing writable collection; views and time-series are read-only here"))?;
        let affected = match change {
            DocumentChange::Insert { json } => {
                let d = document::object(&json)?;
                bounded(client, async { collection.insert_one(d).await }).await?;
                1
            }
            DocumentChange::Replace { snapshot, json } => {
                let guard = document::predicate(document::original(&snapshot)?)?;
                let d = document::object(&json)?;
                let result = bounded(client, async {
                    collection.replace_one(guard, d).upsert(false).await
                })
                .await?;
                if result.matched_count != 1 {
                    return Err(Error::new(
                        "Document changed or disappeared. Refresh before editing.",
                    ));
                }
                result.modified_count
            }
            DocumentChange::Delete { snapshot } => {
                let guard = document::predicate(document::original(&snapshot)?)?;
                let result = bounded(client, async { collection.delete_one(guard).await }).await?;
                if result.deleted_count != 1 {
                    return Err(Error::new(
                        "Document changed or disappeared. Refresh before deleting.",
                    ));
                }
                result.deleted_count
            }
        };
        Ok(MutationResult {
            affected,
            pending_transaction: false,
        })
    }
    async fn execute(
        &self,
        _: String,
        _: mpsc::Sender<Batch>,
        _: CancellationToken,
        _: usize,
    ) -> Result<()> {
        Err(Error::new(
            "MongoDB uses JSON filters and aggregation pipelines, not SQL",
        ))
    }
    async fn tables(&self) -> Result<Vec<Table>> {
        Err(Error::new("Use the MongoDB collection explorer"))
    }
    async fn inspect(&self, _: &Table) -> Result<TableInfo> {
        Err(Error::new("Use native document indexes"))
    }
    async fn apply_changes(&self, _: Table, _: Vec<Change>) -> Result<MutationResult> {
        Err(Error::new("Use reviewed document edits"))
    }
    async fn transaction_state(&self) -> Result<TransactionState> {
        let state = self.state.lock().await;
        let client = &connected(&state)?.client;
        bounded(client, async {
            client
                .database("admin")
                .run_command(doc! { "ping": 1 })
                .await
        })
        .await?;
        Ok(TransactionState::Idle)
    }
    async fn disconnect(&self) -> Result<()> {
        if let Some(state) = self.state.lock().await.take() {
            let _ = tokio::time::timeout(DEADLINE, state.client.shutdown().immediate(true)).await;
        }
        Ok(())
    }
}
