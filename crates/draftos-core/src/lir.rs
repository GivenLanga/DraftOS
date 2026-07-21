//! Legal Intermediate Representation (LIR): the canonical document format.
//! Every assembled draft is LIR, and rendering (DOCX/PDF) reads only LIR.
//! Kept in sync with docs/LIR_SPEC.md.

use serde::{Deserialize, Serialize};

pub const LIR_VERSION: &str = "0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LirDocument {
    pub lir_version: String,
    pub meta: LirMeta,
    pub parties: Vec<Party>,
    pub recitals: Vec<Recital>,
    /// Canonical defined-terms table. The Definitions clause in `clauses` is
    /// rendered from these; this array is what validation resolves against.
    pub definitions: Vec<Definition>,
    pub clauses: Vec<LirClause>,
    pub schedules: Vec<Schedule>,
    pub execution: Execution,
}

impl LirDocument {
    pub fn new(meta: LirMeta) -> Self {
        Self {
            lir_version: LIR_VERSION.to_string(),
            meta,
            parties: Vec::new(),
            recitals: Vec::new(),
            definitions: Vec::new(),
            clauses: Vec::new(),
            schedules: Vec::new(),
            execution: Execution::default(),
        }
    }

    /// Every clause in the document, depth-first (parents before children).
    /// Validation, cross-reference rewriting and rendering all walk this so a
    /// sub-clause is never silently skipped.
    pub fn walk_clauses(&self) -> Vec<&LirClause> {
        let mut out = Vec::new();
        for c in &self.clauses {
            c.collect_into(&mut out);
        }
        out
    }

    /// Total clause count including sub-clauses.
    pub fn clause_count(&self) -> usize {
        self.walk_clauses().len()
    }
}

impl LirClause {
    fn collect_into<'a>(&'a self, out: &mut Vec<&'a LirClause>) {
        out.push(self);
        for c in &self.children {
            c.collect_into(out);
        }
    }

    /// Apply `f` to this clause and every descendant, parents first.
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut LirClause)) {
        f(self);
        for c in &mut self.children {
            c.walk_mut(f);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LirMeta {
    pub title: String,
    pub contract_type: String,
    pub jurisdiction: Option<String>,
    pub language: String,
    pub matter_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Party {
    /// Stable token used to refer to this party ("disclosing", "seller").
    pub id: String,
    pub name: String,
    /// Defined role as it appears in the document ("Disclosing Party").
    pub role: String,
    pub entity_type: Option<String>,
    pub reg_no: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recital {
    pub id: String,
    pub body: Vec<Block>,
    #[serde(default)]
    pub provenance: Provenance,
    /// Original OOXML body paragraphs, as for `LirClause::source_ooxml`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ooxml: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definition {
    pub term: String,
    pub body: Vec<Block>,
    pub provenance: Provenance,
    /// Original OOXML of the definition's source paragraph (DOCX), so the
    /// Definitions clause can render each entry in the precedent's house style
    /// (and numbering) instead of a synthesised list item. Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ooxml: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LirClause {
    pub id: String,
    /// Assigned by the assembler only — never by a model or a precedent.
    /// Hierarchical: "3", "3.1", "3.1.2".
    pub number: String,
    pub heading: String,
    pub body: Vec<Block>,
    /// Sub-clauses, in order. A precedent's nesting is preserved here rather
    /// than flattened into run-on paragraphs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<LirClause>,
    pub cross_refs: Vec<String>,
    pub defined_terms_used: Vec<String>,
    pub provenance: Provenance,
    /// Original OOXML body paragraphs from the source precedent (DOCX), in
    /// order. When present, the DOCX renderer emits these — in the precedent's
    /// own house style, keeping the source's numbering — instead of synthesising
    /// paragraphs from `body`. Empty for clauses drawn from non-DOCX sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ooxml: Vec<String>,
    /// Original OOXML of the source clause's heading paragraph (DOCX). When
    /// present, the renderer emits it verbatim so the heading's own numbering
    /// (e.g. the multilevel list the clause hangs off) drives the clause number,
    /// instead of a synthesised one. `None` for synthetic clauses (Definitions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_ooxml: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub title: String,
    pub body: Vec<Block>,
    #[serde(default)]
    pub provenance: Provenance,
    /// Original OOXML body paragraphs, as for `LirClause::source_ooxml`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ooxml: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_ooxml: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Execution {
    pub blocks: Vec<SignatureBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureBlock {
    pub party_id: String,
    pub party_name: String,
    pub signatory_line: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Provenance {
    /// Source name the content came from ("banking-precedents").
    pub source: Option<String>,
    pub file: Option<String>,
    pub original_clause_id: Option<String>,
    /// True once an LLM has rewritten/filled this node (Phase 3).
    pub adapted_by_model: bool,
}

/// A content block. `variable` blocks are placeholders resolved from the
/// MatterSpec; an unresolved required variable blocks rendering (validation).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph { runs: Vec<Run> },
    List { ordered: bool, items: Vec<Vec<Block>> },
    Table { rows: Vec<Vec<Vec<Block>>> },
    Variable {
        name: String,
        label: String,
        value: Option<String>,
    },
}

impl Block {
    /// Convenience: a plain paragraph of normal-weight text.
    pub fn para(text: impl Into<String>) -> Block {
        Block::Paragraph {
            runs: vec![Run::normal(text)],
        }
    }

    /// Flatten a block's textual content (for term/cross-ref scanning).
    pub fn plain_text(&self) -> String {
        match self {
            Block::Paragraph { runs } => {
                runs.iter().map(|r| r.text.as_str()).collect::<String>()
            }
            Block::List { items, .. } => items
                .iter()
                .flat_map(|blocks| blocks.iter().map(|b| b.plain_text()))
                .collect::<Vec<_>>()
                .join(" "),
            Block::Table { rows } => rows
                .iter()
                .flat_map(|row| row.iter().flat_map(|cell| cell.iter().map(|b| b.plain_text())))
                .collect::<Vec<_>>()
                .join(" "),
            Block::Variable { value, label, .. } => {
                value.clone().unwrap_or_else(|| format!("[{label}]"))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(default)]
    pub style: RunStyle,
}

impl Run {
    pub fn normal(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            style: RunStyle::Normal,
        }
    }
    pub fn bold(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            style: RunStyle::Bold,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStyle {
    #[default]
    Normal,
    Bold,
    Italic,
}
