//! Style donor: lift the visual formatting (fonts, sizes, spacing, page setup)
//! from a real precedent DOCX so an assembled draft renders in the same house
//! style as the pack it was drawn from. We copy the donor's style/theme/font
//! parts verbatim and reuse its body-level section properties (margins, page
//! size); the assembled body inherits the donor's default (`Normal`) paragraph
//! style, so its font and spacing match the source exactly.

use draftos_core::error::{CoreError, Result};
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
        Ok(StyleDonor {
            styles_xml: read_part(&mut archive, "word/styles.xml"),
            theme_xml: read_part(&mut archive, "word/theme/theme1.xml"),
            font_table_xml: read_part(&mut archive, "word/fontTable.xml"),
            numbering_xml: read_part(&mut archive, "word/numbering.xml"),
            settings_xml: read_part(&mut archive, "word/settings.xml"),
            sect_pr,
            document_attrs,
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
}
