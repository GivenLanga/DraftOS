use draftos_core::error::Result;
use draftos_core::{Paragraph, ParsedDocument};
use std::path::Path;

pub fn parse(path: &Path, file_name: String) -> Result<ParsedDocument> {
    let raw = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&raw);
    Ok(ParsedDocument {
        file_name,
        paragraphs: paragraphs_from_plain_text(&text),
    })
}

/// Split plain text into paragraphs on blank lines, joining hard-wrapped lines
/// within a paragraph. Shared by the TXT and PDF parsers.
///
/// Legal documents frequently put one clause per line with no blank line
/// between them, so joining every consecutive line would fuse a whole contract
/// into a handful of paragraphs and destroy its structure before extraction
/// ever sees it. A line therefore starts a new paragraph when it *looks* like a
/// new block (a clause number, a heading, a schedule marker, a defined term) or
/// when the previous line ended a sentence. Genuine hard wrapping — which
/// breaks mid-sentence — still joins.
pub fn paragraphs_from_plain_text(text: &str) -> Vec<Paragraph> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut current, &mut paragraphs);
            continue;
        }
        let previous_ended = current
            .last()
            .is_some_and(|p| p.ends_with(['.', ';', ':', '!', '?']));
        if previous_ended || starts_new_block(trimmed) {
            flush(&mut current, &mut paragraphs);
        }
        current.push(trimmed);
        // A heading or schedule marker is a paragraph in its own right: the line
        // after it begins the body, and fusing the two hides the heading from
        // clause detection entirely.
        if is_standalone_line(trimmed) {
            flush(&mut current, &mut paragraphs);
        }
    }
    flush(&mut current, &mut paragraphs);
    paragraphs
}

/// Whether a line is a complete block on its own — a heading or a schedule
/// marker — so that the following line starts a new paragraph.
fn is_standalone_line(line: &str) -> bool {
    if line.ends_with([',', ';']) {
        return false;
    }
    let all_caps = line.len() <= 80
        && line.chars().any(|c| c.is_alphabetic())
        && line
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_uppercase());
    let lc = line.to_ascii_lowercase();
    let schedule = line.len() <= 80
        && ["schedule", "annexure", "annex ", "appendix", "exhibit"]
            .iter()
            .any(|k| lc.starts_with(k));
    all_caps || schedule
}

/// Whether a line begins a new structural unit rather than continuing the one
/// before it.
fn starts_new_block(line: &str) -> bool {
    let numbered = || {
        let mut chars = line.chars().peekable();
        if !chars.peek().is_some_and(char::is_ascii_digit) {
            return false;
        }
        let head: String = line
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        // "12 ", "12. ", "12.3 " — a number followed by a separator and text.
        line[head.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == ')')
    };
    let all_caps_heading = || {
        line.len() <= 80
            && line.chars().any(|c| c.is_alphabetic())
            && line
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(|c| c.is_uppercase())
    };
    let schedule = || {
        let lc = line.to_ascii_lowercase();
        ["schedule", "annexure", "annex ", "appendix", "exhibit"]
            .iter()
            .any(|k| lc.starts_with(k))
    };
    // A defined term opening a line: “"Business Day" means …”.
    let defined_term = || line.starts_with(['"', '\u{201c}']);

    numbered() || all_caps_heading() || schedule() || defined_term()
}

fn flush(current: &mut Vec<&str>, out: &mut Vec<Paragraph>) {
    if current.is_empty() {
        return;
    }
    let text = current.join(" ");
    current.clear();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.is_empty() {
        out.push(Paragraph {
            text,
            heading_level: None,
            ooxml: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_clause_per_line_stays_one_paragraph_per_clause() {
        let text = "1. DEFINITIONS\n\
                    1.1 In this Agreement the following terms apply:\n\
                    \"Business Day\" means any day other than a Saturday.\n\
                    2. APPOINTMENT\n\
                    2.1 The Customer appoints the Provider to render the Services.\n";
        let paras = paragraphs_from_plain_text(text);
        let texts: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts.len(), 5, "{texts:?}");
        assert_eq!(texts[0], "1. DEFINITIONS");
        assert_eq!(texts[3], "2. APPOINTMENT");
    }

    #[test]
    fn hard_wrapped_prose_still_joins() {
        // A PDF extraction breaking a sentence across lines mid-clause.
        let text = "2.1 The Customer appoints the Provider to render the\n\
                    Services with effect from the Commencement Date, and the\n\
                    Provider accepts the appointment.\n";
        let paras = paragraphs_from_plain_text(text);
        assert_eq!(paras.len(), 1, "{paras:?}");
        assert!(paras[0].text.contains("Commencement Date, and the Provider"));
    }

    #[test]
    fn a_heading_never_absorbs_the_line_below_it() {
        let text = "SCHEDULE 1\n\
                    The Services and the fees payable are set out in this Schedule 1.\n";
        let paras = paragraphs_from_plain_text(text);
        assert_eq!(paras.len(), 2, "{paras:?}");
        assert_eq!(paras[0].text, "SCHEDULE 1");
    }

    #[test]
    fn blank_lines_still_separate() {
        let paras = paragraphs_from_plain_text("first para\n\nsecond para\n");
        assert_eq!(paras.len(), 2);
    }
}
