use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::Row;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

async fn wait(engine: &Engine, id: &str) {
    let began = Instant::now();
    while !engine.job(id).unwrap().status().unwrap().done {
        assert!(began.elapsed() < Duration::from_secs(15));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn query(engine: &Engine, id: &str, sql: &str) -> Vec<Row> {
    let job_id = engine
        .start(id.into(), sql.into(), 50, 10, true)
        .await
        .unwrap();
    wait(engine, &job_id).await;
    let job = engine.job(&job_id).unwrap();
    assert!(
        job.status().unwrap().error.is_none(),
        "{sql}: {:?}",
        job.status().unwrap()
    );
    let rows = job.page(0, 0, 50).unwrap();
    engine.release(&job_id).unwrap();
    rows
}
async fn verified_tls(variable: &str, kind: &str) {
    let base = std::env::var(variable).expect("Set the disposable TLS server URL");
    let dir = PathBuf::from(
        std::env::var("KLYNDB_TEST_TLS_CERT_DIR").expect("Set TLS fixture directory"),
    );
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&state.path().join("state.db")).unwrap());
    let option = if kind == "postgres" { "sslmode" } else { "tls" };
    let secure = if kind == "postgres" {
        "require"
    } else {
        "required"
    };
    let make = |host: Option<&str>, ca: Option<&str>, mode: &str| {
        let mut u = url::Url::parse(&base).unwrap();
        if let Some(host) = host {
            u.set_host(Some(host)).unwrap();
        }
        u.query_pairs_mut()
            .clear()
            .append_pair(option, mode)
            .append_pair("connect_timeout", "45");
        if let Some(file) = ca {
            u.query_pairs_mut()
                .append_pair("sslrootcert", &dir.join(file).to_string_lossy());
        }
        Connection {
            id: String::new(),
            name: "TLS contract".into(),
            engine: kind.into(),
            address: u.to_string(),
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: false,
        }
    };
    // The CA is private to these fixtures; neither chain nor hostname checks may be bypassed.
    for draft in [
        make(None, None, secure),
        make(None, Some("other-ca.pem"), secure),
        make(Some("127.0.0.1"), Some("ca.pem"), secure),
        make(None, Some("invalid.pem"), secure),
        make(None, Some("missing.pem"), secure),
    ] {
        assert!(
            engine
                .test_connection(draft, Some(String::new()), None)
                .await
                .is_err()
        );
    }
    let disabled = if kind == "postgres" {
        "disable"
    } else {
        "disabled"
    };
    assert!(
        engine
            .test_connection(
                make(None, Some("ca.pem"), disabled),
                Some(String::new()),
                None
            )
            .await
            .is_err()
    );
    engine
        .test_connection(
            make(None, Some("bundle.pem"), secure),
            Some(String::new()),
            None,
        )
        .await
        .unwrap();
    let certs =
        klyndb_driver_api::tls::load_ca_certificates(&dir.join("bundle.pem").to_string_lossy())
            .await
            .unwrap();
    let der = state.path().join("ca.der");
    std::fs::write(&der, certs.last().unwrap()).unwrap();
    engine
        .test_connection(
            make(None, Some(der.to_str().unwrap()), secure),
            Some(String::new()),
            None,
        )
        .await
        .unwrap();
    assert!(engine.store.connections().unwrap().is_empty());
    let mut c = make(None, Some("bundle.pem"), secure);
    c.validate().unwrap();
    engine.store.save(&c).unwrap();
    engine
        .connect(&c.id, Some(String::new()), None)
        .await
        .unwrap();
    let rows = query(
        &engine,
        &c.id,
        if kind == "postgres" {
            "SELECT ssl::text FROM pg_stat_ssl WHERE pid=pg_backend_pid();"
        } else {
            "SHOW SESSION STATUS LIKE 'Ssl_cipher';"
        },
    )
    .await;
    if kind == "postgres" {
        assert_eq!(rows[0][0].text(), "true");
    } else {
        assert!(!rows[0][1].text().is_empty());
    }
    assert_eq!(query(&engine, &c.id, "SELECT 42;").await[0][0].text(), "42");
    let table = format!("klyndb_tls_{}", uuid::Uuid::new_v4().simple());
    query(
        &engine,
        &c.id,
        &format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY);"),
    )
    .await;
    assert!(
        engine
            .driver(&c.id)
            .await
            .unwrap()
            .tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == table)
    );
    query(&engine, &c.id, &format!("DROP TABLE {table};")).await;
    let job = engine
        .start(
            c.id.clone(),
            if kind == "postgres" {
                "SELECT pg_sleep(20);"
            } else {
                "SELECT SLEEP(20);"
            }
            .into(),
            50,
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
    assert_eq!(query(&engine, &c.id, "SELECT 43;").await[0][0].text(), "43");
    engine.disconnect(&c.id).await.unwrap();
}
#[tokio::test]
#[ignore = "Requires a disposable PostgreSQL TLS server and generated CA fixtures"]
async fn postgres_verified_tls() {
    verified_tls("KLYNDB_TEST_TLS_POSTGRES_URL", "postgres").await;
}
#[tokio::test]
#[ignore = "Requires a disposable MySQL TLS server and generated CA fixtures"]
async fn mysql_verified_tls() {
    verified_tls("KLYNDB_TEST_TLS_MYSQL_URL", "mysql").await;
}
#[tokio::test]
#[ignore = "Requires a disposable MariaDB TLS server and generated CA fixtures"]
async fn mariadb_verified_tls() {
    verified_tls("KLYNDB_TEST_TLS_MARIADB_URL", "mysql").await;
}

#[tokio::test]
async fn ca_files_are_bounded_and_validated() {
    let dir = tempfile::tempdir().unwrap();
    for (name, size) in [
        ("empty.pem", 0),
        ("large.pem", 1024 * 1024 + 1),
        ("invalid.pem", 16),
    ] {
        let file = dir.path().join(name);
        std::fs::write(&file, vec![b'x'; size]).unwrap();
        assert!(
            klyndb_driver_api::tls::load_ca_certificates(file.to_str().unwrap())
                .await
                .is_err()
        );
        let error = klyndb_driver_api::tls::load_client_identity(
            file.to_str().unwrap(),
            Some("private-fixture-password"),
        )
        .await
        .err()
        .unwrap();
        assert!(!error.to_string().contains("private-fixture-password"));
    }
    assert!(
        klyndb_driver_api::tls::load_ca_certificates("relative.pem")
            .await
            .is_err()
    );
}

async fn mutual_tls(variable: &str, kind: &str) {
    let base = std::env::var(variable).expect("Set the disposable certificate-required server URL");
    let dir = PathBuf::from(
        std::env::var("KLYNDB_TEST_TLS_CERT_DIR").expect("Set TLS fixture directory"),
    );
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&state.path().join("state.db")).unwrap());
    let make = |identity: Option<&str>| {
        let mut address = url::Url::parse(&base).unwrap();
        address
            .query_pairs_mut()
            .clear()
            .append_pair(
                if kind == "postgres" { "sslmode" } else { "tls" },
                if kind == "postgres" {
                    "require"
                } else {
                    "required"
                },
            )
            .append_pair("sslrootcert", &dir.join("ca.pem").to_string_lossy())
            .append_pair("connect_timeout", "45");
        if let Some(identity) = identity {
            address
                .query_pairs_mut()
                .append_pair("sslidentity", &dir.join(identity).to_string_lossy());
        }
        Connection {
            id: String::new(),
            name: "Mutual TLS contract".into(),
            engine: kind.into(),
            address: address.to_string(),
            environment: "development".into(),
            group: String::new(),
            color: "#93d4b5".into(),
            favorite: false,
            read_only: false,
            create_file: false,
        }
    };
    const SECRET: &str = "klyndb-fixture-only";
    for (identity, password) in [
        (None, SECRET),
        (Some("client.p12"), "wrong-password"),
        (Some("wrong-client.p12"), SECRET),
        (Some("missing.p12"), SECRET),
        (Some("invalid.pem"), SECRET),
    ] {
        let error = engine
            .test_connection(make(identity), Some(String::new()), Some(password.into()))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(password));
    }
    engine
        .test_connection(
            make(Some("client.p12")),
            Some(String::new()),
            Some(SECRET.into()),
        )
        .await
        .unwrap();
    assert!(engine.store.connections().unwrap().is_empty());
    let mut connection = make(Some("client.p12"));
    connection.validate().unwrap();
    engine.store.save(&connection).unwrap();
    assert!(
        !serde_json::to_string(&engine.store.connections().unwrap())
            .unwrap()
            .contains(SECRET)
    );
    engine
        .connect(&connection.id, Some(String::new()), Some(SECRET.into()))
        .await
        .unwrap();
    if kind == "postgres" {
        let rows = query(
            &engine,
            &connection.id,
            "SELECT ssl::text, client_dn FROM pg_stat_ssl WHERE pid=pg_backend_pid();",
        )
        .await;
        assert_eq!(rows[0][0].text(), "true");
        assert!(rows[0][1].text().contains("CN=klyndb_mtls"));
    } else {
        assert!(
            !query(
                &engine,
                &connection.id,
                "SHOW SESSION STATUS LIKE 'Ssl_cipher';"
            )
            .await[0][1]
                .text()
                .is_empty()
        );
        assert!(
            query(&engine, &connection.id, "SELECT CURRENT_USER();").await[0][0]
                .text()
                .contains("klyndb_mtls")
        );
    }
    let table = format!("klyndb_mtls_{}", uuid::Uuid::new_v4().simple());
    query(
        &engine,
        &connection.id,
        &format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY);"),
    )
    .await;
    assert!(
        engine
            .driver(&connection.id)
            .await
            .unwrap()
            .tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == table)
    );
    query(&engine, &connection.id, &format!("DROP TABLE {table};")).await;
    let job = engine
        .start(
            connection.id.clone(),
            if kind == "postgres" {
                "SELECT pg_sleep(20);"
            } else {
                "SELECT SLEEP(20);"
            }
            .into(),
            50,
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
        query(&engine, &connection.id, "SELECT 44;").await[0][0].text(),
        "44"
    );
    engine.disconnect(&connection.id).await.unwrap();
    engine
        .connect(&connection.id, Some(String::new()), Some(SECRET.into()))
        .await
        .unwrap();
    assert_eq!(
        query(&engine, &connection.id, "SELECT 45;").await[0][0].text(),
        "45"
    );
    engine.disconnect(&connection.id).await.unwrap();
}
#[tokio::test]
#[ignore = "Requires a disposable PostgreSQL server requiring client certificates"]
async fn postgres_mutual_tls() {
    mutual_tls("KLYNDB_TEST_MTLS_POSTGRES_URL", "postgres").await;
}
#[tokio::test]
#[ignore = "Requires a disposable MySQL server requiring client certificates"]
async fn mysql_mutual_tls() {
    mutual_tls("KLYNDB_TEST_MTLS_MYSQL_URL", "mysql").await;
}
#[tokio::test]
#[ignore = "Requires a disposable MariaDB server requiring client certificates"]
async fn mariadb_mutual_tls() {
    mutual_tls("KLYNDB_TEST_MTLS_MARIADB_URL", "mysql").await;
}
