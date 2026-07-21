//! Clause segmentation heuristics.

use draftos_core::{ClauseKind, ClauseMetadata, ExtractedClause, ParsedDocument};
use regex::Regex;
use std::sync::OnceLock;

fn numbered_heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // "12. Termination", "12.3 Notice of Breach", "12)" — number + short title.
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
    heading: Option<String>,
    /// Original OOXML of the heading paragraph (DOCX), so the renderer can
    /// reproduce the source heading and the numbering it carries.
    heading_ooxml: Option<String>,
    body: Vec<String>,
    /// Original OOXML for each body paragraph (DOCX sources), parallel to the
    /// text collected in `body`. Empty for non-DOCX.
    body_ooxml: Vec<String>,
}

impl Builder {
    /// Start a clause whose heading is `para` (its text already extracted).
    fn heading(kind: ClauseKind, number: Option<String>, heading: String, para: &draftos_core::Paragraph) -> Self {
        Builder {
            kind,
            number,
            heading: Some(heading),
            heading_ooxml: para.ooxml.clone(),
            body: Vec::new(),
            body_ooxml: Vec::new(),
        }
    }

    /// Start a headingless clause (e.g. the preamble before the first heading).
    fn headless(kind: ClauseKind) -> Self {
        Builder {
            kind,
            number: None,
            heading: None,
            heading_ooxml: None,
            body: Vec::new(),
            body_ooxml: Vec::new(),
        }
    }

    fn push_body(&mut self, para: &draftos_core::Paragraph, text: &str) {
        self.body.push(text.to_string());
        if let Some(xml) = &para.ooxml {
            self.body_ooxml.push(xml.clone());
        }
    }

    fn finish(self) -> Option<ExtractedClause> {
        let body = self.body.join("\n");
        if body.trim().is_empty() && self.heading.is_none() {
            return None;
        }
        Some(ExtractedClause {
            kind: self.kind,
            number: self.number,
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

    for para in &doc.paragraphs {
        let text = para.text.trim();
        if text.is_empty() {
            continue;
        }

        let schedule_start = schedule_re().is_match(text) && text.len() <= 80;
        let heading_info = detect_heading(text, para.heading_level);

        if schedule_start {
            if let Some(b) = current.take() {
                out.extend(b.finish());
            }
            in_schedule = true;
            current = Some(Builder::heading(
                ClauseKind::Schedule,
                None,
                text.to_string(),
                para,
            ));
        } else if let Some((number, heading)) = heading_info {
            if let Some(b) = current.take() {
                out.extend(b.finish());
            }
            let kind = if in_schedule {
                ClauseKind::Schedule
            } else if is_execution_heading(&heading) {
                ClauseKind::Execution
            } else if is_recital_heading(&heading) {
                ClauseKind::Recital
            } else {
                ClauseKind::Clause
            };
            current = Some(Builder::heading(kind, number, heading, para));
        } else {
            match &mut current {
                Some(b) => b.push_body(para, text),
                None => {
                    // Preamble before the first heading (title page, parties).
                    let mut b = Builder::headless(ClauseKind::Recital);
                    b.push_body(para, text);
                    current = Some(b);
                }
            }
        }
    }
    if let Some(b) = current.take() {
        out.extend(b.finish());
    }
    out
}

/// Returns Some((number, heading_text)) when the paragraph reads as a heading.
fn detect_heading(text: &str, declared_level: Option<u8>) -> Option<(Option<String>, String)> {
    if declared_level.is_some() && text.len() <= 120 {
        let (num, rest) = split_leading_number(text);
        return Some((num, rest));
    }
    // ALL-CAPS short line, e.g. "TERMINATION".
    if text.len() <= 60
        && text.chars().any(|c| c.is_alphabetic())
        && text
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_uppercase())
    {
        let (num, rest) = split_leading_number(text);
        return Some((num, rest));
    }
    // Numbered short line: "12. Termination".
    if text.len() <= 90 && !text.ends_with(['.', ';', ':', ',']) {
        if let Some(caps) = numbered_heading_re().captures(text) {
            let title = caps[2].trim().to_string();
            let words = title.split_whitespace().count();
            if words <= 10 {
                return Some((Some(caps[1].to_string()), title));
            }
        }
    }
    None
}

fn split_leading_number(text: &str) -> (Option<String>, String) {
    if let Some(caps) = numbered_heading_re().captures(text) {
        (Some(caps[1].to_string()), caps[2].trim().to_string())
    } else {
        (None, text.to_string())
    }
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
