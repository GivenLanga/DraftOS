//! DOCX parsing: unzip `word/document.xml` and walk the WordprocessingML
//! paragraph/run structure directly. This avoids a heavyweight DOCX object
//! model and survives most producer quirks, since we only need text + styles.

use draftos_core::error::{CoreError, Result};
use draftos_core::{Paragraph, ParsedDocument};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub fn parse(path: &Path, file_name: String) -> Result<ParsedDocument> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Parse {
        file: file_name.clone(),
        message: format!("not a valid docx (zip) file: {e}"),
    })?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| CoreError::Parse {
            file: file_name.clone(),
            message: format!("missing word/document.xml: {e}"),
        })?
        .read_to_string(&mut xml)?;

    let paragraphs = extract_paragraphs(&xml, &file_name)?;
    Ok(ParsedDocument {
        file_name,
        paragraphs,
    })
}

fn extract_paragraphs(xml: &str, file_name: &str) -> Result<Vec<Paragraph>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut heading_level: Option<u8> = None;
    let mut in_paragraph = false;
    // Cursor into `xml`: the raw OOXML of each paragraph is sliced out directly
    // by string scan (robust — `<w:p>` never nests), advanced past every
    // paragraph the walker closes so it stays aligned with the text pass.
    let mut scan_from: usize = 0;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"p" => {
                        in_paragraph = true;
                        current.clear();
                        heading_level = None;
                    }
                    b"pStyle" => {
                        if let Some(level) = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"val")
                            .and_then(|a| {
                                heading_level_from_style(&String::from_utf8_lossy(&a.value))
                            })
                        {
                            heading_level = Some(level);
                        }
                    }
                    b"tab" if in_paragraph => current.push(' '),
                    b"br" | b"cr" if in_paragraph => current.push(' '),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_paragraph {
                    let text = t.unescape().map_err(|e| CoreError::Parse {
                        file: file_name.to_string(),
                        message: format!("bad xml text: {e}"),
                    })?;
                    current.push_str(&text);
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == b"p" {
                    in_paragraph = false;
                    // Consume this paragraph's raw span, keeping the cursor aligned
                    // whether or not the paragraph has text.
                    let raw = next_paragraph_span(xml, &mut scan_from);
                    let text = normalize_ws(&current);
                    if !text.is_empty() {
                        paragraphs.push(Paragraph {
                            text,
                            heading_level,
                            ooxml: raw,
                        });
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(CoreError::Parse {
                    file: file_name.to_string(),
                    message: format!("xml error at {}: {e}", reader.buffer_position()),
                })
            }
            _ => {}
        }
    }
    Ok(paragraphs)
}

/// Slice out the next `<w:p …>…</w:p>` element starting at `*from`, advancing
/// `*from` past it. Matches the paragraph element specifically (not `<w:pPr>`,
/// `<w:pStyle>`, …) by requiring `>` or whitespace after `<w:p`, and relies on
/// the fact that `w:p` never nests. Returns the raw XML, or `None` if not found.
fn next_paragraph_span(xml: &str, from: &mut usize) -> Option<String> {
    let bytes = xml.as_bytes();
    let mut i = *from;
    loop {
        let rel = xml[i..].find("<w:p")?;
        let start = i + rel;
        let after = start + 4; // byte just past "<w:p"
        match bytes.get(after) {
            // A real paragraph element opener.
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') => {
                let erel = xml[start..].find("</w:p>")?;
                let end = start + erel + "</w:p>".len();
                *from = end;
                return Some(xml[start..end].to_string());
            }
            // "<w:pPr", "<w:pStyle", "<w:p/>" (empty) … keep scanning.
            _ => i = after,
        }
    }
}

/// Map DOCX paragraph style ids to heading levels: Heading1/heading 1/Title…
fn heading_level_from_style(style: &str) -> Option<u8> {
    let s = style.to_ascii_lowercase().replace(' ', "");
    if s == "title" {
        return Some(1);
    }
    s.strip_prefix("heading")
        .and_then(|rest| rest.parse::<u8>().ok())
        .filter(|l| (1..=9).contains(l))
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_spans_are_captured_whole_and_skip_ppr() {
        let xml = r#"<w:body><w:p w:rsidR="00A"><w:pPr><w:pStyle w:val="Clause2Sub"/></w:pPr><w:r><w:t>First &amp; only.</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body>"#;
        let mut from = 0;
        let p1 = next_paragraph_span(xml, &mut from).unwrap();
        assert!(p1.starts_with("<w:p ") && p1.ends_with("</w:p>"), "whole element: {p1}");
        assert!(p1.contains(r#"w:val="Clause2Sub""#));
        assert!(p1.contains("First &amp; only."));
        let p2 = next_paragraph_span(xml, &mut from).unwrap();
        assert_eq!(p2, "<w:p><w:r><w:t>Second</w:t></w:r></w:p>");
        assert!(next_paragraph_span(xml, &mut from).is_none());
    }
}
