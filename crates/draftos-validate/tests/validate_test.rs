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

#[test]
fn dangling_cross_reference_and_undefined_term_warn() {
    let mut doc = base_doc();
    doc.clauses[0].cross_refs = vec!["9".into()];
    doc.clauses[0].defined_terms_used = vec!["Effective Date".into()];
    let report = validate(&doc);
    assert!(!report.has_errors());
    let codes: Vec<_> = report
        .warnings()
        .map(|f| (f.code, f.severity == Severity::Warning))
        .collect();
    assert!(codes.contains(&("dangling-cross-reference", true)));
    assert!(codes.contains(&("undefined-term", true)));
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
