//! Deterministic validation of an LIR document. Every check is reproducible
//! and reports a precise location. Errors block rendering; warnings do not.
//!
//! This is the safety net that keeps the "LLM never silently produces a bad
//! document" guarantee real: whatever assembled (or, later, adapted) the LIR,
//! it must pass these checks before it can become a DOCX.

use draftos_core::lir::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    /// Human-readable location, e.g. "clause 3 (Termination)".
    pub location: String,
}

#[derive(Debug, Default)]
pub struct ValidationReport {
    pub findings: Vec<Finding>,
}

impl ValidationReport {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.severity == Severity::Error)
    }
    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
    }
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }
    fn error(&mut self, code: &'static str, location: impl Into<String>, message: impl Into<String>) {
        self.findings.push(Finding {
            severity: Severity::Error,
            code,
            location: location.into(),
            message: message.into(),
        });
    }
    fn warn(&mut self, code: &'static str, location: impl Into<String>, message: impl Into<String>) {
        self.findings.push(Finding {
            severity: Severity::Warning,
            code,
            location: location.into(),
            message: message.into(),
        });
    }
}

pub fn validate(doc: &LirDocument) -> ValidationReport {
    let mut r = ValidationReport::default();

    if doc.parties.is_empty() {
        r.error("no-parties", "document", "the document has no parties");
    }
    if doc.clauses.is_empty() {
        r.error("no-clauses", "document", "the document has no clauses");
    }
    if doc.execution.blocks.is_empty() {
        r.error(
            "no-execution",
            "document",
            "the document has no execution/signature block",
        );
    }

    let defined_terms: Vec<&str> = doc.definitions.iter().map(|d| d.term.as_str()).collect();
    let clause_numbers: Vec<&str> = doc.clauses.iter().map(|c| c.number.as_str()).collect();

    // Dense, ordered top-level numbering: 1, 2, 3, …
    for (i, c) in doc.clauses.iter().enumerate() {
        let expected = (i + 1).to_string();
        if c.number != expected {
            r.error(
                "numbering",
                clause_loc(c),
                format!("expected clause number {expected}, found {}", c.number),
            );
        }
    }

    let mut used_terms: Vec<String> = Vec::new();
    for c in &doc.clauses {
        let loc = clause_loc(c);
        let text = clause_text(c);

        if text.trim().is_empty() {
            r.error("empty-clause", &loc, "clause body is empty");
        }

        // Unresolved variable placeholders ({{name}} left after substitution).
        for name in unresolved_vars(&text) {
            r.error(
                "unresolved-variable",
                &loc,
                format!("unresolved variable placeholder: {{{{{name}}}}}"),
            );
        }

        // Cross-references must point at an existing clause number.
        for xref in &c.cross_refs {
            let top = xref.split('.').next().unwrap_or(xref);
            if !clause_numbers.iter().any(|n| *n == top) {
                r.warn(
                    "dangling-cross-reference",
                    &loc,
                    format!(
                        "references clause {xref}, which does not exist under the new numbering"
                    ),
                );
            }
        }

        // Defined-terms-used must resolve to a definition.
        for term in &c.defined_terms_used {
            used_terms.push(term.clone());
            if !defined_terms.iter().any(|t| t == term) {
                r.warn(
                    "undefined-term",
                    &loc,
                    format!("uses \"{term}\", which is not in the Definitions clause"),
                );
            }
        }
    }

    // Definitions that no clause uses (noise worth trimming).
    for d in &doc.definitions {
        if !used_terms.iter().any(|t| t == &d.term) {
            r.warn(
                "unused-definition",
                format!("definition \"{}\"", d.term),
                "is defined but never used",
            );
        }
    }

    r
}

fn clause_loc(c: &LirClause) -> String {
    format!("clause {} ({})", c.number, c.heading)
}

fn clause_text(c: &LirClause) -> String {
    c.body
        .iter()
        .map(|b| b.plain_text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn unresolved_vars(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = text[i + 2..].find("}}") {
                let name = text[i + 2..i + 2 + end]
                    .split('|')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    out.push(name);
                }
                i = i + 2 + end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}
