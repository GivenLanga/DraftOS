//! Prompt compiler: the only producer of [`CompletionRequest`]s for drafting
//! tasks (CLAUDE.md §9). Each task = template + retrieved context + guardrails
//! + output contract. Drafting tasks must return LIR fragments (JSON blocks),
//! never free prose destined for a document; chat/explain tasks return text.

use draftos_core::{Block, ClauseHit, Party};
use draftos_models::{ChatMessage, CompletionRequest};

/// Shared guardrails for every drafting-adjacent task.
const GUARDRAILS: &str = "\
You are the language layer of DraftOS, a legal drafting system. Rules you must never break:
1. You do not draft new legal content. You only adapt, rewrite, explain or summarize \
the precedent text you are given.
2. Never invent obligations, amounts, dates or party names that are not in the input.
3. Preserve defined terms exactly as capitalised (e.g. \"Confidential Information\").
4. Never add clause numbers or cross-references — numbering is assigned by the system.
5. Jurisdiction-specific statements must come from the provided precedent, not memory.";

/// The JSON contract for tasks whose output is spliced into a document.
const BLOCK_SCHEMA: &str = r#"Output a single JSON array of block objects, nothing else. Block shapes:
  {"type":"paragraph","runs":[{"text":"...","style":"normal"|"bold"|"italic"}]}
  {"type":"list","ordered":true|false,"items":[[<block>,...],...]}
  {"type":"variable","name":"snake_case_name","label":"Human label","value":null}
Use a "variable" block for any value the user must still supply. Do not wrap the array in an object or markdown fences."#;

/// Compile a clause-adaptation task. The model rewrites one clause body under
/// `instructions` (party names, tone, plain language …) and returns LIR blocks.
pub fn adapt_clause(
    heading: &str,
    body_text: &str,
    instructions: &str,
    parties: &[Party],
    defined_terms: &[String],
) -> CompletionRequest {
    let mut context = String::new();
    if !parties.is_empty() {
        context.push_str("Parties to this agreement:\n");
        for p in parties {
            context.push_str(&format!("- {} = \"{}\"\n", p.role, p.name));
        }
    }
    if !defined_terms.is_empty() {
        context.push_str(&format!(
            "Defined terms in force (preserve exactly): {}\n",
            defined_terms.join(", ")
        ));
    }
    let user = format!(
        "Adapt the clause below.\n\nInstructions: {instructions}\n\n{context}\nClause heading: {heading}\nClause body:\n{body_text}\n\n{BLOCK_SCHEMA}"
    );
    CompletionRequest {
        messages: vec![ChatMessage::system(GUARDRAILS), ChatMessage::user(user)],
        max_tokens: 4096,
        json: true,
    }
}

/// Compile an assistant question scoped to retrieved knowledge. Free-text
/// output is allowed here (chat feature, never document content).
pub fn ask(question: &str, context: &[ClauseHit]) -> CompletionRequest {
    let mut ctx = String::new();
    for (i, hit) in context.iter().enumerate() {
        let title = hit
            .term
            .clone()
            .or_else(|| hit.heading.clone())
            .unwrap_or_else(|| "(untitled)".to_string());
        ctx.push_str(&format!(
            "[{n}] {title} — source: {src} / {file}\n{body}\n\n",
            n = i + 1,
            src = hit.source_name,
            file = hit.file,
            body = hit.body.trim(),
        ));
    }
    let system = format!(
        "{GUARDRAILS}\n\nAnswer the user's question using ONLY the numbered extracts below. \
Cite extracts as [n]. If the extracts do not answer the question, say so — do not answer \
from general knowledge.\n\nExtracts:\n{ctx}"
    );
    CompletionRequest {
        messages: vec![ChatMessage::system(system), ChatMessage::user(question)],
        max_tokens: 2048,
        json: false,
    }
}

/// Compile an explain-this-clause task (plain-language explanation, free text).
pub fn explain(heading: &str, body_text: &str) -> CompletionRequest {
    let user = format!(
        "Explain the following contract clause in plain language: what it does, who it \
protects, and what a signer should watch out for. Do not give legal advice.\n\n\
Heading: {heading}\nBody:\n{body_text}"
    );
    CompletionRequest {
        messages: vec![ChatMessage::system(GUARDRAILS), ChatMessage::user(user)],
        max_tokens: 1024,
        json: false,
    }
}

/// Parse and validate a model's LIR-fragment output into blocks. Rejects
/// anything that does not deserialize against the Block schema — free prose
/// never reaches a document (CLAUDE.md §2.5, §8).
pub fn parse_blocks(raw: &str) -> Result<Vec<Block>, String> {
    let cleaned = strip_fences(raw);
    // Accept a bare array or {"body": [...]} (some models wrap despite asking).
    let blocks: Vec<Block> = match serde_json::from_str::<Vec<Block>>(cleaned) {
        Ok(b) => b,
        Err(first_err) => {
            #[derive(serde::Deserialize)]
            struct Wrapped {
                body: Vec<Block>,
            }
            serde_json::from_str::<Wrapped>(cleaned)
                .map(|w| w.body)
                .map_err(|_| format!("model output is not valid LIR blocks: {first_err}"))?
        }
    };
    if blocks.is_empty() {
        return Err("model returned no blocks".to_string());
    }
    for b in &blocks {
        validate_block(b)?;
    }
    Ok(blocks)
}

fn validate_block(b: &Block) -> Result<(), String> {
    match b {
        Block::Paragraph { runs } => {
            if runs.iter().all(|r| r.text.trim().is_empty()) {
                return Err("paragraph block with no text".to_string());
            }
        }
        Block::Variable { name, .. } => {
            if name.trim().is_empty() {
                return Err("variable block without a name".to_string());
            }
        }
        Block::List { items, .. } => {
            for item in items {
                for inner in item {
                    validate_block(inner)?;
                }
            }
        }
        Block::Table { rows } => {
            for row in rows {
                for cell in row {
                    for inner in cell {
                        validate_block(inner)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Strip a ```json … ``` fence if the model added one anyway.
fn strip_fences(raw: &str) -> &str {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use draftos_core::ClauseMetadata;

    #[test]
    fn adapt_carries_guardrails_and_schema() {
        let parties = vec![Party {
            id: "seller".into(),
            name: "Acme (Pty) Ltd".into(),
            role: "Seller".into(),
            entity_type: None,
            reg_no: None,
            address: None,
        }];
        let req = adapt_clause("Termination", "The Seller may terminate…", "Use party names", &parties, &["Effective Date".into()]);
        assert!(req.json);
        assert!(req.messages[0].content.contains("never break"));
        let user = &req.messages[1].content;
        assert!(user.contains("Acme (Pty) Ltd"));
        assert!(user.contains("Effective Date"));
        assert!(user.contains("\"type\":\"paragraph\""));
    }

    #[test]
    fn ask_numbers_context_and_forbids_outside_knowledge() {
        let hit = ClauseHit {
            clause_id: "c1".into(),
            source_name: "banking".into(),
            file: "loan.docx".into(),
            kind: draftos_core::ClauseKind::Clause,
            number: Some("12".into()),
            heading: Some("Termination".into()),
            term: None,
            body: "Either party may terminate on 30 days notice.".into(),
            ooxml: Vec::new(),
            heading_ooxml: None,
            metadata: ClauseMetadata::default(),
            score: 1.0,
        };
        let req = ask("How can I terminate?", &[hit]);
        assert!(!req.json);
        let system = &req.messages[0].content;
        assert!(system.contains("[1] Termination — source: banking / loan.docx"));
        assert!(system.contains("do not answer"));
    }

    #[test]
    fn parse_accepts_bare_array_and_fences() {
        let raw = "```json\n[{\"type\":\"paragraph\",\"runs\":[{\"text\":\"Hello\",\"style\":\"normal\"}]}]\n```";
        let blocks = parse_blocks(raw).unwrap();
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn parse_accepts_wrapped_body() {
        let raw = r#"{"body":[{"type":"variable","name":"notice_period","label":"Notice period","value":null}]}"#;
        let blocks = parse_blocks(raw).unwrap();
        assert!(matches!(blocks[0], Block::Variable { .. }));
    }

    #[test]
    fn parse_rejects_prose_and_empty() {
        assert!(parse_blocks("Sure! Here is the adapted clause…").is_err());
        assert!(parse_blocks("[]").is_err());
        assert!(parse_blocks(r#"[{"type":"variable","name":"","label":"x","value":null}]"#).is_err());
    }
}
