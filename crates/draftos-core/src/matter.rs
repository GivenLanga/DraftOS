//! MatterSpec: the structured intake that drives deterministic drafting.
//! Produced by the intake questionnaire (or a JSON file for the CLI); consumed
//! by draftos-assemble. No prose here — just the facts a draft is built from.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterSpec {
    #[serde(default = "crate::new_id")]
    pub matter_id: String,
    /// Document title; if empty the assembler uses the contract type.
    #[serde(default)]
    pub title: String,
    pub contract_type: String,
    #[serde(default)]
    pub jurisdiction: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub parties: Vec<PartyInput>,
    /// Values for `{{variable}}` placeholders found in precedent clauses.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    /// Restrict retrieval to these source names; empty = all attached sources.
    #[serde(default)]
    pub source_scope: Vec<String>,
    /// Optional extra clause types to include beyond the contract-type rule.
    #[serde(default)]
    pub include_clause_types: Vec<String>,
    /// Clause types to exclude even if the rule marks them required.
    #[serde(default)]
    pub exclude_clause_types: Vec<String>,
}

fn default_language() -> String {
    "English".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyInput {
    pub id: String,
    pub name: String,
    /// The defined role, e.g. "Disclosing Party", "Seller", "Lender".
    pub role: String,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub reg_no: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}
