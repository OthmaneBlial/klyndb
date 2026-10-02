#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use klyndb_connections::{Connection, Store};
use klyndb_core::{Engine, QueryStatus};
use klyndb_driver_api::{
    Capabilities, Change, MutationResult, Row, Table, TableInfo, TransactionState,
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
async fn save_connection(
    engine: State<'_, Arc<Engine>>,
    mut connection: Connection,
    password: Option<String>,
    remember: bool,
) -> ApiResult<Connection> {
    let secret = connection.validate().map_err(api)?;
    let password = password.map(zeroize::Zeroizing::new).or(secret);
    engine.disconnect(&connection.id).await.map_err(api)?;
    let store = engine.store.clone();
    blocking(move || {
        if remember {
            if let Some(password) = password {
                klyndb_connections::save_password(&connection.id, &password).map_err(api)?;
            }
        } else if connection.engine != "sqlite" {
            klyndb_connections::delete_password(&connection.id).map_err(api)?;
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
        store.delete(&id).map_err(api)
    })
    .await
}
#[tauri::command]
async fn connect(
    engine: State<'_, Arc<Engine>>,
    id: String,
    password: Option<String>,
) -> ApiResult<Capabilities> {
    engine.connect(&id, password).await.map_err(api)
}
#[tauri::command]
async fn test_connection(
    engine: State<'_, Arc<Engine>>,
    connection: Connection,
    password: Option<String>,
) -> ApiResult<Capabilities> {
    engine
        .test_connection(connection, password)
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
    let result = status
        .sets
        .get(set)
        .cloned()
        .ok_or("Result set not found")?;
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
        let mut offset = 0;
        let mut buffer = Vec::<Row>::new().into_iter();
        let rows = std::iter::from_fn(|| {
            if let Some(row) = buffer.next() {
                return Some(Ok(row));
            }
            if offset >= result.rows {
                return None;
            }
            match job.page(set, offset, 500) {
                Ok(page) => {
                    offset += page.len();
                    buffer = page.into_iter();
                    buffer.next().map(Ok)
                }
                Err(e) => {
                    offset = result.rows;
                    Some(Err(e))
                }
            }
        });
        let count = klyndb_export::export(&mut temporary, &result.columns, rows, &format, &table)
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
            export_result
        ])
        .run(tauri::generate_context!())
        .expect("Could not start Klyndb");
}
