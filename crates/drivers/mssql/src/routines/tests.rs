use super::*;
use std::sync::Arc;

async fn fixture(driver: &SqlServer, sql: String) {
    let mut guard = driver.connection.lock().await;
    guard
        .as_mut()
        .unwrap()
        .simple_query(sql)
        .await
        .unwrap()
        .into_results()
        .await
        .unwrap();
}

async fn pages(driver: &dyn Session, schema: &str, total: usize) -> (RoutinePage, RoutinePage) {
    assert!(driver.capabilities().routines);
    let first = driver.routines(schema, 0).await.unwrap();
    let second = driver.routines(schema, 100).await.unwrap();
    assert_eq!(first.routines.len(), 100);
    assert!(first.has_more);
    assert_eq!(second.routines.len(), total - 100);
    assert!(!second.has_more);
    assert!(
        first
            .routines
            .iter()
            .all(|a| second.routines.iter().all(|b| a.id != b.id))
    );
    assert!(driver.routines(&"a".repeat(1025), 0).await.is_err());
    assert!(driver.routines("", 1_000_001).await.is_err());
    assert!(driver.routine_definition("1 OR true").await.is_err());
    (first, second)
}

async fn assert_not_invoked(driver: &dyn Session, table: &str) {
    let (tx, mut rx) = mpsc::channel(4);
    driver
        .execute(
            format!("SELECT count(*) FROM {table}"),
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
    assert!(empty, "Browsing must not execute write-capable routines");
}

fn evidence(
    engine: &str,
    first: &RoutinePage,
    second: &RoutinePage,
    definitions: serde_json::Map<String, serde_json::Value>,
) {
    if let Ok(directory) = std::env::var("KLYNDB_ROUTINE_EVIDENCE_DIR") {
        let path = std::path::Path::new(&directory);
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(
            path.join(format!("{engine}-catalog.json")),
            serde_json::to_vec_pretty(
                &serde_json::json!({"first":first,"second":second,"definitions":definitions}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}

#[tokio::test]
async fn sql_server_routines_native_signatures_paging_definitions_and_encryption() {
    let Ok(url) = std::env::var("KLYNDB_TEST_MSSQL_URL") else {
        return;
    };
    let password = std::env::var("KLYNDB_TEST_MSSQL_PASSWORD").unwrap();
    let driver = Arc::new(
        SqlServer::connect(&url, Some(&password), false)
            .await
            .unwrap(),
    );
    let schema = format!("klyndb routines]{}", uuid::Uuid::new_v4().simple());
    let quoted = driver.quote_identifier(&schema);
    fixture(&driver, format!("CREATE SCHEMA {quoted}")).await;
    fixture(&driver, format!("CREATE TABLE {quoted}.writes(id integer); CREATE TYPE {quoted}.price FROM decimal(14,3); CREATE TYPE {quoted}.items AS TABLE(id integer)")).await;
    let mut originals = std::collections::HashMap::new();
    for index in 0..101 {
        let name = format!("item_{index:03}");
        let sql = format!(
            "CREATE FUNCTION {quoted}.{name}() RETURNS integer AS BEGIN RETURN {index}; END"
        );
        fixture(&driver, sql.clone()).await;
        originals.insert(name, sql);
    }
    for (name, sql) in [
        (
            "append_value",
            format!(
                "CREATE PROCEDURE {quoted}.append_value @value integer, @label nvarchar(64) OUTPUT, @amount {quoted}.price, @items {quoted}.items READONLY AS INSERT INTO {quoted}.writes VALUES(@value)"
            ),
        ),
        (
            "literal%_[name",
            format!(
                "CREATE FUNCTION {quoted}.[literal%_[name]() RETURNS nvarchar(64) AS BEGIN RETURN N'<script>é</script>'; END"
            ),
        ),
        (
            "table_values",
            format!(
                "CREATE FUNCTION {quoted}.table_values(@value decimal(18,4)) RETURNS TABLE AS RETURN (SELECT @value AS value)"
            ),
        ),
        (
            "encrypted",
            format!(
                "CREATE PROCEDURE {quoted}.encrypted WITH ENCRYPTION AS INSERT INTO {quoted}.writes VALUES(1)"
            ),
        ),
    ] {
        fixture(&driver, sql.clone()).await;
        originals.insert(name.into(), sql);
    }
    let read_only = SqlServer::connect(&url, Some(&password), true)
        .await
        .unwrap();
    let (first, second) = pages(&read_only, &schema, 105).await;
    let all: Vec<_> = first
        .routines
        .iter()
        .chain(second.routines.iter())
        .collect();
    let procedure = all.iter().find(|r| r.name == "append_value").unwrap();
    assert!(procedure.arguments.contains("@value int"));
    assert!(procedure.arguments.contains("@label nvarchar(64) OUTPUT"));
    assert!(
        procedure
            .arguments
            .contains(&format!("@amount {quoted}.[price]"))
    );
    assert!(
        procedure
            .arguments
            .contains(&format!("@items {quoted}.[items] READONLY"))
    );
    assert!(procedure.returns.is_none());
    let table = all.iter().find(|r| r.name == "table_values").unwrap();
    assert_eq!(table.returns.as_deref(), Some("TABLE"));
    assert_eq!(table.arguments, "@value decimal(18,4)");
    let literal = read_only
        .routines(&format!("{schema}.literal%_[name"), 0)
        .await
        .unwrap();
    assert_eq!(literal.routines.len(), 1);
    assert!(
        read_only
            .routines("%_[' OR 1=1 --", 0)
            .await
            .unwrap()
            .routines
            .is_empty()
    );
    let mut definitions = serde_json::Map::new();
    for routine in &all {
        if routine.name == "encrypted" {
            let error = read_only.routine_definition(&routine.id).await.unwrap_err();
            assert!(error.message.contains("encrypted"));
        } else {
            let definition = read_only.routine_definition(&routine.id).await.unwrap();
            assert_eq!(definition, originals[&routine.name]);
            if routine.name.starts_with("item_") {
                assert_eq!(routine.returns.as_deref(), Some("int"));
            }
            definitions.insert(routine.id.clone(), definition.into());
        }
    }
    assert_not_invoked(&read_only, &format!("{quoted}.writes")).await;
    assert!(read_only.routine_definition("0").await.is_err());
    evidence("mssql", &first, &second, definitions);
    for routine in all {
        fixture(
            &driver,
            format!(
                "DROP {} {quoted}.{}",
                routine.kind,
                driver.quote_identifier(&routine.name)
            ),
        )
        .await;
    }
    fixture(&driver, format!("DROP TABLE {quoted}.writes; DROP TYPE {quoted}.items; DROP TYPE {quoted}.price; DROP SCHEMA {quoted}")).await;
    assert!(
        read_only
            .routine_definition(&literal.routines[0].id)
            .await
            .is_err()
    );
    read_only.disconnect().await.unwrap();
    driver.disconnect().await.unwrap();
}
