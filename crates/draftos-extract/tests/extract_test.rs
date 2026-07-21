use draftos_core::{ClauseKind, Paragraph, ParsedDocument};

fn doc(paras: &[(&str, Option<u8>)]) -> ParsedDocument {
    ParsedDocument {
        file_name: "NDA Agreement.txt".to_string(),
        paragraphs: paras
            .iter()
            .map(|(t, h)| Paragraph {
                text: t.to_string(),
                heading_level: *h,
            })
            .collect(),
    }
}

#[test]
fn splits_numbered_and_allcaps_headings_into_clauses() {
    let d = doc(&[
        ("NON-DISCLOSURE AGREEMENT", None),
        ("entered into between Acme (Pty) Ltd and Beta (Pty) Ltd, together the parties hereto.", None),
        ("2. CONFIDENTIALITY OBLIGATIONS", None),
        ("The Receiving Party shall keep all Confidential Information strictly confidential at all times.", None),
        ("3. TERMINATION", None),
        ("Either party may terminate this Agreement on thirty days written notice to the other party.", None),
    ]);
    let clauses = draftos_extract::extract(&d, "nda.txt");

    let headings: Vec<_> = clauses.iter().filter_map(|c| c.heading.clone()).collect();
    assert!(headings.iter().any(|h| h.contains("CONFIDENTIALITY")));
    assert!(headings.iter().any(|h| h.contains("TERMINATION")));

    let term = clauses
        .iter()
        .find(|c| c.heading.as_deref() == Some("TERMINATION"))
        .unwrap();
    assert_eq!(term.kind, ClauseKind::Clause);
    assert_eq!(term.number.as_deref(), Some("3"));
    assert_eq!(term.metadata.clause_type.as_deref(), Some("Termination"));
    assert_eq!(
        term.metadata.contract_type.as_deref(),
        Some("Non-Disclosure Agreement")
    );
}

#[test]
fn harvests_definitions_as_standalone_objects() {
    let d = doc(&[
        ("1. DEFINITIONS", None),
        (
            "\"Effective Date\" means the date of signature of this Agreement by the party signing last.",
            None,
        ),
    ]);
    let clauses = draftos_extract::extract(&d, "nda.txt");
    let def = clauses
        .iter()
        .find(|c| c.kind == ClauseKind::Definition)
        .expect("definition extracted");
    assert_eq!(def.term.as_deref(), Some("Effective Date"));
    assert!(def.body.contains("date of signature"));
}

#[test]
fn detects_schedules_and_jurisdiction() {
    let d = doc(&[
        ("7. GOVERNING LAW", None),
        (
            "This Agreement is governed by the laws of the Republic of South Africa in all respects.",
            None,
        ),
        ("Schedule 1", None),
        ("Conditions precedent to be fulfilled by the Borrower before the Advance Date occurs.", None),
    ]);
    let clauses = draftos_extract::extract(&d, "loan.txt");
    assert!(clauses.iter().any(|c| c.kind == ClauseKind::Schedule));
    assert!(clauses
        .iter()
        .all(|c| c.metadata.jurisdiction.as_deref() == Some("South Africa")));
}
