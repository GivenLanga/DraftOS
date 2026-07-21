use draftos_core::{ClauseKind, ClauseMetadata, ExtractedClause, SourceManifest};
use draftos_index::SourceBundle;

fn clause(heading: &str, body: &str) -> ExtractedClause {
    ExtractedClause {
        kind: ClauseKind::Clause,
        number: None,
        heading: Some(heading.to_string()),
        term: None,
        body: body.to_string(),
        ooxml: Vec::new(),
        heading_ooxml: None,
        metadata: ClauseMetadata {
            clause_type: Some(heading.to_string()),
            ..Default::default()
        },
    }
}

fn unit_vec(dims: usize, hot: usize) -> Vec<f32> {
    let mut v = vec![0.0; dims];
    v[hot] = 1.0;
    v
}

fn manifest(dir: &std::path::Path) -> SourceManifest {
    SourceManifest {
        id: "test".into(),
        name: "test".into(),
        folder: dir.to_path_buf(),
        embed_model: "unit-test".into(),
        embed_dims: 8,
        created_at: draftos_core::now_utc(),
        schema_version: SourceManifest::CURRENT_SCHEMA,
    }
}

#[test]
fn upsert_search_reopen_and_delete_roundtrip() {
    let tmp = std::env::temp_dir().join(format!("draftos-test-{}", draftos_core::new_id()));
    let bundle_dir = tmp.join("bundle");

    {
        let mut bundle = SourceBundle::create(&bundle_dir, manifest(&tmp)).unwrap();
        bundle
            .upsert_document(
                "a.txt",
                "hash1",
                &[
                    clause("Termination", "Either party may terminate on notice."),
                    clause("Payment", "The purchase price is payable in cash."),
                ],
                &[unit_vec(8, 0), unit_vec(8, 1)],
            )
            .unwrap();

        let (docs, clauses) = bundle.stats().unwrap();
        assert_eq!((docs, clauses), (1, 2));

        // FTS finds the termination clause.
        let rowids = bundle.fts_search("terminate", 10).unwrap();
        let hits = bundle.hydrate(&rowids).unwrap();
        assert!(hits
            .iter()
            .any(|(_, h)| h.heading.as_deref() == Some("Termination")));

        // Vector search: query nearest to the "Termination" embedding.
        let rowids = bundle.vec_search(&unit_vec(8, 0), 1).unwrap();
        let hits = bundle.hydrate(&rowids).unwrap();
        assert_eq!(hits[0].1.heading.as_deref(), Some("Termination"));

        // Re-ingesting the same file replaces, not duplicates.
        bundle
            .upsert_document(
                "a.txt",
                "hash2",
                &[clause("Termination", "Updated termination wording here.")],
                &[unit_vec(8, 2)],
            )
            .unwrap();
        let (_, clauses) = bundle.stats().unwrap();
        assert_eq!(clauses, 1);
    }

    // Detach + re-attach: reopening the bundle keeps everything.
    {
        let mut bundle = SourceBundle::open(&bundle_dir).unwrap();
        let (docs, clauses) = bundle.stats().unwrap();
        assert_eq!((docs, clauses), (1, 1));

        bundle.mark_deleted("a.txt").unwrap();
        let (docs, _) = bundle.stats().unwrap();
        assert_eq!(docs, 0);
        assert!(bundle.fts_search("termination", 10).unwrap().is_empty());
    }

    std::fs::remove_dir_all(&tmp).ok();
}
