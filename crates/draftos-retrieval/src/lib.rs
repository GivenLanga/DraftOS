//! Hybrid retrieval across mounted knowledge sources.
//!
//! Per source: BM25 (FTS5) ranking + vector nearest-neighbour ranking, fused
//! with Reciprocal Rank Fusion. Results from all mounted sources are merged
//! by fused score. Sources are queried independently — a slow or detached
//! source never affects the others.

use draftos_core::error::Result;
use draftos_core::{ClauseHit, ClauseKind};
use draftos_embed::EmbeddingProvider;
use draftos_index::SourceBundle;
use std::collections::HashMap;

const RRF_K: f64 = 60.0;

#[derive(Debug, Default, Clone)]
pub struct Filters {
    /// e.g. "Termination" — matches clause_type metadata.
    pub clause_type: Option<String>,
    /// e.g. only definitions.
    pub kind: Option<ClauseKind>,
    /// e.g. "Share Purchase Agreement".
    pub contract_type: Option<String>,
}

pub fn search(
    bundles: &[SourceBundle],
    embedder: &dyn EmbeddingProvider,
    query: &str,
    filters: &Filters,
    limit: usize,
) -> Result<Vec<ClauseHit>> {
    let query_embedding = embedder
        .embed_one(query)
        .map_err(|e| draftos_core::CoreError::Embedding(e.to_string()))?;

    // Over-fetch per source so post-filtering still fills `limit`.
    let per_source = (limit * 3).max(20);
    let mut all_hits: Vec<ClauseHit> = Vec::new();

    for bundle in bundles {
        let fts = bundle.fts_search(query, per_source)?;
        let vec = bundle.vec_search(&query_embedding, per_source)?;

        // Reciprocal Rank Fusion over the two ranked lists.
        let mut fused: HashMap<i64, f64> = HashMap::new();
        for (rank, rowid) in fts.iter().enumerate() {
            *fused.entry(*rowid).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
        for (rank, rowid) in vec.iter().enumerate() {
            *fused.entry(*rowid).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        let mut rowids: Vec<i64> = fused.keys().copied().collect();
        rowids.sort_by(|a, b| fused[b].partial_cmp(&fused[a]).unwrap());
        rowids.truncate(per_source);

        for (rowid, mut hit) in bundle.hydrate(&rowids)? {
            hit.score = fused[&rowid];
            if matches_filters(&hit, filters) {
                all_hits.push(hit);
            }
        }
    }

    all_hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    all_hits.truncate(limit);
    Ok(all_hits)
}

/// One-hop knowledge-graph expansion: for each hit, pull in the clauses it
/// references (in practice: the definitions its body uses) so a retrieved
/// Termination clause brings "Business Day", "Notice" etc. along with it.
/// Returns only *new* hits — callers append them after the ranked results.
pub fn expand_hits(bundles: &[SourceBundle], hits: &[ClauseHit]) -> Result<Vec<ClauseHit>> {
    let mut seen: std::collections::HashSet<String> =
        hits.iter().map(|h| h.clause_id.clone()).collect();
    let mut expanded = Vec::new();
    for bundle in bundles {
        let ids: Vec<String> = hits
            .iter()
            .filter(|h| h.source_name == bundle.manifest.name)
            .map(|h| h.clause_id.clone())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let rowids = bundle.neighbor_rowids(&ids)?;
        for (_, hit) in bundle.hydrate(&rowids)? {
            if seen.insert(hit.clause_id.clone()) {
                expanded.push(hit);
            }
        }
    }
    Ok(expanded)
}

fn matches_filters(hit: &ClauseHit, filters: &Filters) -> bool {
    if let Some(kind) = &filters.kind {
        if hit.kind != *kind {
            return false;
        }
    }
    if let Some(ct) = &filters.clause_type {
        match &hit.metadata.clause_type {
            Some(v) if v.eq_ignore_ascii_case(ct) => {}
            _ => return false,
        }
    }
    if let Some(ct) = &filters.contract_type {
        match &hit.metadata.contract_type {
            Some(v) if v.eq_ignore_ascii_case(ct) => {}
            _ => return false,
        }
    }
    true
}
