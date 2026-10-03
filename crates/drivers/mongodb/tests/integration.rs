use bson::{Bson, Document, doc};
use klyndb_driver_api::*;
use klyndb_mongodb::MongoDb;
use mongodb::{Client, options::ClientOptions};

async fn seed_client() -> Client {
    let url = std::env::var("KLYNDB_TEST_MONGODB_URL").expect("Set disposable MongoDB URL");
    let mut url = url::Url::parse(&url).unwrap();
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
    let mut options = ClientOptions::parse(url.as_str()).await.unwrap();
    options.credential.as_mut().unwrap().password =
        Some(std::env::var("KLYNDB_TEST_MONGODB_PASSWORD").unwrap());
    options.direct_connection = Some(true);
    Client::with_options(options).unwrap()
}
fn query(collection: &str) -> DocumentQuery {
    DocumentQuery {
        database: "klyndb_fixture".into(),
        collection: collection.into(),
        text: "{}".into(),
        sort: "{}".into(),
        aggregate: false,
        offset: 0,
    }
}
#[tokio::test]
#[ignore = "Requires a disposable MongoDB server and private fixture credentials"]
async fn real_mongodb_documents_types_paging_indexes_aggregation_and_editing() {
    let url = std::env::var("KLYNDB_TEST_MONGODB_URL").unwrap();
    let password = std::env::var("KLYNDB_TEST_MONGODB_PASSWORD").unwrap();
    let rejected = MongoDb::connect(&url, Some("invalid-fixture-password"), false, None)
        .await
        .err()
        .expect("Invalid credentials must fail");
    assert!(rejected.message.contains("authentication failed"));
    assert!(!rejected.message.contains("invalid-fixture-password"));
    let driver = MongoDb::connect(&url, Some(&password), false, None)
        .await
        .unwrap();
    assert!(driver.capabilities().document_queries);
    assert!(!driver.capabilities().table_browse);
    let seed = seed_client().await;
    let name = format!("contract_{}", uuid::Uuid::new_v4().simple());
    let collection = seed
        .database("klyndb_fixture")
        .collection::<Document>(&name);
    let mut original = doc! { "_id": 1, "z_order": "first", "a_order": "second", "integer": i64::MAX, "decimal": bson::Decimal128::from_bytes([0;16]), "date": bson::DateTime::from_millis(1700000000000), "bytes": Bson::Binary(bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![97,0,255] }), "html": "<script>é</script>" };
    collection.insert_one(original.clone()).await.unwrap();
    collection
        .insert_many((2..=205).map(|id| doc! { "_id": id, "bucket": id % 2 }))
        .await
        .unwrap();
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "bucket": 1 })
                .build(),
        )
        .await
        .unwrap();
    assert!(
        driver
            .document_databases()
            .await
            .unwrap()
            .iter()
            .any(|n| n == "klyndb_fixture")
    );
    assert!(
        driver
            .document_collections("klyndb_fixture")
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == name)
    );
    assert!(
        driver
            .document_indexes("klyndb_fixture", &name)
            .await
            .unwrap()
            .iter()
            .any(|index| index.contains("bucket_1"))
    );
    let page = driver.document_query(query(&name)).await.unwrap();
    assert_eq!(page.documents.len(), 100);
    assert!(page.has_more);
    assert!(page.documents[0].json.contains("9223372036854775807"));
    assert!(page.documents[0].json.contains("$binary"));
    assert!(page.documents[0].json.contains("1700000000000"));
    let mut q = query(&name);
    q.text = r#"{"bucket":{"$exists":true}}"#.into();
    q.sort = r#"{"bucket":1,"_id":-1}"#.into();
    let sorted = driver.document_query(q).await.unwrap();
    let first: serde_json::Value = serde_json::from_str(&sorted.documents[0].json).unwrap();
    assert_eq!(first["_id"]["$numberInt"], "204");
    let mut q = query(&name);
    q.offset = 100;
    let middle = driver.document_query(q).await.unwrap();
    assert_eq!(middle.documents.len(), 100);
    assert!(middle.has_more);
    let mut q = query(&name);
    q.offset = 200;
    let last = driver.document_query(q).await.unwrap();
    assert_eq!(last.documents.len(), 5);
    assert!(!last.has_more);
    if let Ok(path) = std::env::var("KLYNDB_TEST_MONGODB_OBSERVATIONS_PATH") {
        let observed = serde_json::json!({
            "collections": driver.document_collections("klyndb_fixture").await.unwrap(),
            "collection": name,
            "indexes": driver.document_indexes("klyndb_fixture", &name).await.unwrap(),
            "first": page, "middle": middle, "last": last
        });
        std::fs::write(path, serde_json::to_vec(&observed).unwrap()).unwrap();
    }
    let mut q = query(&name);
    q.text = r#"{"_id":1}"#.into();
    let row = driver
        .document_query(q.clone())
        .await
        .unwrap()
        .documents
        .remove(0);
    // Change an otherwise unedited field: the original whole-document predicate must reject both write paths.
    collection
        .update_one(doc! { "_id": 1 }, doc! { "$set": { "new_field": true } })
        .await
        .unwrap();
    let edit = DocumentChange::Replace {
        snapshot: row.snapshot.clone().unwrap(),
        json: row.json.clone(),
    };
    assert!(
        driver
            .document_change("klyndb_fixture", &name, edit)
            .await
            .unwrap_err()
            .message
            .contains("changed")
    );
    assert!(
        driver
            .document_change(
                "klyndb_fixture",
                &name,
                DocumentChange::Delete {
                    snapshot: row.snapshot.unwrap()
                }
            )
            .await
            .is_err()
    );
    let row = driver
        .document_query(q.clone())
        .await
        .unwrap()
        .documents
        .remove(0);
    original.insert("new_field", true);
    original.insert("html", "Updated safely");
    let edit = DocumentChange::Replace {
        snapshot: row.snapshot.unwrap(),
        json: serde_json::to_string(&Bson::Document(original).into_canonical_extjson()).unwrap(),
    };
    assert_eq!(
        driver
            .document_change("klyndb_fixture", &name, edit)
            .await
            .unwrap()
            .affected,
        1
    );
    assert_eq!(
        collection
            .find_one(doc! { "_id": 1 })
            .await
            .unwrap()
            .unwrap()
            .get_str("html")
            .unwrap(),
        "Updated safely"
    );
    let row = driver.document_query(q).await.unwrap().documents.remove(0);
    assert_eq!(
        driver
            .document_change(
                "klyndb_fixture",
                &name,
                DocumentChange::Delete {
                    snapshot: row.snapshot.unwrap()
                }
            )
            .await
            .unwrap()
            .affected,
        1
    );
    assert!(
        collection
            .find_one(doc! { "_id": 1 })
            .await
            .unwrap()
            .is_none()
    );
    let insert = DocumentChange::Insert {
        json: r#"{"_id":999,"value":{"$numberLong":"9223372036854775807"}}"#.into(),
    };
    assert_eq!(
        driver
            .document_change("klyndb_fixture", &name, insert.clone())
            .await
            .unwrap()
            .affected,
        1
    );
    assert!(
        driver
            .document_change("klyndb_fixture", &name, insert.clone())
            .await
            .is_err()
    );
    let readonly = MongoDb::connect(&url, Some(&password), true, None)
        .await
        .unwrap();
    assert!(
        readonly
            .document_change("klyndb_fixture", &name, insert.clone())
            .await
            .unwrap_err()
            .message
            .contains("read-only")
    );
    readonly.disconnect().await.unwrap();
    let mut q = query(&name);
    q.aggregate = true;
    q.text = r#"[{"$match":{"bucket":1}},{"$group":{"_id":"$bucket","total":{"$sum":1}}}]"#.into();
    let aggregated = driver.document_query(q.clone()).await.unwrap();
    assert_eq!(aggregated.documents.len(), 1);
    assert!(aggregated.documents[0].snapshot.is_none());
    assert!(aggregated.documents[0].json.contains("102"));
    for text in [
        r#"[{"$out":"bad"}]"#,
        r#"[{"$facet":{"x":[{"$merge":"bad"}]}}]"#,
        r#"[{"$match":{"$where":"return true"}}]"#,
    ] {
        q.text = text.into();
        assert!(driver.document_query(q.clone()).await.is_err());
    }
    let reader_url = url.replace("klyndb_writer@", "klyndb_reader@");
    let reader_password = std::env::var("KLYNDB_TEST_MONGODB_READONLY_PASSWORD").unwrap();
    let reader = MongoDb::connect(&reader_url, Some(&reader_password), false, None)
        .await
        .unwrap();
    assert!(
        !reader
            .document_query(query(&name))
            .await
            .unwrap()
            .documents
            .is_empty()
    );
    let rejected = reader
        .document_change("klyndb_fixture", &name, insert)
        .await
        .unwrap_err()
        .message;
    assert!(!rejected.contains(&reader_password));
    reader.disconnect().await.unwrap();
    let view = format!("view_{}", uuid::Uuid::new_v4().simple());
    seed.database("klyndb_fixture")
        .create_collection(&view)
        .view_on(&name)
        .pipeline(vec![doc! { "$match": { "bucket": 1 } }])
        .await
        .unwrap();
    assert!(
        !driver
            .document_query(query(&view))
            .await
            .unwrap()
            .documents
            .is_empty()
    );
    assert!(
        driver
            .document_change(
                "klyndb_fixture",
                &view,
                DocumentChange::Insert { json: "{}".into() }
            )
            .await
            .is_err()
    );
    let oversized = "x".repeat(1024 * 1024 + 1);
    collection
        .insert_one(doc! { "_id": 1001, "large": oversized })
        .await
        .unwrap();
    let mut q = query(&name);
    q.text = r#"{"_id":1001}"#.into();
    assert!(
        driver
            .document_query(q)
            .await
            .unwrap_err()
            .message
            .contains("1 MiB")
    );
    let mut q = query(&name);
    q.text = "[]".into();
    assert!(driver.document_query(q).await.is_err());
    seed.database("klyndb_fixture")
        .collection::<Document>(&view)
        .drop()
        .await
        .unwrap();
    collection.drop().await.unwrap();
    driver.disconnect().await.unwrap();
    assert!(driver.document_query(query(&name)).await.is_err());
    seed.shutdown().immediate(true).await;
}

#[tokio::test]
#[ignore = "Requires disposable TLS/mTLS MongoDB servers and private test identities"]
async fn real_mongodb_verified_tls_and_client_identity() {
    let password = std::env::var("KLYNDB_TEST_MONGODB_PASSWORD").unwrap();
    let ca = std::path::PathBuf::from(std::env::var("KLYNDB_TEST_TLS_CERT_DIR").unwrap());
    let identity =
        std::path::PathBuf::from(std::env::var("KLYNDB_TEST_MONGODB_IDENTITY_DIR").unwrap());
    let tls_url = std::env::var("KLYNDB_TEST_TLS_MONGODB_URL").unwrap();
    let mtls_url = std::env::var("KLYNDB_TEST_MTLS_MONGODB_URL").unwrap();
    let set = |address: &str, root: &str, client: Option<&str>| {
        let mut u = url::Url::parse(address).unwrap();
        u.query_pairs_mut().append_pair("connect_timeout", "2");
        u.query_pairs_mut()
            .append_pair("sslrootcert", ca.join(root).to_str().unwrap());
        if let Some(client) = client {
            u.query_pairs_mut()
                .append_pair("sslidentity", identity.join(client).to_str().unwrap());
        }
        u.to_string()
    };
    let address = set(&tls_url, "ca.pem", None).replace("connect_timeout=2", "connect_timeout=10");
    let driver = MongoDb::connect(&address, Some(&password), true, None)
        .await
        .unwrap();
    assert_eq!(
        driver.transaction_state().await.unwrap(),
        TransactionState::Idle
    );
    driver.disconnect().await.unwrap();
    for bad in [
        set(&tls_url, "other-ca.pem", None),
        address.replace("localhost", "127.0.0.1"),
        set(&mtls_url, "ca.pem", None),
        set(&mtls_url, "ca.pem", Some("wrong-client.pem")),
    ] {
        assert!(
            MongoDb::connect(&bad, Some(&password), true, None)
                .await
                .is_err()
        );
    }
    for (client, key_password) in [
        ("client.pem", None),
        (
            "encrypted-client.pem",
            Some(std::env::var("KLYNDB_TEST_MONGODB_IDENTITY_PASSWORD").unwrap()),
        ),
    ] {
        let address = set(&mtls_url, "ca.pem", Some(client))
            .replace("connect_timeout=2", "connect_timeout=10");
        let connected =
            MongoDb::connect(&address, Some(&password), true, key_password.as_deref()).await;
        let driver = connected.expect(client);
        assert_eq!(
            driver.transaction_state().await.unwrap(),
            TransactionState::Idle
        );
        driver.disconnect().await.unwrap();
    }
    assert!(
        MongoDb::connect(
            &set(&mtls_url, "ca.pem", Some("encrypted-client.pem")),
            Some(&password),
            true,
            Some("wrong-password")
        )
        .await
        .is_err()
    );
}
