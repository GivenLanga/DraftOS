//! App-level database: the registry of knowledge sources (which bundles
//! exist, where their watched folders are, whether they're attached) and the
//! append-only audit log. Per-source content lives in each bundle's own
//! index.db, never here — that separation is what makes sources swappable.

use draftos_core::error::{CoreError, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub name: String,
    pub folder: PathBuf,
    pub bundle_dir: PathBuf,
    pub attached: bool,
    pub created_at: String,
}

pub struct AppDb {
    conn: Connection,
}

impl AppDb {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let conn = Connection::open(data_dir.join("app.db"))
            .map_err(|e| CoreError::Index(format!("open app.db: {e}")))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS sources (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL UNIQUE,
                folder     TEXT NOT NULL,
                bundle_dir TEXT NOT NULL,
                attached   INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS audit_log (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                ts      TEXT NOT NULL,
                event   TEXT NOT NULL,
                details TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| CoreError::Index(format!("app schema: {e}")))?;
        Ok(Self { conn })
    }

    pub fn add_source(&self, rec: &SourceRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO sources (id, name, folder, bundle_dir, attached, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    rec.id,
                    rec.name,
                    rec.folder.to_string_lossy(),
                    rec.bundle_dir.to_string_lossy(),
                    rec.attached as i64,
                    rec.created_at,
                ],
            )
            .map_err(|e| CoreError::Source(format!("register source: {e}")))?;
        Ok(())
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, folder, bundle_dir, attached, created_at
                 FROM sources ORDER BY name",
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let rows = stmt
            .query_map([], row_to_source)
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(rows.flatten().collect())
    }

    pub fn find_source(&self, name: &str) -> Result<Option<SourceRecord>> {
        self.conn
            .query_row(
                "SELECT id, name, folder, bundle_dir, attached, created_at
                 FROM sources WHERE name = ?1",
                params![name],
                row_to_source,
            )
            .optional()
            .map_err(|e| CoreError::Index(e.to_string()))
    }

    /// Attach/detach is a registry flag flip — the bundle on disk is untouched,
    /// which is why re-attaching is instant.
    pub fn set_attached(&self, name: &str, attached: bool) -> Result<bool> {
        let n = self
            .conn
            .execute(
                "UPDATE sources SET attached = ?1 WHERE name = ?2",
                params![attached as i64, name],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(n > 0)
    }

    pub fn remove_source(&self, name: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM sources WHERE name = ?1", params![name])
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(n > 0)
    }

    /// App-level settings (JSON values). Model selection lives here — API keys
    /// never do (they belong in the OS keychain).
    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value.to_string()],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(match raw {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        })
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", params![key])
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(())
    }

    pub fn audit(&self, event: &str, details: serde_json::Value) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO audit_log (ts, event, details) VALUES (?1, ?2, ?3)",
                params![draftos_core::now_utc(), event, details.to_string()],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(())
    }
}

fn row_to_source(r: &rusqlite::Row) -> rusqlite::Result<SourceRecord> {
    Ok(SourceRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        folder: PathBuf::from(r.get::<_, String>(2)?),
        bundle_dir: PathBuf::from(r.get::<_, String>(3)?),
        attached: r.get::<_, i64>(4)? != 0,
        created_at: r.get(5)?,
    })
}
