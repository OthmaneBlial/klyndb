use klyndb_connections::{Connection, Store};
use klyndb_core::Engine;
use std::io::{BufRead, BufReader, Seek};

#[tokio::test]
async fn wide_results_export_beyond_the_ipc_page_limit() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::new(Store::open(&directory.path().join("state.db")).unwrap());
    let mut connection = Connection {
        id: String::new(),
        name: "Wide export contract".into(),
        engine: "sqlite".into(),
        address: directory.path().join("data.db").to_string_lossy().into(),
        environment: "development".into(),
        group: String::new(),
        color: "#93d4b5".into(),
        favorite: false,
        read_only: false,
        create_file: true,
    };
    connection.validate().unwrap();
    engine.store.save(&connection).unwrap();
    engine.connect(&connection.id, None, None).await.unwrap();
    let id = engine.start(connection.id.clone(), "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<40) SELECT x AS id, printf('%0*d',300000,x) AS payload FROM n; SELECT 42 AS another_set".into(), 100, 5, false).await.unwrap();
    let job = engine.job(&id).unwrap();
    while !job.status().unwrap().done {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(job.status().unwrap().error.is_none());
    assert!(job.page(0, 0, 500).unwrap_err().message.contains("8 MiB"));
    assert_eq!(job.page(0, 0, 1).unwrap().len(), 1);
    let mut file = tempfile::tempfile().unwrap();
    assert_eq!(job.export(&mut file, 0, "csv", "").unwrap(), 40);
    file.rewind().unwrap();
    let mut lines = BufReader::new(file).lines();
    assert_eq!(lines.next().unwrap().unwrap(), "id,payload");
    for id in 1..=40 {
        let line = lines.next().unwrap().unwrap();
        let (key, payload) = line.split_once(',').unwrap();
        assert_eq!(key, id.to_string());
        assert_eq!(payload.len(), 300000);
        assert!(payload.ends_with(&id.to_string()));
    }
    assert!(lines.next().is_none());
    let mut second = vec![];
    assert_eq!(job.export(&mut second, 1, "csv", "").unwrap(), 1);
    assert_eq!(second, b"another_set\n42\n");
    assert!(job.export(std::io::sink(), 99, "csv", "").is_err());
    engine.release(&id).unwrap();
    engine.disconnect(&connection.id).await.unwrap();
}
