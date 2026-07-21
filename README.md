# DraftOS

A model-independent legal intelligence platform, built in Rust. See
[CLAUDE.md](CLAUDE.md) for the full architecture and principles.

**Status: Phase 1 (knowledge core) + Phase 2 (deterministic drafting) working.**
Attach folders of contracts as knowledge sources; DraftOS parses them
(DOCX/PDF/TXT/HTML), extracts clauses, definitions and schedules with
metadata, indexes everything into a per-source SQLite bundle (FTS5 +
sqlite-vec), and answers hybrid searches across all attached sources. Sources
are hot-swappable: detaching keeps the index, so re-attaching is instant and
never reprocesses anything. On top of that, `draftos draft` assembles a full
contract from your own precedents — retrieve → assemble → validate → render
to DOCX — with no LLM involved: clause selection follows per-contract-type
rules, every clause keeps provenance to the precedent it came from, and
validation blocks rendering on unresolved variables or structural defects.

## Build

```bash
cargo build --release -p draftos-desktop   # the GUI app → target/release/draftos-desktop
cargo build --release -p draftos-cli       # the CLI harness → target/release/draftos
cargo test --workspace
```

The desktop app (Tauri 2 + WebKitGTK) needs these system packages on Linux:

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
```

## Desktop app

`draftos-desktop` is the GUI. Left sidebar is your **Library** of knowledge
sources; paste a folder path and a name to attach one, and every clause in it
becomes searchable. Each source watches its folder in the background (one
thread per source), so files you save into the folder are indexed live without
freezing the window. Detach / Rescan / Remove act on a source in place —
detaching keeps the index so re-attaching is instant. The main pane is hybrid
search with kind / clause-type / source filters; each result shows the clause,
its classification, and exactly which source and file it came from.

## Use

```bash
# Attach a folder of contracts as a knowledge source (ingests immediately)
draftos source add ~/contracts/banking --name banking

# Keep watching the folder — files saved into it are indexed automatically
draftos watch banking

# Hybrid search (BM25 + vector) across all attached sources
draftos search termination for material breach
draftos search confidential information --kind definition
draftos search payment --type "Payment" --contract-type "Loan Agreement" -k 5 --full

# Swap sources freely — indexes are preserved, re-attach is instant
draftos source detach banking
draftos source attach banking
draftos source list

# Draft a contract from your precedents (deterministic — no LLM)
draftos draft --spec matter.json --out draft.docx --lir
```

A MatterSpec is a small JSON file:

```json
{
  "title": "Mutual Non-Disclosure Agreement",
  "contract_type": "Non-Disclosure Agreement",
  "jurisdiction": "South Africa",
  "parties": [
    { "id": "disclosing", "name": "Acme (Pty) Ltd", "role": "Disclosing Party" },
    { "id": "receiving",  "name": "Beta (Pty) Ltd", "role": "Receiving Party" }
  ],
  "variables": { "monthly_fee": "R85,000.00" },
  "source_scope": ["banking"]
}
```

The draft report shows exactly which precedent filled each clause type, which
required clause types found no precedent, and every validation finding.
Validation errors (e.g. an unfilled `{{variable}}`) block rendering unless
`--force` is passed.

App data (source registry + index bundles) lives in the OS app-data directory;
override with `--data-dir` or `DRAFTOS_DATA`.

## Workspace

| Crate | Responsibility |
|---|---|
| `draftos-core` | Domain types, manifests, errors |
| `draftos-parse` | DOCX / PDF / TXT / HTML → paragraphs |
| `draftos-extract` | Clause / definition / schedule splitting + metadata heuristics |
| `draftos-embed` | `EmbeddingProvider` trait + offline default embedder |
| `draftos-index` | Per-source bundle: SQLite + FTS5 + sqlite-vec + graph edges |
| `draftos-ingest` | Incremental folder scanning + live watcher |
| `draftos-retrieval` | Hybrid search with reciprocal-rank fusion across sources |
| `draftos-rules` | Required clause types + canonical order per contract type |
| `draftos-assemble` | MatterSpec + retrieval → ordered, numbered LIR with provenance |
| `draftos-validate` | Deterministic checks over LIR; errors block rendering |
| `draftos-render` | LIR → DOCX (WordprocessingML) and Markdown |
| `draftos-storage` | App DB: source registry + audit log |
| `apps/draftos-cli` | CLI harness (`draftos`) |
| `apps/draftos-desktop` | Tauri 2 desktop app (thin UI over the same crates) |
