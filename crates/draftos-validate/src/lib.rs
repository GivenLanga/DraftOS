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
    let all_clauses = doc.walk_clauses();
    // Every number in the document, at every level — a reference to "3.1" is
    // valid if clause 3.1 exists, not merely if clause 3 does.
    let clause_numbers: Vec<&str> = all_clauses.iter().map(|c| c.number.as_str()).collect();

    check_numbering(&mut r, &doc.clauses, "");

    let mut used_terms: Vec<String> = Vec::new();
    for c in &all_clauses {
        let loc = clause_loc(c);
        let text = clause_text(c);

        // A parent clause whose text lives entirely in its sub-clauses is
        // structurally fine; a leaf with no text is not.
        if text.trim().is_empty() && c.children.is_empty() {
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

        // Cross-references must point at an existing clause number. Assembly
        // rewrites these to follow the new numbering, so anything still dangling
        // is a real defect in the document and blocks rendering.
        for xref in &c.cross_refs {
            if !clause_numbers.iter().any(|n| *n == xref.as_str()) {
                r.error(
                    "dangling-cross-reference",
                    &loc,
                    format!(
                        "references clause {xref}, which does not exist in this document; the \
                         reference could not be remapped from the precedent's numbering"
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

    // A company named in the text that is not a party to this agreement is
    // almost always a precedent's parties surviving a copy-paste. Deterministic,
    // and it catches the single most damaging drafting error there is.
    let party_names: Vec<String> = doc.parties.iter().map(|p| p.name.to_lowercase()).collect();
    let mut flagged: Vec<String> = Vec::new();
    for (loc, text) in document_texts(doc) {
        for name in entity_names(&text) {
            let lc = name.to_lowercase();
            if party_names.iter().any(|p| p.contains(&lc) || lc.contains(p)) {
                continue;
            }
            if flagged.contains(&lc) {
                continue;
            }
            flagged.push(lc);
            r.warn(
                "foreign-party-name",
                &loc,
                format!(
                    "names \"{name}\", which is not a party to this agreement — check it is not \
                     left over from the precedent"
                ),
            );
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

/// Numbering must be dense and hierarchical: 1, 1.1, 1.2, 2, 2.1 … Each level
/// restarts at 1 and every child's number extends its parent's.
fn check_numbering(r: &mut ValidationReport, clauses: &[LirClause], prefix: &str) {
    for (i, c) in clauses.iter().enumerate() {
        let expected = if prefix.is_empty() {
            (i + 1).to_string()
        } else {
            format!("{prefix}.{}", i + 1)
        };
        if c.number != expected {
            r.error(
                "numbering",
                clause_loc(c),
                format!("expected clause number {expected}, found {}", c.number),
            );
        }
        check_numbering(r, &c.children, &expected);
    }
}

fn clause_loc(c: &LirClause) -> String {
    if c.heading.trim().is_empty() {
        format!("clause {}", c.number)
    } else {
        format!("clause {} ({})", c.number, c.heading)
    }
}

fn clause_text(c: &LirClause) -> String {
    c.body
        .iter()
        .map(|b| b.plain_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every piece of prose in the document, with a location label.
fn document_texts(doc: &LirDocument) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = doc
        .walk_clauses()
        .iter()
        .map(|c| (clause_loc(c), clause_text(c)))
        .collect();
    for (i, rec) in doc.recitals.iter().enumerate() {
        let text = rec
            .body
            .iter()
            .map(|b| b.plain_text())
            .collect::<Vec<_>>()
            .join("\n");
        out.push((format!("recital {}", i + 1), text));
    }
    for s in &doc.schedules {
        let text = s
            .body
            .iter()
            .map(|b| b.plain_text())
            .collect::<Vec<_>>()
            .join("\n");
        out.push((format!("schedule \"{}\"", s.title), text));
    }
    out
}

/// Company-like names in a run of text: a capitalised phrase ending in a legal
/// entity suffix ("Alpha Holdings (Pty) Ltd", "Beta Inc").
fn entity_names(text: &str) -> Vec<String> {
    const SUFFIXES: &[&str] = &[
        "(pty) ltd", "(pty) limited", "ltd", "limited", "inc", "incorporated",
        "llc", "llp", "plc", "cc", "n.v.", "b.v.", "gmbh", "sa", "spa",
    ];
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();

    for (i, w) in words.iter().enumerate() {
        let cleaned = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != ')');
        let lc = cleaned.to_lowercase();
        let lc = lc.trim_end_matches([',', '.', ';', ':']).to_string();
        // Match the longest suffix first: "(Pty) Ltd" before bare "Ltd".
        let matched = if i > 0 && words[i - 1].to_lowercase().starts_with("(pty)") {
            Some(2)
        } else if SUFFIXES.contains(&lc.as_str()) {
            Some(1)
        } else {
            None
        };
        let Some(suffix_len) = matched else { continue };

        // Walk back over the capitalised words that form the name.
        let start = i + 1 - suffix_len;
        let mut first = start;
        while first > 0 {
            let prev = words[first - 1].trim_matches(|c: char| !c.is_alphanumeric());
            let starts_upper = prev.chars().next().is_some_and(|c| c.is_uppercase());
            if !starts_upper || prev.is_empty() {
                break;
            }
            first -= 1;
        }
        if first == i + 1 - suffix_len {
            continue; // a bare "Ltd" with no name in front of it
        }
        let name = words[first..=i]
            .join(" ")
            .trim_matches(|c: char| c == ',' || c == '.' || c == ';' || c == ':')
            .to_string();
        if name.split_whitespace().count() >= 2 && !out.contains(&name) {
            out.push(name);
        }
    }
    out
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
