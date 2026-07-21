//! Deterministic assembly: MatterSpec + precedents → LIR document.
//!
//! **Precedent-led.** The document's structure comes from a precedent in the
//! user's own knowledge sources — its clause order, its nesting, its
//! definitions, its schedules, its recitals. DraftOS does not own a template.
//! That is what makes the output resemble the corpus it was given, and what
//! lets the same code draft a charterparty and an NDA without a line of
//! contract-type-specific logic.
//!
//! The pipeline:
//!   1. choose the best-matching precedent as the *skeleton*;
//!   2. rebuild its clause tree (parents and sub-clauses) from the index;
//!   3. substitute `{{variables}}` from the spec;
//!   4. audit against draftos-rules and fill genuine gaps from other precedents;
//!   5. renumber hierarchically and rewrite cross-references to match;
//!   6. carry definitions, schedules, recitals and execution through.
//!
//! No LLM. Every step is reproducible and unit-tested. Anything that could not
//! be resolved is reported, never silently dropped.

use draftos_core::lir::*;
use draftos_core::{ClauseHit, ClauseKind, MatterSpec};
use draftos_embed::EmbeddingProvider;
use draftos_index::SourceBundle;
use draftos_retrieval::Filters;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub struct AssembledDraft {
    pub document: LirDocument,
    pub report: AssemblyReport,
}

/// Which precedent supplied the document's structure, and how well it matched.
#[derive(Debug, Clone)]
pub struct SkeletonChoice {
    pub source_name: String,
    pub file: String,
    pub title: Option<String>,
    pub contract_type: Option<String>,
    /// True when the precedent is of the contract type the matter asked for.
    /// When false the draft is still built from it, and this is reported.
    pub exact_type_match: bool,
    pub clause_count: usize,
}

#[derive(Debug, Default)]
pub struct AssemblyReport {
    /// The precedent the structure came from.
    pub skeleton: Option<SkeletonChoice>,
    /// Top-level clauses taken from the skeleton, with their labels.
    pub from_skeleton: Vec<String>,
    /// (clause_type, source, file) for clauses the checklist flagged as absent
    /// and that were filled from another precedent.
    pub gap_filled: Vec<(String, String, String)>,
    /// Required clause types absent from the skeleton and unavailable anywhere.
    pub missing: Vec<String>,
    /// Clause types dropped because the spec excluded them.
    pub excluded: Vec<String>,
    /// Defined terms carried into the document.
    pub definitions: Vec<String>,
    /// Renumbering applied, old → new, for audit and cross-reference tracing.
    pub renumbered: Vec<(String, String)>,
    /// Schedules/annexures carried through from the precedent.
    pub schedules: Vec<String>,
    /// Non-fatal notes for the user (e.g. skeleton of a different type).
    pub notes: Vec<String>,
}

/// How many candidates to consider when filling a gap the skeleton left.
const CANDIDATES: usize = 6;

pub fn assemble(
    spec: &MatterSpec,
    bundles: &[SourceBundle],
    embedder: &dyn EmbeddingProvider,
) -> draftos_core::error::Result<AssembledDraft> {
    let mut report = AssemblyReport::default();

    let choice = choose_skeleton(spec, bundles, &mut report)?;
    let (bundle, doc_id, choice) = match choice {
        Some(c) => c,
        None => {
            return Err(draftos_core::CoreError::Assembly(
                "no precedent found in the mounted sources to draft from — attach a source \
                 containing precedents of this kind, or rescan an existing one"
                    .to_string(),
            ))
        }
    };
    report.skeleton = Some(choice.clone());

    let objects = bundle.document_clauses(&doc_id)?;
    let vars = &spec.variables;

    // ---- 1. Clause tree, straight from the precedent's own structure --------
    let excluded = |label: &Option<String>| -> bool {
        match label {
            Some(l) => spec
                .exclude_clause_types
                .iter()
                .any(|x| x.eq_ignore_ascii_case(l)),
            None => false,
        }
    };

    let clause_hits: Vec<&ClauseHit> = objects
        .iter()
        .filter(|h| h.kind == ClauseKind::Clause)
        .collect();
    let mut nodes = build_tree(&clause_hits);
    nodes.retain(|n| {
        let drop = excluded(&n.hit.metadata.clause_type);
        if drop {
            if let Some(l) = &n.hit.metadata.clause_type {
                report.excluded.push(l.clone());
            }
        }
        !drop
    });

    // ---- 2. Audit against the checklist and fill genuine gaps ---------------
    let present: Vec<String> = nodes
        .iter()
        .filter_map(|n| n.hit.metadata.clause_type.clone())
        .collect();
    for want in draftos_rules::missing_required(&spec.contract_type, &present) {
        if spec
            .exclude_clause_types
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&want))
        {
            continue;
        }
        match best_clause(bundles, embedder, spec, &want)? {
            Some(hit) => {
                report
                    .gap_filled
                    .push((want.clone(), hit.source_name.clone(), hit.file.clone()));
                insert_by_order(&mut nodes, &spec.contract_type, &want, hit);
            }
            None => report.missing.push(want),
        }
    }

    // Extra clause types the spec explicitly asked for beyond the precedent.
    for want in &spec.include_clause_types {
        if present.iter().any(|p| p.eq_ignore_ascii_case(want)) {
            continue;
        }
        if report.gap_filled.iter().any(|(l, _, _)| l.eq_ignore_ascii_case(want)) {
            continue;
        }
        if let Some(hit) = best_clause(bundles, embedder, spec, want)? {
            report
                .gap_filled
                .push((want.clone(), hit.source_name.clone(), hit.file.clone()));
            insert_by_order(&mut nodes, &spec.contract_type, want, hit);
        } else {
            report.missing.push(want.clone());
        }
    }

    // ---- 3. Renumber, keeping the hierarchy --------------------------------
    let mut number_map: BTreeMap<String, String> = BTreeMap::new();
    let mut clauses: Vec<LirClause> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        clauses.push(to_lir_clause(node, &(i + 1).to_string(), vars, &mut number_map));
    }
    report.renumbered = number_map
        .iter()
        .map(|(a, b)| (a.clone(), b.clone()))
        .collect();
    report.from_skeleton = nodes
        .iter()
        .map(|n| label_of(&n.hit))
        .collect();

    // ---- 4. Cross-references follow the new numbering ----------------------
    for c in &mut clauses {
        c.walk_mut(&mut |clause| {
            for b in &mut clause.body {
                rewrite_block_refs(b, &number_map);
            }
            for x in &mut clause.source_ooxml {
                *x = rewrite_ooxml_refs(x, &number_map);
            }
            let text = clause_text(clause);
            clause.cross_refs = cross_refs(&text);
        });
    }

    // ---- 5. Definitions, schedules, recitals, execution --------------------
    let definitions: Vec<Definition> = objects
        .iter()
        .filter(|h| h.kind == ClauseKind::Definition)
        .filter_map(|h| {
            let term = h.term.clone().filter(|t| !t.is_empty())?;
            Some(Definition {
                term,
                body: text_to_blocks(&substitute(&h.body, vars)),
                provenance: provenance(h),
                ooxml: h.ooxml.iter().map(|x| substitute(x, vars)).collect(),
            })
        })
        .fold(Vec::new(), |mut acc, d| {
            if !acc.iter().any(|e: &Definition| e.term == d.term) {
                acc.push(d);
            }
            acc
        });
    report.definitions = definitions.iter().map(|d| d.term.clone()).collect();

    let schedules: Vec<Schedule> = build_schedules(&objects, vars, &number_map);
    report.schedules = schedules.iter().map(|s| s.title.clone()).collect();

    let recitals = build_recitals(spec, &objects, vars);
    let execution = build_execution(spec, &objects);

    // ---- 6. Assemble the document ------------------------------------------
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
    doc.parties = spec.parties.iter().map(to_party).collect();
    doc.recitals = recitals;
    doc.definitions = definitions;
    doc.clauses = clauses;
    doc.schedules = schedules;
    doc.execution = execution;

    // Defined-terms-used is computed last, against the final term table.
    let terms: Vec<String> = doc.definitions.iter().map(|d| d.term.clone()).collect();
    for c in &mut doc.clauses {
        c.walk_mut(&mut |clause| {
            let text = clause_text(clause);
            clause.defined_terms_used = terms_used(&text, &terms);
        });
    }

    Ok(AssembledDraft {
        document: doc,
        report,
    })
}

// ---------------------------------------------------------------------------
// Skeleton selection
// ---------------------------------------------------------------------------

/// Pick the precedent whose structure the draft will follow. Scored on contract
/// type, then title, jurisdiction, and how complete the precedent is. Nothing
/// here is specific to any contract type or jurisdiction — a corpus of shipping
/// charterparties scores exactly the same way a corpus of NDAs does.
fn choose_skeleton<'a>(
    spec: &MatterSpec,
    bundles: &'a [SourceBundle],
    report: &mut AssemblyReport,
) -> draftos_core::error::Result<Option<(&'a SourceBundle, String, SkeletonChoice)>> {
    let mut best: Option<(f64, &SourceBundle, String, SkeletonChoice)> = None;
    let mut any_document = false;
    let mut stale_sources: Vec<String> = Vec::new();

    for bundle in bundles {
        if bundle.needs_rebuild()? {
            stale_sources.push(bundle.manifest.name.clone());
        }
        for d in bundle.list_documents()? {
            any_document = true;
            if d.clause_count == 0 {
                continue;
            }
            let exact = d
                .contract_type
                .as_deref()
                .is_some_and(|ct| ct.eq_ignore_ascii_case(&spec.contract_type));

            // An explicitly named precedent wins outright.
            let forced = spec.skeleton_precedent.as_deref().is_some_and(|want| {
                let want = want.to_ascii_lowercase();
                d.rel_path.to_ascii_lowercase().contains(&want)
                    || d.title
                        .as_deref()
                        .is_some_and(|t| t.to_ascii_lowercase().contains(&want))
            });

            let mut score = 0.0f64;
            if forced {
                score += 10_000.0;
            }
            if exact {
                score += 1_000.0;
            } else if let Some(t) = &d.title {
                score += 200.0 * title_similarity(t, &spec.contract_type);
            }
            if let (Some(a), Some(b)) = (&d.jurisdiction, &spec.jurisdiction) {
                if a.eq_ignore_ascii_case(b) {
                    score += 50.0;
                }
            }
            // Prefer a complete precedent, with diminishing returns.
            score += (d.clause_count.min(60) as f64).sqrt() * 10.0;

            let choice = SkeletonChoice {
                source_name: d.source_name.clone(),
                file: d.rel_path.clone(),
                title: d.title.clone(),
                contract_type: d.contract_type.clone(),
                exact_type_match: exact,
                clause_count: d.clause_count,
            };
            if best.as_ref().is_none_or(|(s, ..)| score > *s) {
                best = Some((score, bundle, d.id.clone(), choice));
            }
        }
    }

    for name in stale_sources {
        report.notes.push(format!(
            "source '{name}' was indexed before structure-aware ingestion — rescan it so its \
             precedents can supply document structure"
        ));
    }
    if !any_document {
        return Ok(None);
    }
    match best {
        Some((_, bundle, doc_id, choice)) => {
            if !choice.exact_type_match {
                report.notes.push(format!(
                    "no precedent of type '{}' was found; drafted from '{}'{} instead — review \
                     the structure carefully",
                    spec.contract_type,
                    choice.file,
                    choice
                        .contract_type
                        .as_deref()
                        .map(|c| format!(" (a {c})"))
                        .unwrap_or_default(),
                ));
            }
            Ok(Some((bundle, doc_id, choice)))
        }
        None => Ok(None),
    }
}

/// Crude token overlap, enough to prefer "Service Level Agreement" over
/// "Lease Agreement" when the matter asks for a "Service Agreement".
fn title_similarity(a: &str, b: &str) -> f64 {
    let toks = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 2)
            .map(str::to_string)
            .collect()
    };
    let (ta, tb) = (toks(a), toks(b));
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let shared = tb.iter().filter(|t| ta.contains(t)).count();
    shared as f64 / tb.len() as f64
}

// ---------------------------------------------------------------------------
// Clause tree
// ---------------------------------------------------------------------------

struct Node {
    hit: ClauseHit,
    children: Vec<Node>,
}

/// Rebuild the precedent's clause hierarchy from the flat, `seq`-ordered index
/// rows, using each row's `depth`. A clause deeper than the one before it
/// becomes its child; this is what stops "5.1 / 5.2 / 5.3" collapsing into one
/// run-on paragraph.
fn build_tree(hits: &[&ClauseHit]) -> Vec<Node> {
    let mut roots: Vec<Node> = Vec::new();
    // Path of indices into the tree identifying the current open node per depth.
    let mut path: Vec<usize> = Vec::new();

    for hit in hits {
        let depth = hit.depth as usize;
        // A node can only nest one level deeper than its parent, however deep
        // its printed number claims to be.
        let depth = depth.min(path.len());
        path.truncate(depth);

        let node = Node {
            hit: (*hit).clone(),
            children: Vec::new(),
        };
        let mut cursor = &mut roots;
        for idx in &path {
            cursor = &mut cursor[*idx].children;
        }
        cursor.push(node);
        path.push(cursor.len() - 1);
    }
    roots
}

/// Insert a gap-filled clause at the position the checklist implies, leaving
/// the precedent's own ordering otherwise untouched.
fn insert_by_order(nodes: &mut Vec<Node>, contract_type: &str, label: &str, hit: ClauseHit) {
    let want = draftos_rules::order_index(contract_type, label);
    let at = nodes
        .iter()
        .position(|n| draftos_rules::order_index(contract_type, &label_of(&n.hit)) > want)
        .unwrap_or(nodes.len());
    nodes.insert(
        at,
        Node {
            hit,
            children: Vec::new(),
        },
    );
}

fn to_lir_clause(
    node: &Node,
    number: &str,
    vars: &BTreeMap<String, String>,
    map: &mut BTreeMap<String, String>,
) -> LirClause {
    if let Some(old) = &node.hit.number {
        map.insert(old.clone(), number.to_string());
    }
    let body_text = substitute(&node.hit.body, vars);
    let children = node
        .children
        .iter()
        .enumerate()
        .map(|(i, child)| to_lir_clause(child, &format!("{number}.{}", i + 1), vars, map))
        .collect();

    LirClause {
        id: draftos_core::new_id(),
        number: number.to_string(),
        heading: heading_for(&node.hit),
        body: text_to_blocks(&body_text),
        children,
        cross_refs: Vec::new(),
        defined_terms_used: Vec::new(),
        provenance: provenance(&node.hit),
        source_ooxml: node.hit.ooxml.iter().map(|x| substitute(x, vars)).collect(),
        heading_ooxml: node.hit.heading_ooxml.as_ref().map(|x| substitute(x, vars)),
    }
}

// ---------------------------------------------------------------------------
// Gap filling
// ---------------------------------------------------------------------------

/// Best precedent clause for a clause type the skeleton lacked. Never returns
/// front matter, a schedule or a signature block — those are not clauses.
fn best_clause(
    bundles: &[SourceBundle],
    embedder: &dyn EmbeddingProvider,
    spec: &MatterSpec,
    clause_type: &str,
) -> draftos_core::error::Result<Option<ClauseHit>> {
    let strict = Filters {
        clause_type: Some(clause_type.to_string()),
        contract_type: Some(spec.contract_type.clone()),
        approved_only: spec.approved_only,
        ..Filters::operative_clause()
    };
    let mut hits = draftos_retrieval::search(bundles, embedder, clause_type, &strict, CANDIDATES)?;
    if hits.is_empty() {
        let relaxed = Filters {
            clause_type: Some(clause_type.to_string()),
            approved_only: spec.approved_only,
            ..Filters::operative_clause()
        };
        hits = draftos_retrieval::search(bundles, embedder, clause_type, &relaxed, CANDIDATES)?;
    }
    Ok(hits.into_iter().next())
}

// ---------------------------------------------------------------------------
// Front matter, schedules, execution
// ---------------------------------------------------------------------------

fn build_schedules(
    objects: &[ClauseHit],
    vars: &BTreeMap<String, String>,
    map: &BTreeMap<String, String>,
) -> Vec<Schedule> {
    let mut out: Vec<Schedule> = Vec::new();
    for h in objects.iter().filter(|h| h.kind == ClauseKind::Schedule) {
        let title = h
            .heading
            .as_deref()
            .map(|t| substitute(t, vars))
            .unwrap_or_else(|| format!("Schedule {}", out.len() + 1));
        let body_text = rewrite_refs(&substitute(&h.body, vars), map);
        // A schedule's continuation paragraphs arrive as separate objects with
        // no heading; fold them into the schedule they belong to.
        match (h.heading.is_none(), out.last_mut()) {
            (true, Some(last)) => {
                last.body.extend(text_to_blocks(&body_text));
                last.source_ooxml
                    .extend(h.ooxml.iter().map(|x| rewrite_ooxml_refs(&substitute(x, vars), map)));
            }
            _ => out.push(Schedule {
                id: draftos_core::new_id(),
                title,
                body: text_to_blocks(&body_text),
                provenance: provenance(h),
                source_ooxml: h
                    .ooxml
                    .iter()
                    .map(|x| rewrite_ooxml_refs(&substitute(x, vars), map))
                    .collect(),
                heading_ooxml: h.heading_ooxml.as_ref().map(|x| substitute(x, vars)),
            }),
        }
    }
    out
}

/// Recitals come from the precedent when it has them. Only when it has none do
/// we synthesise a framing recital from the spec.
fn build_recitals(
    spec: &MatterSpec,
    objects: &[ClauseHit],
    vars: &BTreeMap<String, String>,
) -> Vec<Recital> {
    let from_precedent: Vec<Recital> = objects
        .iter()
        .filter(|h| h.kind == ClauseKind::Recital)
        .filter(|h| !h.body.trim().is_empty())
        // The title page and parties block state who is contracting, and in a
        // precedent that is the *previous* deal's parties. Those are rebuilt
        // from the matter spec; only substantive recitals carry through.
        .filter(|h| is_substantive_recital(h))
        .map(|h| Recital {
            id: draftos_core::new_id(),
            body: text_to_blocks(&substitute(&h.body, vars)),
            provenance: provenance(h),
            source_ooxml: h.ooxml.iter().map(|x| substitute(x, vars)).collect(),
        })
        .collect();
    if !from_precedent.is_empty() {
        return from_precedent;
    }

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
        provenance: Provenance::default(),
        source_ooxml: Vec::new(),
    }]
}

/// Whether a front-matter object is a real recital ("WHEREAS the parties…")
/// rather than the precedent's title page and parties block.
fn is_substantive_recital(hit: &ClauseHit) -> bool {
    let heading = hit.heading.as_deref().unwrap_or("").to_ascii_lowercase();
    let labelled = ["recital", "preamble", "background", "introduction"]
        .iter()
        .any(|k| heading.contains(k));
    let body = hit.body.to_ascii_lowercase();
    labelled || body.contains("whereas")
}

/// Signature blocks are always built from the matter's parties — the
/// precedent's signatories belong to a different deal. The precedent's
/// execution *wording* is not reused for that reason.
fn build_execution(spec: &MatterSpec, _objects: &[ClauseHit]) -> Execution {
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn label_of(hit: &ClauseHit) -> String {
    hit.metadata
        .clause_type
        .clone()
        .or_else(|| hit.heading.clone())
        .unwrap_or_default()
}

/// The clause's heading, in the precedent's own words. Sub-clauses usually have
/// none, and that is fine — they render as numbered body text.
fn heading_for(hit: &ClauseHit) -> String {
    match &hit.heading {
        Some(h) if !h.trim().is_empty() => draftos_extract::normalize_label(h),
        _ => String::new(),
    }
}

fn clause_text(c: &LirClause) -> String {
    c.body
        .iter()
        .map(|b| b.plain_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Split a precedent body into paragraph blocks. Leading numbers are *not*
/// stripped here — a numbered sub-clause is its own node, so any number still
/// embedded in a body line is part of the drafter's prose.
fn text_to_blocks(text: &str) -> Vec<Block> {
    text.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(Block::para)
        .filter(|b| !b.plain_text().is_empty())
        .collect()
}

fn substitute(text: &str, vars: &BTreeMap<String, String>) -> String {
    var_re()
        .replace_all(text, |caps: &regex::Captures| {
            let name = caps[1].trim();
            match vars.get(name) {
                Some(v) => v.clone(),
                // Leave the placeholder intact so validation can flag it.
                None => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// Rewrite "clause 14.1" to whatever 14.1 became under the new numbering.
/// A reference with no mapping is left alone so validation reports it.
pub fn rewrite_refs(text: &str, map: &BTreeMap<String, String>) -> String {
    cross_ref_re()
        .replace_all(text, |caps: &regex::Captures| {
            match map.get(&caps[1]) {
                Some(new) => caps[0].replace(&caps[1], new),
                None => caps[0].to_string(),
            }
        })
        .into_owned()
}

fn rewrite_block_refs(block: &mut Block, map: &BTreeMap<String, String>) {
    match block {
        Block::Paragraph { runs } => {
            for r in runs {
                r.text = rewrite_refs(&r.text, map);
            }
        }
        Block::List { items, .. } => {
            for item in items {
                for b in item {
                    rewrite_block_refs(b, map);
                }
            }
        }
        Block::Table { rows } => {
            for row in rows {
                for cell in row {
                    for b in cell {
                        rewrite_block_refs(b, map);
                    }
                }
            }
        }
        Block::Variable { value, .. } => {
            if let Some(v) = value {
                *v = rewrite_refs(v, map);
            }
        }
    }
}

/// Rewrite cross-references inside lifted OOXML, touching only `<w:t>` text so
/// no markup is disturbed. Best-effort: a reference split across two runs (as
/// Word does after a flattened REF field) will not match here and is caught by
/// validation instead.
fn rewrite_ooxml_refs(xml: &str, map: &BTreeMap<String, String>) -> String {
    if map.is_empty() {
        return xml.to_string();
    }
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(open) = rest.find("<w:t") {
        let Some(gt) = rest[open..].find('>').map(|i| open + i + 1) else {
            break;
        };
        let Some(close) = rest[gt..].find("</w:t>").map(|i| gt + i) else {
            break;
        };
        out.push_str(&rest[..gt]);
        out.push_str(&rewrite_refs(&rest[gt..close], map));
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
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

fn var_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // {{name}} or {{name|Human Label}} — we only key off the name.
    RE.get_or_init(|| Regex::new(r"\{\{\s*([A-Za-z0-9_]+)(?:\s*\|[^}]*)?\s*\}\}").unwrap())
}

fn cross_ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)clauses?\s+(\d+(?:\.\d+)*)").unwrap())
}
