use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::Row;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const SECRET: &str = "klyndb-ssh-fixture-only";
const TLS_SECRET: &str = "klyndb-fixture-only";
async fn wait(engine: &Engine, id: &str) {
    let began = Instant::now();
    while !engine.job(id).unwrap().status().unwrap().done {
        assert!(began.elapsed() < Duration::from_secs(15));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn query(engine: &Engine, id: &str, sql: &str) -> Vec<Row> {
    let job = engine
        .start(id.into(), sql.into(), 50, 10, true)
        .await
        .unwrap();
    wait(engine, &job).await;
    assert!(
        engine.job(&job).unwrap().status().unwrap().error.is_none(),
        "{:?}",
        engine.job(&job).unwrap().status().unwrap()
    );
    let rows = engine.job(&job).unwrap().page(0, 0, 50).unwrap();
    engine.release(&job).unwrap();
    rows
}
async fn ssh_contract(variable: &str, kind: &str) {
    let base = std::env::var(variable).expect("Set a disposable certificate-required database URL");
    let ssh_dir =
        PathBuf::from(std::env::var("KLYNDB_TEST_SSH_DIR").expect("Set the SSH fixture directory"));
    let tls_dir = PathBuf::from(std::env::var("KLYNDB_TEST_TLS_CERT_DIR").unwrap());
    let fingerprint =
        std::env::var("KLYNDB_TEST_SSH_FINGERPRINT").expect("Set the verified fixture fingerprint");
    let user =
        std::env::var("KLYNDB_TEST_SSH_USER").expect("Set the forwarding-only SSH fixture user");
    let make = |identity: &str, pin: &str| {
        let mut url = url::Url::parse(&base).unwrap();
        url.query_pairs_mut()
            .clear()
            .append_pair(
                if kind == "postgres" { "sslmode" } else { "tls" },
                if kind == "postgres" {
                    "require"
                } else {
                    "required"
                },
            )
            .append_pair("connect_timeout", "10")
            .append_pair("sslrootcert", &tls_dir.join("ca.pem").to_string_lossy())
            .append_pair("sslidentity", &tls_dir.join("client.p12").to_string_lossy())
            .append_pair("ssh_host", "127.0.0.1")
            .append_pair("ssh_port", "15441")
            .append_pair("ssh_user", &user)
            .append_pair("ssh_auth", "key")
            .append_pair("ssh_identity", &ssh_dir.join(identity).to_string_lossy())
            .append_pair("ssh_fingerprint", pin);
        Connection {
            id: String::new(),
            name: "SSH contract".into(),
            engine: kind.into(),
            address: url.to_string(),
            environment: "development".into(),
            group: String::new(),
            color: "#79c7a4".into(),
            favorite: false,
            read_only: false,
            create_file: false,
        }
    };
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&state.path().join("state.db")).unwrap());
    for (draft, password) in [
        (
            make("encrypted_key", &format!("SHA256:{}", "A".repeat(43))),
            SECRET,
        ),
        (
            make("encrypted_key", &fingerprint),
            "wrong-ssh-key-password",
        ),
        (make("host_key", &fingerprint), ""), // Correct host trust, unauthorized client key.
        (make("missing_key", &fingerprint), SECRET),
    ] {
        let error = engine
            .test_connection(
                draft,
                Some(String::new()),
                Some(TLS_SECRET.into()),
                Some(password.into()),
            )
            .await
            .unwrap_err();
        assert!(
            !error.message.contains(SECRET) && !error.message.contains("wrong-ssh-key-password")
        );
    }
    let mut wrong_name = make("encrypted_key", &fingerprint);
    let mut url = url::Url::parse(&wrong_name.address).unwrap();
    url.set_host(Some("127.0.0.1")).unwrap();
    wrong_name.address = url.to_string();
    // The fixture explicitly allows this forwarding target, so rejection must occur at database TLS.
    let error = engine
        .test_connection(
            wrong_name,
            Some(String::new()),
            Some(TLS_SECRET.into()),
            Some(SECRET.into()),
        )
        .await
        .unwrap_err();
    assert!(
        !error.message.contains("SSH connection failed"),
        "{}",
        error.message
    );
    let mut wrong_ca = make("encrypted_key", &fingerprint);
    let mut url = url::Url::parse(&wrong_ca.address).unwrap();
    let options: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k != "sslrootcert")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(options)
        .append_pair(
            "sslrootcert",
            &tls_dir.join("other-ca.pem").to_string_lossy(),
        );
    wrong_ca.address = url.to_string();
    assert!(
        engine
            .test_connection(
                wrong_ca,
                Some(String::new()),
                Some(TLS_SECRET.into()),
                Some(SECRET.into())
            )
            .await
            .is_err()
    );
    if std::env::var_os("KLYNDB_TEST_SSH_AGENT").is_some() {
        let mut agent = make("encrypted_key", &fingerprint);
        let mut url = url::Url::parse(&agent.address).unwrap();
        let options: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| !["ssh_auth", "ssh_identity"].contains(&k.as_ref()))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(options)
            .append_pair("ssh_auth", "agent");
        agent.address = url.to_string();
        engine
            .test_connection(agent, Some(String::new()), Some(TLS_SECRET.into()), None)
            .await
            .unwrap();
    }
    for (identity, password) in [("rsa_key", SECRET), ("ec_key", "")] {
        engine
            .test_connection(
                make(identity, &fingerprint),
                Some(String::new()),
                Some(TLS_SECRET.into()),
                Some(password.into()),
            )
            .await
            .unwrap();
    }
    let connection = make("encrypted_key", &fingerprint);
    engine
        .test_connection(
            connection.clone(),
            Some(String::new()),
            Some(TLS_SECRET.into()),
            Some(SECRET.into()),
        )
        .await
        .unwrap();
    assert!(engine.store.connections().unwrap().is_empty());
    let mut connection = connection;
    connection.validate().unwrap();
    engine.store.save(&connection).unwrap();
    let metadata = serde_json::to_string(&engine.store.connections().unwrap()).unwrap();
    assert!(!metadata.contains(SECRET) && !metadata.contains(TLS_SECRET));
    engine
        .connect(
            &connection.id,
            Some(String::new()),
            Some(TLS_SECRET.into()),
            Some(SECRET.into()),
        )
        .await
        .unwrap();
    if kind == "postgres" {
        let rows = query(
            &engine,
            &connection.id,
            "SELECT ssl, client_dn FROM pg_stat_ssl WHERE pid=pg_backend_pid()",
        )
        .await;
        assert_eq!(rows[0][0].text(), "t");
        assert!(rows[0][1].text().contains("klyndb_mtls"));
    } else {
        let rows = query(
            &engine,
            &connection.id,
            "SHOW SESSION STATUS LIKE 'Ssl_cipher'",
        )
        .await;
        assert!(!rows[0][1].text().is_empty());
    }
    assert_eq!(
        query(&engine, &connection.id, "SELECT 42").await[0][0].text(),
        "42"
    );
    let large = query(&engine, &connection.id, "SELECT REPEAT('z', 2097152)").await;
    assert_eq!(large[0][0].text().len(), 2097152);
    let table = format!("ssh_{}", uuid::Uuid::new_v4().simple());
    query(
        &engine,
        &connection.id,
        &format!("CREATE TABLE {table}(id INT PRIMARY KEY)"),
    )
    .await;
    let driver = engine.driver(&connection.id).await.unwrap();
    let found = driver
        .tables()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == table)
        .unwrap();
    assert!(driver.inspect(&found).await.unwrap().columns[0].primary_key);
    query(&engine, &connection.id, &format!("DROP TABLE {table}")).await;
    let job = engine
        .start(
            connection.id.clone(),
            if kind == "postgres" {
                "SELECT pg_sleep(20)"
            } else {
                "SELECT SLEEP(20)"
            }
            .into(),
            10,
            30,
            true,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    engine.cancel(&job).unwrap();
    wait(&engine, &job).await;
    assert!(
        engine
            .job(&job)
            .unwrap()
            .status()
            .unwrap()
            .error
            .unwrap()
            .to_lowercase()
            .contains("cancel")
    );
    engine.release(&job).unwrap();
    assert_eq!(
        query(&engine, &connection.id, "SELECT 43").await[0][0].text(),
        "43"
    );
    engine.disconnect(&connection.id).await.unwrap();
    engine
        .connect(
            &connection.id,
            Some(String::new()),
            Some(TLS_SECRET.into()),
            Some(SECRET.into()),
        )
        .await
        .unwrap();
    assert_eq!(
        query(&engine, &connection.id, "SELECT 44").await[0][0].text(),
        "44"
    );
    engine.disconnect(&connection.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires forwarding-only SSH and certificate-required PostgreSQL fixtures"]
async fn postgres_ssh() {
    ssh_contract("KLYNDB_TEST_MTLS_POSTGRES_URL", "postgres").await;
}
#[tokio::test]
#[ignore = "requires forwarding-only SSH and certificate-required MySQL fixtures"]
async fn mysql_ssh() {
    ssh_contract("KLYNDB_TEST_MTLS_MYSQL_URL", "mysql").await;
}
#[tokio::test]
#[ignore = "requires forwarding-only SSH and certificate-required MariaDB fixtures"]
async fn mariadb_ssh() {
    ssh_contract("KLYNDB_TEST_MTLS_MARIADB_URL", "mysql").await;
}
