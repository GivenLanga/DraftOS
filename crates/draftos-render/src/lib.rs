//! Rendering: LIR → DOCX (primary) and LIR → Markdown (preview/tests).
//! Rendering reads only LIR; it makes no legal decisions. Callers must pass an
//! LIR that has cleared draftos-validate — rendering does not re-check.

mod docx;
mod markdown;
mod style;

pub use docx::{render_docx, render_docx_with_style};
pub use markdown::render_markdown;
pub use style::StyleDonor;
