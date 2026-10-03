pub mod ssh;
use klyndb_driver_api::{Error, Result};
use rusqlite::{Connection as LocalDb, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub address: String,
    pub environment: String,
    pub group: String,
    pub color: String,
    pub favorite: bool,
    pub read_only: bool,
    pub create_file: bool,
}
impl Connection {
    pub fn is_local_file(&self) -> bool {
        matches!(self.engine.as_str(), "sqlite" | "duckdb")
    }
    pub fn ssh(&self) -> Result<Option<ssh::Config>> {
        if self.is_local_file() {
            return Ok(None);
        }
        let url = url::Url::parse(&self.address)
            .map_err(|_| Error::new("Enter a valid database connection URL"))?;
        ssh::parse(&url)
    }
    pub fn connect_timeout(&self) -> Result<std::time::Duration> {
        if self.is_local_file() {
            return Ok(std::time::Duration::from_secs(10));
        }
        let url = url::Url::parse(&self.address)
            .map_err(|_| Error::new("Enter a valid database connection URL"))?;
        klyndb_driver_api::connect_timeout(
            url.query_pairs()
                .filter(|(k, _)| k == "connect_timeout")
                .map(|(_, v)| v),
        )
    }
    pub fn has_client_identity(&self) -> bool {
        if !["postgres", "mysql", "clickhouse", "redis", "mongodb"].contains(&self.engine.as_str())
        {
            return false;
        }
        url::Url::parse(&self.address)
            .is_ok_and(|u| u.query_pairs().any(|(k, _)| k == "sslidentity"))
    }
    pub fn validate(&mut self) -> Result<Option<Zeroizing<String>>> {
        if self.id.is_empty() {
            self.id = uuid::Uuid::new_v4().to_string();
        }
        uuid::Uuid::parse_str(&self.id).map_err(|_| Error::new("Invalid connection ID"))?;
        if self.name.trim().is_empty() || self.name.len() > 200 {
            return Err(Error::new("Connection name must contain 1–200 characters"));
        }
        if !["development", "staging", "production"].contains(&self.environment.as_str()) {
            return Err(Error::new("Invalid environment"));
        }
        match self.engine.as_str() {
            "sqlite" | "duckdb" => {
                if self.address.is_empty() || self.address.contains('\0') {
                    return Err(Error::new("Choose a local database file"));
                }
                Ok(None)
            }
            "postgres" | "mysql" | "clickhouse" | "mssql" | "redis" | "mongodb" => {
                let mut url = url::Url::parse(&self.address)
                    .map_err(|_| Error::new("Enter a valid database connection URL"))?;
                let native_tls_mode = self.engine != "postgres";
                let schemes: &[&str] = if self.engine == "mongodb" {
                    &["mongodb", "mongodb+srv"]
                } else if self.engine == "redis" {
                    &["redis", "rediss"]
                } else if native_tls_mode {
                    &[self.engine.as_str()]
                } else {
                    &["postgres", "postgresql"]
                };
                if !schemes.contains(&url.scheme()) || url.host_str().is_none() {
                    return Err(Error::new(if native_tls_mode {
                        if self.engine == "mongodb" {
                            "Expected a single-seed mongodb://user@host/database or mongodb+srv:// URL"
                        } else if self.engine == "redis" {
                            "Expected redis://user@host:6379/0"
                        } else if self.engine == "mssql" {
                            "Expected mssql://user@host:1433/database"
                        } else if self.engine == "clickhouse" {
                            "Expected clickhouse://user@host:9000/database"
                        } else {
                            "Expected mysql://user@host/database"
                        }
                    } else {
                        "Expected postgresql://user@host/database"
                    }));
                }
                let options: &[&str] = if self.engine == "mongodb" {
                    &[
                        "tls",
                        "sslrootcert",
                        "sslidentity",
                        "connect_timeout",
                        "authSource",
                        "authMechanism",
                        "replicaSet",
                        "directConnection",
                    ]
                } else if self.engine == "mssql" {
                    &["tls", "sslrootcert", "connect_timeout"]
                } else if native_tls_mode {
                    &["tls", "sslrootcert", "sslidentity", "connect_timeout"]
                } else {
                    &[
                        "sslmode",
                        "connect_timeout",
                        "application_name",
                        "sslrootcert",
                        "sslidentity",
                    ]
                };
                let mut seen = std::collections::HashSet::new();
                for (key, value) in url.query_pairs() {
                    if !seen.insert(key.to_string()) {
                        return Err(Error::new(
                            "Repeated connection URL parameters are not supported",
                        ));
                    }
                    if ["sslrootcert", "sslidentity"].contains(&key.as_ref())
                        && (!std::path::Path::new(value.as_ref()).is_absolute()
                            || value.len() > 16384
                            || value.contains('\0'))
                    {
                        return Err(Error::new("Choose an absolute certificate file path"));
                    }
                    if native_tls_mode
                        && key == "tls"
                        && !["required", "disabled"].contains(&value.as_ref())
                    {
                        return Err(Error::new(format!(
                            "{} tls must be required or disabled",
                            self.engine
                        )));
                    }
                    if !options.contains(&key.as_ref()) && !ssh::OPTIONS.contains(&key.as_ref()) {
                        return Err(Error::new(format!(
                            "Unsupported URL parameter: {key}. Use the password field for credentials."
                        )));
                    }
                }
                self.connect_timeout()?;
                if self.engine == "mongodb"
                    && (url.fragment().is_some()
                        || url
                            .query_pairs()
                            .any(|(k, _)| ssh::OPTIONS.contains(&k.as_ref())))
                {
                    return Err(Error::new(
                        "MongoDB SSH tunnels are not available yet; use a verified direct/SRV connection",
                    ));
                }
                self.ssh()?;
                if self.engine == "mongodb"
                    && url.scheme() == "mongodb+srv"
                    && url
                        .query_pairs()
                        .any(|(k, v)| k == "tls" && v == "disabled")
                {
                    return Err(Error::new("MongoDB SRV URLs require verified TLS"));
                }
                if self.engine == "redis" {
                    let database = url.path().strip_prefix('/').unwrap_or(url.path());
                    if !database.is_empty()
                        && (database.parse::<u32>().is_err()
                            || !database.bytes().all(|c| c.is_ascii_digit()))
                    {
                        return Err(Error::new("Redis database must be a nonnegative integer"));
                    }
                    if url.scheme() == "rediss"
                        && url
                            .query_pairs()
                            .any(|(k, v)| k == "tls" && v == "disabled")
                    {
                        return Err(Error::new("rediss URLs require verified TLS"));
                    }
                }
                let password = url.password().map(|p| Zeroizing::new(percent_decode(p)));
                url.set_password(None)
                    .map_err(|_| Error::new("Invalid URL credentials"))?;
                let tls_key = if native_tls_mode { "tls" } else { "sslmode" };
                if !url.query_pairs().any(|(k, _)| k == tls_key) {
                    url.query_pairs_mut().append_pair(
                        tls_key,
                        if native_tls_mode {
                            "required"
                        } else {
                            "require"
                        },
                    );
                }
                if url
                    .query_pairs()
                    .any(|(k, _)| ["sslrootcert", "sslidentity"].contains(&k.as_ref()))
                    && url.query_pairs().any(|(k, v)| {
                        k == tls_key
                            && v != if native_tls_mode {
                                "required"
                            } else {
                                "require"
                            }
                    })
                {
                    return Err(Error::new(
                        "Certificate files require verified TLS without plaintext fallback",
                    ));
                }
                self.address = url.to_string();
                Ok(password)
            }
            _ => Err(Error::new("This database engine is not installed")),
        }
    }
}
fn percent_decode(s: &str) -> String {
    // URL form decoding also converts '+', which passwords must preserve.
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

pub struct Store {
    db: Mutex<LocalDb>,
}
fn err(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut db = LocalDb::open(path).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(err)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(err)?;
        if version > 1 {
            return Err(Error::new(
                "Local state was created by a newer version. Update the application.",
            ));
        }
        if version == 0 {
            let tx = db.transaction().map_err(err)?;
            tx.execute_batch("CREATE TABLE connections(id TEXT PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE documents(id TEXT PRIMARY KEY,data TEXT NOT NULL); CREATE TABLE history(id INTEGER PRIMARY KEY,connection_id TEXT,sql TEXT,created_at TEXT DEFAULT CURRENT_TIMESTAMP,error TEXT,elapsed_ms INTEGER); PRAGMA user_version=1;").map_err(err)?;
            tx.commit().map_err(err)?;
        }
        Ok(Self { db: Mutex::new(db) })
    }
    pub fn connections(&self) -> Result<Vec<Connection>> {
        let db = self.db.lock().map_err(err)?;
        let mut stmt = db.prepare("SELECT data FROM connections ORDER BY json_extract(data,'$.favorite') DESC,json_extract(data,'$.name')").map_err(err)?;
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|s| serde_json::from_str(&s.map_err(err)?).map_err(err))
            .collect()
    }
    pub fn connection(&self, id: &str) -> Result<Connection> {
        let db = self.db.lock().map_err(err)?;
        let data: String = db
            .query_row("SELECT data FROM connections WHERE id=?", [id], |r| {
                r.get(0)
            })
            .map_err(|_| Error::new("Saved connection not found"))?;
        serde_json::from_str(&data).map_err(err)
    }
    pub fn save(&self, connection: &Connection) -> Result<()> {
        self.db.lock().map_err(err)?.execute("INSERT INTO connections VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data", params![connection.id,serde_json::to_string(connection).map_err(err)?]).map_err(err)?;
        Ok(())
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        self.db
            .lock()
            .map_err(err)?
            .execute("DELETE FROM connections WHERE id=?", [id])
            .map_err(err)?;
        Ok(())
    }
    pub fn document(&self, id: &str) -> Result<Option<serde_json::Value>> {
        use rusqlite::OptionalExtension;
        self.db
            .lock()
            .map_err(err)?
            .query_row("SELECT data FROM documents WHERE id=?", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(err)?
            .map(|s| serde_json::from_str(&s).map_err(err))
            .transpose()
    }
    pub fn save_document(&self, id: &str, data: &serde_json::Value) -> Result<()> {
        let json = serde_json::to_string(data).map_err(err)?;
        if json.len() > 8 * 1024 * 1024 {
            return Err(Error::new("Workspace exceeds 8 MiB"));
        }
        self.db.lock().map_err(err)?.execute("INSERT INTO documents VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data", params![id,json]).map_err(err)?;
        Ok(())
    }
    pub fn add_history(
        &self,
        connection: &str,
        sql: &str,
        error: Option<&str>,
        elapsed: u64,
    ) -> Result<()> {
        let db = self.db.lock().map_err(err)?;
        db.execute(
            "INSERT INTO history(connection_id,sql,error,elapsed_ms) VALUES(?,?,?,?)",
            params![connection, sql, error, elapsed as i64],
        )
        .map_err(err)?;
        db.execute("DELETE FROM history WHERE id NOT IN(SELECT id FROM history ORDER BY id DESC LIMIT 500)", []).map_err(err)?;
        Ok(())
    }
    pub fn history(&self) -> Result<Vec<serde_json::Value>> {
        let db = self.db.lock().map_err(err)?;
        let mut stmt = db.prepare("SELECT id,connection_id,sql,created_at,error,elapsed_ms FROM history ORDER BY id DESC LIMIT 500").map_err(err)?;
        stmt.query_map([], |r| Ok(serde_json::json!({"id":r.get::<_, i64>(0)?,"connection_id":r.get::<_, String>(1)?,"sql":r.get::<_, String>(2)?,"created_at":r.get::<_, String>(3)?,"error":r.get::<_, Option<String>>(4)?,"elapsed_ms":r.get::<_, i64>(5)?}))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
    }
    pub fn clear_history(&self) -> Result<()> {
        self.db
            .lock()
            .map_err(err)?
            .execute("DELETE FROM history", [])
            .map_err(err)?;
        Ok(())
    }
}
pub fn client_identity_key(id: &str) -> String {
    format!("tls-{id}")
}

pub fn save_password(id: &str, secret: &str) -> Result<()> {
    keyring::Entry::new("io.klyndb.desktop", id).and_then(|e| e.set_password(secret)).map_err(|_| Error::new("OS credential storage is unavailable. Unlock your keychain, or use a session-only password."))
}
pub fn password(id: &str) -> Result<Option<Zeroizing<String>>> {
    match keyring::Entry::new("io.klyndb.desktop", id).and_then(|e| e.get_password()) {
        Ok(s) => Ok(Some(Zeroizing::new(s))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(Error::new("Could not access the OS credential store")),
    }
}
pub fn delete_password(id: &str) -> Result<()> {
    match keyring::Entry::new("io.klyndb.desktop", id).and_then(|e| e.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(Error::new("Could not remove stored credentials")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_credentials_never_enter_local_state() {
        let mut c = Connection {
            id: String::new(),
            name: "test".into(),
            engine: "postgres".into(),
            address: "postgresql://alice:p%40ss+word@localhost/db".into(),
            environment: "development".into(),
            group: String::new(),
            color: "#78c6a3".into(),
            favorite: false,
            read_only: false,
            create_file: false,
        };
        assert_eq!(c.validate().unwrap().unwrap().as_str(), "p@ss+word");
        assert!(!c.address.contains("word"));
        assert!(c.address.contains("sslmode=require"));
        c.engine = "mysql".into();
        c.address = "mysql://alice:p%40ss+word@localhost/db".into();
        assert_eq!(c.validate().unwrap().unwrap().as_str(), "p@ss+word");
        assert!(!c.address.contains("word"));
        assert!(c.address.contains("tls=required"));
        let mut redis = c.clone();
        redis.engine = "redis".into();
        redis.address = "redis://alice:p%40ss+word@localhost:6379/0".into();
        assert_eq!(redis.validate().unwrap().unwrap().as_str(), "p@ss+word");
        assert!(!redis.address.contains("word"));
        assert!(redis.address.contains("tls=required"));
        for address in [
            "redis://localhost/-1",
            "redis://localhost/+1",
            "redis://localhost/abc",
            "redis://localhost/0/1",
            "rediss://localhost/0?tls=disabled",
            "redis://localhost/0?tls=disabled&sslrootcert=%2Ftmp%2Fca.pem",
            "redis://localhost/0?tls=required&tls=disabled",
        ] {
            redis.address = address.into();
            assert!(redis.validate().is_err());
        }
        redis.address = "rediss://alice@localhost/1?sslidentity=%2Ftmp%2Fclient.p12".into();
        redis.validate().unwrap();
        assert!(redis.has_client_identity());
        for address in [
            "mysql://alice@localhost/db?tls=invalid",
            "mysql://alice@localhost/db?password=leak",
            "mysql://alice@localhost/db?tls=required&tls=disabled",
            "mysql://alice@localhost/db?tls=disabled&sslrootcert=%2Ftmp%2Fca.pem",
            "mysql://alice@localhost/db?sslrootcert=relative.pem",
            "mysql://alice@localhost/db?sslrootcert=%2Ftmp%2Fca%00.pem",
            "mysql://alice@localhost/db?tls=disabled&sslidentity=%2Ftmp%2Fclient.p12",
            "mysql://alice@localhost/db?sslidentity=relative.p12",
            "mysql://alice@localhost/db?sslidentity=%2Ftmp%2Fclient.p12&identity_password=never-store-this",
        ] {
            let mut invalid = c.clone();
            invalid.address = address.into();
            assert!(invalid.validate().is_err());
        }
        for engine in ["postgres", "mysql", "clickhouse", "mssql"] {
            let mut timed = c.clone();
            timed.engine = engine.into();
            timed.address = format!("{engine}://alice@localhost/db");
            assert_eq!(timed.connect_timeout().unwrap().as_secs(), 10);
            for seconds in ["1", "45", "300"] {
                timed.address = format!("{engine}://alice@localhost/db?connect_timeout={seconds}");
                timed.validate().unwrap();
                assert_eq!(
                    timed.connect_timeout().unwrap().as_secs(),
                    seconds.parse::<u64>().unwrap()
                );
            }
            for seconds in [
                "",
                "0",
                "301",
                "-1",
                "1.5",
                "+2",
                "1e2",
                "999999999999999999999999",
                "5&connect_timeout=10",
            ] {
                timed.address = format!("{engine}://alice@localhost/db?connect_timeout={seconds}");
                assert!(timed.validate().is_err());
            }
        }
        assert_ne!(client_identity_key(&c.id), c.id);
        let mut identity = c.clone();
        identity.address = "mysql://alice@localhost/db?sslidentity=%2Ftmp%2Fclient.p12".into();
        identity.validate().unwrap();
        assert!(identity.has_client_identity());
        assert!(identity.address.contains("tls=required"));
        let mut ca = c.clone();
        ca.address = "mysql://alice@localhost/db?sslrootcert=%2Ftmp%2Fca+bundle.pem".into();
        ca.validate().unwrap();
        assert!(ca.address.contains("tls=required"));
        for mode in ["disable", "prefer"] {
            ca.engine = "postgres".into();
            ca.address = format!(
                "postgresql://alice@localhost/db?sslmode={mode}&sslrootcert=%2Ftmp%2Fca.pem"
            );
            assert!(ca.validate().is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state.db")).unwrap();
        store.save(&c).unwrap();
        assert_eq!(store.connections().unwrap().len(), 1);
        store
            .save_document("workspace", &serde_json::json!({"tabs":[]}))
            .unwrap();
        drop(store);
        let store = Store::open(&dir.path().join("state.db")).unwrap();
        assert!(store.document("workspace").unwrap().is_some());
    }
}
