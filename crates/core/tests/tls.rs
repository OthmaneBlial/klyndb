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
        u.query_pairs_mut().clear().append_pair(option, mode);
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
                .test_connection(draft, Some(String::new()))
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
            .test_connection(make(None, Some("ca.pem"), disabled), Some(String::new()))
            .await
            .is_err()
    );
    engine
        .test_connection(make(None, Some("bundle.pem"), secure), Some(String::new()))
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
        )
        .await
        .unwrap();
    assert!(engine.store.connections().unwrap().is_empty());
    let mut c = make(None, Some("bundle.pem"), secure);
    c.validate().unwrap();
    engine.store.save(&c).unwrap();
    engine.connect(&c.id, Some(String::new())).await.unwrap();
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
    }
    assert!(
        klyndb_driver_api::tls::load_ca_certificates("relative.pem")
            .await
            .is_err()
    );
}
