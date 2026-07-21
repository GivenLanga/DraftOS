//! DraftOS CLI — the development harness for the knowledge core.
//! The Tauri desktop app calls the same crates; this binary exists so the
//! whole pipeline is exercisable and testable without a GUI.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use draftos_core::{ClauseKind, SourceManifest};
use draftos_embed::{EmbeddingProvider, HashEmbedder};
use draftos_index::SourceBundle;
use draftos_retrieval::Filters;
use draftos_storage::{AppDb, SourceRecord};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "draftos", about = "DraftOS — legal knowledge engine (Phase 1 CLI)")]
struct Cli {
    /// Override the app data directory (default: OS app-data dir).
    #[arg(long, global = true, env = "DRAFTOS_DATA")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage knowledge sources (watched folders).
    #[command(subcommand)]
    Source(SourceCmd),
    /// Re-scan a source's folder now (incremental, hash-based).
    Ingest { name: String },
    /// Watch a source's folder and re-index on every change (Ctrl-C to stop).
    Watch { name: String },
    /// Hybrid search (BM25 + vector) across all attached sources.
    Search {
        query: Vec<String>,
        /// Limit to one source by name.
        #[arg(long)]
        source: Option<String>,
        /// Filter by clause type (e.g. "Termination", "Confidentiality").
        #[arg(long = "type")]
        clause_type: Option<String>,
        /// Filter by kind: clause | definition | schedule | recital.
        #[arg(long)]
        kind: Option<String>,
        /// Filter by contract type (e.g. "Loan Agreement").
        #[arg(long)]
        contract_type: Option<String>,
        #[arg(short = 'k', long, default_value_t = 10)]
        limit: usize,
        /// Print full clause bodies instead of snippets.
        #[arg(long)]
        full: bool,
        /// Also pull in definitions the hits reference (knowledge graph, 1 hop).
        #[arg(long)]
        expand: bool,
    },
    /// Assemble a draft from a MatterSpec: retrieve → assemble → validate → render.
    Draft {
        /// Path to a MatterSpec JSON file.
        #[arg(long)]
        spec: PathBuf,
        /// Output file. Extension decides the format: .docx or .md.
        #[arg(long)]
        out: PathBuf,
        /// Also write the intermediate LIR document as JSON next to --out.
        #[arg(long)]
        lir: bool,
        /// Render even if validation reports errors (writes a marked draft).
        #[arg(long)]
        force: bool,
    },
    /// Configure and inspect the LLM adapter (models are plugins).
    #[command(subcommand)]
    Model(ModelCmd),
    /// Ask the AI assistant a question, answered ONLY from attached sources.
    Ask {
        question: Vec<String>,
        /// Limit retrieval to one source by name.
        #[arg(long)]
        source: Option<String>,
        /// How many knowledge extracts to give the model.
        #[arg(short = 'k', long, default_value_t = 8)]
        limit: usize,
    },
    /// Rewrite one clause of an assembled LIR draft with the model
    /// (party names, tone, plain language). Output is validated LIR.
    Adapt {
        /// Path to a LIR JSON file (from `draftos draft --lir`).
        #[arg(long)]
        lir: PathBuf,
        /// Clause number to adapt, e.g. "4".
        #[arg(long)]
        clause: String,
        /// What to change, e.g. "use the actual party names, plain language".
        #[arg(long)]
        instructions: String,
        /// Output path (default: overwrite the input file).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ModelCmd {
    /// Select the model backend: anthropic | openai | ollama | vllm | deepseek | mistral | qwen.
    Set {
        adapter: String,
        /// Provider model id (default: adapter's default).
        #[arg(long)]
        model: Option<String>,
        /// Endpoint override (OpenAI-compatible / self-hosted providers).
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Show the configured model backend and its capabilities.
    Show,
    /// Store an API key for an adapter in the OS keychain (reads from stdin).
    Key { adapter: String },
    /// Remove an adapter's API key from the OS keychain.
    ClearKey { adapter: String },
    /// Unset the configured model backend.
    Clear,
}

#[derive(Subcommand)]
enum SourceCmd {
    /// Attach a folder as a knowledge source and ingest it.
    Add {
        folder: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// List registered sources and their index stats.
    List,
    /// Detach a source (keeps its index; re-attach is instant).
    Detach { name: String },
    /// Re-attach a previously detached source.
    Attach { name: String },
    /// Remove a source registration AND delete its index bundle.
    Remove {
        name: String,
        /// Required confirmation flag, since this deletes the built index.
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let data_dir = resolve_data_dir(cli.data_dir)?;
    let db = AppDb::open(&data_dir)?;
    let embedder = HashEmbedder::new(HashEmbedder::default_dims());

    match cli.command {
        Command::Source(cmd) => run_source(cmd, &db, &data_dir, &embedder),
        Command::Ingest { name } => run_ingest(&name, &db, &embedder),
        Command::Watch { name } => run_watch(&name, &db, &embedder),
        Command::Search {
            query,
            source,
            clause_type,
            kind,
            contract_type,
            limit,
            full,
            expand,
        } => run_search(
            &query.join(" "),
            source,
            clause_type,
            kind,
            contract_type,
            limit,
            full,
            expand,
            &db,
            &embedder,
        ),
        Command::Draft {
            spec,
            out,
            lir,
            force,
        } => run_draft(&spec, &out, lir, force, &db, &embedder),
        Command::Model(cmd) => run_model(cmd, &db),
        Command::Ask {
            question,
            source,
            limit,
        } => run_ask(&question.join(" "), source, limit, &db, &embedder),
        Command::Adapt {
            lir,
            clause,
            instructions,
            out,
        } => run_adapt(&lir, &clause, &instructions, out, &db),
    }
}

const MODEL_SETTING: &str = "model_config";

fn load_model_config(db: &AppDb) -> Result<Option<draftos_models::ModelConfig>> {
    Ok(match db.get_setting(MODEL_SETTING)? {
        Some(v) => Some(serde_json::from_value(v).context("parse stored model config")?),
        None => None,
    })
}

fn require_adapter(db: &AppDb) -> Result<Box<dyn draftos_models::ModelAdapter>> {
    let cfg = load_model_config(db)?
        .context("no model configured — run `draftos model set <adapter>` first")?;
    Ok(draftos_models::build_adapter(&cfg)?)
}

fn resolve_data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir);
    }
    let dirs = directories::ProjectDirs::from("za", "draftos", "DraftOS")
        .context("cannot determine app data directory")?;
    Ok(dirs.data_dir().to_path_buf())
}

fn open_bundle(rec: &SourceRecord) -> Result<SourceBundle> {
    SourceBundle::open(&rec.bundle_dir)
        .with_context(|| format!("open bundle for source '{}'", rec.name))
}

fn require_source(db: &AppDb, name: &str) -> Result<SourceRecord> {
    db.find_source(name)?
        .with_context(|| format!("no source named '{name}' — see `draftos source list`"))
}

fn run_source(
    cmd: SourceCmd,
    db: &AppDb,
    data_dir: &PathBuf,
    embedder: &dyn EmbeddingProvider,
) -> Result<()> {
    match cmd {
        SourceCmd::Add { folder, name } => {
            let folder = folder
                .canonicalize()
                .with_context(|| format!("folder not found: {}", folder.display()))?;
            if !folder.is_dir() {
                bail!("{} is not a directory", folder.display());
            }
            if db.find_source(&name)?.is_some() {
                bail!("a source named '{name}' already exists");
            }
            let id = draftos_core::new_id();
            let bundle_dir = data_dir.join("sources").join(&id);
            let manifest = SourceManifest {
                id: id.clone(),
                name: name.clone(),
                folder: folder.clone(),
                embed_model: embedder.id(),
                embed_dims: embedder.dims(),
                created_at: draftos_core::now_utc(),
                schema_version: SourceManifest::CURRENT_SCHEMA,
            };
            let mut bundle = SourceBundle::create(&bundle_dir, manifest)?;
            db.add_source(&SourceRecord {
                id,
                name: name.clone(),
                folder: folder.clone(),
                bundle_dir,
                attached: true,
                created_at: draftos_core::now_utc(),
            })?;
            db.audit(
                "source_attached",
                serde_json::json!({ "name": name, "folder": folder }),
            )?;
            println!("Attached '{name}' → {}", folder.display());
            let stats = draftos_ingest::scan_source(&mut bundle, embedder)?;
            print_stats(&name, &stats);
            Ok(())
        }
        SourceCmd::List => {
            let sources = db.list_sources()?;
            if sources.is_empty() {
                println!("No knowledge sources. Add one with: draftos source add <folder> --name <name>");
                return Ok(());
            }
            for rec in sources {
                let status = if rec.attached { "attached" } else { "detached" };
                match open_bundle(&rec) {
                    Ok(bundle) => {
                        let (docs, clauses) = bundle.stats()?;
                        println!(
                            "{:<20} [{status}] {}  ({docs} documents, {clauses} knowledge objects)",
                            rec.name,
                            rec.folder.display()
                        );
                    }
                    Err(_) => println!(
                        "{:<20} [{status}] {}  (bundle missing!)",
                        rec.name,
                        rec.folder.display()
                    ),
                }
            }
            Ok(())
        }
        SourceCmd::Detach { name } => {
            if db.set_attached(&name, false)? {
                db.audit("source_detached", serde_json::json!({ "name": name }))?;
                println!("Detached '{name}'. Index kept — re-attach any time with: draftos source attach {name}");
            } else {
                bail!("no source named '{name}'");
            }
            Ok(())
        }
        SourceCmd::Attach { name } => {
            if db.set_attached(&name, true)? {
                db.audit("source_reattached", serde_json::json!({ "name": name }))?;
                println!("Re-attached '{name}' (instant — no reprocessing).");
            } else {
                bail!("no source named '{name}'");
            }
            Ok(())
        }
        SourceCmd::Remove { name, yes } => {
            if !yes {
                bail!("this deletes the built index for '{name}'; re-run with --yes to confirm");
            }
            let rec = require_source(db, &name)?;
            db.remove_source(&name)?;
            std::fs::remove_dir_all(&rec.bundle_dir).ok();
            db.audit("source_removed", serde_json::json!({ "name": name }))?;
            println!("Removed '{name}' and deleted its index bundle.");
            Ok(())
        }
    }
}

fn run_ingest(name: &str, db: &AppDb, embedder: &dyn EmbeddingProvider) -> Result<()> {
    let rec = require_source(db, name)?;
    let mut bundle = open_bundle(&rec)?;
    let stats = draftos_ingest::scan_source(&mut bundle, embedder)?;
    db.audit(
        "source_scanned",
        serde_json::json!({ "name": name, "indexed": stats.indexed, "removed": stats.removed }),
    )?;
    print_stats(name, &stats);
    Ok(())
}

fn run_watch(name: &str, db: &AppDb, embedder: &dyn EmbeddingProvider) -> Result<()> {
    let rec = require_source(db, name)?;
    let mut bundle = open_bundle(&rec)?;
    // Catch up first, then watch.
    let stats = draftos_ingest::scan_source(&mut bundle, embedder)?;
    print_stats(name, &stats);
    println!("Watching {} — save files into it and they'll be indexed. Ctrl-C to stop.",
        rec.folder.display());
    draftos_ingest::watch_source(&mut bundle, embedder, || false, |stats| {
        if stats.indexed > 0 || stats.removed > 0 {
            print_stats(name, stats);
        }
    })?;
    Ok(())
}

/// Open every attached bundle, optionally restricted to one source by name.
fn attached_bundles(db: &AppDb, only: Option<&str>) -> Result<Vec<SourceBundle>> {
    let mut bundles = Vec::new();
    for rec in db.list_sources()? {
        if !rec.attached {
            continue;
        }
        if let Some(only) = only {
            if !rec.name.eq_ignore_ascii_case(only) {
                continue;
            }
        }
        bundles.push(open_bundle(&rec)?);
    }
    Ok(bundles)
}

#[allow(clippy::too_many_arguments)]
fn run_search(
    query: &str,
    source: Option<String>,
    clause_type: Option<String>,
    kind: Option<String>,
    contract_type: Option<String>,
    limit: usize,
    full: bool,
    expand: bool,
    db: &AppDb,
    embedder: &dyn EmbeddingProvider,
) -> Result<()> {
    if query.trim().is_empty() {
        bail!("empty query");
    }
    let bundles = attached_bundles(db, source.as_deref())?;
    if bundles.is_empty() {
        bail!("no attached sources to search");
    }

    let filters = Filters {
        clause_type,
        contract_type,
        kind: kind.as_deref().map(ClauseKind::parse),
    };
    let hits = draftos_retrieval::search(&bundles, embedder, query, &filters, limit)?;
    db.audit(
        "search",
        serde_json::json!({ "query": query, "hits": hits.len() }),
    )?;

    if hits.is_empty() {
        println!("No results.");
        return Ok(());
    }
    for (i, hit) in hits.iter().enumerate() {
        print_hit(i + 1, hit, full);
    }

    if expand {
        let extra = draftos_retrieval::expand_hits(&bundles, &hits)?;
        if !extra.is_empty() {
            println!("\nReferenced by the results above (knowledge graph, 1 hop):");
            for (i, hit) in extra.iter().enumerate() {
                print_hit(i + 1, hit, full);
            }
        }
    }
    Ok(())
}

fn print_hit(n: usize, hit: &draftos_core::ClauseHit, full: bool) {
    let title = match (&hit.term, &hit.heading) {
        (Some(term), _) => format!("“{term}”"),
        (_, Some(h)) => match &hit.number {
            Some(num) => format!("{num}. {h}"),
            None => h.clone(),
        },
        _ => "(untitled)".to_string(),
    };
    let tags: Vec<String> = [
        Some(format!("{:?}", hit.kind).to_lowercase()),
        hit.metadata.clause_type.clone(),
        hit.metadata.contract_type.clone(),
        hit.metadata.jurisdiction.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    println!("\n{n}. {title}  [{}]", tags.join(" · "));
    println!("   source: {} — {}", hit.source_name, hit.file);
    let body = if full {
        hit.body.clone()
    } else {
        snippet(&hit.body, 300)
    };
    for line in body.lines() {
        println!("   {line}");
    }
}

/// The on-disk `.docx` precedent that contributed the most clauses — used as the
/// style donor so the rendered draft matches the pack's formatting.
fn dominant_donor(doc: &draftos_core::LirDocument, records: &[SourceRecord]) -> Option<PathBuf> {
    use std::collections::HashMap;
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for c in &doc.clauses {
        if let (Some(src), Some(file)) = (&c.provenance.source, &c.provenance.file) {
            if !file.is_empty() && file.to_ascii_lowercase().ends_with(".docx") {
                *counts.entry((src.clone(), file.clone())).or_default() += 1;
            }
        }
    }
    let (src, file) = counts.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k)?;
    let rec = records.iter().find(|r| r.name.eq_ignore_ascii_case(&src))?;
    let path = rec.folder.join(&file);
    path.is_file().then_some(path)
}

fn run_draft(
    spec_path: &PathBuf,
    out: &PathBuf,
    write_lir: bool,
    force: bool,
    db: &AppDb,
    embedder: &dyn EmbeddingProvider,
) -> Result<()> {
    let spec_json = std::fs::read_to_string(spec_path)
        .with_context(|| format!("read matter spec: {}", spec_path.display()))?;
    let spec: draftos_core::MatterSpec =
        serde_json::from_str(&spec_json).context("parse matter spec JSON")?;

    // Open the attached sources in scope for retrieval.
    let scope = &spec.source_scope;
    let records = db.list_sources()?;
    let mut bundles = Vec::new();
    for rec in &records {
        if !rec.attached {
            continue;
        }
        if !scope.is_empty() && !scope.iter().any(|s| s.eq_ignore_ascii_case(&rec.name)) {
            continue;
        }
        bundles.push(open_bundle(rec)?);
    }
    if bundles.is_empty() {
        bail!("no attached sources in scope to draft from — attach a source first");
    }

    // Retrieve → assemble.
    let draft = draftos_assemble::assemble(&spec, &bundles, embedder)?;
    let doc = &draft.document;

    println!("Assembled '{}' ({})", doc.meta.title, spec.contract_type);
    for (ct, source, file) in &draft.report.filled {
        if file.is_empty() {
            println!("  ✓ {ct:<24} ← {source}");
        } else {
            println!("  ✓ {ct:<24} ← {source} / {file}");
        }
    }
    for missing in &draft.report.missing {
        println!("  ✗ {missing:<24} (no precedent found)");
    }

    // Validate.
    let report = draftos_validate::validate(doc);
    for f in report.warnings() {
        println!("  ! warning [{}] {}: {}", f.code, f.location, f.message);
    }
    for f in report.errors() {
        println!("  ✗ error   [{}] {}: {}", f.code, f.location, f.message);
    }

    if report.has_errors() && !force {
        db.audit(
            "draft_blocked",
            serde_json::json!({ "title": doc.meta.title, "errors": report.errors().count() }),
        )?;
        bail!(
            "validation failed with {} error(s); fix the matter spec or pass --force to render anyway",
            report.errors().count()
        );
    }

    // Render by extension.
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "docx" => {
            // Inherit the styling of the precedent that contributed most clauses.
            let donor = dominant_donor(doc, &records).and_then(|p| {
                match draftos_render::StyleDonor::from_path(&p) {
                    Ok(d) if d.is_usable() => {
                        println!("  · styled from {}", p.display());
                        Some(d)
                    }
                    _ => None,
                }
            });
            let bytes = draftos_render::render_docx_with_style(doc, donor.as_ref())?;
            std::fs::write(out, bytes)?;
        }
        "md" | "markdown" => {
            std::fs::write(out, draftos_render::render_markdown(doc))?;
        }
        other => bail!("unsupported output extension '.{other}' — use .docx or .md"),
    }

    if write_lir {
        let lir_path = out.with_extension("lir.json");
        std::fs::write(&lir_path, serde_json::to_vec_pretty(doc)?)?;
        println!("  · LIR written to {}", lir_path.display());
    }

    db.audit(
        "draft_rendered",
        serde_json::json!({
            "title": doc.meta.title,
            "clauses": doc.clauses.len(),
            "out": out.to_string_lossy(),
        }),
    )?;
    println!("Wrote {}", out.display());
    Ok(())
}

fn run_model(cmd: ModelCmd, db: &AppDb) -> Result<()> {
    match cmd {
        ModelCmd::Set {
            adapter,
            model,
            base_url,
        } => {
            let cfg = draftos_models::ModelConfig {
                adapter,
                model: model.unwrap_or_default(),
                base_url,
            };
            // Fail fast on unknown adapters before persisting anything.
            let built = draftos_models::build_adapter(&cfg)?;
            db.set_setting(MODEL_SETTING, &serde_json::to_value(&cfg)?)?;
            db.audit(
                "model_configured",
                serde_json::json!({ "adapter": cfg.adapter, "model": cfg.model }),
            )?;
            let caps = built.capabilities();
            println!(
                "Model backend set: {} ({}){}",
                built.id(),
                if cfg.model.is_empty() { "adapter default model" } else { &cfg.model },
                if caps.local { " — local, no data leaves this machine" } else { " — CLOUD adapter: prompts are sent to an external service" },
            );
            Ok(())
        }
        ModelCmd::Show => {
            match load_model_config(db)? {
                None => println!("No model configured. DraftOS works fully without one; \
`ask`/`adapt` need `draftos model set <adapter>`."),
                Some(cfg) => {
                    let adapter = draftos_models::build_adapter(&cfg)?;
                    let caps = adapter.capabilities();
                    println!("adapter:  {}", cfg.adapter);
                    println!(
                        "model:    {}",
                        if cfg.model.is_empty() { "(adapter default)" } else { &cfg.model }
                    );
                    if let Some(url) = &cfg.base_url {
                        println!("endpoint: {url}");
                    }
                    println!(
                        "caps:     json_mode={} streaming={} local={} max_context≈{}",
                        caps.json_mode, caps.streaming, caps.local, caps.max_context
                    );
                }
            }
            Ok(())
        }
        ModelCmd::Key { adapter } => {
            eprint!("Paste API key for '{adapter}' (input is not echoed back): ");
            let mut key = String::new();
            std::io::stdin().read_line(&mut key)?;
            let key = key.trim();
            if key.is_empty() {
                bail!("empty key — nothing stored");
            }
            draftos_models::store_key(&adapter, key)?;
            db.audit("model_key_stored", serde_json::json!({ "adapter": adapter }))?;
            println!("Key stored in the OS keychain (service 'draftos', account '{adapter}').");
            Ok(())
        }
        ModelCmd::ClearKey { adapter } => {
            draftos_models::delete_key(&adapter)?;
            db.audit("model_key_cleared", serde_json::json!({ "adapter": adapter }))?;
            println!("Key removed for '{adapter}'.");
            Ok(())
        }
        ModelCmd::Clear => {
            db.delete_setting(MODEL_SETTING)?;
            println!("Model backend unset.");
            Ok(())
        }
    }
}

fn run_ask(
    question: &str,
    source: Option<String>,
    limit: usize,
    db: &AppDb,
    embedder: &dyn EmbeddingProvider,
) -> Result<()> {
    if question.trim().is_empty() {
        bail!("empty question");
    }
    let adapter = require_adapter(db)?;
    let bundles = attached_bundles(db, source.as_deref())?;
    if bundles.is_empty() {
        bail!("no attached sources — the assistant only answers from your knowledge base");
    }

    // Retrieve, then expand one hop so cited clauses carry their definitions.
    let mut hits =
        draftos_retrieval::search(&bundles, embedder, question, &Filters::default(), limit)?;
    let extra = draftos_retrieval::expand_hits(&bundles, &hits)?;
    hits.extend(extra.into_iter().take(limit));
    if hits.is_empty() {
        bail!("nothing relevant found in the attached sources");
    }

    let req = draftos_prompt::ask(question, &hits);
    let resp = adapter.complete(&req)?;
    db.audit(
        "model_call",
        serde_json::json!({
            "task": "ask",
            "adapter": adapter.id(),
            "model": resp.model,
            "input_tokens": resp.input_tokens,
            "output_tokens": resp.output_tokens,
        }),
    )?;

    println!("{}\n", resp.text.trim());
    println!("Sources:");
    for (i, hit) in hits.iter().enumerate() {
        let title = hit
            .term
            .clone()
            .or_else(|| hit.heading.clone())
            .unwrap_or_else(|| "(untitled)".to_string());
        println!("  [{}] {} — {} / {}", i + 1, title, hit.source_name, hit.file);
    }
    Ok(())
}

fn run_adapt(
    lir_path: &PathBuf,
    clause_number: &str,
    instructions: &str,
    out: Option<PathBuf>,
    db: &AppDb,
) -> Result<()> {
    let adapter = require_adapter(db)?;
    let raw = std::fs::read_to_string(lir_path)
        .with_context(|| format!("read LIR file: {}", lir_path.display()))?;
    let mut doc: draftos_core::LirDocument =
        serde_json::from_str(&raw).context("parse LIR JSON")?;

    let idx = doc
        .clauses
        .iter()
        .position(|c| c.number == clause_number)
        .with_context(|| {
            format!(
                "no clause numbered '{clause_number}' — available: {}",
                doc.clauses
                    .iter()
                    .map(|c| format!("{} ({})", c.number, c.heading))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    let body_text = doc.clauses[idx]
        .body
        .iter()
        .map(|b| b.plain_text())
        .collect::<Vec<_>>()
        .join("\n");
    let def_terms: Vec<String> = doc.definitions.iter().map(|d| d.term.clone()).collect();
    let req = draftos_prompt::adapt_clause(
        &doc.clauses[idx].heading,
        &body_text,
        instructions,
        &doc.parties,
        &def_terms,
    );

    println!("Adapting clause {} ({}) with '{}'…", clause_number, doc.clauses[idx].heading, adapter.id());
    let resp = adapter.complete(&req)?;
    db.audit(
        "model_call",
        serde_json::json!({
            "task": "adapt_clause",
            "adapter": adapter.id(),
            "model": resp.model,
            "clause": clause_number,
            "input_tokens": resp.input_tokens,
            "output_tokens": resp.output_tokens,
        }),
    )?;

    // The model's output must be valid LIR blocks or it never touches the doc.
    let blocks = draftos_prompt::parse_blocks(&resp.text)
        .map_err(|e| anyhow::anyhow!("adaptation rejected: {e}"))?;
    let new_text = blocks.iter().map(|b| b.plain_text()).collect::<Vec<_>>().join("\n");
    let clause = &mut doc.clauses[idx];
    clause.body = blocks;
    clause.defined_terms_used = draftos_assemble::terms_used(&new_text, &def_terms);
    clause.cross_refs = draftos_assemble::cross_refs(&new_text);
    clause.provenance.adapted_by_model = true;

    // Re-validate the whole document; errors block the write.
    let report = draftos_validate::validate(&doc);
    for f in report.warnings() {
        println!("  ! warning [{}] {}: {}", f.code, f.location, f.message);
    }
    if report.has_errors() {
        for f in report.errors() {
            println!("  ✗ error   [{}] {}: {}", f.code, f.location, f.message);
        }
        bail!("adapted clause failed validation — LIR file left unchanged");
    }

    let out_path = out.unwrap_or_else(|| lir_path.clone());
    std::fs::write(&out_path, serde_json::to_vec_pretty(&doc)?)?;
    println!("Clause {clause_number} adapted and validated. Wrote {}", out_path.display());
    Ok(())
}

fn print_stats(name: &str, stats: &draftos_ingest::ScanStats) {
    println!(
        "[{name}] indexed {} file(s) ({} knowledge objects), {} unchanged, {} removed, {} failed",
        stats.indexed, stats.clauses, stats.unchanged, stats.removed, stats.failed
    );
}

fn snippet(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() <= max {
        return flat;
    }
    let mut cut = max;
    while !flat.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &flat[..cut])
}
