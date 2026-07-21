# DraftOS — Project Context

> This file is the canonical context for building DraftOS. Every design decision below was
> agreed with the product owner. Read it fully before writing code. When implementation
> reality forces a deviation, record the deviation and the reason in `docs/DECISIONS.md`.

## 1. What DraftOS Is

DraftOS (formerly LexOS) is a **model-independent legal intelligence platform**, delivered as a
**native Rust desktop application**. It is an enterprise-grade legal operating system, not a
simple contract drafting assistant.

Core capability areas (one common architecture underneath all of them):

1. Contract Drafting
2. Knowledge Management
3. Clause Library
4. Legal Research
5. Document Comparison
6. Risk Analysis
7. Contract Lifecycle Management
8. Digital Signing
9. AI Legal Assistant

Primary jurisdiction focus: **South Africa** (POPIA compliance matters), but jurisdiction is
metadata, never hardcoded.

## 2. Non-Negotiable Principles

1. **The LLM never drafts a contract.** Drafting is deterministic:
   `Matter Intake → Retrieve → Assemble → Validate → Render`.
   The LLM is only ever used to: adapt, rewrite, explain, summarize, and fill variables.
   Everything else — clause selection, ordering, cross-references, numbering, definitions,
   schedules — is done by deterministic code that can be unit-tested.

2. **Models are plugins.** All LLM access goes through a `ModelAdapter` trait. GPT, Claude,
   Gemini, Qwen, Llama, DeepSeek, Mistral, Ollama, vLLM, llama.cpp are interchangeable
   backends. No crate outside `draftos-models` may import a provider SDK or hardcode a
   provider API shape.

3. **Knowledge sources are attachable, detachable, and hot-swappable** with zero impact on
   app performance (see §5). The user points DraftOS at folders on their own filesystem;
   there is no upload/import step.

4. **Ingestion happens once, retrieval happens forever.** We do not run RAG over raw
   documents. We run a Knowledge Ingestion Engine that parses, extracts, enriches, and
   indexes once; queries hit the resulting knowledge base.

5. **Everything the model outputs is Legal Intermediate Representation (LIR)**, never
   free-form prose destined directly for a document (see §8). DOCX/PDF are rendered from
   LIR by the rendering engine.

6. **Local-first.** All indexes, databases, and documents live on the user's machine.
   Network calls happen only when a cloud model adapter is explicitly selected. The app
   must be fully functional (ingest, search, assemble, render) with zero network access
   when paired with a local model or no model.

## 3. Technology Stack

| Concern | Choice | Notes |
|---|---|---|
| App shell | **Tauri 2.x** | Rust core + webview UI. All business logic in Rust; UI is a thin client. |
| UI frontend | Svelte + TypeScript + Tailwind | Talks to Rust only via Tauri commands/events. No business logic in the frontend. |
| Async runtime | tokio | |
| Storage (app + per-source) | SQLite via `rusqlite` | One app DB + one DB **per knowledge source** (§5). |
| Vector search | `sqlite-vec` extension | Embedded in each source's SQLite file. Hybrid search with FTS5 (BM25) + vectors. |
| Full-text search | SQLite FTS5 | Same file as vectors. |
| Knowledge graph | SQLite edge tables (`nodes`, `edges`) per source | No external graph DB at desktop scale. |
| Folder watching | `notify` crate (debounced) | |
| DOCX parse/render | `docx-rs` (or fork if needed) | DOCX is the primary render target. |
| PDF text extraction | `pdfium-render` (bundled pdfium) | Fallback: `pdf-extract`. |
| OCR (scanned PDFs) | `leptess` (Tesseract) behind a cargo feature `ocr` | Optional at build time. |
| Local embeddings | `fastembed` (ONNX, e.g. bge-small-en) | Default. Embeddings must work offline. Adapter trait allows API embeddings too. |
| Serialization | serde + JSON | LIR, manifests, metadata. |
| Errors | `thiserror` (libs), `anyhow` (app edges) | |
| Logging | `tracing` + rotating file in app data dir | Doubles as the audit trail source. |

Rule of thumb: if a pure-Rust option and a binding both work, prefer pure Rust for
portability (Windows + Linux + macOS are all targets — the owner uses `D:\Contracts\` and
`/home/given/contracts/` style paths interchangeably).

## 4. Workspace Layout (modular monolith, not microservices)

The original design listed ~20 services. In a desktop app those become **crates in one
Cargo workspace** with the same single-responsibility boundaries. Service = crate.

```
draftos/
├── Cargo.toml                 # workspace
├── CLAUDE.md                  # this file
├── docs/
│   ├── DECISIONS.md           # deviations from this spec, with reasons
│   └── LIR_SPEC.md            # full LIR schema (§8)
├── crates/
│   ├── draftos-core/          # domain types: Matter, Contract, Clause, LIR, ids, errors
│   ├── draftos-storage/       # app SQLite: matters, contracts, settings, audit log
│   ├── draftos-ingest/        # folder watcher, pipeline orchestrator, job queue
│   ├── draftos-parse/         # DOCX/PDF/TXT/HTML/RTF/ODT → normalized text + structure
│   ├── draftos-ocr/           # feature-gated Tesseract wrapper
│   ├── draftos-extract/       # clause / definition / schedule / metadata extraction
│   ├── draftos-embed/         # EmbeddingProvider trait; fastembed default impl
│   ├── draftos-index/         # per-source bundle: SQLite + sqlite-vec + FTS5 + graph tables
│   ├── draftos-retrieval/     # hybrid search + graph expansion across mounted sources
│   ├── draftos-rules/         # deterministic rule engine (clause requirements, conflicts)
│   ├── draftos-assemble/      # retrieve → order → number → cross-reference → LIR
│   ├── draftos-validate/      # deterministic legal & structural checks over LIR
│   ├── draftos-prompt/        # prompt compiler: task + context + guardrails → messages
│   ├── draftos-models/        # ModelAdapter trait + provider impls (each feature-gated)
│   ├── draftos-render/        # LIR → DOCX (and PDF) 
│   └── draftos-sign/          # digital signing (later phase)
└── apps/
    └── draftos-desktop/       # Tauri app: commands, events, state; ui/ subfolder for Svelte
```

Dependency direction is strictly downward: `apps → crates`, `draftos-core` depends on
nothing internal, nothing depends on `draftos-desktop`. `draftos-models` and
`draftos-embed` are the only crates allowed to make network calls.

## 5. Knowledge Sources — the swappable RAG design (CRITICAL)

The owner's requirement, verbatim: *"I should be able to attach a RAG from my file system
and also I should be able to freely change RAGs without affecting the performance of
DraftOS."*

### Design

A **Knowledge Source** is a self-contained, relocatable bundle:

```
<app-data>/sources/<source-id>/
├── manifest.json      # id, name, watched folder path, embed model + dims, stats, version
└── index.db           # SQLite: documents, clauses, metadata, FTS5, sqlite-vec, graph tables
```

- **Attach** = user picks a folder (e.g. `~/contracts/banking/`). DraftOS creates the
  bundle, starts a watcher, and ingests in the background. The UI shows per-source
  progress; the app stays fully responsive.
- **Detach** = unmount. Watcher stops, DB handle closes. The bundle **is not deleted** —
  re-attaching is instant (O(1), just reopening the file). Deleting a source is a
  separate, explicit, confirmed action.
- **Swap** = detach + attach, i.e. two O(1) mount operations. Zero re-embedding, zero
  effect on other sources, zero effect on app responsiveness.
- **Multiple sources active at once**: Corporate Contracts, Employment, Banking,
  Construction, Litigation, Templates, Policies, Legislation, Case Law, Precedents…
  each with its own bundle and its own watcher. Retrieval fans out across whichever
  sources are currently mounted (user can scope any query to a subset).

### Performance rules (enforced in code review)

1. Ingestion and embedding run on background tokio tasks / a bounded job queue — never on
   the UI thread, never blocking a Tauri command.
2. One SQLite connection pool per mounted source; sources never share write locks.
3. The watcher debounces filesystem events (2s) and diffs by content hash — re-saving an
   unchanged file re-indexes nothing.
4. A source being ingested is already queryable for the portion indexed so far.
5. Embedding model + dimensions are recorded in `manifest.json`. If the global embedding
   model changes, existing sources keep working with their recorded model; re-embedding is
   an explicit per-source "rebuild" the user triggers, run in the background.

### Folder watcher behavior

Like VS Code watches files: new file → ingest; modified → re-ingest (by hash); deleted →
tombstone its rows in the index. No import buttons anywhere.

## 6. Knowledge Ingestion Pipeline

Per file, orchestrated by `draftos-ingest`:

```
New/changed file
→ Parser            (draftos-parse: DOCX, PDF, TXT, HTML, RTF, ODT)
→ OCR               (draftos-ocr: only if scanned/no text layer; feature-gated)
→ Text Cleaner      (normalize whitespace, headers/footers, page artifacts)
→ Clause Detection  (split into clauses — heading heuristics + numbering patterns first;
                     LLM-assisted splitting only as an optional enhancement)
→ Definition Detection    ("X means …" patterns; builds the defined-terms table)
→ Schedule/Annexure Detection
→ Metadata Extraction     (contract type, clause type, jurisdiction, industry, risk,
                           language, version, source file, approved flag)
→ Embeddings        (per clause/definition/recital/schedule/warranty/indemnity/
                     representation/obligation — NEVER whole documents)
→ Write to source index.db (rows + vectors + FTS + graph edges)
```

Clause metadata shape (stored as columns + JSON overflow):

```json
{
  "contract_type": "Share Purchase Agreement",
  "clause_type": "Termination",
  "jurisdiction": "South Africa",
  "industry": "Technology",
  "risk": "High",
  "language": "English",
  "version": "3",
  "source": "SPA.docx",
  "approved": true
}
```

Graph tables capture relationships: Agreement —contains→ Clause; Clause —references→
Definition; Clause —references→ Clause. Retrieval uses vector+BM25 hits, then expands one
hop through the graph so a retrieved Termination clause brings its referenced Definitions
along.

Future source connectors (post-v1, design for but don't build): .msg/Outlook, SharePoint,
OneDrive, Google Drive.

## 7. Drafting Pipeline (deterministic core)

**The corpus supplies the structure, not DraftOS.** DraftOS owns no template. A
draft follows a precedent from the user's own knowledge sources — its clause
order, its nesting, its definitions, its schedules. `draftos-rules` is a
*checklist* that audits the result and reports gaps; it never generates
structure. An unknown contract type has no checklist and drafts from its own
precedents unaudited. This is what makes "attach any legal RAG and it still
works" true rather than aspirational (see docs/DECISIONS.md, 2026-07-21).

```
Matter Intake   → structured questionnaire → typed MatterSpec (draftos-core)
Choose skeleton → the best-matching indexed precedent (contract type, title,
                  jurisdiction, completeness); MatterSpec.skeleton_precedent overrides
Assemble        → rebuild that precedent's clause tree (parents + sub-clauses),
                  substitute variables, renumber hierarchically, remap
                  cross-references, carry definitions/schedules/recitals → LIR
Audit + gap-fill→ draftos-rules reports required clause types the precedent lacks;
                  draftos-retrieval supplies them from other precedents
                  (filtered by contract_type, jurisdiction, approved=true first)
Adapt (LLM)     → only where a clause needs party names, variables, or tone changes:
                  prompt compiler builds a constrained rewrite task; output is LIR nodes
Validate        → draftos-validate: undefined terms, dangling cross-references, missing
                  execution blocks, conflicting clauses, jurisdiction rules — all
                  deterministic, all reported with locations
Render          → draftos-render: LIR → DOCX (primary), PDF (secondary)
```

If validation fails, the document does not render; the user sees the exact failures.

## 8. Legal Intermediate Representation (LIR)

The canonical JSON document format. Every model output and every assembled draft is LIR.
Full schema lives in `docs/LIR_SPEC.md`; core shape:

- `Document { meta, parties[], recitals[], definitions[], clauses[], schedules[], execution }`
- `Clause { id, number, heading, body: Vec<Block>, children: Vec<Clause>,
  cross_refs[], defined_terms_used[], source_provenance }` — `children` holds
  sub-clauses, so a precedent's nesting survives; `number` is hierarchical
  ("3", "3.1", "3.1.2") and assigned only by the assembler
- `Block` = paragraph / numbered list / table / placeholder-variable
- Every node carries provenance (which source, which file, which original clause) so any
  rendered paragraph is traceable back to a precedent.

LLM adapters must return LIR fragments (JSON) validated against the schema before they
touch the document; free-text responses are only permitted for chat/explain features.

## 9. Model Adapter SDK

```rust
#[async_trait]
pub trait ModelAdapter: Send + Sync {
    fn id(&self) -> &str;                     // "anthropic", "openai", "ollama", ...
    fn capabilities(&self) -> Capabilities;   // json_mode, max_context, streaming, local
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>;
    async fn complete_streaming(...) -> Result<CompletionStream>;
}
```

- Provider implementations (each behind a cargo feature): Anthropic (Claude), OpenAI,
  Google Gemini, Ollama, llama.cpp server, vLLM (OpenAI-compatible endpoints share one
  impl), DeepSeek/Mistral/Qwen via the OpenAI-compatible impl.
- API keys stored in the OS keychain (`keyring` crate), never in SQLite or config files.
- The prompt compiler (`draftos-prompt`) is the only producer of `CompletionRequest`s for
  drafting tasks: it assembles task template + retrieved context + guardrails + LIR output
  schema. Ad-hoc prompt strings scattered through the codebase are a defect.
- `EmbeddingProvider` is a parallel, smaller trait in `draftos-embed` (local fastembed is
  the default; API embeddings optional).

## 10. Security & Compliance

- Local-first (§2.6) is the primary POPIA posture: client documents never leave the
  machine unless the user explicitly selects a cloud model, and the UI must make it
  visually obvious when a cloud adapter is active on a matter.
- Append-only audit log table in the app DB: ingestion events, retrievals used in a draft,
  model calls (adapter id, token counts, never full document bodies), renders, exports.
- Single-user desktop in v1; the schema still carries `org_id`/`user_id` columns so the
  later multi-user/server phase doesn't need migrations of meaning.
- Secrets in OS keychain; no telemetry.

## 11. Roadmap

**Phase 1 — Knowledge core (MVP)**
Workspace scaffold; app DB; attach/detach/watch folder sources; parse DOCX+PDF+TXT;
clause/definition extraction (heuristics); local embeddings; hybrid search UI
("find me termination clauses from banking precedents"). *This alone is already useful.*

**Phase 2 — Deterministic drafting**
MatterSpec intake, rule engine, assembly to LIR, validation engine, DOCX rendering.
No LLM required yet.

**Phase 3 — Model layer**
ModelAdapter SDK + prompt compiler; clause adaptation, variable filling, explain/
summarize; AI assistant chat scoped to mounted sources; knowledge-graph expansion
in retrieval.

**Phase 4 — Lifecycle & enterprise**
Document comparison, risk analysis, CLM states, digital signing, OCR polish, extra
connectors (email/SharePoint/Drive), multi-user groundwork.

## 12. Build & Test (Phase 1 implemented)

```bash
cargo build --release -p draftos-cli   # → target/release/draftos
cargo test --workspace
draftos source add <folder> --name <n> # attach + ingest a knowledge source
draftos watch <n>                      # live folder watching
draftos search <query> [--kind definition] [--type Termination] [-k N] [--expand]
draftos draft --spec matter.json --out draft.docx [--lir] [--force]
draftos model set <adapter> [--model M] [--base-url URL]   # anthropic|openai|ollama|vllm|deepseek|mistral|qwen
draftos model key <adapter>            # store API key in the OS keychain
draftos ask <question> [--source S] [-k N]                 # RAG chat, cites [n]
draftos adapt --lir d.lir.json --clause 4 --instructions "…" [--out o.lir.json]
```

Implemented so far: core (incl. LIR + MatterSpec), parse (DOCX/PDF/TXT/HTML),
extract, embed (offline hash embedder), index (per-source SQLite + FTS5 +
sqlite-vec + graph edges), ingest (incremental scan + watcher), retrieval
(RRF hybrid), storage (registry + audit + settings), CLI, the Tauri 2 desktop
app (`draftos-desktop`: Library sidebar with attach/detach/rescan/remove,
background watcher per source, hybrid search with filters, Model settings +
an Assistant tab with a cloud/local privacy banner), **and the Phase 2
deterministic drafting pipeline**: rules (an *advisory checklist* of required
clause types per contract type, plus where a missing one belongs — it never
generates structure), assemble (**precedent-led**: picks the best-matching
indexed precedent as a skeleton, rebuilds its clause tree with sub-clauses,
substitutes `{{variables}}`, renumbers hierarchically, remaps cross-references
old→new, and carries its definitions, schedules and recitals through, with
provenance on every node), validate (unresolved variables, hierarchical
numbering, dangling cross-refs, undefined terms, missing parties/execution,
and `foreign-party-name` — a company named in the draft that is not a party,
i.e. a precedent's parties surviving a copy-paste — errors block rendering),
render (LIR → DOCX and Markdown, walking the clause tree), **and the Phase 3 model layer**: draftos-models
(synchronous `ModelAdapter` over ureq; Anthropic + one OpenAI-compatible
adapter covering OpenAI/Ollama/vLLM/DeepSeek/Mistral/Qwen; keys in the OS
keychain with env-var override; selection persisted in the app-DB settings
table), draftos-prompt (prompt compiler — sole producer of drafting
CompletionRequests; adapt-clause output must parse as LIR blocks and
re-validate before it is written; ask/explain are free-text with [n]
citations), one-hop knowledge-graph expansion in retrieval (`expand_hits`,
used by `search --expand` and `ask`), the CLI commands `model`, `ask`,
`adapt`, **and the desktop assistant UI**: a Model settings box (adapter +
keychain key management) and a Search/Assistant tab split in the Tauri app,
with an unmissable cloud-vs-local privacy banner (CLAUDE.md §10), **and the
desktop drafting UI**: a Draft tab with a matter-intake form (contract type
from draftos-rules + custom, parties, jurisdiction, source scope) that runs
the same assemble → validate → render pipeline as `draftos draft` off the
async runtime via `spawn_blocking` — it previews the assembled clauses with
provenance, surfaces the assembly report (filled/missing) and validation
findings (errors block, warnings don't), reveals the precedents' `{{variable}}`
placeholders as fields to fill, and saves a DOCX/MD via a native save dialog
(`pick_save_path`, same offline shell-out as the folder picker). Not yet:
streaming completions, OCR, digital signing, and a clause-adapt UI in the
desktop app. Deviations from this spec are logged in docs/DECISIONS.md.

MatterSpec JSON shape (see `crates/draftos-core/src/matter.rs`): title,
contract_type, jurisdiction, parties[{id,name,role,reg_no,…}], variables
{name→value}, source_scope[], include/exclude_clause_types[],
skeleton_precedent (name a precedent to draft from, overriding the automatic
choice), approved_only.

Desktop app: `cargo build --release -p draftos-desktop`. The UI lives in
`apps/draftos-desktop/ui/` (plain HTML/CSS/JS, no build step) and calls the
Rust crates through Tauri commands in `src/main.rs`. Every command delegates
to the same crates the CLI uses; ingestion and watching run on background
threads and report to the UI via `source-updated` / `source-error` events.

## 13. Working Agreements for Claude Code

- Keep this file authoritative; update it when the owner changes direction.
- Test the deterministic core hard: parsers, extractors, assembler, validator, renderer
  all get unit tests with fixture documents in `crates/*/tests/fixtures/`.
- Never put business logic in the Svelte frontend; it renders state and sends commands.
- Any new network call outside `draftos-models`/`draftos-embed` is a design violation.
- Prefer boring, testable heuristics over LLM calls whenever both would work.
