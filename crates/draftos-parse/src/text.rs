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

/// Split plain text into paragraphs on blank lines; join hard-wrapped lines
/// within a paragraph. Shared by the TXT and PDF parsers.
pub fn paragraphs_from_plain_text(text: &str) -> Vec<Paragraph> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut current, &mut paragraphs);
        } else {
            current.push(trimmed);
        }
    }
    flush(&mut current, &mut paragraphs);
    paragraphs
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
        });
    }
}
