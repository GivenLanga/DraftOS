//! Domain types shared by every DraftOS crate.
//!
//! This crate depends on nothing internal. Nothing here may perform I/O.

pub mod error;
pub mod lir;
pub mod manifest;
pub mod matter;
pub mod model;

pub use error::CoreError;
pub use lir::*;
pub use manifest::SourceManifest;
pub use matter::{MatterSpec, PartyInput};
pub use model::*;

/// Generate a new opaque id (UUID v4, lowercase, no braces).
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Current UTC timestamp in RFC 3339, the only time format stored on disk.
pub fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339()
}
