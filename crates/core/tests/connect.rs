use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use klyndb_driver_api::{Batch, Cell};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

async fn delayed_connection(variable: &str, kind: &str) {
    let mut url =
        url::Url::parse(&std::env::var(variable).expect("Set a disposable local database URL"))
            .unwrap();
    let host = url.host_str().unwrap().to_owned();
    let port = url
        .port()
        .unwrap_or(if kind == "postgres" { 5432 } else { 3306 });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    let proxy = tokio::spawn(async move {
        // A real server behind a deliberately slow handshake exposes hard-coded ten-second limits.
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            let host = host.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(11)).await;
                if let Ok(mut server) = tokio::net::TcpStream::connect((host.as_str(), port)).await
                {
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                }
            });
        }
    });
    url.set_host(Some("127.0.0.1")).unwrap();
    url.set_port(Some(local.port())).unwrap();
    url.query_pairs_mut()
        .clear()
        .append_pair("connect_timeout", "15")
        .append_pair(
            if kind == "postgres" { "sslmode" } else { "tls" },
            if kind == "postgres" {
                "disable"
            } else {
                "disabled"
            },
        );
    let mut connection = Connection {
        id: String::new(),
        name: "Delayed handshake contract".into(),
        engine: kind.into(),
        address: url.to_string(),
        environment: "development".into(),
        group: String::new(),
        color: "#79c7a4".into(),
        favorite: false,
        read_only: false,
        create_file: false,
    };
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&state.path().join("state.db")).unwrap());
    let began = Instant::now();
    engine
        .test_connection(connection.clone(), None, None, None)
        .await
        .unwrap();
    assert!(began.elapsed() >= Duration::from_secs(11));
    assert!(engine.store.connections().unwrap().is_empty());
    let password = connection
        .validate()
        .unwrap()
        .map(|p| p.to_string())
        .unwrap_or_default();
    engine.store.save(&connection).unwrap();
    let began = Instant::now();
    // Explicit session-only credentials avoid touching the OS keychain in this contract.
    engine
        .connect(&connection.id, Some(password), None, None)
        .await
        .unwrap();
    assert!(began.elapsed() >= Duration::from_secs(11));
    let driver = engine.driver(&connection.id).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    driver
        .execute("SELECT 42".into(), tx, CancellationToken::new(), 10)
        .await
        .unwrap();
    let mut found = false;
    while let Some(batch) = rx.recv().await {
        if let Batch::Rows(rows) = batch {
            found |= rows.iter().any(|r| {
                r == &vec![Cell::Number("42".into())] || r == &vec![Cell::Text("42".into())]
            });
        }
    }
    assert!(found);
    engine.disconnect(&connection.id).await.unwrap();
    proxy.abort();
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server"]
async fn postgres_delayed_connection() {
    delayed_connection("KLYNDB_TEST_POSTGRES_URL", "postgres").await;
}
#[tokio::test]
#[ignore = "requires a disposable MySQL server"]
async fn mysql_delayed_connection() {
    delayed_connection("KLYNDB_TEST_MYSQL_URL", "mysql").await;
}
#[tokio::test]
#[ignore = "requires a disposable MariaDB server"]
async fn mariadb_delayed_connection() {
    delayed_connection("KLYNDB_TEST_MARIADB_URL", "mysql").await;
}
