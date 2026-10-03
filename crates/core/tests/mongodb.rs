use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{DocumentChange, DocumentQuery};

#[tokio::test]
async fn mongodb_production_readonly_pinned_configuration_and_reconnect() {
    let Ok(url) = std::env::var("KLYNDB_TEST_MONGODB_URL") else {
        return;
    };
    let password = std::env::var("KLYNDB_TEST_MONGODB_PASSWORD").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut connection = Connection {
        id: String::new(),
        name: "MongoDB production contract".into(),
        engine: "mongodb".into(),
        address: url,
        environment: "production".into(),
        group: String::new(),
        color: "#93d4b5".into(),
        favorite: false,
        read_only: false,
        create_file: false,
    };
    connection.validate().unwrap();
    engine.store.save(&connection).unwrap();
    let capabilities = engine
        .connect(&connection.id, Some(password.clone()), None, None)
        .await
        .unwrap();
    assert!(capabilities.document_queries);
    let id = uuid::Uuid::new_v4().to_string();
    let change = DocumentChange::Insert {
        json: serde_json::json!({"_id":id,"value":"guarded"}).to_string(),
    };
    assert!(
        engine
            .document_change(
                &connection.id,
                "klyndb_fixture",
                "core_contract",
                change.clone(),
                false
            )
            .await
            .unwrap_err()
            .message
            .contains("Confirmation required")
    );
    connection.environment = "development".into();
    engine.store.save(&connection).unwrap();
    assert!(
        engine
            .document_change(
                &connection.id,
                "klyndb_fixture",
                "core_contract",
                change.clone(),
                false
            )
            .await
            .is_err(),
        "Open session configuration stays pinned"
    );
    engine
        .document_change(
            &connection.id,
            "klyndb_fixture",
            "core_contract",
            change,
            true,
        )
        .await
        .unwrap();
    let query = DocumentQuery {
        database: "klyndb_fixture".into(),
        collection: "core_contract".into(),
        text: serde_json::json!({"_id":id}).to_string(),
        sort: "{}".into(),
        aggregate: false,
        offset: 0,
    };
    let row = engine
        .driver(&connection.id)
        .await
        .unwrap()
        .document_query(query.clone())
        .await
        .unwrap()
        .documents
        .remove(0);
    engine
        .document_change(
            &connection.id,
            "klyndb_fixture",
            "core_contract",
            DocumentChange::Delete {
                snapshot: row.snapshot.unwrap(),
            },
            true,
        )
        .await
        .unwrap();
    engine.disconnect(&connection.id).await.unwrap();
    assert!(engine.driver(&connection.id).await.is_err());
    connection.read_only = true;
    engine.store.save(&connection).unwrap();
    engine
        .connect(&connection.id, Some(password), None, None)
        .await
        .unwrap();
    assert!(
        engine
            .document_change(
                &connection.id,
                "klyndb_fixture",
                "core_contract",
                DocumentChange::Insert { json: "{}".into() },
                true
            )
            .await
            .unwrap_err()
            .message
            .contains("read-only")
    );
    engine.disconnect(&connection.id).await.unwrap();
}
