//! Graph expansion: a retrieved clause brings the definitions it uses along.

use draftos_core::{ClauseKind, ClauseMetadata, ExtractedClause, SourceManifest};
use draftos_embed::{EmbeddingProvider, HashEmbedder};
use draftos_index::SourceBundle;
use draftos_retrieval::{expand_hits, search, Filters};

fn extracted(kind: ClauseKind, heading: Option<&str>, term: Option<&str>, body: &str) -> ExtractedClause {
    ExtractedClause {
        kind,
        number: None,
        heading: heading.map(str::to_string),
        term: term.map(str::to_string),
        body: body.to_string(),
        metadata: ClauseMetadata::default(),
    }
}

#[test]
fn expansion_pulls_referenced_definitions() {
    let tmp = std::env::temp_dir().join(format!("draftos-expand-{}", draftos_core::new_id()));
    let embedder = HashEmbedder::new(HashEmbedder::default_dims());
    let manifest = SourceManifest {
        id: "t".into(),
        name: "test".into(),
        folder: tmp.clone(),
        embed_model: embedder.id(),
        embed_dims: embedder.dims(),
        created_at: draftos_core::now_utc(),
        schema_version: SourceManifest::CURRENT_SCHEMA,
    };
    let mut bundle = SourceBundle::create(&tmp.join("bundle"), manifest).unwrap();

    let clauses = vec![
        extracted(
            ClauseKind::Clause,
            Some("Termination"),
            None,
            "Either party may terminate this Agreement on 30 Business Days written notice.",
        ),
        extracted(
            ClauseKind::Definition,
            None,
            Some("Business Day"),
            "\"Business Day\" means any day other than a Saturday, Sunday or public holiday.",
        ),
        extracted(
            ClauseKind::Definition,
            None,
            Some("Purchase Price"),
            "\"Purchase Price\" means the amount set out in Schedule 1.",
        ),
    ];
    let embeddings: Vec<Vec<f32>> = clauses.iter().map(|c| embedder.embed_one(&c.body).unwrap()).collect();
    bundle.upsert_document("spa.txt", "h1", &clauses, &embeddings).unwrap();

    let bundles = vec![bundle];
    let hits = search(
        &bundles,
        &embedder,
        "terminate the agreement",
        &Filters { kind: Some(ClauseKind::Clause), ..Default::default() },
        5,
    )
    .unwrap();
    assert!(hits.iter().any(|h| h.heading.as_deref() == Some("Termination")));

    let expanded = expand_hits(&bundles, &hits).unwrap();
    // The Termination clause uses "Business Day" → its definition comes along…
    assert!(expanded.iter().any(|h| h.term.as_deref() == Some("Business Day")));
    // …but not the unrelated "Purchase Price" definition.
    assert!(!expanded.iter().any(|h| h.term.as_deref() == Some("Purchase Price")));
    // Expansion never duplicates the original hits.
    let hit_ids: Vec<&str> = hits.iter().map(|h| h.clause_id.as_str()).collect();
    assert!(expanded.iter().all(|h| !hit_ids.contains(&h.clause_id.as_str())));

    std::fs::remove_dir_all(&tmp).ok();
}
