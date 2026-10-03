use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{Cell, KeyValue};

#[tokio::test]
async fn redis_connection_testing_production_confirmation_and_reconnect() {
    let Ok(url) = std::env::var("KLYNDB_TEST_REDIS_URL") else {
        return;
    };
    let password =
        std::env::var("KLYNDB_TEST_REDIS_PASSWORD").expect("Set disposable Redis password");
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut connection = Connection {
        id: String::new(),
        name: "Redis production contract".into(),
        engine: "redis".into(),
        address: url,
        environment: "production".into(),
        group: String::new(),
        color: "#93d4b5".into(),
        favorite: false,
        read_only: false,
        create_file: false,
    };
    let capabilities = engine
        .test_connection(connection.clone(), Some(password.clone()), None, None)
        .await
        .unwrap();
    assert!(capabilities.key_value);
    assert!(!capabilities.transactions);
    assert!(engine.store.connections().unwrap().is_empty());
    connection.validate().unwrap();
    engine.store.save(&connection).unwrap();
    engine
        .connect(&connection.id, Some(password.clone()), None, None)
        .await
        .unwrap();
    let key = format!("klyndb:core:{}", uuid::Uuid::new_v4().simple());
    let write = serde_json::json!(["SET", key, "<script>é</script>"]).to_string();
    assert!(
        engine
            .key_command(&connection.id, &write, false)
            .await
            .unwrap_err()
            .message
            .contains("Confirmation required")
    );
    let get = serde_json::json!(["GET", key]).to_string();
    assert!(matches!(
        engine
            .key_command(&connection.id, &get, false)
            .await
            .unwrap(),
        KeyValue::Cell(Cell::Null)
    ));
    connection.environment = "development".into();
    engine.store.save(&connection).unwrap();
    assert!(
        engine
            .key_command(&connection.id, &write, false)
            .await
            .is_err(),
        "Open-session production config must remain pinned"
    );
    engine
        .key_command(&connection.id, &write, true)
        .await
        .unwrap();
    assert!(
        matches!(engine.key_command(&connection.id, &get, false).await.unwrap(), KeyValue::Cell(Cell::Text(ref s)) if s == "<script>é</script>")
    );
    for blocked in [
        "AUTH", "SELECT", "CONFIG", "FLUSHDB", "EVAL", "MULTI", "BLPOP",
    ] {
        assert!(
            engine
                .key_command(
                    &connection.id,
                    &serde_json::json!([blocked]).to_string(),
                    true
                )
                .await
                .is_err()
        );
    }
    assert!(
        !serde_json::to_string(&engine.store.connections().unwrap())
            .unwrap()
            .contains(&password)
    );
    engine.disconnect(&connection.id).await.unwrap();
    assert!(engine.driver(&connection.id).await.is_err());
    engine
        .connect(&connection.id, Some(password), None, None)
        .await
        .unwrap();
    engine
        .key_command(
            &connection.id,
            &serde_json::json!(["DEL", key]).to_string(),
            false,
        )
        .await
        .unwrap();
    engine.disconnect(&connection.id).await.unwrap();
}
