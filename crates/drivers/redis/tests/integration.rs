use klyndb_driver_api::{Cell, KeyValue, Session};
use klyndb_redis::Redis;
use redis::IntoConnectionInfo;
use std::collections::HashSet;

fn configuration() -> (String, String) {
    (
        std::env::var("KLYNDB_TEST_REDIS_URL").expect("Set disposable Redis URL"),
        std::env::var("KLYNDB_TEST_REDIS_PASSWORD").expect("Set disposable Redis password"),
    )
}
async fn command(driver: &Redis, args: &[&str]) -> KeyValue {
    driver
        .key_command(&serde_json::to_string(args).unwrap())
        .await
        .unwrap()
}
fn leaf(value: KeyValue) -> Cell {
    let KeyValue::Cell(value) = value else {
        panic!("expected native scalar")
    };
    value
}

#[tokio::test]
#[ignore = "Requires disposable authenticated Redis"]
async fn real_redis_types_paging_native_commands_readonly_and_limits() {
    let (url, password) = configuration();
    let driver = Redis::connect(&url, Some(&password), false, None)
        .await
        .unwrap();
    assert!(driver.capabilities().key_value);
    assert!(!driver.capabilities().table_browse);
    assert!(
        Redis::connect(&url, Some("wrong-fixture-secret"), false, None)
            .await
            .is_err()
    );
    let prefix = format!("klyndb:{}:", uuid::Uuid::new_v4().simple());
    let keys: Vec<_> = [
        "string",
        "hash",
        "list",
        "set",
        "zset",
        "stream",
        "empty",
        "binary",
        "wide",
        "max",
        "oversized",
        "literal[*]?",
        "expiring",
    ]
    .map(|name| format!("{prefix}{name}"))
    .into();
    command(&driver, &["SET", &keys[0], "<script>é</script>"]).await;
    command(
        &driver,
        &["HSET", &keys[1], "name", "Ada", "role", "developer"],
    )
    .await;
    for index in 0..205 {
        let text = index.to_string();
        command(&driver, &["RPUSH", &keys[2], &text]).await;
        command(&driver, &["SADD", &keys[3], &text]).await;
        command(&driver, &["ZADD", &keys[4], &text, &text]).await;
        command(&driver, &["XADD", &keys[5], "*", "index", &text]).await;
    }
    command(&driver, &["SET", &keys[6], ""]).await;
    command(&driver, &["SET", &keys[9], "9223372036854775806"]).await;
    assert_eq!(
        leaf(command(&driver, &["INCR", &keys[9]]).await),
        Cell::Number(i64::MAX.to_string())
    );
    command(&driver, &["SET", &keys[11], "literal key"]).await;
    command(&driver, &["SET", &keys[12], "soon", "PX", "60000"]).await;
    let address = url::Url::parse(&url).unwrap();
    let info = (address.host_str().unwrap(), address.port().unwrap_or(6379))
        .into_connection_info()
        .unwrap()
        .set_redis_settings(
            redis::RedisConnectionInfo::default()
                .set_username(address.username())
                .set_password(&password)
                .set_skip_set_lib_name(),
        );
    let mut fixture = redis::Client::open(info)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let binary = [b'a', 0, 255];
    let binary_key = format!("{prefix}raw:")
        .into_bytes()
        .into_iter()
        .chain([0, 255])
        .collect::<Vec<_>>();
    let _: () = redis::cmd("SET")
        .arg(&keys[7])
        .arg(binary.as_slice())
        .query_async(&mut fixture)
        .await
        .unwrap();
    let _: () = redis::cmd("SET")
        .arg(&binary_key)
        .arg(binary.as_slice())
        .query_async(&mut fixture)
        .await
        .unwrap();
    let _: () = redis::cmd("SET")
        .arg(&keys[8])
        .arg(vec![b'x'; 65540])
        .query_async(&mut fixture)
        .await
        .unwrap();
    let _: () = redis::cmd("SET")
        .arg(&keys[10])
        .arg(vec![b'x'; 9 * 1024 * 1024])
        .query_async(&mut fixture)
        .await
        .unwrap();
    let mut found = HashSet::new();
    let mut cursor = "0".to_owned();
    loop {
        let page = driver
            .scan_keys(&format!("{prefix}*"), &cursor)
            .await
            .unwrap();
        for key in page.keys {
            found.insert(serde_json::to_string(&key.key).unwrap());
        }
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    assert_eq!(found.len(), 14);
    assert!(
        found.contains(&serde_json::to_string(&Cell::Binary(hex::encode(&binary_key))).unwrap())
    );
    for (key, kind, length) in [
        (0, "string", "19"),
        (1, "hash", "2"),
        (2, "list", "205"),
        (3, "set", "205"),
        (4, "zset", "205"),
        (5, "stream", "205"),
        (6, "string", "0"),
    ] {
        let result = driver
            .inspect_key(&Cell::Text(keys[key].clone()), "")
            .await
            .unwrap();
        assert_eq!(result.entry.data_type, kind);
        assert_eq!(result.length, length);
        assert_eq!(result.entry.ttl_ms, "-1");
    }
    let binary_result = driver
        .inspect_key(&Cell::Binary(hex::encode(&binary_key)), "")
        .await
        .unwrap();
    assert_eq!(leaf(binary_result.value), Cell::Binary("6100ff".into()));
    let first = driver
        .inspect_key(&Cell::Text(keys[8].clone()), "")
        .await
        .unwrap();
    assert_eq!(leaf(first.value).byte_len(), 65536);
    let second = driver
        .inspect_key(&Cell::Text(keys[8].clone()), &first.next.unwrap())
        .await
        .unwrap();
    assert_eq!(leaf(second.value).byte_len(), 4);
    assert!(second.next.is_none());
    for key in [2, 3, 4, 5] {
        let mut position = String::new();
        let mut pages = 0;
        let mut items = HashSet::new();
        loop {
            let page = driver
                .inspect_key(&Cell::Text(keys[key].clone()), &position)
                .await
                .unwrap();
            let KeyValue::Array(values) = page.value else {
                panic!("collection array lost")
            };
            for (index, value) in values.into_iter().enumerate() {
                if key == 4 && index % 2 == 1 {
                    continue;
                }
                items.insert(serde_json::to_string(&value).unwrap());
            }
            pages += 1;
            assert!(pages < 20);
            let Some(next) = page.next else {
                break;
            };
            position = next;
        }
        assert_eq!(items.len(), 205);
    }
    let expiring = driver
        .inspect_key(&Cell::Text(keys[12].clone()), "")
        .await
        .unwrap();
    assert!(expiring.entry.ttl_ms.parse::<u64>().unwrap() > 0);
    let missing = driver
        .inspect_key(&Cell::Text(format!("{prefix}missing")), "")
        .await
        .unwrap();
    assert_eq!(missing.entry.data_type, "none");
    assert_eq!(missing.entry.ttl_ms, "-2");
    assert_eq!(leaf(missing.value), Cell::Null);
    assert!(
        driver
            .key_command(&serde_json::json!(["HGET", &keys[0], "field"]).to_string())
            .await
            .is_err()
    );
    assert_eq!(
        leaf(command(&driver, &["PING"]).await),
        Cell::Text("PONG".into())
    );
    assert!(
        driver
            .inspect_key(&Cell::Text(keys[0].clone()), "-1")
            .await
            .is_err()
    );
    let readonly = Redis::connect(&url, Some(&password), true, None)
        .await
        .unwrap();
    assert!(
        readonly
            .key_command(&serde_json::json!(["SET", &keys[0], "wrong"]).to_string())
            .await
            .is_err()
    );
    assert_eq!(
        leaf(command(&readonly, &["GET", &keys[0]]).await),
        Cell::Text("<script>é</script>".into())
    );
    let mut reader_url = address.clone();
    reader_url.set_username("reader").unwrap();
    let reader_password = std::env::var("KLYNDB_TEST_REDIS_READONLY_PASSWORD")
        .expect("Set disposable read-only ACL password");
    let reader = Redis::connect(reader_url.as_str(), Some(&reader_password), false, None)
        .await
        .unwrap();
    assert!(
        reader
            .key_command(&serde_json::json!(["SET", &keys[0], "wrong"]).to_string())
            .await
            .is_err()
    );
    assert_eq!(
        leaf(command(&reader, &["PING"]).await),
        Cell::Text("PONG".into())
    );
    let limited = Redis::connect(&url, Some(&password), false, None)
        .await
        .unwrap();
    let error = limited
        .key_command(&serde_json::json!(["GET", &keys[10]]).to_string())
        .await
        .unwrap_err();
    assert!(error.message.contains("connection closed"));
    assert!(!error.message.contains(&password));
    assert!(limited.key_command("[\"PING\"]").await.is_err());
    assert_eq!(
        leaf(command(&driver, &["PING"]).await),
        Cell::Text("PONG".into())
    );
    if let Ok(path) = std::env::var("KLYNDB_REDIS_EVIDENCE_FILE") {
        let listing = driver.scan_keys(&format!("{prefix}*"), "0").await.unwrap();
        let mut values = vec![];
        for key in &keys[..9] {
            values.push(
                driver
                    .inspect_key(&Cell::Text(key.clone()), "")
                    .await
                    .unwrap(),
            );
        }
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({"listing":listing,"values":values}))
                .unwrap(),
        )
        .unwrap();
    }
    let _: u64 = redis::cmd("DEL")
        .arg(&keys)
        .arg(&binary_key)
        .query_async(&mut fixture)
        .await
        .unwrap();
    driver.disconnect().await.unwrap();
    readonly.disconnect().await.unwrap();
    reader.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires disposable Redis TLS/mTLS and CA fixtures"]
async fn real_redis_verified_tls_and_client_identity() {
    let (_, password) = configuration();
    let directory = std::path::PathBuf::from(
        std::env::var("KLYNDB_TEST_TLS_CERT_DIR").expect("Set CA directory"),
    );
    let secure = |base: &str, ca: &str, identity: Option<&str>| {
        let mut url = url::Url::parse(base).unwrap();
        url.query_pairs_mut()
            .append_pair("sslrootcert", &directory.join(ca).to_string_lossy());
        if let Some(identity) = identity {
            url.query_pairs_mut()
                .append_pair("sslidentity", &directory.join(identity).to_string_lossy());
        }
        url.to_string()
    };
    let base = std::env::var("KLYNDB_TEST_TLS_REDIS_URL").expect("Set TLS URL");
    assert!(
        Redis::connect(&base, Some(&password), false, None)
            .await
            .is_err()
    );
    assert!(
        Redis::connect(
            &secure(&base, "other-ca.pem", None),
            Some(&password),
            false,
            None
        )
        .await
        .is_err()
    );
    let good = secure(&base, "ca.pem", None);
    let mut wrong_host = url::Url::parse(&good).unwrap();
    wrong_host.set_host(Some("127.0.0.1")).unwrap();
    assert!(
        Redis::connect(wrong_host.as_str(), Some(&password), false, None)
            .await
            .is_err()
    );
    let driver = Redis::connect(&good, Some(&password), false, None)
        .await
        .unwrap();
    assert_eq!(
        leaf(command(&driver, &["PING"]).await),
        Cell::Text("PONG".into())
    );
    driver.disconnect().await.unwrap();
    let mtls = std::env::var("KLYNDB_TEST_MTLS_REDIS_URL").expect("Set mTLS URL");
    assert!(
        Redis::connect(&secure(&mtls, "ca.pem", None), Some(&password), false, None)
            .await
            .is_err()
    );
    assert!(
        Redis::connect(
            &secure(&mtls, "ca.pem", Some("wrong-client.p12")),
            Some(&password),
            false,
            Some("klyndb-fixture-only")
        )
        .await
        .is_err()
    );
    assert!(
        Redis::connect(
            &secure(&mtls, "ca.pem", Some("client.p12")),
            Some(&password),
            false,
            Some("wrong-password")
        )
        .await
        .is_err()
    );
    let driver = Redis::connect(
        &secure(&mtls, "ca.pem", Some("client.p12")),
        Some(&password),
        false,
        Some("klyndb-fixture-only"),
    )
    .await
    .unwrap();
    assert_eq!(
        leaf(command(&driver, &["PING"]).await),
        Cell::Text("PONG".into())
    );
    driver.disconnect().await.unwrap();
}
