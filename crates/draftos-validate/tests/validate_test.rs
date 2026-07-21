use draftos_core::lir::*;
use draftos_validate::{validate, Severity};

fn base_doc() -> LirDocument {
    let mut doc = LirDocument::new(LirMeta {
        title: "Test Agreement".into(),
        contract_type: "Service Agreement".into(),
        jurisdiction: Some("South Africa".into()),
        language: "English".into(),
        matter_id: None,
        created_at: draftos_core::now_utc(),
    });
    doc.parties.push(Party {
        id: "a".into(),
        name: "Alpha (Pty) Ltd".into(),
        role: "Client".into(),
        entity_type: None,
        reg_no: None,
        address: None,
    });
    doc.execution.blocks.push(SignatureBlock {
        party_id: "a".into(),
        party_name: "Alpha (Pty) Ltd".into(),
        signatory_line: "for and on behalf of Alpha (Pty) Ltd".into(),
    });
    doc.clauses.push(clause("1", "Payment", "The fee is payable monthly."));
    doc
}

fn clause(number: &str, heading: &str, body: &str) -> LirClause {
    LirClause {
        id: draftos_core::new_id(),
        number: number.into(),
        heading: heading.into(),
        body: vec![Block::para(body)],
        children: Vec::new(),
        cross_refs: Vec::new(),
        defined_terms_used: Vec::new(),
        provenance: Provenance::default(),
        source_ooxml: Vec::new(),
        heading_ooxml: None,
    }
}

#[test]
fn clean_document_passes() {
    let report = validate(&base_doc());
    assert!(!report.has_errors(), "findings: {:?}", report.findings);
}

#[test]
fn unresolved_variable_is_an_error() {
    let mut doc = base_doc();
    doc.clauses[0].body = vec![Block::para("The fee is {{monthly_fee}} per month.")];
    let report = validate(&doc);
    assert!(report
        .errors()
        .any(|f| f.code == "unresolved-variable" && f.message.contains("monthly_fee")));
}

#[test]
fn non_dense_numbering_is_an_error() {
    let mut doc = base_doc();
    doc.clauses.push(clause("5", "Termination", "Either party may terminate."));
    let report = validate(&doc);
    assert!(report.errors().any(|f| f.code == "numbering"));
}

/// A reference to a clause that does not exist is a legal defect, not a
/// cosmetic one: assembly rewrites references to follow the new numbering, so
/// anything still dangling means the document says something untrue.
#[test]
fn dangling_cross_reference_is_an_error() {
    let mut doc = base_doc();
    doc.clauses[0].cross_refs = vec!["9".into()];
    let report = validate(&doc);
    assert!(report.errors().any(|f| f.code == "dangling-cross-reference"));
}

#[test]
fn undefined_term_warns() {
    let mut doc = base_doc();
    doc.clauses[0].defined_terms_used = vec!["Effective Date".into()];
    let report = validate(&doc);
    assert!(!report.has_errors(), "findings: {:?}", report.findings);
    assert!(report
        .warnings()
        .any(|f| f.code == "undefined-term" && f.severity == Severity::Warning));
}

/// Sub-clause numbers extend their parent's and restart at 1 per level.
#[test]
fn hierarchical_numbering_is_checked() {
    let mut doc = base_doc();
    let mut parent = clause("2", "Termination", "");
    parent.children = vec![
        clause("2.1", "", "Either party may terminate on notice."),
        clause("2.2", "", "Termination does not affect accrued rights."),
    ];
    doc.clauses.push(parent);
    let report = validate(&doc);
    assert!(!report.has_errors(), "findings: {:?}", report.findings);

    // A mis-numbered child is caught.
    doc.clauses[1].children[1].number = "2.5".into();
    assert!(validate(&doc).errors().any(|f| f.code == "numbering"));
}

/// A cross-reference to a sub-clause resolves against sub-clause numbers, not
/// just top-level ones.
#[test]
fn cross_reference_to_a_sub_clause_resolves() {
    let mut doc = base_doc();
    let mut parent = clause("2", "Termination", "");
    parent.children = vec![clause("2.1", "", "Either party may terminate.")];
    doc.clauses.push(parent);
    doc.clauses[0].cross_refs = vec!["2.1".into()];
    let report = validate(&doc);
    assert!(!report.has_errors(), "findings: {:?}", report.findings);
}

/// A parent clause whose text lives in its sub-clauses is not "empty".
#[test]
fn heading_only_parent_is_not_empty() {
    let mut doc = base_doc();
    let mut parent = clause("2", "Termination", "");
    parent.children = vec![clause("2.1", "", "Either party may terminate.")];
    doc.clauses.push(parent);
    assert!(!validate(&doc).errors().any(|f| f.code == "empty-clause"));
}

/// The most damaging drafting error there is: a precedent's parties surviving
/// into a new deal's document.
#[test]
fn a_company_that_is_not_a_party_is_flagged() {
    let mut doc = base_doc();
    doc.clauses[0].body = vec![Block::para(
        "The fee is payable by Beta Holdings Ltd on the first Business Day of each month.",
    )];
    let report = validate(&doc);
    assert!(
        report
            .warnings()
            .any(|f| f.code == "foreign-party-name" && f.message.contains("Beta Holdings Ltd")),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn the_documents_own_parties_are_not_flagged() {
    let mut doc = base_doc();
    doc.clauses[0].body = vec![Block::para(
        "The fee is payable by Alpha (Pty) Ltd on the first Business Day of each month.",
    )];
    let report = validate(&doc);
    assert!(
        !report.warnings().any(|f| f.code == "foreign-party-name"),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn missing_parties_and_execution_are_errors() {
    let mut doc = base_doc();
    doc.parties.clear();
    doc.execution.blocks.clear();
    let report = validate(&doc);
    assert!(report.errors().any(|f| f.code == "no-parties"));
    assert!(report.errors().any(|f| f.code == "no-execution"));
}
