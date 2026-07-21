# Legal Intermediate Representation (LIR) — Specification v0.1 (draft)

LIR is the canonical JSON format for every document DraftOS assembles and every structured
output a model adapter returns. DOCX/PDF are rendered from LIR only; no component writes
prose directly into a document.

Status: implemented in `crates/draftos-core/src/lir.rs` (v0.1). That module is
authoritative; this file describes it. Notable additions over the original
sketch: parties carry a `role` (the defined role, e.g. "Disclosing Party"),
definitions carry `provenance`, and every clause's `number` is assigned only
by draftos-assemble. Runs use a `style` enum (normal/bold/italic) rather than
a string.

## Top level

```jsonc
{
  "lir_version": "0.1",
  "meta": {
    "title": "Share Purchase Agreement",
    "contract_type": "Share Purchase Agreement",
    "jurisdiction": "South Africa",
    "language": "en",
    "matter_id": "…",
    "created_at": "…"
  },
  "parties": [
    { "id": "seller", "name": "…", "type": "company", "reg_no": "…", "address": "…" }
  ],
  "recitals": [ { "id": "r1", "body": [ /* Block */ ] } ],
  "definitions": [
    { "term": "Effective Date", "body": [ /* Block */ ], "used_by": ["c-termination"] }
  ],
  "clauses": [ /* Clause */ ],
  "schedules": [ { "id": "sch1", "title": "…", "body": [ /* Block */ ] } ],
  "execution": { "blocks": [ /* signature blocks per party */ ] }
}
```

## Clause

```jsonc
{
  "id": "c-termination",
  "number": "12",                       // assigned by the assembler, never by a model
  "heading": "Termination",
  "body": [ /* Block */ ],
  "cross_refs": ["c-breach", "c-notice"],
  "defined_terms_used": ["Effective Date", "Material Breach"],
  "provenance": {
    "source_id": "banking-precedents",
    "file": "SPA.docx",
    "original_clause_id": "…",
    "adapted_by_model": false           // true if an LLM rewrote/filled this clause
  }
}
```

## Block

A `Block` is one of:

- `{ "type": "paragraph", "runs": [ { "text": "…", "style": "normal|bold|italic" } ] }`
- `{ "type": "list", "ordered": true, "items": [ [ /* Block */ ] ] }`
- `{ "type": "table", "rows": [ [ /* cell: Vec<Block> */ ] ] }`
- `{ "type": "variable", "name": "purchase_price", "label": "Purchase Price", "value": null }`

`variable` blocks are placeholders the intake/LLM fill step resolves; rendering fails
validation while any required variable is unresolved.

## Invariants (enforced by draftos-validate)

1. Every entry in `cross_refs` resolves to an existing clause id.
2. Every term in `defined_terms_used` exists in `definitions`.
3. Every definition is used at least once (warning, not error).
4. Numbering is dense, ordered, and assigned only by the assembler.
5. Every clause carries provenance.
6. No unresolved required `variable` blocks at render time.
7. Model-returned LIR fragments must validate against this schema before merging.
