//! Per-source index bundle: `<bundle_dir>/manifest.json` + `<bundle_dir>/index.db`.
//!
//! A bundle is the entire knowledge source. Attach = open the files; detach =
//! close them. Nothing is shared between bundles, so mounting/unmounting one
//! source can never affect another source or the app's responsiveness.
//!
//! index.db layout (SQLite):
//!   documents    — one row per ingested file (content hash for change detection)
//!   clauses      — one row per knowledge object (clause/definition/schedule…)
//!   clauses_fts  — FTS5 (BM25) over heading+body, rowid-aligned with clauses
//!   clause_vec   — sqlite-vec vec0 table, rowid-aligned with clauses
//!   edges        — knowledge-graph edges (clause → referenced definition, …)

use draftos_core::error::{CoreError, Result};
use draftos_core::{ClauseHit, ClauseKind, ClauseMetadata, ExtractedClause, SourceManifest};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Once;

const MANIFEST_FILE: &str = "manifest.json";
const DB_FILE: &str = "index.db";

/// Register sqlite-vec for every future connection in this process.
fn install_sqlite_vec() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        type ExtInit = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<*const (), ExtInit>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

pub struct SourceBundle {
    pub manifest: SourceManifest,
    pub dir: PathBuf,
    conn: Connection,
}

impl SourceBundle {
    /// Create a new bundle on disk for a watched folder.
    pub fn create(dir: &Path, manifest: SourceManifest) -> Result<Self> {
        install_sqlite_vec();
        std::fs::create_dir_all(dir)?;
        let manifest_path = dir.join(MANIFEST_FILE);
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        let conn = Connection::open(dir.join(DB_FILE))
            .map_err(|e| CoreError::Index(format!("open index.db: {e}")))?;
        let bundle = Self {
            manifest,
            dir: dir.to_path_buf(),
            conn,
        };
        bundle.init_schema()?;
        Ok(bundle)
    }

    /// Mount an existing bundle. O(1): reads the manifest, opens the DB.
    pub fn open(dir: &Path) -> Result<Self> {
        install_sqlite_vec();
        let manifest: SourceManifest =
            serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE))?)?;
        let conn = Connection::open(dir.join(DB_FILE))
            .map_err(|e| CoreError::Index(format!("open index.db: {e}")))?;
        let bundle = Self {
            manifest,
            dir: dir.to_path_buf(),
            conn,
        };
        bundle.init_schema()?;
        Ok(bundle)
    }

    fn init_schema(&self) -> Result<()> {
        let dims = self.manifest.embed_dims;
        self.conn
            .execute_batch(&format!(
                "
                PRAGMA busy_timeout = 5000;
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                CREATE TABLE IF NOT EXISTS documents (
                    id          TEXT PRIMARY KEY,
                    rel_path    TEXT NOT NULL UNIQUE,
                    content_hash TEXT NOT NULL,
                    indexed_at  TEXT NOT NULL,
                    deleted     INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS clauses (
                    id            TEXT PRIMARY KEY,
                    doc_id        TEXT NOT NULL REFERENCES documents(id),
                    kind          TEXT NOT NULL,
                    number        TEXT,
                    heading       TEXT,
                    term          TEXT,
                    body          TEXT NOT NULL,
                    clause_type   TEXT,
                    contract_type TEXT,
                    jurisdiction  TEXT,
                    metadata_json TEXT NOT NULL DEFAULT '{{}}',
                    -- JSON array of the clause's original OOXML body paragraphs
                    -- (DOCX sources), for house-style-faithful rendering.
                    body_ooxml    TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_clauses_doc ON clauses(doc_id);
                CREATE INDEX IF NOT EXISTS idx_clauses_type ON clauses(clause_type);
                CREATE VIRTUAL TABLE IF NOT EXISTS clauses_fts USING fts5(heading, body);
                CREATE VIRTUAL TABLE IF NOT EXISTS clause_vec USING vec0(embedding float[{dims}]);
                CREATE TABLE IF NOT EXISTS edges (
                    src_clause TEXT NOT NULL,
                    relation   TEXT NOT NULL,
                    dst_clause TEXT NOT NULL,
                    PRIMARY KEY (src_clause, relation, dst_clause)
                );
                "
            ))
            .map_err(|e| CoreError::Index(format!("schema: {e}")))?;
        // Bundles created before body_ooxml existed: add the column. Ignore the
        // "duplicate column name" error when it is already present.
        let _ = self
            .conn
            .execute("ALTER TABLE clauses ADD COLUMN body_ooxml TEXT", []);
        Ok(())
    }

    /// Content hash currently indexed for a file, if any (and not tombstoned).
    pub fn document_hash(&self, rel_path: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT content_hash FROM documents WHERE rel_path = ?1 AND deleted = 0",
                params![rel_path],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Index(e.to_string()))
    }

    /// All live rel_paths — used to detect deletions on rescan.
    pub fn live_documents(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT rel_path FROM documents WHERE deleted = 0")
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(rows.flatten().collect())
    }

    /// Replace a document's knowledge objects atomically (insert or re-index).
    pub fn upsert_document(
        &mut self,
        rel_path: &str,
        content_hash: &str,
        clauses: &[ExtractedClause],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        assert_eq!(clauses.len(), embeddings.len());
        let tx = self
            .conn
            .transaction()
            .map_err(|e| CoreError::Index(e.to_string()))?;

        // Remove any previous version of this document.
        let old_doc: Option<String> = tx
            .query_row(
                "SELECT id FROM documents WHERE rel_path = ?1",
                params![rel_path],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Index(e.to_string()))?;
        if let Some(doc_id) = &old_doc {
            delete_doc_clauses(&tx, doc_id)?;
            tx.execute("DELETE FROM documents WHERE id = ?1", params![doc_id])
                .map_err(|e| CoreError::Index(e.to_string()))?;
        }

        let doc_id = draftos_core::new_id();
        tx.execute(
            "INSERT INTO documents (id, rel_path, content_hash, indexed_at, deleted)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![doc_id, rel_path, content_hash, draftos_core::now_utc()],
        )
        .map_err(|e| CoreError::Index(e.to_string()))?;

        let mut clause_rows: Vec<(String, Option<String>)> = Vec::new(); // (clause_id, term)
        for (clause, embedding) in clauses.iter().zip(embeddings) {
            let clause_id = draftos_core::new_id();
            let body_ooxml: Option<String> = if clause.ooxml.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&clause.ooxml)?)
            };
            tx.execute(
                "INSERT INTO clauses
                 (id, doc_id, kind, number, heading, term, body,
                  clause_type, contract_type, jurisdiction, metadata_json, body_ooxml)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    clause_id,
                    doc_id,
                    clause.kind.as_str(),
                    clause.number,
                    clause.heading,
                    clause.term,
                    clause.body,
                    clause.metadata.clause_type,
                    clause.metadata.contract_type,
                    clause.metadata.jurisdiction,
                    serde_json::to_string(&clause.metadata)?,
                    body_ooxml,
                ],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
            let rowid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO clauses_fts (rowid, heading, body) VALUES (?1, ?2, ?3)",
                params![rowid, clause.heading.as_deref().unwrap_or(""), clause.body],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
            tx.execute(
                "INSERT INTO clause_vec (rowid, embedding) VALUES (?1, ?2)",
                params![rowid, vec_to_blob(embedding)],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
            clause_rows.push((clause_id, clause.term.clone()));
        }

        // Graph edges: clause —uses_term→ definition, when a clause body
        // mentions a term defined in the same document.
        for (i, clause) in clauses.iter().enumerate() {
            if clause.kind == ClauseKind::Definition {
                continue;
            }
            for (j, other) in clauses.iter().enumerate() {
                if i == j {
                    continue;
                }
                if let Some(term) = &other.term {
                    if term.len() >= 3 && clause.body.contains(term.as_str()) {
                        tx.execute(
                            "INSERT OR IGNORE INTO edges (src_clause, relation, dst_clause)
                             VALUES (?1, 'uses_term', ?2)",
                            params![clause_rows[i].0, clause_rows[j].0],
                        )
                        .map_err(|e| CoreError::Index(e.to_string()))?;
                    }
                }
            }
        }

        tx.commit().map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(())
    }

    /// Tombstone a document whose file disappeared from the watched folder.
    pub fn mark_deleted(&mut self, rel_path: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let doc_id: Option<String> = tx
            .query_row(
                "SELECT id FROM documents WHERE rel_path = ?1 AND deleted = 0",
                params![rel_path],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Index(e.to_string()))?;
        if let Some(doc_id) = doc_id {
            delete_doc_clauses(&tx, &doc_id)?;
            tx.execute(
                "UPDATE documents SET deleted = 1 WHERE id = ?1",
                params![doc_id],
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        }
        tx.commit().map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(())
    }

    /// BM25 full-text search. Returns (clause rowid, rank) best-first.
    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<i64>> {
        let fts_query = sanitize_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT rowid FROM clauses_fts WHERE clauses_fts MATCH ?1
                 ORDER BY bm25(clauses_fts, 2.0, 1.0) LIMIT ?2",
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], |r| r.get::<_, i64>(0))
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(rows.flatten().collect())
    }

    /// Nearest-neighbour vector search. Returns clause rowids best-first.
    pub fn vec_search(&self, embedding: &[f32], limit: usize) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT rowid FROM clause_vec WHERE embedding MATCH ?1 AND k = ?2
                 ORDER BY distance",
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let rows = stmt
            .query_map(params![vec_to_blob(embedding), limit as i64], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok(rows.flatten().collect())
    }

    /// Hydrate clause rowids into full hits (with provenance).
    pub fn hydrate(&self, rowids: &[i64]) -> Result<Vec<(i64, ClauseHit)>> {
        let mut out = Vec::with_capacity(rowids.len());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.id, c.kind, c.number, c.heading, c.term, c.body,
                        c.metadata_json, d.rel_path, c.body_ooxml
                 FROM clauses c JOIN documents d ON d.id = c.doc_id
                 WHERE c.rowid = ?1 AND d.deleted = 0",
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        for rowid in rowids {
            let hit = stmt
                .query_row(params![rowid], |r| {
                    let metadata_json: String = r.get(6)?;
                    let ooxml: Vec<String> = r
                        .get::<_, Option<String>>(8)?
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default();
                    Ok(ClauseHit {
                        clause_id: r.get(0)?,
                        source_name: self.manifest.name.clone(),
                        file: r.get(7)?,
                        kind: ClauseKind::parse(&r.get::<_, String>(1)?),
                        number: r.get(2)?,
                        heading: r.get(3)?,
                        term: r.get(4)?,
                        body: r.get(5)?,
                        ooxml,
                        metadata: serde_json::from_str::<ClauseMetadata>(&metadata_json)
                            .unwrap_or_default(),
                        score: 0.0,
                    })
                })
                .optional()
                .map_err(|e| CoreError::Index(e.to_string()))?;
            if let Some(hit) = hit {
                out.push((*rowid, hit));
            }
        }
        Ok(out)
    }

    /// One hop through the knowledge graph: rowids of clauses that the given
    /// clauses point at via edges (e.g. the definitions a clause uses).
    pub fn neighbor_rowids(&self, clause_ids: &[String]) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT dst.rowid
                 FROM edges e
                 JOIN clauses dst ON dst.id = e.dst_clause
                 JOIN documents d ON d.id = dst.doc_id
                 WHERE e.src_clause = ?1 AND d.deleted = 0",
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let mut out = Vec::new();
        for id in clause_ids {
            let rows = stmt
                .query_map(params![id], |r| r.get::<_, i64>(0))
                .map_err(|e| CoreError::Index(e.to_string()))?;
            out.extend(rows.flatten());
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// Counts for status displays: (live documents, knowledge objects).
    pub fn stats(&self) -> Result<(u64, u64)> {
        let docs: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM documents WHERE deleted = 0", [], |r| {
                r.get(0)
            })
            .map_err(|e| CoreError::Index(e.to_string()))?;
        let clauses: u64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM clauses c JOIN documents d ON d.id = c.doc_id
                 WHERE d.deleted = 0",
                [],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Index(e.to_string()))?;
        Ok((docs, clauses))
    }
}

fn delete_doc_clauses(tx: &rusqlite::Transaction, doc_id: &str) -> Result<()> {
    let mut stmt = tx
        .prepare("SELECT rowid FROM clauses WHERE doc_id = ?1")
        .map_err(|e| CoreError::Index(e.to_string()))?;
    let rowids: Vec<i64> = stmt
        .query_map(params![doc_id], |r| r.get(0))
        .map_err(|e| CoreError::Index(e.to_string()))?
        .flatten()
        .collect();
    drop(stmt);
    for rowid in rowids {
        tx.execute("DELETE FROM clauses_fts WHERE rowid = ?1", params![rowid])
            .map_err(|e| CoreError::Index(e.to_string()))?;
        tx.execute("DELETE FROM clause_vec WHERE rowid = ?1", params![rowid])
            .map_err(|e| CoreError::Index(e.to_string()))?;
    }
    tx.execute(
        "DELETE FROM edges WHERE src_clause IN (SELECT id FROM clauses WHERE doc_id = ?1)
         OR dst_clause IN (SELECT id FROM clauses WHERE doc_id = ?1)",
        params![doc_id],
    )
    .map_err(|e| CoreError::Index(e.to_string()))?;
    tx.execute("DELETE FROM clauses WHERE doc_id = ?1", params![doc_id])
        .map_err(|e| CoreError::Index(e.to_string()))?;
    Ok(())
}

/// Turn arbitrary user input into a safe FTS5 OR-query of quoted terms.
fn sanitize_fts_query(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}
