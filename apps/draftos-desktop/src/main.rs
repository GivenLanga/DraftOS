//! DraftOS desktop shell (Tauri 2).
//!
//! The UI is a thin client: every command here delegates to the same crates
//! the CLI uses. Ingestion and folder watching run on dedicated threads — one
//! watcher per attached source — and report back to the webview via events,
//! so the window never blocks on indexing.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use draftos_core::{ClauseHit, ClauseKind, SourceManifest};
use draftos_embed::{EmbeddingProvider, HashEmbedder};
use draftos_index::SourceBundle;
use draftos_retrieval::Filters;
use draftos_storage::{AppDb, SourceRecord};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    data_dir: PathBuf,
    db: Mutex<AppDb>,
    /// Per-source stop flags for the watcher threads.
    stops: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Serialize, Clone)]
struct SourceView {
    name: String,
    folder: String,
    attached: bool,
    documents: u64,
    clauses: u64,
}

#[derive(Serialize, Clone)]
struct SourceUpdate {
    name: String,
    indexed: usize,
    unchanged: usize,
    removed: usize,
    failed: usize,
    clauses: usize,
}

#[derive(Serialize, Clone)]
struct SourceError {
    name: String,
    message: String,
}

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn embedder() -> HashEmbedder {
    HashEmbedder::new(HashEmbedder::default_dims())
}

fn emit_update(app: &AppHandle, name: &str, stats: &draftos_ingest::ScanStats) {
    let _ = app.emit(
        "source-updated",
        SourceUpdate {
            name: name.to_string(),
            indexed: stats.indexed,
            unchanged: stats.unchanged,
            removed: stats.removed,
            failed: stats.failed,
            clauses: stats.clauses,
        },
    );
}

fn emit_error(app: &AppHandle, name: &str, message: String) {
    let _ = app.emit(
        "source-error",
        SourceError {
            name: name.to_string(),
            message,
        },
    );
}

/// Start the scan + watch loop for one source on its own thread.
fn spawn_watcher(app: AppHandle, rec: SourceRecord, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let embedder = embedder();
        let mut bundle = match SourceBundle::open(&rec.bundle_dir) {
            Ok(b) => b,
            Err(e) => return emit_error(&app, &rec.name, e.to_string()),
        };
        // Catch up on anything that changed while we weren't watching…
        match draftos_ingest::scan_source(&mut bundle, &embedder) {
            Ok(stats) => emit_update(&app, &rec.name, &stats),
            Err(e) => emit_error(&app, &rec.name, e.to_string()),
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        // …then keep watching until detached or the app exits.
        let app2 = app.clone();
        let name = rec.name.clone();
        let stop2 = stop.clone();
        let result = draftos_ingest::watch_source(
            &mut bundle,
            &embedder,
            move || stop2.load(Ordering::Relaxed),
            move |stats| emit_update(&app2, &name, stats),
        );
        if let Err(e) = result {
            emit_error(&app, &rec.name, e.to_string());
        }
    });
}

fn start_watching(app: &AppHandle, state: &AppState, rec: SourceRecord) {
    let stop = Arc::new(AtomicBool::new(false));
    let mut stops = state.stops.lock().unwrap();
    if let Some(old) = stops.insert(rec.name.clone(), stop.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    spawn_watcher(app.clone(), rec, stop);
}

fn stop_watching(state: &AppState, name: &str) {
    if let Some(stop) = state.stops.lock().unwrap().remove(name) {
        stop.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
async fn list_sources(state: State<'_, AppState>) -> Result<Vec<SourceView>, String> {
    let records = state.db.lock().unwrap().list_sources().map_err(err_str)?;
    let mut out = Vec::with_capacity(records.len());
    for rec in records {
        // Don't mask a failed read as "0 docs" — that once made a fully-indexed
        // source look empty. Log it so it can never hide silently again.
        let (documents, clauses) = match SourceBundle::open(&rec.bundle_dir).and_then(|b| b.stats())
        {
            Ok(counts) => counts,
            Err(e) => {
                tracing::warn!(source = %rec.name, error = %e, "could not read source stats");
                (0, 0)
            }
        };
        out.push(SourceView {
            name: rec.name,
            folder: rec.folder.to_string_lossy().into_owned(),
            attached: rec.attached,
            documents,
            clauses,
        });
    }
    Ok(out)
}

#[tauri::command]
async fn add_source(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    name: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the source a name".into());
    }
    let folder = PathBuf::from(folder.trim())
        .canonicalize()
        .map_err(|_| "that folder doesn't exist — check the path".to_string())?;
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }

    let rec = {
        let db = state.db.lock().unwrap();
        if db.find_source(&name).map_err(err_str)?.is_some() {
            return Err(format!("a source named '{name}' already exists"));
        }
        let e = embedder();
        let id = draftos_core::new_id();
        let bundle_dir = state.data_dir.join("sources").join(&id);
        SourceBundle::create(
            &bundle_dir,
            SourceManifest {
                id: id.clone(),
                name: name.clone(),
                folder: folder.clone(),
                embed_model: e.id(),
                embed_dims: e.dims(),
                created_at: draftos_core::now_utc(),
                schema_version: SourceManifest::CURRENT_SCHEMA,
            },
        )
        .map_err(err_str)?;
        let rec = SourceRecord {
            id,
            name: name.clone(),
            folder,
            bundle_dir,
            attached: true,
            created_at: draftos_core::now_utc(),
        };
        db.add_source(&rec).map_err(err_str)?;
        db.audit(
            "source_attached",
            serde_json::json!({ "name": rec.name, "folder": rec.folder }),
        )
        .ok();
        rec
    };
    start_watching(&app, &state, rec);
    Ok(())
}

#[tauri::command]
async fn set_attached(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    attached: bool,
) -> Result<(), String> {
    let rec = {
        let db = state.db.lock().unwrap();
        if !db.set_attached(&name, attached).map_err(err_str)? {
            return Err(format!("no source named '{name}'"));
        }
        db.audit(
            if attached { "source_reattached" } else { "source_detached" },
            serde_json::json!({ "name": name }),
        )
        .ok();
        db.find_source(&name).map_err(err_str)?
    };
    if attached {
        if let Some(rec) = rec {
            start_watching(&app, &state, rec);
        }
    } else {
        stop_watching(&state, &name);
    }
    Ok(())
}

#[tauri::command]
async fn remove_source(state: State<'_, AppState>, name: String) -> Result<(), String> {
    stop_watching(&state, &name);
    let db = state.db.lock().unwrap();
    let rec = db
        .find_source(&name)
        .map_err(err_str)?
        .ok_or_else(|| format!("no source named '{name}'"))?;
    db.remove_source(&name).map_err(err_str)?;
    std::fs::remove_dir_all(&rec.bundle_dir).ok();
    db.audit("source_removed", serde_json::json!({ "name": name }))
        .ok();
    Ok(())
}

#[tauri::command]
async fn rescan_source(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let rec = state
        .db
        .lock()
        .unwrap()
        .find_source(&name)
        .map_err(err_str)?
        .ok_or_else(|| format!("no source named '{name}'"))?;
    std::thread::spawn(move || {
        let embedder = embedder();
        match SourceBundle::open(&rec.bundle_dir) {
            Ok(mut bundle) => match draftos_ingest::scan_source(&mut bundle, &embedder) {
                Ok(stats) => emit_update(&app, &rec.name, &stats),
                Err(e) => emit_error(&app, &rec.name, e.to_string()),
            },
            Err(e) => emit_error(&app, &rec.name, e.to_string()),
        }
    });
    Ok(())
}

#[tauri::command]
async fn search_clauses(
    state: State<'_, AppState>,
    query: String,
    kind: Option<String>,
    clause_type: Option<String>,
    contract_type: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ClauseHit>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let records = state.db.lock().unwrap().list_sources().map_err(err_str)?;
    let mut bundles = Vec::new();
    for rec in records {
        if !rec.attached {
            continue;
        }
        if let Some(only) = &source {
            if !rec.name.eq_ignore_ascii_case(only) {
                continue;
            }
        }
        if let Ok(bundle) = SourceBundle::open(&rec.bundle_dir) {
            bundles.push(bundle);
        }
    }
    if bundles.is_empty() {
        return Ok(Vec::new());
    }
    let filters = Filters {
        kind: kind.filter(|k| !k.is_empty()).map(|k| ClauseKind::parse(&k)),
        clause_type: clause_type.filter(|s| !s.is_empty()),
        contract_type: contract_type.filter(|s| !s.is_empty()),
    };
    let e = embedder();
    let hits = draftos_retrieval::search(&bundles, &e, &query, &filters, limit.unwrap_or(20))
        .map_err(err_str)?;
    state
        .db
        .lock()
        .unwrap()
        .audit(
            "search",
            serde_json::json!({ "query": query, "hits": hits.len() }),
        )
        .ok();
    Ok(hits)
}

// ---------- Model layer (Phase 3) ----------

/// Setting key shared with the CLI, so one configuration serves both.
const MODEL_SETTING: &str = "model_config";

#[derive(Serialize, Clone)]
struct ModelView {
    adapter: String,
    model: String,
    base_url: Option<String>,
    /// False means a CLOUD adapter — the UI must make this visually obvious
    /// (POPIA posture, CLAUDE.md §10).
    local: bool,
    json_mode: bool,
    max_context: usize,
}

fn model_view(cfg: &draftos_models::ModelConfig) -> Result<ModelView, String> {
    let adapter = draftos_models::build_adapter(cfg).map_err(err_str)?;
    let caps = adapter.capabilities();
    Ok(ModelView {
        adapter: cfg.adapter.clone(),
        model: cfg.model.clone(),
        base_url: cfg.base_url.clone(),
        local: caps.local,
        json_mode: caps.json_mode,
        max_context: caps.max_context,
    })
}

fn load_model_config(db: &AppDb) -> Result<Option<draftos_models::ModelConfig>, String> {
    match db.get_setting(MODEL_SETTING).map_err(err_str)? {
        Some(v) => Ok(Some(serde_json::from_value(v).map_err(err_str)?)),
        None => Ok(None),
    }
}

#[tauri::command]
async fn get_model(state: State<'_, AppState>) -> Result<Option<ModelView>, String> {
    let cfg = load_model_config(&state.db.lock().unwrap())?;
    cfg.map(|c| model_view(&c)).transpose()
}

#[tauri::command]
async fn set_model(
    state: State<'_, AppState>,
    adapter: String,
    model: Option<String>,
    base_url: Option<String>,
) -> Result<ModelView, String> {
    let cfg = draftos_models::ModelConfig {
        adapter,
        model: model.unwrap_or_default().trim().to_string(),
        base_url: base_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
    };
    let view = model_view(&cfg)?; // rejects unknown adapters before persisting
    let db = state.db.lock().unwrap();
    db.set_setting(MODEL_SETTING, &serde_json::to_value(&cfg).map_err(err_str)?)
        .map_err(err_str)?;
    db.audit(
        "model_configured",
        serde_json::json!({ "adapter": cfg.adapter, "model": cfg.model }),
    )
    .ok();
    Ok(view)
}

#[tauri::command]
async fn clear_model(state: State<'_, AppState>) -> Result<(), String> {
    state
        .db
        .lock()
        .unwrap()
        .delete_setting(MODEL_SETTING)
        .map_err(err_str)
}

#[tauri::command]
async fn store_model_key(
    state: State<'_, AppState>,
    adapter: String,
    key: String,
) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("empty key — nothing stored".into());
    }
    draftos_models::store_key(&adapter, key).map_err(err_str)?;
    state
        .db
        .lock()
        .unwrap()
        .audit("model_key_stored", serde_json::json!({ "adapter": adapter }))
        .ok();
    Ok(())
}

#[derive(Serialize, Clone)]
struct CitedSource {
    title: String,
    source_name: String,
    file: String,
}

#[derive(Serialize, Clone)]
struct AskView {
    text: String,
    sources: Vec<CitedSource>,
    adapter: String,
    model: String,
    local: bool,
}

#[tauri::command]
async fn ask_assistant(
    state: State<'_, AppState>,
    question: String,
    source: Option<String>,
    limit: Option<usize>,
) -> Result<AskView, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("ask something first".into());
    }
    // Snapshot everything the blocking task needs, then release the lock.
    let (cfg, records) = {
        let db = state.db.lock().unwrap();
        let cfg = load_model_config(&db)?
            .ok_or("no model configured — pick one under Model in the sidebar")?;
        (cfg, db.list_sources().map_err(err_str)?)
    };
    let limit = limit.unwrap_or(8);

    // Retrieval + the network call run off the async runtime's core threads.
    let question2 = question.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let adapter = draftos_models::build_adapter(&cfg).map_err(err_str)?;
        let mut bundles = Vec::new();
        for rec in records.into_iter().filter(|r| r.attached) {
            if let Some(only) = &source {
                if !rec.name.eq_ignore_ascii_case(only) {
                    continue;
                }
            }
            if let Ok(b) = SourceBundle::open(&rec.bundle_dir) {
                bundles.push(b);
            }
        }
        if bundles.is_empty() {
            return Err(
                "no attached sources — the assistant only answers from your knowledge base"
                    .to_string(),
            );
        }
        let e = embedder();
        let mut hits =
            draftos_retrieval::search(&bundles, &e, &question2, &Filters::default(), limit)
                .map_err(err_str)?;
        let extra = draftos_retrieval::expand_hits(&bundles, &hits).map_err(err_str)?;
        hits.extend(extra.into_iter().take(limit));
        if hits.is_empty() {
            return Err("nothing relevant found in the attached sources".to_string());
        }
        let req = draftos_prompt::ask(&question2, &hits);
        let resp = adapter.complete(&req).map_err(err_str)?;
        let caps = adapter.capabilities();
        let sources = hits
            .iter()
            .map(|h| CitedSource {
                title: h
                    .term
                    .clone()
                    .map(|t| format!("“{t}”"))
                    .or_else(|| h.heading.clone())
                    .unwrap_or_else(|| "(untitled)".to_string()),
                source_name: h.source_name.clone(),
                file: h.file.clone(),
            })
            .collect();
        Ok((
            AskView {
                text: resp.text.trim().to_string(),
                sources,
                adapter: adapter.id().to_string(),
                model: resp.model.clone(),
                local: caps.local,
            },
            resp.input_tokens,
            resp.output_tokens,
        ))
    })
    .await
    .map_err(err_str)?;

    let (view, input_tokens, output_tokens) = outcome?;
    state
        .db
        .lock()
        .unwrap()
        .audit(
            "model_call",
            serde_json::json!({
                "task": "ask",
                "adapter": view.adapter,
                "model": view.model,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
            }),
        )
        .ok();
    Ok(view)
}

// ---------- Drafting (Phase 2, surfaced in the desktop) ----------

/// The contract types with hand-written rules (draftos-rules). Anything else is
/// still draftable — the assembler falls back to a generic clause order.
const KNOWN_CONTRACT_TYPES: &[&str] = &[
    "Non-Disclosure Agreement",
    "Share Purchase Agreement",
    "Loan Agreement",
    "Employment Agreement",
    "Service Agreement",
    "Lease Agreement",
];

#[derive(Serialize, Clone)]
struct ContractTypeInfo {
    contract_type: String,
    required: Vec<String>,
}

#[tauri::command]
fn draft_contract_types() -> Vec<ContractTypeInfo> {
    KNOWN_CONTRACT_TYPES
        .iter()
        .map(|ct| ContractTypeInfo {
            contract_type: ct.to_string(),
            required: draftos_rules::required_clause_types(ct),
        })
        .collect()
}

#[derive(Serialize, Clone)]
struct FindingView {
    code: String,
    message: String,
    location: String,
}

#[derive(Serialize, Clone)]
struct VarView {
    name: String,
    label: String,
    filled: bool,
}

#[derive(Serialize, Clone)]
struct ClausePreview {
    number: String,
    heading: String,
    source: Option<String>,
    file: Option<String>,
    adapted: bool,
    snippet: String,
}

#[derive(Serialize, Clone)]
struct FilledView {
    clause_type: String,
    source: String,
    file: String,
}

#[derive(Serialize, Clone)]
struct DraftPreview {
    title: String,
    contract_type: String,
    clauses: Vec<ClausePreview>,
    filled: Vec<FilledView>,
    missing: Vec<String>,
    variables: Vec<VarView>,
    errors: Vec<FindingView>,
    warnings: Vec<FindingView>,
    /// True when validation found no errors — the only state that may render.
    renderable: bool,
}

fn finding_view(f: &draftos_validate::Finding) -> FindingView {
    FindingView {
        code: f.code.to_string(),
        message: f.message.clone(),
        location: f.location.clone(),
    }
}

/// Walk every block in a document, depth-first (into list items and table cells).
fn walk_blocks(blocks: &[draftos_core::Block], f: &mut impl FnMut(&draftos_core::Block)) {
    use draftos_core::Block;
    for b in blocks {
        f(b);
        match b {
            Block::List { items, .. } => {
                for item in items {
                    walk_blocks(item, f);
                }
            }
            Block::Table { rows } => {
                for row in rows {
                    for cell in row {
                        walk_blocks(cell, f);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Collect the distinct `{{variable}}` placeholders in the assembled document,
/// so the UI can present exactly the fields the chosen precedents need filled.
fn collect_variables(doc: &draftos_core::LirDocument) -> Vec<VarView> {
    use draftos_core::Block;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut visit = |b: &Block| {
        if let Block::Variable { name, label, value } = b {
            if seen.insert(name.clone()) {
                out.push(VarView {
                    name: name.clone(),
                    label: label.clone(),
                    filled: value.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false),
                });
            }
        }
    };
    for c in &doc.clauses {
        walk_blocks(&c.body, &mut visit);
    }
    for d in &doc.definitions {
        walk_blocks(&d.body, &mut visit);
    }
    for r in &doc.recitals {
        walk_blocks(&r.body, &mut visit);
    }
    for s in &doc.schedules {
        walk_blocks(&s.body, &mut visit);
    }
    out
}

fn clause_snippet(clause: &draftos_core::LirClause) -> String {
    let text: String = clause
        .body
        .iter()
        .map(|b| b.plain_text())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    const MAX: usize = 320;
    if text.chars().count() > MAX {
        let truncated: String = text.chars().take(MAX).collect();
        format!("{truncated}…")
    } else {
        text
    }
}

/// Open every attached source that is in the spec's scope (empty scope = all).
fn bundles_in_scope(
    records: &[SourceRecord],
    scope: &[String],
) -> Result<Vec<SourceBundle>, String> {
    let mut bundles = Vec::new();
    for rec in records.iter().filter(|r| r.attached) {
        if !scope.is_empty() && !scope.iter().any(|s| s.eq_ignore_ascii_case(&rec.name)) {
            continue;
        }
        if let Ok(b) = SourceBundle::open(&rec.bundle_dir) {
            bundles.push(b);
        }
    }
    if bundles.is_empty() {
        return Err(
            "no attached sources in scope — attach a source (and pick it in Sources) first"
                .to_string(),
        );
    }
    Ok(bundles)
}

/// Retrieve → assemble → validate, without writing anything. Deterministic, so
/// re-running from the same spec reproduces the same draft.
fn build_preview(spec: draftos_core::MatterSpec, records: Vec<SourceRecord>) -> Result<DraftPreview, String> {
    let bundles = bundles_in_scope(&records, &spec.source_scope)?;
    let e = embedder();
    let draft = draftos_assemble::assemble(&spec, &bundles, &e).map_err(err_str)?;
    let doc = &draft.document;
    let report = draftos_validate::validate(doc);

    let clauses = doc
        .clauses
        .iter()
        .map(|c| ClausePreview {
            number: c.number.clone(),
            heading: c.heading.clone(),
            source: c.provenance.source.clone(),
            file: c.provenance.file.clone(),
            adapted: c.provenance.adapted_by_model,
            snippet: clause_snippet(c),
        })
        .collect();
    let filled = draft
        .report
        .filled
        .iter()
        .map(|(ct, source, file)| FilledView {
            clause_type: ct.clone(),
            source: source.clone(),
            file: file.clone(),
        })
        .collect();

    Ok(DraftPreview {
        title: doc.meta.title.clone(),
        contract_type: doc.meta.contract_type.clone(),
        clauses,
        filled,
        missing: draft.report.missing.clone(),
        variables: collect_variables(doc),
        errors: report.errors().map(finding_view).collect(),
        warnings: report.warnings().map(finding_view).collect(),
        renderable: !report.has_errors(),
    })
}

#[tauri::command]
async fn draft_preview(
    state: State<'_, AppState>,
    spec: draftos_core::MatterSpec,
) -> Result<DraftPreview, String> {
    let records = state.db.lock().unwrap().list_sources().map_err(err_str)?;
    tauri::async_runtime::spawn_blocking(move || build_preview(spec, records))
        .await
        .map_err(err_str)?
}

#[derive(Serialize, Clone)]
struct SaveResult {
    path: String,
    clauses: usize,
    format: String,
}

/// Assemble + validate + render to `path`. Errors block rendering unless `force`
/// (mirrors the CLI's `--force`, for inspecting an incomplete draft).
fn build_and_render(
    spec: draftos_core::MatterSpec,
    records: Vec<SourceRecord>,
    path: String,
    force: bool,
) -> Result<SaveResult, String> {
    let bundles = bundles_in_scope(&records, &spec.source_scope)?;
    let e = embedder();
    let draft = draftos_assemble::assemble(&spec, &bundles, &e).map_err(err_str)?;
    let doc = &draft.document;
    let report = draftos_validate::validate(doc);
    if report.has_errors() && !force {
        let n = report.errors().count();
        return Err(format!(
            "{n} validation error(s) block rendering — fix them and preview again"
        ));
    }

    let out = PathBuf::from(&path);
    let ext = out
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let format = match ext.as_str() {
        "docx" => {
            let bytes = draftos_render::render_docx(doc).map_err(err_str)?;
            std::fs::write(&out, bytes).map_err(err_str)?;
            "docx"
        }
        "md" | "markdown" => {
            std::fs::write(&out, draftos_render::render_markdown(doc)).map_err(err_str)?;
            "markdown"
        }
        "" => return Err("add a file name ending in .docx or .md".to_string()),
        other => return Err(format!("unsupported extension '.{other}' — use .docx or .md")),
    };

    Ok(SaveResult {
        path,
        clauses: doc.clauses.len(),
        format: format.to_string(),
    })
}

#[tauri::command]
async fn draft_save(
    state: State<'_, AppState>,
    spec: draftos_core::MatterSpec,
    path: String,
    force: bool,
) -> Result<SaveResult, String> {
    let records = state.db.lock().unwrap().list_sources().map_err(err_str)?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        build_and_render(spec, records, path, force)
    })
    .await
    .map_err(err_str)??;
    state
        .db
        .lock()
        .unwrap()
        .audit(
            "draft_rendered",
            serde_json::json!({
                "title": result.path,
                "clauses": result.clauses,
                "format": result.format,
            }),
        )
        .ok();
    Ok(result)
}

/// Open the OS-native folder chooser and return the chosen path, or `None` if
/// the user cancelled. Shells out to whatever native picker the platform ships
/// so we pull in no extra crates — the app stays buildable fully offline. If no
/// picker is found the UI falls back to typing the path by hand.
fn pick_folder_blocking() -> Result<Option<String>, String> {
    use std::process::Command;

    // First candidate whose binary exists wins. Directory-selection flags per tool.
    #[cfg(target_os = "linux")]
    let candidates: &[(&str, &[&str])] = &[
        (
            "zenity",
            &[
                "--file-selection",
                "--directory",
                "--title=Choose a folder of contracts",
            ],
        ),
        ("kdialog", &["--getexistingdirectory", "."]),
        ("qarma", &["--file-selection", "--directory"]),
    ];
    #[cfg(target_os = "macos")]
    let candidates: &[(&str, &[&str])] = &[(
        "osascript",
        &[
            "-e",
            "POSIX path of (choose folder with prompt \"Choose a folder of contracts\")",
        ],
    )];
    #[cfg(target_os = "windows")]
    let candidates: &[(&str, &[&str])] = &[(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; \
             $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
             if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath }",
        ],
    )];

    for (program, args) in candidates {
        let out = match Command::new(program).args(*args).output() {
            Ok(o) => o,
            Err(_) => continue, // not installed — try the next picker
        };
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // Success with a path = chosen. Anything else (non-zero, empty stdout)
        // once the picker actually ran means the user cancelled — not an error.
        if out.status.success() && !path.is_empty() {
            return Ok(Some(path));
        }
        return Ok(None);
    }
    Err("no folder picker found — type the path instead".to_string())
}

#[tauri::command]
async fn pick_folder() -> Result<Option<String>, String> {
    // The picker blocks until the user chooses, so keep it off the async core.
    tauri::async_runtime::spawn_blocking(pick_folder_blocking)
        .await
        .map_err(err_str)?
}

/// Native "save as" chooser, seeded with `default_name`. Same offline-safe
/// shell-out approach as `pick_folder` — no `tauri-plugin-dialog`. Returns the
/// chosen path, or `None` if cancelled.
fn pick_save_path_blocking(default_name: String) -> Result<Option<String>, String> {
    use std::process::Command;

    #[cfg(target_os = "linux")]
    let candidates: Vec<(&str, Vec<String>)> = vec![
        (
            "zenity",
            vec![
                "--file-selection".into(),
                "--save".into(),
                "--confirm-overwrite".into(),
                "--title=Save draft".into(),
                format!("--filename={default_name}"),
            ],
        ),
        ("kdialog", vec!["--getsavefilename".into(), default_name.clone()]),
    ];
    #[cfg(target_os = "macos")]
    let candidates: Vec<(&str, Vec<String>)> = vec![(
        "osascript",
        vec![
            "-e".into(),
            format!(
                "POSIX path of (choose file name with prompt \"Save draft\" default name \"{default_name}\")"
            ),
        ],
    )];
    #[cfg(target_os = "windows")]
    let candidates: Vec<(&str, Vec<String>)> = vec![(
        "powershell",
        vec![
            "-NoProfile".into(),
            "-Command".into(),
            format!(
                "Add-Type -AssemblyName System.Windows.Forms; \
                 $d = New-Object System.Windows.Forms.SaveFileDialog; \
                 $d.FileName = '{default_name}'; \
                 if ($d.ShowDialog() -eq 'OK') {{ $d.FileName }}"
            ),
        ],
    )];

    for (program, args) in &candidates {
        let out = match Command::new(program).args(args).output() {
            Ok(o) => o,
            Err(_) => continue,
        };
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !path.is_empty() {
            return Ok(Some(path));
        }
        return Ok(None);
    }
    Err("no save dialog found — install zenity or type a path".to_string())
}

#[tauri::command]
async fn pick_save_path(default_name: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || pick_save_path_blocking(default_name))
        .await
        .map_err(err_str)?
}

fn resolve_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("DRAFTOS_DATA") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("za", "draftos", "DraftOS")
        .expect("cannot determine app data directory")
        .data_dir()
        .to_path_buf()
}

fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    let data_dir = resolve_data_dir();
    let db = AppDb::open(&data_dir).expect("open app database");

    tauri::Builder::default()
        .manage(AppState {
            data_dir,
            db: Mutex::new(db),
            stops: Mutex::new(HashMap::new()),
        })
        .setup(|app| {
            // Resume watching every attached source from the last session.
            let state: State<AppState> = app.state();
            let records = state.db.lock().unwrap().list_sources().unwrap_or_default();
            let handle = app.handle().clone();
            for rec in records.into_iter().filter(|r| r.attached) {
                start_watching(&handle, &state, rec);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sources,
            pick_folder,
            pick_save_path,
            draft_contract_types,
            draft_preview,
            draft_save,
            add_source,
            set_attached,
            remove_source,
            rescan_source,
            search_clauses,
            get_model,
            set_model,
            clear_model,
            store_model_key,
            ask_assistant
        ])
        .run(tauri::generate_context!())
        .expect("error while running DraftOS");
}
