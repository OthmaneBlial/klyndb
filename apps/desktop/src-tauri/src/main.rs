#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use klyndb_connections::{Connection, Store};
use klyndb_core::import::{
    ImportFormat, ImportOptions, ImportRequest, ImportSource, ImportStatus, Preview,
    SqlImportRequest, SqlSource,
};
use klyndb_core::{Engine, QueryStatus};
use klyndb_driver_api::{
    Capabilities, Change, MutationResult, Row, Table, TableInfo, TableQuery, TransactionState,
};
use std::sync::Arc;
use tauri::{Manager, State};

type ApiResult<T> = Result<T, String>;
fn api(e: impl std::fmt::Display) -> String {
    e.to_string()
}
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(api)?
}

#[tauri::command]
async fn connections(engine: State<'_, Arc<Engine>>) -> ApiResult<Vec<Connection>> {
    let store = engine.store.clone();
    blocking(move || store.connections().map_err(api)).await
}
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Separate typed IPC secrets retain compatibility with saved-connection callers.
async fn save_connection(
    engine: State<'_, Arc<Engine>>,
    mut connection: Connection,
    password: Option<String>,
    remember: bool,
    identity_password: Option<String>,
    remember_identity: Option<bool>,
    ssh_password: Option<String>,
    remember_ssh: Option<bool>,
) -> ApiResult<Connection> {
    let secret = connection.validate().map_err(api)?;
    let password = password.map(zeroize::Zeroizing::new).or(secret);
    let identity_password = identity_password.map(zeroize::Zeroizing::new);
    let ssh_password = ssh_password.map(zeroize::Zeroizing::new);
    if let Some(password) = &ssh_password {
        klyndb_connections::ssh::validate_password(password).map_err(api)?;
    }
    if let Some(password) = &identity_password {
        klyndb_driver_api::tls::validate_identity_password(password).map_err(api)?;
    }
    engine.disconnect(&connection.id).await.map_err(api)?;
    let store = engine.store.clone();
    blocking(move || {
        let previous = store
            .connections()
            .map_err(api)?
            .into_iter()
            .find(|c| c.id == connection.id);
        if remember {
            if let Some(password) = password {
                klyndb_connections::save_password(&connection.id, &password).map_err(api)?;
            }
        } else if previous.as_ref().is_some_and(|c| c.engine != "sqlite") {
            klyndb_connections::delete_password(&connection.id).map_err(api)?;
        }
        let identity_key = klyndb_connections::client_identity_key(&connection.id);
        if connection.has_client_identity() && remember_identity.unwrap_or(false) {
            if let Some(password) = identity_password {
                klyndb_connections::save_password(&identity_key, &password).map_err(api)?;
            }
        } else if previous
            .as_ref()
            .is_some_and(Connection::has_client_identity)
        {
            klyndb_connections::delete_password(&identity_key).map_err(api)?;
        }
        let ssh_key = klyndb_connections::ssh::credential_key(&connection.id);
        let current_ssh = connection.ssh().map_err(api)?;
        let previous_ssh = previous
            .as_ref()
            .map(Connection::ssh)
            .transpose()
            .map_err(api)?
            .flatten();
        if current_ssh.as_ref().is_some_and(|s| s.auth != "agent") && remember_ssh.unwrap_or(false)
        {
            if let Some(password) = ssh_password {
                klyndb_connections::save_password(&ssh_key, &password).map_err(api)?;
            } else if previous_ssh.is_some() && previous_ssh != current_ssh {
                klyndb_connections::delete_password(&ssh_key).map_err(api)?;
            }
        } else if previous_ssh.is_some() {
            klyndb_connections::delete_password(&ssh_key).map_err(api)?;
        }
        store.save(&connection).map_err(api)?;
        Ok(connection)
    })
    .await
}
#[tauri::command]
async fn delete_connection(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<()> {
    engine.disconnect(&id).await.map_err(api)?;
    let store = engine.store.clone();
    blocking(move || {
        let c = store.connection(&id).map_err(api)?;
        if c.engine != "sqlite" {
            klyndb_connections::delete_password(&id).map_err(api)?;
        }
        if c.has_client_identity() {
            klyndb_connections::delete_password(&klyndb_connections::client_identity_key(&id))
                .map_err(api)?;
        }
        if c.ssh().map_err(api)?.is_some() {
            klyndb_connections::delete_password(&klyndb_connections::ssh::credential_key(&id))
                .map_err(api)?;
        }
        store.delete(&id).map_err(api)
    })
    .await
}
#[tauri::command]
async fn connect(
    engine: State<'_, Arc<Engine>>,
    id: String,
    password: Option<String>,
    identity_password: Option<String>,
    ssh_password: Option<String>,
) -> ApiResult<Capabilities> {
    engine
        .connect(&id, password, identity_password, ssh_password)
        .await
        .map_err(api)
}
#[tauri::command]
async fn test_connection(
    engine: State<'_, Arc<Engine>>,
    connection: Connection,
    password: Option<String>,
    identity_password: Option<String>,
    ssh_password: Option<String>,
) -> ApiResult<Capabilities> {
    engine
        .test_connection(connection, password, identity_password, ssh_password)
        .await
        .map_err(api)
}
#[tauri::command]
async fn disconnect(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<()> {
    engine.disconnect(&id).await.map_err(api)
}
#[tauri::command]
async fn tables(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<Vec<Table>> {
    engine
        .driver(&id)
        .await
        .map_err(api)?
        .tables()
        .await
        .map_err(api)
}
#[tauri::command]
async fn inspect_table(
    engine: State<'_, Arc<Engine>>,
    id: String,
    table: Table,
) -> ApiResult<TableInfo> {
    engine
        .driver(&id)
        .await
        .map_err(api)?
        .inspect(&table)
        .await
        .map_err(api)
}
#[tauri::command]
async fn table_select_sql(
    engine: State<'_, Arc<Engine>>,
    id: String,
    table: Table,
    limit: usize,
) -> ApiResult<String> {
    engine
        .driver(&id)
        .await
        .map_err(api)?
        .table_select_sql(&table, limit)
        .map_err(api)
}
#[tauri::command]
async fn table_query_sql(
    engine: State<'_, Arc<Engine>>,
    id: String,
    table: Table,
    query: TableQuery,
) -> ApiResult<String> {
    engine
        .table_query_sql(&id, &table, &query)
        .await
        .map_err(api)
}
#[tauri::command]
async fn diagram_tables(
    engine: State<'_, Arc<Engine>>,
    id: String,
    tables: Vec<Table>,
) -> ApiResult<klyndb_core::diagram::Diagram> {
    engine.diagram(&id, &tables).await.map_err(api)
}
#[tauri::command]
async fn export_diagram(
    model: klyndb_core::diagram::Diagram,
    positions: klyndb_core::diagram::Positions,
) -> ApiResult<Option<u64>> {
    let svg = model.svg(&positions).map_err(api)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter("SVG diagram", &["svg"])
        .set_file_name("relationship-diagram.svg")
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_owned();
    blocking(move || {
        use std::io::Write;
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid export path")?)
            .map_err(api)?;
        temp.write_all(svg.as_bytes()).map_err(api)?;
        temp.as_file().sync_all().map_err(api)?;
        temp.persist(path).map_err(api)?;
        Ok(Some(svg.len() as u64))
    })
    .await
}
#[tauri::command]
async fn transaction_state(
    engine: State<'_, Arc<Engine>>,
    id: String,
) -> ApiResult<TransactionState> {
    engine
        .driver(&id)
        .await
        .map_err(api)?
        .transaction_state()
        .await
        .map_err(api)
}
#[tauri::command]
async fn apply_changes(
    engine: State<'_, Arc<Engine>>,
    id: String,
    table: Table,
    changes: Vec<Change>,
    confirmed: bool,
) -> ApiResult<MutationResult> {
    engine
        .apply_changes(&id, table, changes, confirmed)
        .await
        .map_err(api)
}
#[tauri::command]
async fn analyze_query(sql: String, engine: String) -> ApiResult<klyndb_query::Analysis> {
    klyndb_query::analyze(&sql, &engine).map_err(api)
}
#[tauri::command]
async fn start_query(
    engine: State<'_, Arc<Engine>>,
    connection: String,
    sql: String,
    limit: usize,
    timeout_seconds: u64,
    confirmed: bool,
) -> ApiResult<String> {
    engine
        .start(connection, sql, limit, timeout_seconds, confirmed)
        .await
        .map_err(api)
}
#[tauri::command]
async fn query_status(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<QueryStatus> {
    engine.job(&id).map_err(api)?.status().map_err(api)
}
#[tauri::command]
async fn start_plan(
    engine: State<'_, Arc<Engine>>,
    connection: String,
    sql: String,
    analyze: bool,
    timeout_seconds: u64,
    confirmed: bool,
) -> ApiResult<String> {
    engine
        .start_plan(connection, sql, analyze, timeout_seconds, confirmed)
        .await
        .map_err(api)
}
#[tauri::command]
async fn execution_plan(
    engine: State<'_, Arc<Engine>>,
    id: String,
) -> ApiResult<klyndb_query::plan::Plan> {
    let job = engine.job(&id).map_err(api)?;
    blocking(move || job.plan().map_err(api)).await
}
#[tauri::command]
async fn result_page(
    engine: State<'_, Arc<Engine>>,
    id: String,
    set: usize,
    offset: usize,
    limit: usize,
) -> ApiResult<Vec<Row>> {
    let job = engine.job(&id).map_err(api)?;
    blocking(move || job.page(set, offset, limit).map_err(api)).await
}
#[tauri::command]
async fn cancel_query(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<()> {
    engine.cancel(&id).map_err(api)
}
#[tauri::command]
async fn release_result(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<()> {
    engine.release(&id).map_err(api)
}
#[tauri::command]
async fn load_document(
    engine: State<'_, Arc<Engine>>,
    id: String,
) -> ApiResult<Option<serde_json::Value>> {
    let store = engine.store.clone();
    blocking(move || store.document(&id).map_err(api)).await
}
#[tauri::command]
async fn save_document(
    engine: State<'_, Arc<Engine>>,
    id: String,
    data: serde_json::Value,
) -> ApiResult<()> {
    let store = engine.store.clone();
    blocking(move || store.save_document(&id, &data).map_err(api)).await
}
#[tauri::command]
async fn history(engine: State<'_, Arc<Engine>>) -> ApiResult<Vec<serde_json::Value>> {
    let store = engine.store.clone();
    blocking(move || store.history().map_err(api)).await
}
#[tauri::command]
async fn clear_history(engine: State<'_, Arc<Engine>>) -> ApiResult<()> {
    let store = engine.store.clone();
    blocking(move || store.clear_history().map_err(api)).await
}
#[tauri::command]
async fn choose_ssh_identity_file() -> ApiResult<Option<String>> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("Choose SSH private key")
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().into_owned()))
}
#[tauri::command]
async fn choose_client_identity_file() -> ApiResult<Option<String>> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("Choose client identity")
        .add_filter("PKCS#12 client identity", &["p12", "pfx"])
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().into_owned()))
}
#[tauri::command]
async fn choose_ca_file() -> ApiResult<Option<String>> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("Choose CA certificates")
        .add_filter("CA certificates", &["pem", "crt", "cer", "der"])
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().into_owned()))
}
#[tauri::command]
async fn choose_database_file(create: bool) -> ApiResult<Option<String>> {
    let dialog =
        rfd::AsyncFileDialog::new().add_filter("SQLite database", &["sqlite", "db", "sqlite3"]);
    let file = if create {
        dialog.save_file().await
    } else {
        dialog.pick_file().await
    };
    Ok(file.map(|f| f.path().to_string_lossy().into_owned()))
}
#[tauri::command]
async fn choose_import_file(
    engine: State<'_, Arc<Engine>>,
    options: ImportOptions,
) -> ApiResult<Option<ImportSource>> {
    options.validate().map_err(api)?;
    let dialog = rfd::AsyncFileDialog::new();
    let dialog = if options.format == ImportFormat::Csv {
        dialog.add_filter("CSV data", &["csv", "tsv", "txt"])
    } else {
        dialog.add_filter("JSON data", &["json"])
    };
    let Some(file) = dialog.pick_file().await else {
        return Ok(None);
    };
    engine
        .imports
        .prepare(file.path().to_owned(), options)
        .await
        .map(Some)
        .map_err(api)
}
#[tauri::command]
async fn preview_import(
    engine: State<'_, Arc<Engine>>,
    id: String,
    options: ImportOptions,
) -> ApiResult<Preview> {
    engine.imports.preview(&id, options).await.map_err(api)
}
#[tauri::command]
async fn choose_sql_import_file(
    engine: State<'_, Arc<Engine>>,
    connection: String,
) -> ApiResult<Option<SqlSource>> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter("SQL script", &["sql", "txt"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    engine
        .prepare_sql_import(file.path().to_owned(), &connection)
        .await
        .map(Some)
        .map_err(api)
}
#[tauri::command]
async fn start_sql_import(
    engine: State<'_, Arc<Engine>>,
    request: SqlImportRequest,
) -> ApiResult<String> {
    engine.start_sql_import(request).await.map_err(api)
}
#[tauri::command]
async fn start_import(engine: State<'_, Arc<Engine>>, request: ImportRequest) -> ApiResult<String> {
    engine.start_import(request).await.map_err(api)
}
#[tauri::command]
async fn import_status(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<ImportStatus> {
    engine.imports.status(&id).map_err(api)
}
#[tauri::command]
async fn cancel_import(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<()> {
    engine.imports.cancel(&id).map_err(api)
}
#[tauri::command]
async fn release_import(engine: State<'_, Arc<Engine>>, id: String) -> ApiResult<()> {
    engine.imports.release(&id).map_err(api)
}
#[tauri::command]
async fn export_result(
    engine: State<'_, Arc<Engine>>,
    id: String,
    set: usize,
    format: String,
    table: String,
) -> ApiResult<Option<u64>> {
    if !["csv", "json", "jsonl", "sql", "markdown"].contains(&format.as_str()) {
        return Err("Unknown export format".into());
    }
    let job = engine.job(&id).map_err(api)?;
    let status = job.status().map_err(api)?;
    if !status.done {
        return Err("Wait for the query to finish before exporting".into());
    }
    status.sets.get(set).ok_or("Result set not found")?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name(format!(
            "query-result.{}",
            if format == "markdown" { "md" } else { &format }
        ))
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_owned();
    blocking(move || {
        let parent = path.parent().ok_or("Invalid export path")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(api)?;
        let count = job
            .export(&mut temporary, set, &format, &table)
            .map_err(api)?;
        temporary.as_file().sync_all().map_err(api)?;
        temporary.persist(path).map_err(api)?;
        Ok(Some(count))
    })
    .await
}
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let engine = Engine::new(Store::open(&directory.join("state.sqlite"))?);
            app.manage(Arc::new(engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connections,
            save_connection,
            delete_connection,
            connect,
            test_connection,
            disconnect,
            tables,
            inspect_table,
            table_select_sql,
            table_query_sql,
            diagram_tables,
            export_diagram,
            apply_changes,
            transaction_state,
            analyze_query,
            start_query,
            start_plan,
            execution_plan,
            query_status,
            result_page,
            cancel_query,
            release_result,
            load_document,
            save_document,
            history,
            clear_history,
            choose_database_file,
            choose_ca_file,
            choose_client_identity_file,
            choose_ssh_identity_file,
            choose_import_file,
            choose_sql_import_file,
            start_sql_import,
            preview_import,
            start_import,
            import_status,
            cancel_import,
            release_import,
            export_result
        ])
        .build(tauri::generate_context!())
        .expect("Could not start Klyndb")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let imports = app.state::<Arc<Engine>>().imports.clone();
                if imports.begin_shutdown() {
                    api.prevent_exit();
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        match imports.shutdown().await {
                            Ok(()) => handle.exit(0),
                            Err(e) => {
                                tracing::error!(error=%e, "could not terminate imports before exit")
                            }
                        }
                    });
                }
            }
        });
}
