//! LIR → DOCX. Emits WordprocessingML directly into the OOXML zip package.
//! We only need text, headings, bold/italic runs, bulleted lists, and simple
//! tables — a minimal hand-written document.xml is more robust across Word
//! versions than a heavy object model, and keeps the dependency surface small.
//!
//! Two modes:
//! - **plain** (`render_docx`): references Word's built-in style ids and ships
//!   no styles.xml, so Word applies its defaults. Dependency-free, predictable.
//! - **styled** (`render_docx_with_style`): embeds a [`StyleDonor`]'s style,
//!   theme, font and section parts so the draft inherits a precedent pack's
//!   exact font, spacing and page setup. Headings use direct bold formatting
//!   (not the donor's heading styles) so our assembler-assigned numbering can't
//!   collide with a donor style's own auto-numbering.

use crate::style::StyleDonor;
use draftos_core::error::{CoreError, Result};
use draftos_core::lir::*;
use std::io::Write;
use zip::write::SimpleFileOptions;

const CONTENT_TYPES_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

/// Render with Word's built-in defaults (no styles.xml embedded).
pub fn render_docx(doc: &LirDocument) -> Result<Vec<u8>> {
    render_inner(doc, None)
}

/// Render inheriting a precedent's styling. If `donor` is `None` or carries no
/// usable styles, this behaves exactly like [`render_docx`].
pub fn render_docx_with_style(doc: &LirDocument, donor: Option<&StyleDonor>) -> Result<Vec<u8>> {
    render_inner(doc, donor.filter(|d| d.is_usable()))
}

fn render_inner(doc: &LirDocument, donor: Option<&StyleDonor>) -> Result<Vec<u8>> {
    let styled = donor.is_some();
    let document_xml = build_document_xml(doc, donor);

    let buf = Vec::new();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(buf));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut write = |name: &str, data: &str| -> Result<()> {
        zip.start_file(name, opts)
            .map_err(|e| CoreError::Index(format!("docx zip: {e}")))?;
        zip.write_all(data.as_bytes())?;
        Ok(())
    };

    write("[Content_Types].xml", &content_types(donor))?;
    write("_rels/.rels", RELS)?;
    write("word/document.xml", &document_xml)?;

    if let Some(d) = donor {
        // Only the parts the donor actually had; document.xml.rels wires them in.
        write("word/_rels/document.xml.rels", &document_rels(d))?;
        if let Some(s) = &d.styles_xml {
            write("word/styles.xml", s)?;
        }
        if let Some(t) = &d.theme_xml {
            write("word/theme/theme1.xml", t)?;
        }
        if let Some(f) = &d.font_table_xml {
            write("word/fontTable.xml", f)?;
        }
        if let Some(n) = &d.numbering_xml {
            write("word/numbering.xml", n)?;
        }
        if let Some(st) = &d.settings_xml {
            write("word/settings.xml", st)?;
        }
    }
    let _ = styled; // (kept for clarity; document_xml already reflects it)

    let cursor = zip
        .finish()
        .map_err(|e| CoreError::Index(format!("docx finalize: {e}")))?;
    Ok(cursor.into_inner())
}

fn content_types(donor: Option<&StyleDonor>) -> String {
    let mut s = String::from(CONTENT_TYPES_HEAD);
    if let Some(d) = donor {
        let mut over = |part: &str, ct: &str| {
            s.push_str(&format!(
                r#"<Override PartName="{part}" ContentType="{ct}"/>"#
            ));
        };
        if d.styles_xml.is_some() {
            over("/word/styles.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml");
        }
        if d.theme_xml.is_some() {
            over("/word/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml");
        }
        if d.font_table_xml.is_some() {
            over("/word/fontTable.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml");
        }
        if d.numbering_xml.is_some() {
            over("/word/numbering.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml");
        }
        if d.settings_xml.is_some() {
            over("/word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml");
        }
    }
    s.push_str("</Types>");
    s
}

fn document_rels(donor: &StyleDonor) -> String {
    const BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    let mut id = 0;
    let mut rel = |ty: &str, target: &str| {
        id += 1;
        rels.push_str(&format!(
            r#"<Relationship Id="rId{id}" Type="{BASE}/{ty}" Target="{target}"/>"#
        ));
    };
    if donor.styles_xml.is_some() {
        rel("styles", "styles.xml");
    }
    if donor.theme_xml.is_some() {
        rel("theme", "theme/theme1.xml");
    }
    if donor.font_table_xml.is_some() {
        rel("fontTable", "fontTable.xml");
    }
    if donor.numbering_xml.is_some() {
        rel("numbering", "numbering.xml");
    }
    if donor.settings_xml.is_some() {
        rel("settings", "settings.xml");
    }
    rels.push_str("</Relationships>");
    rels
}

fn build_document_xml(doc: &LirDocument, donor: Option<&StyleDonor>) -> String {
    let styled = donor.is_some();
    let mut body = String::new();

    body.push_str(&heading(&doc.meta.title, HeadingKind::Title, styled));

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
        body.push_str(&blocks_xml(&r.body, styled));
    }

    for c in &doc.clauses {
        body.push_str(&heading(
            &format!("{}. {}", c.number, c.heading),
            HeadingKind::Section,
            styled,
        ));
        body.push_str(&blocks_xml(&c.body, styled));
    }

    for s in &doc.schedules {
        body.push_str(&heading(&s.title, HeadingKind::Section, styled));
        body.push_str(&blocks_xml(&s.body, styled));
    }

    if !doc.execution.blocks.is_empty() {
        body.push_str(&heading("Execution", HeadingKind::Section, styled));
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

    // Page size + margins from the donor, so the page setup matches too.
    if let Some(sect) = donor.and_then(|d| d.sect_pr.as_deref()) {
        body.push_str(sect);
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}</w:body></w:document>"#
    )
}

enum HeadingKind {
    Title,
    Section,
}

/// A heading paragraph. In styled mode we format directly (bold, and a size
/// bump + centring for the title) rather than reference the donor's heading
/// styles — those may carry their own numbering that would collide with the
/// numbers the assembler already assigned. In plain mode we reference Word's
/// built-in `Title`/`Heading1` styles as before.
fn heading(text: &str, kind: HeadingKind, styled: bool) -> String {
    if styled {
        match kind {
            HeadingKind::Title => format!(
                r#"<w:p><w:pPr><w:jc w:val="center"/><w:spacing w:after="240"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                esc(text)
            ),
            HeadingKind::Section => format!(
                r#"<w:p><w:pPr><w:spacing w:before="200" w:after="80"/><w:keepNext/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                esc(text)
            ),
        }
    } else {
        let style = match kind {
            HeadingKind::Title => "Title",
            HeadingKind::Section => "Heading1",
        };
        format!(
            r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            esc(text)
        )
    }
}

fn blocks_xml(blocks: &[Block], styled: bool) -> String {
    // In styled mode, don't stamp a "ListParagraph" pStyle — the donor may not
    // define it, and body paragraphs should inherit its default style.
    let list_style = if styled { None } else { Some("ListParagraph") };
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
                        let bulleted = if first { format!("•  {text}") } else { text };
                        out.push_str(&para(&[run(&bulleted, false, false)], list_style));
                        first = false;
                    }
                }
            }
            Block::Table { rows } => {
                // Minimal: render each row as a tab-separated paragraph.
                for row in rows {
                    let cells: Vec<String> = row.iter().map(|cell| cell_text(cell)).collect();
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
