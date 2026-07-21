use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `manifest.json` of a knowledge-source bundle. The bundle directory holds
/// this file plus `index.db`; together they are the entire source — copyable,
/// detachable, re-attachable without any reprocessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceManifest {
    pub id: String,
    pub name: String,
    /// The watched folder on the user's filesystem.
    pub folder: PathBuf,
    /// Embedding model this bundle was built with. A bundle is only ever
    /// queried with the model recorded here; changing models means an explicit
    /// user-triggered rebuild.
    pub embed_model: String,
    pub embed_dims: usize,
    pub created_at: String,
    pub schema_version: u32,
}

impl SourceManifest {
    pub const CURRENT_SCHEMA: u32 = 1;
}
