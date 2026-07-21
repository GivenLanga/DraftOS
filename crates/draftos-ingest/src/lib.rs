//! Ingestion: scan a source's watched folder, (re)index changed files, and
//! optionally keep watching for changes. Ingestion is incremental by content
//! hash — re-saving an unchanged file costs one hash, nothing more.

use draftos_core::error::Result;
use draftos_embed::EmbeddingProvider;
use draftos_index::SourceBundle;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};
use walkdir::WalkDir;

#[derive(Debug, Default, Clone)]
pub struct ScanStats {
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub failed: usize,
    pub clauses: usize,
}

/// One full incremental pass over the watched folder.
pub fn scan_source(
    bundle: &mut SourceBundle,
    embedder: &dyn EmbeddingProvider,
) -> Result<ScanStats> {
    let folder = bundle.manifest.folder.clone();
    let mut stats = ScanStats::default();
    let mut seen: HashSet<String> = HashSet::new();

    for entry in WalkDir::new(&folder)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if !draftos_parse::is_supported(path) {
            continue;
        }
        let rel_path = path
            .strip_prefix(&folder)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        seen.insert(rel_path.clone());

        match ingest_file(bundle, embedder, path, &rel_path) {
            Ok(FileOutcome::Indexed(n)) => {
                stats.indexed += 1;
                stats.clauses += n;
                info!(file = %rel_path, clauses = n, "indexed");
            }
            Ok(FileOutcome::Unchanged) => stats.unchanged += 1,
            Err(e) => {
                stats.failed += 1;
                warn!(file = %rel_path, error = %e, "ingestion failed; skipping file");
            }
        }
    }

    // Files that vanished from the folder get tombstoned in the index.
    for rel_path in bundle.live_documents()? {
        if !seen.contains(&rel_path) {
            bundle.mark_deleted(&rel_path)?;
            stats.removed += 1;
            info!(file = %rel_path, "removed from index (file deleted)");
        }
    }

    Ok(stats)
}

enum FileOutcome {
    Indexed(usize),
    Unchanged,
}

fn ingest_file(
    bundle: &mut SourceBundle,
    embedder: &dyn EmbeddingProvider,
    path: &Path,
    rel_path: &str,
) -> Result<FileOutcome> {
    let bytes = std::fs::read(path)?;
    let hash = hex(&Sha256::digest(&bytes));
    if bundle.document_hash(rel_path)?.as_deref() == Some(hash.as_str()) {
        return Ok(FileOutcome::Unchanged);
    }

    let parsed = draftos_parse::parse_file(path)?;
    let clauses = draftos_extract::extract(&parsed, rel_path);

    let texts: Vec<String> = clauses
        .iter()
        .map(|c| {
            // Embed heading + body together so short bodies still carry signal.
            match (&c.heading, &c.term) {
                (Some(h), _) => format!("{h}\n{}", c.body),
                (None, Some(t)) => format!("{t}\n{}", c.body),
                _ => c.body.clone(),
            }
        })
        .collect();
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let embeddings = embedder
        .embed(&text_refs)
        .map_err(|e| draftos_core::CoreError::Embedding(e.to_string()))?;

    let n = clauses.len();
    bundle.upsert_document(rel_path, &hash, &clauses, &embeddings)?;
    Ok(FileOutcome::Indexed(n))
}

/// Watch the source's folder and rescan on changes. Blocks the calling thread;
/// callers that need concurrency run this on its own thread (CLI `watch`
/// command) or a background task (desktop app). Rescans are cheap because
/// unchanged files are skipped by hash. `should_stop` is polled roughly twice
/// a second so a detach in the UI ends the watcher promptly.
pub fn watch_source(
    bundle: &mut SourceBundle,
    embedder: &dyn EmbeddingProvider,
    should_stop: impl Fn() -> bool,
    mut on_scan: impl FnMut(&ScanStats),
) -> Result<()> {
    let folder = bundle.manifest.folder.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = notify_debouncer_mini::new_debouncer(Duration::from_secs(2), tx)
        .map_err(|e| draftos_core::CoreError::Source(format!("watcher: {e}")))?;
    debouncer
        .watcher()
        .watch(&folder, notify::RecursiveMode::Recursive)
        .map_err(|e| draftos_core::CoreError::Source(format!("watch {folder:?}: {e}")))?;

    info!(folder = %folder.display(), "watching for changes");
    loop {
        if should_stop() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(_events)) => {
                let stats = scan_source(bundle, embedder)?;
                on_scan(&stats);
            }
            Ok(Err(e)) => warn!(error = ?e, "watcher error"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
