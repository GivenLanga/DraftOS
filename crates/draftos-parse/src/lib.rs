//! File parsers: DOCX, PDF, TXT/MD, HTML → `ParsedDocument`.
//!
//! Parsers only recover text and coarse structure (paragraphs, declared
//! heading levels). All legal interpretation happens in draftos-extract.

mod docx;
mod html;
mod pdf;
mod text;

use draftos_core::error::{CoreError, Result};
use draftos_core::ParsedDocument;
use std::path::Path;

/// File extensions the ingestion pipeline picks up.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["docx", "pdf", "txt", "md", "html", "htm"];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn parse_file(path: &Path) -> Result<ParsedDocument> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    match ext.as_str() {
        "docx" => docx::parse(path, file_name),
        "pdf" => pdf::parse(path, file_name),
        "txt" | "md" => text::parse(path, file_name),
        "html" | "htm" => html::parse(path, file_name),
        other => Err(CoreError::UnsupportedFileType(other.to_string())),
    }
}
