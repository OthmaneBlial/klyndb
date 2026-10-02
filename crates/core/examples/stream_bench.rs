use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "100000".into())
        .parse()?;
    if !(1..=10_000_000).contains(&rows) {
        return Err("rows must be 1–10,000,000".into());
    }
    let directory = tempfile::tempdir()?;
    let store = Store::open(&directory.path().join("state.sqlite"))?;
    let mut connection = Connection {
        id: String::new(),
        name: "benchmark".into(),
        engine: "sqlite".into(),
        address: directory
            .path()
            .join("data.sqlite")
            .to_string_lossy()
            .into(),
        environment: "development".into(),
        group: String::new(),
        color: "#93d4b5".into(),
        favorite: false,
        read_only: false,
        create_file: true,
    };
    connection.validate()?;
    store.save(&connection)?;
    let engine = Engine::new(store);
    engine.connect(&connection.id, None, None).await?;
    let started = Instant::now();
    let id=engine.start(connection.id.clone(),format!("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{rows}) SELECT x,printf('row-%d',x) AS label FROM n"),rows,3600,false).await?;
    let job = engine.job(&id)?;
    let mut first_row_ms = None;
    loop {
        let status = job.status()?;
        if first_row_ms.is_none() && status.sets.first().is_some_and(|s| s.rows > 0) {
            first_row_ms = Some(started.elapsed().as_millis());
        }
        if status.done {
            if let Some(error) = status.error {
                return Err(error.into());
            }
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    let elapsed = started.elapsed();
    assert_eq!(job.status()?.sets[0].rows, rows);
    let page_start = Instant::now();
    let page = job.page(0, rows.saturating_sub(500), 500)?;
    assert_eq!(page.len(), rows.min(500));
    println!(
        "{}",
        serde_json::json!({"rows":rows,"elapsed_ms":elapsed.as_millis(),"rows_per_second":rows as f64/elapsed.as_secs_f64(),"first_row_ms":first_row_ms,"last_page_us":page_start.elapsed().as_micros(),"frontend_rows_per_page":500,"channel_batches":4})
    );
    engine.release(&id)?;
    engine.disconnect(&connection.id).await?;
    Ok(())
}
