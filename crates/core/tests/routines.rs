use klyndb_driver_api::{Batch, Session, quote_identifier};
use klyndb_postgres::Postgres;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn execute(driver: Arc<Postgres>, sql: String) {
    let (sender, mut receiver) = mpsc::channel(4);
    let task = tokio::spawn(async move {
        driver
            .execute(sql, sender, CancellationToken::new(), 10)
            .await
    });
    while receiver.recv().await.is_some() {}
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn postgres_routines_native_signatures_paging_definitions_and_readonly() {
    let Ok(url) = std::env::var("KLYNDB_TEST_POSTGRES_URL") else {
        return;
    };
    let driver = Arc::new(Postgres::connect(&url, None, false, None).await.unwrap());
    let schema = format!("klyndb routines {}", uuid::Uuid::new_v4().simple());
    let quoted = quote_identifier(&schema);
    let mut sql = format!("CREATE SCHEMA {quoted}; CREATE TABLE {quoted}.writes(id integer);");
    for index in 0..101 {
        sql.push_str(&format!("CREATE FUNCTION {quoted}.item_{index:03}() RETURNS integer LANGUAGE sql AS $$ SELECT {index} $$;"));
    }
    sql.push_str(&format!("CREATE FUNCTION {quoted}.overloaded(value integer) RETURNS integer LANGUAGE sql AS $$ SELECT value $$; CREATE FUNCTION {quoted}.overloaded(value text) RETURNS text LANGUAGE sql AS $$ SELECT value $$; CREATE PROCEDURE {quoted}.append_value(value integer) LANGUAGE sql AS $$ INSERT INTO {quoted}.writes VALUES(value) $$; CREATE FUNCTION {quoted}.\"literal%_name\"() RETURNS text LANGUAGE sql AS $$ SELECT '<script>é</script>'::text $$;"));
    execute(driver.clone(), sql).await;
    let read_only = Postgres::connect(&url, None, true, None).await.unwrap();
    assert!(read_only.capabilities().routines);
    let first = read_only.routines(&schema, 0).await.unwrap();
    let second = read_only.routines(&schema, 100).await.unwrap();
    assert_eq!(first.routines.len(), 100);
    assert!(first.has_more);
    assert_eq!(second.routines.len(), 5);
    assert!(!second.has_more);
    assert!(
        first
            .routines
            .iter()
            .all(|a| second.routines.iter().all(|b| a.id != b.id))
    );
    let overloads = read_only
        .routines(&format!("{schema}.overloaded"), 0)
        .await
        .unwrap();
    assert_eq!(overloads.routines.len(), 2);
    assert_ne!(overloads.routines[0].id, overloads.routines[1].id);
    assert!(
        overloads
            .routines
            .iter()
            .any(|r| r.arguments == "value integer" && r.returns.as_deref() == Some("integer"))
    );
    assert!(
        overloads
            .routines
            .iter()
            .any(|r| r.arguments == "value text" && r.returns.as_deref() == Some("text"))
    );
    let literal = read_only
        .routines(&format!("{schema}.literal%_name"), 0)
        .await
        .unwrap();
    assert_eq!(literal.routines.len(), 1);
    assert!(
        read_only
            .routines("%_name' OR true --", 0)
            .await
            .unwrap()
            .routines
            .is_empty()
    );
    assert!(read_only.routines(&"a".repeat(1025), 0).await.is_err());
    assert!(read_only.routines("", 1_000_001).await.is_err());
    let mut definitions = serde_json::Map::new();
    for routine in first.routines.iter().chain(second.routines.iter()) {
        let definition = read_only.routine_definition(&routine.id).await.unwrap();
        assert!(definition.starts_with("CREATE OR REPLACE"));
        assert!(definition.contains(&quoted));
        if routine.kind == "procedure" {
            assert!(definition.contains("INSERT INTO"));
            assert!(routine.returns.is_none());
        }
        definitions.insert(routine.id.clone(), serde_json::Value::String(definition));
    }
    // Catalog inspection never calls even write-capable routines.
    let (tx, mut rx) = mpsc::channel(4);
    driver
        .execute(
            format!("SELECT count(*) FROM {quoted}.writes"),
            tx,
            CancellationToken::new(),
            1,
        )
        .await
        .unwrap();
    let mut empty = false;
    while let Some(batch) = rx.recv().await {
        if let Batch::Rows(rows) = batch {
            empty = rows[0][0].text() == "0";
        }
    }
    assert!(empty);
    assert!(read_only.routine_definition("0").await.is_err());
    assert!(read_only.routine_definition("1 OR true").await.is_err());
    if let Ok(directory) = std::env::var("KLYNDB_ROUTINE_EVIDENCE_DIR") {
        let path = std::path::Path::new(&directory);
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("native-catalog.json"), serde_json::to_vec_pretty(&serde_json::json!({"first": first,"second":second,"overloads":overloads,"literal":literal,"definitions":definitions})).unwrap()).unwrap();
    }
    execute(driver.clone(), format!("DROP SCHEMA {quoted} CASCADE")).await;
    assert!(
        read_only
            .routine_definition(&literal.routines[0].id)
            .await
            .is_err()
    );
    read_only.disconnect().await.unwrap();
    driver.disconnect().await.unwrap();
}
