//! Clause segmentation heuristics.
//!
//! Output is a flat list in document order, where every object carries its
//! `seq` (position) and `depth` (nesting implied by its number). The assembler
//! rebuilds the tree from those two fields, so a precedent's structure —
//! "5", "5.1", "5.1.2" — survives ingestion instead of collapsing into one
//! run-on paragraph.

use draftos_core::{ClauseKind, ClauseMetadata, ExtractedClause, ParsedDocument};
use regex::Regex;
use std::sync::OnceLock;

fn leading_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // "12. ", "12.3 ", "12.3.1) " followed by the rest of the paragraph.
    RE.get_or_init(|| Regex::new(r"^(\d{1,3}(?:\.\d{1,3})*)[.)]?\s+(.+)$").unwrap())
}

fn schedule_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(schedule|annexure|annex|appendix|exhibit)\b\s*([A-Z0-9]{0,4})").unwrap()
    })
}

fn definition_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // “"Effective Date" means …” / “"Confidential Information" shall mean …”
    RE.get_or_init(|| {
        Regex::new(r#"["“”']{1}([A-Z][A-Za-z0-9 /\-]{1,60}?)["“”']{1}\s+(?:shall\s+mean|means|shall\s+have\s+the\s+meaning)"#)
            .unwrap()
    })
}

struct Builder {
    kind: ClauseKind,
    number: Option<String>,
    depth: u8,
    heading: Option<String>,
    heading_ooxml: Option<String>,
    body: Vec<String>,
    body_ooxml: Vec<String>,
}

impl Builder {
    fn new(kind: ClauseKind, number: Option<String>, depth: u8) -> Self {
        Builder {
            kind,
            number,
            depth,
            heading: None,
            heading_ooxml: None,
            body: Vec::new(),
            body_ooxml: Vec::new(),
        }
    }

    fn with_heading(mut self, heading: String, para: &draftos_core::Paragraph) -> Self {
        self.heading = Some(heading);
        self.heading_ooxml = para.ooxml.clone();
        self
    }

    fn push_body(&mut self, para: &draftos_core::Paragraph, text: &str) {
        self.body.push(text.to_string());
        if let Some(xml) = &para.ooxml {
            self.body_ooxml.push(xml.clone());
        }
    }

    fn finish(self, seq: usize) -> Option<ExtractedClause> {
        let body = self.body.join("\n");
        // A heading-only parent (its text lives in its sub-clauses) is still a
        // real node — dropping it would orphan the whole subtree.
        if body.trim().is_empty() && self.heading.is_none() {
            return None;
        }
        Some(ExtractedClause {
            kind: self.kind,
            number: self.number,
            seq,
            depth: self.depth,
            heading: self.heading,
            term: None,
            body,
            ooxml: self.body_ooxml,
            heading_ooxml: self.heading_ooxml,
            metadata: ClauseMetadata::default(),
        })
    }
}

pub fn split_into_clauses(doc: &ParsedDocument) -> Vec<ExtractedClause> {
    let mut out: Vec<ExtractedClause> = Vec::new();
    let mut current: Option<Builder> = None;
    let mut in_schedule = false;
    let mut seen_numbered = false;
    // The kind of the section currently open. Sub-clauses inherit it rather
    // than being re-classified from their own prose — a clause that happens to
    // mention "signed by both parties" is not a signature block.
    let mut section_kind = ClauseKind::Recital;

    let flush = |current: &mut Option<Builder>, out: &mut Vec<ExtractedClause>| {
        if let Some(b) = current.take() {
            let seq = out.len();
            out.extend(b.finish(seq));
        }
    };

    for para in &doc.paragraphs {
        let text = para.text.trim();
        if text.is_empty() {
            continue;
        }

        if schedule_re().is_match(text) && text.len() <= 80 {
            flush(&mut current, &mut out);
            in_schedule = true;
            section_kind = ClauseKind::Schedule;
            current = Some(
                Builder::new(ClauseKind::Schedule, None, 0).with_heading(text.to_string(), para),
            );
            continue;
        }

        // A leading number path starts a new node at the depth it implies.
        if let Some(caps) = leading_number_re().captures(text) {
            let path = caps[1].to_string();
            let rest = caps[2].trim().to_string();
            let depth = path.matches('.').count().min(u8::MAX as usize) as u8;
            flush(&mut current, &mut out);
            seen_numbered = true;
            let heading = is_heading_text(&rest).then(|| rest.clone());
            // Only a heading may re-open a section; a numbered body paragraph
            // stays inside the section it belongs to.
            let kind = match (&heading, depth) {
                (Some(h), _) => {
                    section_kind = node_kind(in_schedule, h, seen_numbered);
                    section_kind
                }
                (None, 0) => {
                    section_kind = node_kind(in_schedule, "", seen_numbered);
                    section_kind
                }
                (None, _) => section_kind,
            };
            let mut b = Builder::new(kind, Some(path), depth);
            match heading {
                Some(h) => b = b.with_heading(h, para),
                // A numbered sub-clause: the text after the number is its body.
                None => b.push_body(para, &rest),
            }
            current = Some(b);
            continue;
        }

        // Unnumbered heading: a declared heading style, or a short ALL-CAPS line.
        if is_unnumbered_heading(text, para.heading_level) {
            flush(&mut current, &mut out);
            section_kind = node_kind(in_schedule, text, seen_numbered);
            current = Some(Builder::new(section_kind, None, 0).with_heading(text.to_string(), para));
            continue;
        }

        match &mut current {
            Some(b) => b.push_body(para, text),
            None => {
                // Preamble before anything else (title page, parties block).
                let mut b = Builder::new(ClauseKind::Recital, None, 0);
                b.push_body(para, text);
                current = Some(b);
            }
        }
    }
    flush(&mut current, &mut out);
    out
}

/// What kind of node a heading starts. Anything before the first numbered
/// clause is front matter (title, parties, recitals) rather than an operative
/// clause — that is what keeps the document title out of the clause list.
fn node_kind(in_schedule: bool, heading: &str, seen_numbered: bool) -> ClauseKind {
    if in_schedule {
        ClauseKind::Schedule
    } else if is_execution_heading(heading) {
        ClauseKind::Execution
    } else if is_recital_heading(heading) || !seen_numbered {
        ClauseKind::Recital
    } else {
        ClauseKind::Clause
    }
}

/// Whether the text after a clause number reads as a heading rather than the
/// clause's own body ("Termination" vs "The Customer shall pay…").
fn is_heading_text(text: &str) -> bool {
    text.len() <= 90
        && !text.ends_with(['.', ';', ':', ','])
        && text.split_whitespace().count() <= 10
}

fn is_unnumbered_heading(text: &str, declared_level: Option<u8>) -> bool {
    if declared_level.is_some() && text.len() <= 120 {
        return true;
    }
    text.len() <= 60
        && text.chars().any(|c| c.is_alphabetic())
        && text
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_uppercase())
}

fn is_execution_heading(heading: &str) -> bool {
    let h = heading.to_ascii_lowercase();
    h.contains("signature") || h.contains("signed") || h.contains("execution")
}

fn is_recital_heading(heading: &str) -> bool {
    let h = heading.to_ascii_lowercase();
    h.contains("recital")
        || h.contains("preamble")
        || h.contains("introduction")
        || h.contains("whereas")
        || h.contains("background")
}

/// Find "Term" means … definitions inside a clause body. Returns
/// (term, defining sentence/paragraph) pairs.
pub fn find_definitions(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for para in body.split('\n') {
        for caps in definition_re().captures_iter(para) {
            let term = caps[1].trim().to_string();
            out.push((term, para.trim().to_string()));
        }
    }
    out
}
