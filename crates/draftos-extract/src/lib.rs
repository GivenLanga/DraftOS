//! Deterministic extraction: split a parsed document into clauses,
//! definitions and schedules, and infer metadata with keyword heuristics.
//! No LLM calls here, ever — this layer must be reproducible and testable.

mod metadata;
mod split;

pub use metadata::{classify_clause_type, detect_contract_type, normalize_label};

use draftos_core::{ClauseKind, ClauseMetadata, ExtractedClause, ParsedDocument};

/// Extract knowledge objects from a parsed document.
///
/// `rel_path` is the document's path relative to the watched folder; it is
/// recorded as provenance on every clause.
pub fn extract(doc: &ParsedDocument, rel_path: &str) -> Vec<ExtractedClause> {
    let full_text: String = doc
        .paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let contract_type = metadata::detect_contract_type(&doc.file_name, &full_text);
    let jurisdiction = metadata::detect_jurisdiction(&full_text);

    let mut clauses = split::split_into_clauses(doc);

    // Second pass: harvest definitions out of clause bodies as standalone
    // knowledge objects, and classify each clause. Each body paragraph lines up
    // with its OOXML (parallel vectors), so a harvested definition can keep the
    // source paragraph it came from — letting the Definitions clause later render
    // in the precedent's house style.
    let mut definitions = Vec::new();
    for clause in &clauses {
        for (idx, para_text) in clause.body.split('\n').enumerate() {
            for (term, sentence) in split::find_definitions(para_text) {
                let ooxml = clause.ooxml.get(idx).cloned().into_iter().collect();
                definitions.push(ExtractedClause {
                    kind: ClauseKind::Definition,
                    number: None,
                    // Definitions are addressed by term, not position; they keep
                    // their defining clause's seq so provenance stays ordered.
                    seq: clause.seq,
                    depth: 0,
                    heading: None,
                    term: Some(term),
                    body: sentence,
                    ooxml,
                    heading_ooxml: None,
                    metadata: ClauseMetadata::default(),
                });
            }
        }
    }
    clauses.append(&mut definitions);

    for clause in &mut clauses {
        let heading = clause.heading.as_deref().unwrap_or("");
        clause.metadata = ClauseMetadata {
            contract_type: contract_type.clone(),
            clause_type: if clause.kind == ClauseKind::Definition {
                Some("Definitions".to_string())
            } else {
                metadata::classify_clause_type(heading, &clause.body)
            },
            jurisdiction: jurisdiction.clone(),
            language: Some("English".to_string()),
            source: Some(rel_path.to_string()),
            ..ClauseMetadata::default()
        };
    }

    // Drop noise: fragments too short to be a meaningful knowledge object.
    // A heading-only node is kept — it is the parent of its sub-clauses, and
    // dropping it would orphan the whole subtree.
    clauses.retain(|c| {
        c.body.len() >= 40 || c.kind == ClauseKind::Definition || c.heading.is_some()
    });
    clauses
}
