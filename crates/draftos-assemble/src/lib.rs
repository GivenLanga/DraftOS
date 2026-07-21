//! Deterministic assembly: MatterSpec + retrieved precedents → LIR document.
//!
//! No LLM. For each clause type the contract-type rule requires, retrieve the
//! best-matching precedent clause from the mounted sources, then order, number,
//! and cross-reference the results into a single LIR document. Every clause
//! keeps provenance back to the precedent it came from. Variable substitution
//! is literal (`{{name}}` from the spec); anything left unresolved is caught by
//! draftos-validate, never silently dropped.

use draftos_core::lir::*;
use draftos_core::{ClauseHit, ClauseKind, MatterSpec};
use draftos_embed::EmbeddingProvider;
use draftos_index::SourceBundle;
use draftos_retrieval::Filters;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Result of assembly: the document plus a report of what was and wasn't found.
pub struct AssembledDraft {
    pub document: LirDocument,
    pub report: AssemblyReport,
}

#[derive(Debug, Default)]
pub struct AssemblyReport {
    /// (clause_type, source_name, file) for each clause that was placed.
    pub filled: Vec<(String, String, String)>,
    /// Required clause types for which no precedent could be found.
    pub missing: Vec<String>,
    /// Definition terms placed in the Definitions clause.
    pub definitions: Vec<String>,
}

/// How many candidates to retrieve per clause type before picking the best.
const CANDIDATES: usize = 6;

pub fn assemble(
    spec: &MatterSpec,
    bundles: &[SourceBundle],
    embedder: &dyn EmbeddingProvider,
) -> draftos_core::error::Result<AssembledDraft> {
    let title = if spec.title.trim().is_empty() {
        spec.contract_type.clone()
    } else {
        spec.title.clone()
    };
    let mut doc = LirDocument::new(LirMeta {
        title,
        contract_type: spec.contract_type.clone(),
        jurisdiction: spec.jurisdiction.clone(),
        language: spec.language.clone(),
        matter_id: Some(spec.matter_id.clone()),
        created_at: draftos_core::now_utc(),
    });
    let mut report = AssemblyReport::default();

    // Parties + a framing recital, built purely from the spec.
    doc.parties = spec.parties.iter().map(to_party).collect();
    doc.recitals = build_recitals(spec);
    doc.execution = build_execution(spec);

    // Which clause types to assemble, in canonical order, honouring the spec's
    // include/exclude overrides. "Definitions" is handled separately below.
    let clause_types = resolve_clause_types(spec);

    let mut substantive: Vec<(String, ClauseHit)> = Vec::new();
    for ct in &clause_types {
        if ct.eq_ignore_ascii_case("Definitions") {
            continue;
        }
        match best_clause(bundles, embedder, spec, ct)? {
            Some(hit) => substantive.push((ct.clone(), hit)),
            None => {
                if draftos_rules::is_required(&spec.contract_type, ct) {
                    report.missing.push(ct.clone());
                }
            }
        }
    }

    // Substituted bodies drive definition pruning and term detection.
    let vars = &spec.variables;
    let substituted: Vec<String> = substantive
        .iter()
        .map(|(_, hit)| substitute(&hit.body, vars))
        .collect();

    // Definitions: keep only those a placed clause actually uses.
    let want_definitions = clause_types
        .iter()
        .any(|ct| ct.eq_ignore_ascii_case("Definitions"));
    let mut definitions: Vec<Definition> = Vec::new();
    if want_definitions {
        let all_bodies = substituted.join("\n");
        for hit in candidate_definitions(bundles, embedder, spec)? {
            let term = match &hit.term {
                Some(t) if !t.is_empty() => t.clone(),
                _ => continue,
            };
            if definitions.iter().any(|d| d.term == term) {
                continue;
            }
            if all_bodies.contains(&term) {
                report.definitions.push(term.clone());
                definitions.push(Definition {
                    term,
                    body: text_to_blocks(&substitute(&hit.body, vars)),
                    provenance: provenance(&hit),
                });
            }
        }
    }
    doc.definitions = definitions.clone();

    // Numbering: Definitions clause takes number 1 when present.
    let mut number = 1u32;
    if !definitions.is_empty() {
        doc.clauses.push(build_definitions_clause(number, &definitions));
        report
            .filled
            .push(("Definitions".to_string(), aggregate_sources(&definitions), String::new()));
        number += 1;
    }

    let def_terms: Vec<String> = definitions.iter().map(|d| d.term.clone()).collect();
    for ((ct, hit), body_text) in substantive.iter().zip(&substituted) {
        let blocks = text_to_blocks(body_text);
        doc.clauses.push(LirClause {
            id: draftos_core::new_id(),
            number: number.to_string(),
            heading: heading_for(ct, hit),
            defined_terms_used: terms_used(body_text, &def_terms),
            cross_refs: cross_refs(body_text),
            provenance: provenance(hit),
            body: blocks,
            // Carry the precedent's original paragraph XML so the renderer can
            // reproduce its house style; substitute variables into it too.
            source_ooxml: hit.ooxml.iter().map(|x| substitute(x, vars)).collect(),
        });
        report
            .filled
            .push((ct.clone(), hit.source_name.clone(), hit.file.clone()));
        number += 1;
    }

    Ok(AssembledDraft {
        document: doc,
        report,
    })
}

fn resolve_clause_types(spec: &MatterSpec) -> Vec<String> {
    let mut types: Vec<String> = draftos_rules::required_clause_types(&spec.contract_type);
    for extra in &spec.include_clause_types {
        if !types.iter().any(|t| t.eq_ignore_ascii_case(extra)) {
            types.push(extra.clone());
        }
    }
    types.retain(|t| {
        !spec
            .exclude_clause_types
            .iter()
            .any(|x| x.eq_ignore_ascii_case(t))
    });
    types.sort_by_key(|t| draftos_rules::order_index(&spec.contract_type, t));
    types
}

/// Retrieve the best precedent clause for a clause type. Prefers a matching
/// contract type, then relaxes to any contract type.
fn best_clause(
    bundles: &[SourceBundle],
    embedder: &dyn EmbeddingProvider,
    spec: &MatterSpec,
    clause_type: &str,
) -> draftos_core::error::Result<Option<ClauseHit>> {
    let strict = Filters {
        clause_type: Some(clause_type.to_string()),
        contract_type: Some(spec.contract_type.clone()),
        kind: Some(ClauseKind::Clause),
    };
    let mut hits = draftos_retrieval::search(bundles, embedder, clause_type, &strict, CANDIDATES)?;
    if hits.is_empty() {
        let relaxed = Filters {
            clause_type: Some(clause_type.to_string()),
            contract_type: None,
            kind: None,
        };
        hits = draftos_retrieval::search(bundles, embedder, clause_type, &relaxed, CANDIDATES)?;
    }
    Ok(hits.into_iter().find(|h| h.kind != ClauseKind::Definition))
}

fn candidate_definitions(
    bundles: &[SourceBundle],
    embedder: &dyn EmbeddingProvider,
    spec: &MatterSpec,
) -> draftos_core::error::Result<Vec<ClauseHit>> {
    let strict = Filters {
        kind: Some(ClauseKind::Definition),
        contract_type: Some(spec.contract_type.clone()),
        clause_type: None,
    };
    let mut hits =
        draftos_retrieval::search(bundles, embedder, "definitions meaning interpretation", &strict, 40)?;
    if hits.is_empty() {
        let relaxed = Filters {
            kind: Some(ClauseKind::Definition),
            contract_type: None,
            clause_type: None,
        };
        hits = draftos_retrieval::search(
            bundles,
            embedder,
            "definitions meaning interpretation",
            &relaxed,
            40,
        )?;
    }
    Ok(hits)
}

fn build_definitions_clause(number: u32, definitions: &[Definition]) -> LirClause {
    let items: Vec<Vec<Block>> = definitions.iter().map(|d| d.body.clone()).collect();
    LirClause {
        id: draftos_core::new_id(),
        number: number.to_string(),
        heading: "Definitions and Interpretation".to_string(),
        body: vec![
            Block::para("In this Agreement, unless the context indicates otherwise, the following terms bear the meanings assigned to them below:"),
            Block::List { ordered: false, items },
        ],
        cross_refs: Vec::new(),
        defined_terms_used: definitions.iter().map(|d| d.term.clone()).collect(),
        provenance: Provenance::default(),
        source_ooxml: Vec::new(),
    }
}

fn build_recitals(spec: &MatterSpec) -> Vec<Recital> {
    let mut runs = vec![Run::normal(format!(
        "This {} is entered into between ",
        spec.contract_type
    ))];
    for (i, p) in spec.parties.iter().enumerate() {
        if i > 0 {
            runs.push(Run::normal(if i + 1 == spec.parties.len() {
                " and "
            } else {
                ", "
            }));
        }
        runs.push(Run::bold(p.name.clone()));
        runs.push(Run::normal(format!(" (the \"{}\")", p.role)));
    }
    runs.push(Run::normal(
        ". The parties wish to record the terms and conditions set out below.",
    ));
    vec![Recital {
        id: draftos_core::new_id(),
        body: vec![Block::Paragraph { runs }],
    }]
}

fn build_execution(spec: &MatterSpec) -> Execution {
    Execution {
        blocks: spec
            .parties
            .iter()
            .map(|p| SignatureBlock {
                party_id: p.id.clone(),
                party_name: p.name.clone(),
                signatory_line: format!("for and on behalf of {}", p.name),
            })
            .collect(),
    }
}

fn to_party(p: &draftos_core::PartyInput) -> Party {
    Party {
        id: p.id.clone(),
        name: p.name.clone(),
        role: p.role.clone(),
        entity_type: p.entity_type.clone(),
        reg_no: p.reg_no.clone(),
        address: p.address.clone(),
    }
}

fn provenance(hit: &ClauseHit) -> Provenance {
    Provenance {
        source: Some(hit.source_name.clone()),
        file: Some(hit.file.clone()),
        original_clause_id: Some(hit.clause_id.clone()),
        adapted_by_model: false,
    }
}

fn heading_for(clause_type: &str, hit: &ClauseHit) -> String {
    match &hit.heading {
        Some(h) if !h.trim().is_empty() => title_case(h),
        _ => clause_type.to_string(),
    }
}

fn aggregate_sources(definitions: &[Definition]) -> String {
    let mut names: Vec<String> = definitions
        .iter()
        .filter_map(|d| d.provenance.source.clone())
        .collect();
    names.sort();
    names.dedup();
    names.join(", ")
}

/// Split a precedent body into paragraph blocks; strip any leading clause
/// number the precedent carried (fresh numbering is assigned by the assembler).
fn text_to_blocks(text: &str) -> Vec<Block> {
    let strip = leading_number_re();
    text.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| Block::para(strip.replace(line, "").trim().to_string()))
        .filter(|b| !b.plain_text().is_empty())
        .collect()
}

fn substitute(text: &str, vars: &BTreeMap<String, String>) -> String {
    let re = var_re();
    re.replace_all(text, |caps: &regex::Captures| {
        let name = caps[1].trim();
        match vars.get(name) {
            Some(v) => v.clone(),
            // Leave the placeholder intact so validation can flag it.
            None => caps[0].to_string(),
        }
    })
    .into_owned()
}

/// Which defined terms a clause body actually mentions. Public so tools that
/// rewrite a clause (e.g. LLM adaptation) can keep `defined_terms_used` honest.
pub fn terms_used(body: &str, terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .filter(|t| t.len() >= 3 && body.contains(t.as_str()))
        .cloned()
        .collect()
}

/// Clause-number cross-references mentioned in a body ("clause 12.3" → "12.3").
pub fn cross_refs(body: &str) -> Vec<String> {
    let mut refs: Vec<String> = cross_ref_re()
        .captures_iter(body)
        .map(|c| c[1].to_string())
        .collect();
    refs.sort();
    refs.dedup();
    refs
}

fn title_case(s: &str) -> String {
    // Precedent headings are often ALL CAPS; normalise to Title Case.
    if s.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) {
        s.split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        s.to_string()
    }
}

fn var_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // {{name}} or {{name|Human Label}} — we only key off the name.
    RE.get_or_init(|| Regex::new(r"\{\{\s*([A-Za-z0-9_]+)(?:\s*\|[^}]*)?\s*\}\}").unwrap())
}

fn cross_ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)clause\s+(\d+(?:\.\d+)*)").unwrap())
}

fn leading_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\d{1,3}(?:\.\d{1,3})*[.)]?\s+").unwrap())
}
