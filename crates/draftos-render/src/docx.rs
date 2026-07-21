//! LIR → DOCX. Emits WordprocessingML directly into the OOXML zip package.
//! We only need text, headings, bold/italic runs, bulleted lists, and simple
//! tables — a minimal hand-written document.xml is more robust across Word
//! versions than a heavy object model, and keeps the dependency surface small.

use draftos_core::error::{CoreError, Result};
use draftos_core::lir::*;
use std::io::Write;
use zip::write::SimpleFileOptions;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

pub fn render_docx(doc: &LirDocument) -> Result<Vec<u8>> {
    let document_xml = build_document_xml(doc);

    let buf = Vec::new();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(buf));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut write = |name: &str, data: &str| -> Result<()> {
        zip.start_file(name, opts)
            .map_err(|e| CoreError::Index(format!("docx zip: {e}")))?;
        zip.write_all(data.as_bytes())?;
        Ok(())
    };
    write("[Content_Types].xml", CONTENT_TYPES)?;
    write("_rels/.rels", RELS)?;
    write("word/document.xml", &document_xml)?;

    let cursor = zip
        .finish()
        .map_err(|e| CoreError::Index(format!("docx finalize: {e}")))?;
    Ok(cursor.into_inner())
}

fn build_document_xml(doc: &LirDocument) -> String {
    let mut body = String::new();

    body.push_str(&heading_para(&doc.meta.title, "Title"));

    if !doc.parties.is_empty() {
        body.push_str(&para(&[run("Between:", true, false)], None));
        for p in &doc.parties {
            let mut runs = vec![run(&p.name, true, false)];
            let mut tail = format!(" (the \"{}\")", p.role);
            if let Some(reg) = &p.reg_no {
                tail.push_str(&format!(", registration number {reg}"));
            }
            if let Some(t) = &p.entity_type {
                tail.push_str(&format!(", a {t}"));
            }
            runs.push(run(&tail, false, false));
            body.push_str(&para(&runs, None));
        }
    }

    for r in &doc.recitals {
        body.push_str(&blocks_xml(&r.body));
    }

    for c in &doc.clauses {
        body.push_str(&heading_para(
            &format!("{}. {}", c.number, c.heading),
            "Heading1",
        ));
        body.push_str(&blocks_xml(&c.body));
    }

    for s in &doc.schedules {
        body.push_str(&heading_para(&s.title, "Heading1"));
        body.push_str(&blocks_xml(&s.body));
    }

    if !doc.execution.blocks.is_empty() {
        body.push_str(&heading_para("Execution", "Heading1"));
        for b in &doc.execution.blocks {
            body.push_str(&para(&[run(&format!("Signed {}", b.signatory_line), false, false)], None));
            body.push_str(&para(&[run("_____________________________", false, false)], None));
            body.push_str(&para(&[run("Name / Capacity / Date", false, true)], None));
        }
    }

    if let Some(j) = &doc.meta.jurisdiction {
        body.push_str(&para(
            &[run(&format!("Governing law: {j}."), false, true)],
            None,
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    )
}

fn blocks_xml(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Paragraph { runs } => {
                let rs: Vec<String> = runs
                    .iter()
                    .map(|r| run(&r.text, r.style == RunStyle::Bold, r.style == RunStyle::Italic))
                    .collect();
                out.push_str(&para(&rs, None));
            }
            Block::List { items, .. } => {
                for item in items {
                    // Bullet the first paragraph of the item; render the rest plain.
                    let mut first = true;
                    for blk in item {
                        let text = blk.plain_text();
                        let bulleted = if first {
                            format!("•  {text}")
                        } else {
                            text
                        };
                        out.push_str(&para(&[run(&bulleted, false, false)], Some("ListParagraph")));
                        first = false;
                    }
                }
            }
            Block::Table { rows } => {
                // Minimal: render each row as a tab-separated paragraph.
                for row in rows {
                    let cells: Vec<String> =
                        row.iter().map(|cell| cell_text(cell)).collect();
                    out.push_str(&para(&[run(&cells.join("\t"), false, false)], None));
                }
            }
            Block::Variable { label, value, .. } => {
                let text = value.clone().unwrap_or_else(|| format!("[{label}]"));
                out.push_str(&para(&[run(&text, false, false)], None));
            }
        }
    }
    out
}

fn cell_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|b| b.plain_text())
        .collect::<Vec<_>>()
        .join(" ")
}

fn heading_para(text: &str, style: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="{}"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        style,
        esc(text)
    )
}

/// A paragraph from pre-built run XML, with an optional paragraph style.
fn para(runs: &[String], style: Option<&str>) -> String {
    let ppr = match style {
        Some(s) => format!(r#"<w:pPr><w:pStyle w:val="{s}"/></w:pPr>"#),
        None => String::new(),
    };
    format!("<w:p>{ppr}{}</w:p>", runs.concat())
}

/// A single run with optional bold/italic.
fn run(text: &str, bold: bool, italic: bool) -> String {
    let mut rpr = String::new();
    if bold {
        rpr.push_str("<w:b/>");
    }
    if italic {
        rpr.push_str("<w:i/>");
    }
    let rpr = if rpr.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{rpr}</w:rPr>")
    };
    format!(
        r#"<w:r>{rpr}<w:t xml:space="preserve">{}</w:t></w:r>"#,
        esc(text)
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
