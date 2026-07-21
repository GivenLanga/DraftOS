# DraftOS — Decision Log

Deviations from `CLAUDE.md` and notable implementation decisions, newest first.
Each entry: date, decision, reason, alternative considered.

## 2026-07-21 — Follow the precedent's numbering scheme, don't synthesise decimals
The previous pass stripped source `<w:numPr>` and applied DraftOS's own "1. 2.
3." — itself a hardcode, ignoring that each pack defines its own scheme
(measured: the loan pack's numbering.xml has 62 abstract lists — decimal `%1.`,
`%1.%2.%3`, upperLetter `A.`, lowerRoman `(i)`, `Annexure %1`…). OOXML numbering
is *dynamic*: numbers are computed from a paragraph's position among others
sharing its `numId`+`ilvl`. So the faithful fix is to keep the source `numPr`,
carry the source's `numbering.xml` (already via StyleDonor), include the clause
*heading* paragraphs, and let Word/LibreOffice renumber. Validated by rendering
a reassembled draft through LibreOffice → PDF: clauses came out
`1 / 2 / 3 / 3.1 / 3.1.1 / 3.2.1.1 / 4 / 4.1.1 …` in the pack's own decimal
multilevel scheme, coherently renumbered for the new document order.

Implementation: capture the clause heading paragraph's OOXML too
(`ExtractedClause.heading_ooxml` → index column → `ClauseHit`/`LirClause`); the
DOCX renderer emits the source heading verbatim (numbering intact) instead of a
synthesised number. Synthetic clauses (Definitions) and headings for clauses
whose heading OOXML we lack join the donor's *backbone list* — StyleDonor now
detects `num_ids` (defined in numbering.xml) and `main_num_id` (the numId used by
the most ilvl-0 paragraphs), and a synthesised heading is given
`<w:numPr><w:ilvl 0/><w:numId main/></w:numPr>` so it numbers in the pack's own
top-level format. A lifted paragraph keeps its numPr only if its numId is
defined in the donor (else it's stripped — the list can't resolve). Definitions
also carry each entry's source paragraph OOXML (`Definition.ooxml`, mapped by
the body↔ooxml parallel arrays in extract). Field codes (`REF`/`PAGEREF` cross-
references) are flattened to their cached text — unwrap `<w:fldSimple>`, drop
the `<w:r>` runs holding `<w:fldChar>`/`<w:instrText>` — so a reference to a
bookmark that isn't in the assembled doc shows its last value instead of
"Error: Reference source not found". Known limitation: those cached cross-
reference numbers are *stale* (e.g. "clause 14.1.3" for what is now 3.1.3) —
true cross-reference re-resolution (bookmark tracking + renumber mapping) is a
separate, larger feature.

## 2026-07-21 — Preserve original clause OOXML for true formatting fidelity
The style-donor fix below matched the pack's *defaults* but drafts still looked
wrong. Root cause (measured on the real pack): the source's appearance comes
almost entirely from formatting that ingestion discarded — 527/732 paragraphs
auto-numbered, 544 indented, 365 justified, 3042/3060 runs with direct fonts,
19 paragraph styles — while the index stored only normalized plain text. No
renderer-only fix can reproduce what was never captured. Owner chose "keep the
look, renumber". Pivotal finding: the pack's numbering is *paragraph-level*
(`<w:numPr>`), not baked into the styles (which carry only indent/font/size), so
we can keep the styles and strip the numbers cleanly.

Fix (threads original OOXML through the whole pipeline): the DOCX parser
captures each paragraph's raw `<w:p>…</w:p>` (sliced by string scan — robust,
`<w:p>` never nests — not quick_xml byte offsets, which came up 3 bytes short
and truncated `</w:p>`); extract carries the body paragraphs' XML per clause
(heading excluded — we synthesise it); the index stores it as a `body_ooxml`
JSON column (guarded `ALTER TABLE` upgrades old bundles; re-ingest populates it);
retrieval hydrates it onto `ClauseHit`; assemble puts it on `LirClause.source_ooxml`
(with `{{var}}` substituted); render, when a style donor is active, emits those
paragraphs instead of synthesising from plain text. Each lifted paragraph is
sanitised: strip `<w:numPr>` (source numbers gone, DraftOS numbers applied),
unwrap `<w:hyperlink>` (its `r:id` points at a source relationship we don't
carry — keep the text, drop the dangling link), and drop `<w:drawing>/<w:pict>/
<w:object>` (media we don't carry); the donor's `<w:document>` root namespaces
are reused so `w14:`/`mc:` attributes on lifted paragraphs stay valid. Non-DOCX
sources and synthetic clauses (Definitions) keep the plain-text path. Verified:
a drafted NDA now renders its clauses in the precedent's own styles (Clause2Sub,
Clause4Sub…), numPr count 0, no dangling r:id, all parts well-formed. Known
limitations: sub-clause numeric labels are dropped (indentation preserved), and
definitions still render as plain text.

## 2026-07-21 — Rendered DOCX inherits a precedent's styling (style donor)
Owner: an assembled draft must look like the pack it was drawn from, not a bare
default-Word document (the renderer shipped NO styles.xml, so Word applied
Calibri/default spacing). Fix: `draftos-render` gains `StyleDonor` +
`render_docx_with_style`. A donor is a real precedent `.docx` whose
`styles.xml`, `theme1.xml`, `fontTable.xml`, `numbering.xml`, `settings.xml`
and body-level `<w:sectPr>` (page size + margins, header/footer refs stripped)
are copied verbatim into the output package and wired via a generated
`document.xml.rels` + content-type overrides. The assembled body carries no
`pStyle`, so it inherits the donor's `Normal`/docDefaults — its font, size and
spacing then match the source exactly (verified: output default font came out
Century Gothic, byte-identical `styles.xml` to the donor). Headings are
deliberately rendered with **direct** bold formatting rather than the donor's
heading styles: legal heading styles often auto-number, which would collide
with the numbers the assembler already assigns (double numbering). The apps
choose the donor as the on-disk `.docx` precedent that contributed the most
clauses (from clause provenance source+file → SourceRecord.folder), falling
back to the plain built-in path when none is a usable `.docx`. Caveat: a draft
can pull clauses from several differently-styled files; the donor gives the
whole document one consistent house style (the dominant contributor's), which
is what a final contract wants anyway. `render_docx` (no donor) is unchanged
and stays byte-compatible with its existing test.

## 2026-07-21 — Desktop app: Draft tab surfaces the Phase 2 pipeline
The Tauri shell gains a third workbench tab, Draft, that makes deterministic
drafting usable without the CLI. New commands: `draft_contract_types` (known
types + their required clauses from draftos-rules), `draft_preview` (assemble →
validate, writes nothing) and `draft_save` (re-runs the pipeline and renders to
a path). Both heavy commands run in `spawn_blocking` like `ask_assistant`.
Design points: (1) preview and save each re-run assemble+validate from the
MatterSpec rather than round-tripping an LIR blob through the webview — the
deterministic pipeline is the single source of truth, and the webview never
holds authority over document content; (2) `draft_save` re-validates and blocks
on errors server-side (honours a `force` flag mirroring the CLI's `--force`),
so a stale/edited form can't slip an invalid document past validation; (3) the
form reveals the chosen precedents' `{{variable}}` placeholders as fields to
fill — `collect_variables` walks the assembled LIR (into list items/table
cells) so the user fills exactly what those clauses need, iterating
preview→fill→preview until no unresolved-variable errors remain; (4) DOCX/MD
output path comes from a native save dialog (`pick_save_path`) using the same
offline zenity/kdialog/osascript/PowerShell shell-out as the folder picker (no
tauri-plugin-dialog — not in the offline cargo cache). Verified end-to-end
against the real Templates source via the CLI (which shares the pipeline):
assembled a 5-clause NDA with provenance, correctly reported Termination as
having no precedent, and rendered a DOCX.

## 2026-07-21 — Per-source SQLite connections must set a busy_timeout
Bug: a fully-indexed source showed "0 docs · 0 clauses" in the desktop sidebar
even though search returned its clauses. Cause: each mounted source has a
long-lived writer connection (the watcher thread) plus short-lived reader
connections (list_sources/search open their own). `init_schema` set
`journal_mode=WAL` and `synchronous=NORMAL` but no `busy_timeout`, so rusqlite's
default (0) made any reader that opened *while the watcher was mid-write* fail
immediately with SQLITE_BUSY. `list_sources` swallowed that into
`.unwrap_or((0,0))`, and since no further `source-updated` event fired, the
sidebar stayed frozen at zero. Fix: `PRAGMA busy_timeout = 5000` as the first
statement in `init_schema` (set before `journal_mode=WAL`, the pragma most
likely to contend), so concurrent opens wait out the writer instead of erroring;
and `list_sources` now logs a `tracing::warn!` on a failed stat read instead of
silently reporting 0. Invariant going forward: every per-source connection sets
a busy_timeout — reader/writer contention across connections is expected by
design (§5: one pool per source, background watchers), so immediate-fail on lock
is always wrong here.

## 2026-07-21 — Desktop app: folder picker shells out to the native chooser
"Attach a folder" now has a Browse… button backed by a `pick_folder` Tauri
command that shells out to the OS-native directory chooser (zenity → kdialog →
qarma on Linux; `osascript choose folder` on macOS; a WinForms
`FolderBrowserDialog` via PowerShell on Windows), run in `spawn_blocking`. The
obvious route — `tauri-plugin-dialog` — was rejected because that crate (and its
`rfd`/`ashpd` backends) is not in the local cargo cache and this machine has no
crates.io access, so adding it would break the offline build; shelling out needs
zero new dependencies. Wrapping the picker in our own command also keeps the
webview calling only app-defined commands, so no ACL/capabilities file is needed
(plugin IPC commands would have required `dialog:allow-open` grants). The text
input stays for paste/power users; picking a folder auto-suggests the source
name from its basename. If no picker binary is found the command errors and the
user just types the path. Revisit `tauri-plugin-dialog` once crates.io is
reachable — it would restore a true native dialog on headless/portal setups.

## 2026-07-21 — Desktop app: Assistant tab + Model settings, cloud is unmissable
The Tauri shell now exposes the Phase 3 model layer. Sidebar gains a Model box
(pick adapter, model id, endpoint; store an API key straight into the OS
keychain; unset). The workbench gains a Search/Assistant tab split; the
Assistant runs the same retrieve → expand → prompt-compile → complete pipeline
as `draftos ask`, off the async runtime via `spawn_blocking` so the window
never blocks on the network call. Answers render as a card with the model's
text, a provenance badge, and a numbered Sources list. Per CLAUDE.md §10, the
active adapter's locality drives a colour signal that can't be missed: a red
"CLOUD MODEL ACTIVE — your questions and retrieved clause text are sent to
<adapter>" banner above the ask box (and a red Model summary) whenever a
cloud adapter is selected, a muted grey "nothing leaves this machine" banner
for local ones. Model *selection* persists in the shared app-DB `settings`
table, so the CLI and desktop app read one configuration. Verified end-to-end
under Xvfb (python-xlib/XTEST driving clicks + keys): both privacy banners,
and a full local-model ask against a mock OpenAI-compatible server returning a
cited answer with graph-expanded sources.

## 2026-07-21 — Phase 3: synchronous ModelAdapter over ureq; streaming deferred
CLAUDE.md §9 sketches an async trait; the implemented `ModelAdapter` in
draftos-models is synchronous, extending the "sync core, async only at the
shell" decision below. HTTP goes through `ureq` (blocking, pure-Rust rustls) —
no tokio/reqwest in the dependency tree; the CLI calls adapters inline and the
desktop shell will use its existing background threads. Two implementations
cover the whole provider matrix: a native Anthropic Messages-API adapter and
one OpenAI-compatible adapter that serves OpenAI, Ollama, vLLM, llama.cpp,
DeepSeek, Mistral and Qwen (each id gets sensible base-url/model defaults, and
`localhost` endpoints are treated as local/no-key). `complete_streaming` is
deferred — `Capabilities::streaming` reports `false` honestly; add it when a
UI actually streams. API keys resolve env-var first (`ANTHROPIC_API_KEY`,
`OPENAI_API_KEY`, `DRAFTOS_<ID>_API_KEY`) then the OS keychain (`keyring`,
service "draftos"), so headless machines work without a keychain daemon; keys
are never written to SQLite or config. The model *selection* (adapter id,
model, base_url) persists in a new app-DB `settings` table. The prompt
compiler (draftos-prompt) is the only producer of `CompletionRequest`s for
drafting tasks: adapt-clause output must parse as LIR blocks (`parse_blocks`
rejects prose, fences tolerated) and the adapted document must re-validate
before it is written; `ask`/`explain` are the only free-text paths and answer
strictly from retrieved extracts with [n] citations. Every model call is
audited with adapter id + token counts, never bodies. Retrieval gained
one-hop graph expansion (`expand_hits`): a retrieved clause pulls in the
definitions it references via the per-source `edges` table; `search --expand`
and `ask` use it.

## 2026-07-21 — Phase 2: drafting is retrieval + assembly, LLM-free
The deterministic drafting pipeline is live: `draftos draft --spec m.json
--out d.docx`. For each clause type the contract-type rule requires,
draftos-assemble retrieves the best precedent (strict contract-type filter
first, then relaxed), strips the precedent's numbering, substitutes
`{{variable}}` values from the MatterSpec, and assembles ordered LIR with
fresh dense numbering and per-clause provenance. The Definitions clause is
built from retrieved definitions *filtered to terms the placed clauses
actually use*. draftos-validate blocks rendering on unresolved variables,
broken numbering, or missing parties/execution (warnings for dangling
cross-refs/undefined terms); `--force` renders anyway for inspection. DOCX is
written by emitting WordprocessingML directly into the OOXML zip (no docx-rs:
we need ~6 constructs and direct emission is more robust and dependency-light).
Rule tables in draftos-rules are plain Rust consts — auditable data, no DSL —
covering NDA/SPA/Loan/Employment/Service/Lease plus a generic canonical order
fallback. The CONFLICTS table is scaffolded but intentionally not enforced
until real legal conflict pairs are added with documented reasons.

## 2026-07-21 — Desktop app: plain-JS frontend, thread-per-source watchers
The Tauri 2 shell's UI is plain HTML/CSS/JS with no bundler — the app is small
enough that a build toolchain (Svelte/Vite, as CLAUDE.md §3 originally
suggested) would add friction without payoff; revisit if the UI grows complex.
The synchronous core crates are driven from the shell on dedicated OS threads:
one watcher thread per attached source, each with an `AtomicBool` stop flag,
progress surfaced to the webview via Tauri events. Verified end-to-end under a
virtual display (Xvfb) — the app boots, resumes watchers, loads real index
stats, and renders. Built against WebKitGTK 2.52 after the webkit2gtk-4.1 dev
package was installed.

## 2026-07-21 — Synchronous core crates; async only at the shell
The pipeline crates (parse, extract, embed, index, ingest, retrieval) are
synchronous. The desktop shell runs them on background tasks
(`spawn_blocking`); the CLI runs them inline. This keeps SQLite usage simple
(one connection per bundle, no Send/Sync juggling) and the crates trivially
testable. Revisit only if a real concurrency need appears inside the core.

## 2026-07-21 — Offline hash embedder first, ONNX embedder later
The default `EmbeddingProvider` is a deterministic feature-hashing embedder
(word unigrams + char trigrams, 256 dims). It is fully offline, instant, and
dependency-free; semantic quality meanwhile comes mostly from FTS5/BM25 in the
hybrid ranking. `fastembed` (ONNX, bge-small) will be added behind a cargo
feature as a drop-in provider. Because every bundle records its embed model in
manifest.json, the upgrade never silently invalidates existing sources — users
rebuild a source explicitly when they want the better model.

## 2026-07-21 — CLI harness before GUI
`apps/draftos-cli` (`draftos`) exercises the entire knowledge core without a
GUI. The Tauri shell is blocked on the `libwebkit2gtk-4.1-dev` system package
(see apps/draftos-desktop/README.md); the CLI proves the crates and remains a
permanent dev/test tool once the desktop app exists.

## 2026-07-21 — Project renamed LexOS → DraftOS
Owner decision. Folder name remains `LexOS` on disk for now; all code, crates, and
documents use the DraftOS name.

## 2026-07-21 — Desktop modular monolith instead of ~20 microservices
The original architecture pack described ~20 independent services. As a Rust desktop
application, those boundaries are preserved as single-responsibility crates in one Cargo
workspace instead. Same separation, no network hops, no orchestration overhead. The
service split remains the blueprint if a server edition is built later.

## 2026-07-21 — Per-source SQLite bundles for swappable RAG
Each knowledge source gets its own `index.db` (SQLite + sqlite-vec + FTS5) so
attach/detach/swap is a file mount/unmount — O(1), no re-embedding, no cross-source lock
contention. Alternative considered: one shared vector DB with a `source_id` column —
rejected because detaching would require mass deletes and swapping sources would churn
the shared index.
