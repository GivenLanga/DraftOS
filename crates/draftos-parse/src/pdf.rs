//! PDF parsing via pdf-extract (text layer only). Scanned PDFs with no text
//! layer produce empty/near-empty output; OCR is a feature-gated later phase
//! (draftos-ocr).

use draftos_core::error::{CoreError, Result};
use draftos_core::ParsedDocument;
use std::path::Path;

pub fn parse(path: &Path, file_name: String) -> Result<ParsedDocument> {
    let text = pdf_extract::extract_text(path).map_err(|e| CoreError::Parse {
        file: file_name.clone(),
        message: format!("pdf extraction failed: {e}"),
    })?;
    Ok(ParsedDocument {
        file_name,
        paragraphs: crate::text::paragraphs_from_plain_text(&text),
    })
}
