use serde::{Deserialize, Serialize};

/// A document as produced by the parser layer: ordered paragraphs with
/// whatever structural hints the file format exposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub file_name: String,
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub text: String,
    /// Heading level (1-based) when the source format declares one (e.g. DOCX
    /// paragraph styles). Heuristic heading detection happens later, in extract.
    pub heading_level: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClauseKind {
    Recital,
    Definition,
    Clause,
    Schedule,
    Execution,
    Other,
}

impl ClauseKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClauseKind::Recital => "recital",
            ClauseKind::Definition => "definition",
            ClauseKind::Clause => "clause",
            ClauseKind::Schedule => "schedule",
            ClauseKind::Execution => "execution",
            ClauseKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> ClauseKind {
        match s {
            "recital" => ClauseKind::Recital,
            "definition" => ClauseKind::Definition,
            "clause" => ClauseKind::Clause,
            "schedule" => ClauseKind::Schedule,
            "execution" => ClauseKind::Execution,
            _ => ClauseKind::Other,
        }
    }
}

/// One extracted knowledge object: a clause, definition, recital, schedule…
/// These are the embedding unit — whole documents are never embedded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedClause {
    pub kind: ClauseKind,
    /// Clause number as it appeared in the document ("12", "12.3").
    pub number: Option<String>,
    pub heading: Option<String>,
    /// The defined term, for `kind == Definition` ("Effective Date").
    pub term: Option<String>,
    pub body: String,
    pub metadata: ClauseMetadata,
}

/// Metadata attached to every clause. All fields optional — extraction fills
/// what it can, users can correct later. Stored per-clause in the source index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClauseMetadata {
    pub contract_type: Option<String>,
    pub clause_type: Option<String>,
    pub jurisdiction: Option<String>,
    pub industry: Option<String>,
    pub risk: Option<String>,
    pub language: Option<String>,
    pub version: Option<String>,
    /// Relative path of the file this clause came from.
    pub source: Option<String>,
    pub approved: Option<bool>,
}

/// A retrieval hit returned to the UI/CLI. Carries provenance always.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClauseHit {
    pub clause_id: String,
    pub source_name: String,
    pub file: String,
    pub kind: ClauseKind,
    pub number: Option<String>,
    pub heading: Option<String>,
    pub term: Option<String>,
    pub body: String,
    pub metadata: ClauseMetadata,
    /// Fused relevance score (higher is better).
    pub score: f64,
}
