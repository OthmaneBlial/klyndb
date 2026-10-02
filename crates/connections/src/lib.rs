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
            "sqlite" => {
                if self.address.is_empty() || self.address.contains('\0') {
                    return Err(Error::new("Choose a SQLite file"));
                }
                Ok(None)
            }
            "postgres" | "mysql" => {
                let mut url = url::Url::parse(&self.address)
                    .map_err(|_| Error::new("Enter a valid database connection URL"))?;
                let mysql = self.engine == "mysql";
                let schemes: &[&str] = if mysql {
                    &["mysql"]
                } else {
                    &["postgres", "postgresql"]
                };
                if !schemes.contains(&url.scheme()) || url.host_str().is_none() {
                    return Err(Error::new(if mysql {
                        "Expected mysql://user@host/database"
                    } else {
                        "Expected postgresql://user@host/database"
                    }));
                }
                let options: &[&str] = if mysql {
                    &["tls"]
                } else {
                    &["sslmode", "connect_timeout", "application_name"]
                };
                for (key, value) in url.query_pairs() {
                    if mysql && key == "tls" && !["required", "disabled"].contains(&value.as_ref())
                    {
                        return Err(Error::new("MySQL tls must be required or disabled"));
                    }
                    if !options.contains(&key.as_ref()) {
                        return Err(Error::new(format!(
                            "Unsupported URL parameter: {key}. Use the password field for credentials."
                        )));
                    }
                }
                let password = url.password().map(|p| Zeroizing::new(percent_decode(p)));
                url.set_password(None)
                    .map_err(|_| Error::new("Invalid URL credentials"))?;
                let tls_key = if mysql { "tls" } else { "sslmode" };
                if !url.query_pairs().any(|(k, _)| k == tls_key) {
                    url.query_pairs_mut()
                        .append_pair(tls_key, if mysql { "required" } else { "require" });
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
        for address in [
            "mysql://alice@localhost/db?tls=invalid",
            "mysql://alice@localhost/db?password=leak",
        ] {
            let mut invalid = c.clone();
            invalid.address = address.into();
            assert!(invalid.validate().is_err());
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
