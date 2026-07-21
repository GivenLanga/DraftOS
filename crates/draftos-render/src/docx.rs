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
        clause_xml(&mut body, c, styled, donor);
    }

    for s in &doc.schedules {
        match (styled, donor) {
            (true, Some(d)) if s.heading_ooxml.is_some() => {
                body.push_str(&prepare_source_paragraph(s.heading_ooxml.as_deref().unwrap(), d));
            }
            _ => body.push_str(&heading(&s.title, HeadingKind::Section, styled)),
        }
        if let (true, Some(d), false) = (styled, donor, s.source_ooxml.is_empty()) {
            for p in &s.source_ooxml {
                body.push_str(&prepare_source_paragraph(p, d));
            }
        } else {
            body.push_str(&blocks_xml(&s.body, styled));
        }
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

    // Reuse the donor's root namespace declarations when styling, so lifted
    // paragraphs (w14:/mc:/…) stay valid; otherwise a minimal w+r set.
    const DEFAULT_ATTRS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let attrs = donor
        .and_then(|d| d.document_attrs.as_deref())
        .unwrap_or(DEFAULT_ATTRS);

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {attrs}><w:body>{body}</w:body></w:document>"#
    )
}

/// One clause and, recursively, its sub-clauses. A sub-clause is emitted in its
/// own right — with the precedent's own paragraph formatting where we have it —
/// rather than being folded into the parent's body.
fn clause_xml(body: &mut String, c: &LirClause, styled: bool, donor: Option<&StyleDonor>) {
    // Heading: reproduce the precedent's own heading paragraph (with its
    // numbering) when we have it; otherwise synthesise one — joined into the
    // donor's list so it numbers in the pack's own scheme, not a hardcode.
    // Sub-clauses commonly have no heading at all; they get none here either.
    match (styled, donor) {
        (true, Some(d)) if c.heading_ooxml.is_some() => {
            body.push_str(&prepare_source_paragraph(c.heading_ooxml.as_deref().unwrap(), d));
        }
        (true, Some(d)) if !c.heading.trim().is_empty() => {
            body.push_str(&synth_heading(&c.heading, d.main_num_id.as_deref()));
        }
        (true, Some(_)) => {}
        _ => {
            if c.heading.trim().is_empty() {
                body.push_str(&heading(&format!("{}.", c.number), HeadingKind::Section, styled));
            } else {
                body.push_str(&heading(
                    &format!("{}. {}", c.number, c.heading),
                    HeadingKind::Section,
                    styled,
                ));
            }
        }
    }
    // Body: the precedent's own paragraphs (house style + numbering) when we
    // have them and a donor is active; otherwise synthesise from plain text.
    if let (true, Some(d), false) = (styled, donor, c.source_ooxml.is_empty()) {
        for p in &c.source_ooxml {
            body.push_str(&prepare_source_paragraph(p, d));
        }
    } else {
        body.push_str(&blocks_xml(&c.body, styled));
    }
    for child in &c.children {
        clause_xml(body, child, styled, donor);
    }
}

/// Prepare a lifted source paragraph for a new document. Keeps everything that
/// drives the *look and numbering* — paragraph style, direct run fonts, indent,
/// spacing, justification, and the `<w:numPr>` list membership — so the pack's
/// own numbering scheme (decimal, `(a)`, roman…) renders exactly. It only
/// removes what would break or dangle in the new package:
/// - a `<w:numPr>` whose `w:numId` isn't defined in the donor's numbering.xml
///   (its list definition is absent, so it can't resolve);
/// - `<w:hyperlink>` wrappers (kept: their text; dropped: the `r:id` link);
/// - embedded drawings/objects (`<w:drawing>/<w:pict>/<w:object>`) — media we
///   don't carry;
/// - complex/simple fields (`REF` cross-references etc.) flattened to their last
///   cached text, so they don't render as "Error: Reference source not found".
fn prepare_source_paragraph(p: &str, donor: &StyleDonor) -> String {
    let mut out = p.to_string();
    // Drop a numPr only when its list definition isn't present.
    if let Some(numid) = num_id_of(&out) {
        if !donor.num_ids.contains(&numid) {
            out = strip_element(&out, "<w:numPr", "</w:numPr>");
        }
    }
    out = strip_element(&out, "<w:drawing", "</w:drawing>");
    out = strip_element(&out, "<w:pict", "</w:pict>");
    out = strip_element(&out, "<w:object", "</w:object>");
    out = flatten_fields(&out);
    unwrap_element(&out, "w:hyperlink")
}

/// The `w:numId` referenced by a paragraph's `<w:numPr>`, if any.
fn num_id_of(p: &str) -> Option<String> {
    let np = between(p, "<w:numPr", "</w:numPr>")?;
    let i = np.find("<w:numId")?;
    let v = np[i..].find("w:val=\"")? + i + "w:val=\"".len();
    let end = np[v..].find('"')? + v;
    Some(np[v..end].to_string())
}

fn between<'a>(xml: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let s = xml.find(open)?;
    let e = xml[s..].find(close)? + s;
    Some(&xml[s..e])
}

/// Flatten Word field codes to their cached result text: unwrap `<w:fldSimple>`
/// (keep inner runs) and drop the `<w:r>` runs that carry `<w:fldChar>` or
/// `<w:instrText>` (the field machinery), leaving the last-rendered text. This
/// stops re-computable references (REF/PAGEREF…) showing an error when the
/// bookmark they point at isn't in the assembled document.
fn flatten_fields(p: &str) -> String {
    let mut out = unwrap_element(p, "w:fldSimple");
    out = drop_runs_containing(&out, "<w:fldChar");
    drop_runs_containing(&out, "<w:instrText")
}

/// Remove whole `<w:r>…</w:r>` runs that contain `needle`.
fn drop_runs_containing(s: &str, needle: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find("<w:r>").or_else(|| rest.find("<w:r ")) {
        let (before, from_open) = rest.split_at(open);
        out.push_str(before);
        match from_open.find("</w:r>") {
            Some(close_rel) => {
                let end = close_rel + "</w:r>".len();
                let run = &from_open[..end];
                if !run.contains(needle) {
                    out.push_str(run);
                }
                rest = &from_open[end..];
            }
            None => {
                out.push_str(from_open);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Synthesise a clause heading paragraph. When the donor exposes a backbone
/// list (`main_num_id`), the heading joins it at level 0 so it auto-numbers in
/// the pack's own scheme; otherwise it falls back to bold text (no number).
fn synth_heading(text: &str, main_num_id: Option<&str>) -> String {
    match main_num_id {
        Some(id) => format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="{id}"/></w:numPr><w:spacing w:before="200" w:after="80"/><w:keepNext/><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            esc(text)
        ),
        None => heading(text, HeadingKind::Section, true),
    }
}

/// Remove an element's open and close tags but keep its inner content
/// (`<tag …>inner</tag>` → `inner`). Used to dissolve hyperlinks.
fn unwrap_element(s: &str, tag: &str) -> String {
    let close = format!("</{tag}>");
    let mut out = s.replace(&close, "");
    let open = format!("<{tag}");
    loop {
        let Some(start) = out.find(&open) else { break };
        match out[start..].find('>') {
            Some(rel) => out.replace_range(start..start + rel + 1, ""),
            None => break,
        }
    }
    out
}

/// Remove every `<tag …>…</close>` (or self-closing `<tag …/>`) region.
fn strip_element(s: &str, open: &str, close: &str) -> String {
    let mut out = s.to_string();
    loop {
        let Some(start) = out.find(open) else { break };
        if let Some(rel) = out[start..].find(close) {
            let end = start + rel + close.len();
            out.replace_range(start..end, "");
        } else if let Some(rel) = out[start..].find("/>") {
            let end = start + rel + 2;
            out.replace_range(start..end, "");
        } else {
            break;
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn donor_with(num_ids: &[&str]) -> StyleDonor {
        StyleDonor {
            num_ids: num_ids.iter().map(|s| s.to_string()).collect::<HashSet<_>>(),
            ..StyleDonor::default()
        }
    }

    #[test]
    fn keeps_numbering_when_list_is_defined_and_dissolves_hyperlinks() {
        let p = r#"<w:p><w:pPr><w:pStyle w:val="Clause2Sub"/><w:numPr><w:ilvl w:val="1"/><w:numId w:val="51"/></w:numPr></w:pPr><w:hyperlink w:history="1" r:id="Rabc"><w:r><w:t>see clause 8</w:t></w:r></w:hyperlink></w:p>"#;
        let out = prepare_source_paragraph(p, &donor_with(&["51"]));
        assert!(out.contains("<w:numPr"), "numbering kept when 51 is defined: {out}");
        assert!(out.contains(r#"w:val="Clause2Sub""#), "style kept");
        assert!(!out.contains("<w:hyperlink") && !out.contains("r:id"), "hyperlink dissolved: {out}");
        assert!(out.contains("see clause 8"), "link text kept");
    }

    #[test]
    fn strips_numbering_when_list_is_not_defined() {
        let p = r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="99"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#;
        let out = prepare_source_paragraph(p, &donor_with(&["51"]));
        assert!(!out.contains("<w:numPr"), "undefined numId 99 stripped: {out}");
    }

    #[test]
    fn flattens_ref_field_to_cached_text() {
        // Complex field: begin / instrText / separate / cached "2" / end.
        let p = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> REF _Ref1 \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>2</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
        let out = prepare_source_paragraph(p, &donor_with(&[]));
        assert!(!out.contains("fldChar") && !out.contains("instrText"), "field machinery gone: {out}");
        assert!(out.contains("<w:t>2</w:t>"), "cached result kept: {out}");
    }

    #[test]
    fn synth_heading_joins_donor_list() {
        let h = synth_heading("Definitions", Some("51"));
        assert!(h.contains(r#"<w:numId w:val="51"/>"#) && h.contains("Definitions"));
        // No backbone list → a bold heading with no number, and no numPr.
        let fallback = synth_heading("X", None);
        assert!(fallback.contains("<w:b/>") && fallback.contains("X"));
        assert!(!fallback.contains("<w:numPr"));
    }
}
