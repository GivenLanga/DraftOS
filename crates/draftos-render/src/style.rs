//! Style donor: lift the visual formatting (fonts, sizes, spacing, page setup)
//! from a real precedent DOCX so an assembled draft renders in the same house
//! style as the pack it was drawn from. We copy the donor's style/theme/font
//! parts verbatim and reuse its body-level section properties (margins, page
//! size); the assembled body inherits the donor's default (`Normal`) paragraph
//! style, so its font and spacing match the source exactly.

use draftos_core::error::{CoreError, Result};
use std::collections::HashSet;
use std::io::{Read, Seek};
use std::path::Path;

/// The reusable formatting parts extracted from a donor DOCX. Any part may be
/// absent; `styles_xml` present is what makes a donor useful.
#[derive(Debug, Clone, Default)]
pub struct StyleDonor {
    pub styles_xml: Option<String>,
    pub theme_xml: Option<String>,
    pub font_table_xml: Option<String>,
    pub numbering_xml: Option<String>,
    pub settings_xml: Option<String>,
    /// The body-level `<w:sectPr>` (page size + margins), with header/footer
    /// references stripped since we don't copy those parts.
    pub sect_pr: Option<String>,
    /// The attributes of the donor's `<w:document …>` root (namespace
    /// declarations). We reuse these so the original clause paragraphs we lift
    /// — which use prefixes like `w14:`/`mc:` — remain namespace-valid.
    pub document_attrs: Option<String>,
    /// The `w:numId`s defined in the donor's numbering.xml. A lifted paragraph
    /// keeps its `<w:numPr>` only when its numId is one of these — otherwise the
    /// reference would dangle and the numbering wouldn't resolve.
    pub num_ids: HashSet<String>,
    /// The donor's dominant top-level (ilvl 0) numId — the multilevel list its
    /// clauses hang off. Synthetic headings (e.g. Definitions) join this list so
    /// the whole document numbers coherently in the donor's own scheme.
    pub main_num_id: Option<String>,
}

fn read_part<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut s = String::new();
    archive.by_name(name).ok()?.read_to_string(&mut s).ok()?;
    Some(s)
}

impl StyleDonor {
    /// Extract the style parts from the bytes of a `.docx` file.
    pub fn from_docx_bytes(bytes: &[u8]) -> Result<StyleDonor> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| CoreError::Index(format!("open donor docx: {e}")))?;
        let document_xml = read_part(&mut archive, "word/document.xml");
        let sect_pr = document_xml.as_deref().and_then(extract_sect_pr);
        let document_attrs = document_xml.as_deref().and_then(extract_document_attrs);
        let numbering_xml = read_part(&mut archive, "word/numbering.xml");
        let num_ids = numbering_xml.as_deref().map(defined_num_ids).unwrap_or_default();
        let main_num_id = document_xml
            .as_deref()
            .and_then(|d| dominant_top_num_id(d, &num_ids));
        Ok(StyleDonor {
            styles_xml: read_part(&mut archive, "word/styles.xml"),
            theme_xml: read_part(&mut archive, "word/theme/theme1.xml"),
            font_table_xml: read_part(&mut archive, "word/fontTable.xml"),
            numbering_xml,
            settings_xml: read_part(&mut archive, "word/settings.xml"),
            sect_pr,
            document_attrs,
            num_ids,
            main_num_id,
        })
    }

    /// Read a `.docx` from disk and extract its style parts.
    pub fn from_path(path: &Path) -> Result<StyleDonor> {
        Self::from_docx_bytes(&std::fs::read(path)?)
    }

    /// A donor is only worth applying if it carries style definitions.
    pub fn is_usable(&self) -> bool {
        self.styles_xml.is_some()
    }
}

/// The set of `w:numId`s declared by `<w:num w:numId="…">` in numbering.xml.
/// Note: here the id is the `w:numId` attribute's own value (`w:numId="51"`),
/// unlike a paragraph's `<w:numId w:val="51"/>` reference.
fn defined_num_ids(numbering_xml: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut rest = numbering_xml;
    while let Some(i) = rest.find("<w:num ") {
        rest = &rest[i + 6..];
        if let Some(id) = attr_direct(rest, "w:numId") {
            ids.insert(id);
        }
    }
    ids
}

/// Value of a direct attribute `name="value"` at/after the start of `xml`.
fn attr_direct(xml: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = xml.find(&key)? + key.len();
    let end = xml[i..].find('"')? + i;
    Some(xml[i..end].to_string())
}

/// The numId used by the most ilvl-0 paragraphs in the document body — the
/// backbone list the clauses hang off. Restricted to defined numIds.
fn dominant_top_num_id(document_xml: &str, defined: &HashSet<String>) -> Option<String> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in split_paragraphs(document_xml) {
        // Only count a paragraph as top-level when its numPr has ilvl 0 (or no
        // explicit ilvl, which defaults to 0).
        if let Some(np) = between(p, "<w:numPr", "</w:numPr>") {
            let ilvl = attr_val(np, "w:ilvl").unwrap_or_else(|| "0".to_string());
            if ilvl == "0" {
                if let Some(id) = attr_val(np, "w:numId") {
                    if defined.contains(&id) {
                        *counts.entry(id).or_default() += 1;
                    }
                }
            }
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(id, _)| id)
}

/// Value of `w:val` for the first occurrence of `attr_name` after position 0.
fn attr_val(xml: &str, attr_name: &str) -> Option<String> {
    let i = xml.find(attr_name)?;
    let rest = &xml[i + attr_name.len()..];
    let v = rest.find("w:val=\"")? + "w:val=\"".len();
    let end = rest[v..].find('"')? + v;
    Some(rest[v..end].to_string())
}

/// The substring between `open` (up to its `>`) and `close`, if present.
fn between<'a>(xml: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let s = xml.find(open)?;
    let e = xml[s..].find(close)? + s;
    Some(&xml[s..e])
}

/// Split a document.xml body into its `<w:p …>…</w:p>` elements (w:p never nests).
fn split_paragraphs(xml: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    while let Some(rel) = xml[i..].find("<w:p") {
        let start = i + rel;
        match bytes.get(start + 4) {
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') => {
                if let Some(erel) = xml[start..].find("</w:p>") {
                    let end = start + erel + "</w:p>".len();
                    out.push(&xml[start..end]);
                    i = end;
                } else {
                    break;
                }
            }
            _ => i = start + 4,
        }
    }
    out
}

/// The attribute text of the `<w:document …>` root element (all the `xmlns:*`
/// declarations), so lifted paragraphs keep valid namespaces.
fn extract_document_attrs(document_xml: &str) -> Option<String> {
    let start = document_xml.find("<w:document")? + "<w:document".len();
    let end = document_xml[start..].find('>')? + start;
    let attrs = document_xml[start..end].trim().trim_end_matches('/').trim();
    (!attrs.is_empty()).then(|| attrs.to_string())
}

/// Pull the last `<w:sectPr>…</w:sectPr>` (the body-level one) out of a
/// document.xml, dropping header/footer references we can't satisfy.
fn extract_sect_pr(document_xml: &str) -> Option<String> {
    let start = document_xml.rfind("<w:sectPr")?;
    const END: &str = "</w:sectPr>";
    let end = document_xml[start..].find(END)? + start + END.len();
    Some(strip_refs(&document_xml[start..end]))
}

/// Remove `<w:headerReference…/>` and `<w:footerReference…/>` — they point at
/// header/footer parts (via r:id) that we don't copy, which would dangle.
fn strip_refs(sect_pr: &str) -> String {
    let mut s = sect_pr.to_string();
    for tag in ["<w:headerReference", "<w:footerReference"] {
        while let Some(start) = s.find(tag) {
            match s[start..].find("/>") {
                Some(close) => s.replace_range(start..start + close + 2, ""),
                None => break,
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_body_sectpr_and_strips_headers() {
        let doc = r#"<w:body><w:p/><w:sectPr><w:headerReference w:type="default" r:id="rId7"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440"/></w:sectPr></w:body>"#;
        let sect = extract_sect_pr(doc).unwrap();
        assert!(sect.contains("<w:pgSz"));
        assert!(sect.contains("<w:pgMar"));
        assert!(!sect.contains("headerReference"), "header ref must be stripped");
    }

    #[test]
    fn no_sectpr_returns_none() {
        assert!(extract_sect_pr("<w:body><w:p/></w:body>").is_none());
    }

    #[test]
    fn detects_defined_and_dominant_num_ids() {
        let numbering = r#"<w:numbering><w:num w:numId="51"><w:abstractNumId w:val="9"/></w:num><w:num w:numId="7"><w:abstractNumId w:val="3"/></w:num></w:numbering>"#;
        let ids = defined_num_ids(numbering);
        assert!(ids.contains("51") && ids.contains("7"));
        // Three ilvl-0 paras on numId 51, one on numId 7 → 51 dominates.
        let doc = r#"<w:body>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="51"/></w:numPr></w:pPr></w:p>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="51"/></w:numPr></w:pPr></w:p>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="51"/></w:numPr></w:pPr></w:p>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="51"/></w:numPr></w:pPr></w:p>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr></w:p>
        </w:body>"#;
        assert_eq!(dominant_top_num_id(doc, &ids), Some("51".to_string()));
    }
}
