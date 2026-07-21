//! Rendering: LIR → DOCX (primary) and LIR → Markdown (preview/tests).
//! Rendering reads only LIR; it makes no legal decisions. Callers must pass an
//! LIR that has cleared draftos-validate — rendering does not re-check.

mod docx;
mod markdown;

pub use docx::render_docx;
pub use markdown::render_markdown;
