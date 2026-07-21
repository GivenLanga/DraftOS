//! The assembler's contract: a draft must resemble the precedent it was drawn
//! from. Every test here failed against the pre-precedent-led implementation.

mod common;

use common::*;

/// The headline guarantee. A precedent with 15 operative clauses must produce a
/// draft with 15 operative clauses — not a subset chosen by a hardcoded table.
#[test]
fn draft_keeps_every_clause_of_the_precedent() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();
    let doc = &draft.document;

    assert_eq!(
        doc.clauses.len(),
        15,
        "expected all 15 top-level clauses, got {:?}",
        doc.clauses.iter().map(|c| &c.heading).collect::<Vec<_>>()
    );

    // Specifically the clauses that fall outside any generic boilerplate
    // vocabulary — the commercially operative heart of the agreement.
    for heading in ["Appointment", "Service Levels", "Service Credits", "Duration"] {
        assert!(
            doc.clauses.iter().any(|c| c.heading == heading),
            "'{heading}' was dropped; headings: {:?}",
            doc.clauses.iter().map(|c| &c.heading).collect::<Vec<_>>()
        );
    }
    assert!(draft.report.missing.is_empty(), "{:?}", draft.report.missing);
}

/// Two clauses that canonicalise to the same label must both survive. The old
/// one-clause-per-type assembler kept whichever it retrieved first.
#[test]
fn clauses_sharing_a_label_both_survive() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();
    let headings: Vec<&str> = draft
        .document
        .clauses
        .iter()
        .map(|c| c.heading.as_str())
        .collect();
    assert!(headings.contains(&"Duration"), "{headings:?}");
    assert!(headings.contains(&"Breach and Termination"), "{headings:?}");
}

/// Sub-clauses stay sub-clauses instead of collapsing into a run-on paragraph.
#[test]
fn sub_clause_hierarchy_survives() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();

    let fees = draft
        .document
        .clauses
        .iter()
        .find(|c| c.heading == "Fees and Payment")
        .expect("fees clause");
    assert_eq!(fees.children.len(), 3, "5.1, 5.2 and 5.3 should be children");
    assert_eq!(fees.children[0].number, format!("{}.1", fees.number));

    // Each sub-clause keeps its own text rather than being merged.
    let first = fees.children[0].body[0].plain_text();
    assert!(first.contains("monthly in arrear"), "{first}");
    assert!(!first.contains("value-added tax"), "sub-clauses were merged: {first}");
}

/// Schedules the precedent carried must reach the document.
#[test]
fn schedules_are_carried_through() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();
    assert_eq!(
        draft.document.schedules.len(),
        2,
        "titles: {:?}",
        draft.document.schedules.iter().map(|s| &s.title).collect::<Vec<_>>()
    );
    // The precedent's own casing is preserved (house-style fidelity).
    assert!(draft.document.schedules[0]
        .title
        .to_lowercase()
        .starts_with("schedule"));
}

/// Definitions come through whole, and are not pruned by whether some other
/// surviving clause happens to mention them.
#[test]
fn definitions_survive_intact() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();
    let terms: Vec<&str> = draft
        .document
        .definitions
        .iter()
        .map(|d| d.term.as_str())
        .collect();
    for want in ["Business Day", "Services", "Service Credit"] {
        assert!(terms.contains(&want), "missing {want}; have {terms:?}");
    }
}

/// Renumbering must carry cross-references with it: the confidentiality clause
/// says "this clause 9 survives", and clause 9 must still be that clause.
#[test]
fn cross_references_follow_the_new_numbering() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();
    let doc = &draft.document;

    let conf = doc
        .clauses
        .iter()
        .find(|c| c.heading == "Confidentiality")
        .expect("confidentiality clause");
    let text = conf
        .children
        .iter()
        .flat_map(|c| c.body.iter().map(|b| b.plain_text()))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains(&format!("clause {}", conf.number)),
        "self-reference was not remapped: {text}"
    );

    // And the document as a whole validates, with no dangling references.
    let report = draftos_validate::validate(doc);
    assert!(
        !report.has_errors(),
        "validation errors: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

/// The whole point: no hardcoded structure. An instrument DraftOS has never
/// heard of drafts from its own precedent, with its own clause names and order.
#[test]
fn unknown_instrument_drafts_from_its_own_precedent() {
    let charter = parsed(
        "Time Charterparty - Vessel Meridian.docx",
        &[
            ("TIME CHARTERPARTY", Some(1)),
            ("entered into between Owners and Charterers", None),
            ("1. DESCRIPTION OF VESSEL", Some(1)),
            ("1.1 The Owners let and the Charterers hire the vessel described in the Schedule for the period stated.", None),
            ("2. PERIOD OF HIRE", Some(1)),
            ("2.1 The vessel shall be delivered to the Charterers at the port stated and redelivered on expiry of the period of hire.", None),
            ("3. LAYTIME AND DEMURRAGE", Some(1)),
            ("3.1 Laytime shall commence six hours after tender of notice of readiness, and demurrage shall accrue at the daily rate stated.", None),
            ("4. BUNKERS", Some(1)),
            ("4.1 The Charterers shall accept and pay for the bunkers on board at delivery at the price stated.", None),
            ("5. OFF-HIRE", Some(1)),
            ("5.1 In the event of loss of time from deficiency of crew or breakdown of machinery, hire shall cease until the vessel is again in an efficient state.", None),
            ("6. LIEN", Some(1)),
            ("6.1 The Owners shall have a lien upon all cargoes and sub-freights for any amounts due under this charter.", None),
            ("7. ARBITRATION", Some(1)),
            ("7.1 Any dispute arising out of this charter shall be referred to arbitration in London.", None),
        ],
    );
    let (bundle, embedder) = bundle_with("shipping", &[charter]);
    let draft =
        draftos_assemble::assemble(&spec("Time Charterparty"), &[bundle], &embedder).unwrap();
    let headings: Vec<&str> = draft
        .document
        .clauses
        .iter()
        .map(|c| c.heading.as_str())
        .collect();

    assert_eq!(headings.len(), 7, "{headings:?}");
    for want in ["Description of Vessel", "Laytime and Demurrage", "Off-Hire", "Lien"] {
        assert!(headings.contains(&want), "missing {want}; have {headings:?}");
    }
    // No checklist exists for a charterparty, so nothing is reported missing —
    // and critically, no generic NDA skeleton was imposed.
    assert!(draft.report.missing.is_empty(), "{:?}", draft.report.missing);
    assert!(!headings.contains(&"Definitions"), "invented a clause: {headings:?}");
}

/// The precedent's own clause order is preserved, not re-sorted into a
/// canonical order DraftOS made up.
#[test]
fn precedent_order_is_preserved() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let draft = draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder).unwrap();
    let headings: Vec<&str> = draft
        .document
        .clauses
        .iter()
        .map(|c| c.heading.as_str())
        .collect();
    let appointment = headings.iter().position(|h| *h == "Appointment").unwrap();
    let fees = headings.iter().position(|h| *h == "Fees and Payment").unwrap();
    let governing = headings.iter().position(|h| *h == "Governing Law").unwrap();
    assert!(appointment < fees && fees < governing, "{headings:?}");
}

/// A required clause the precedent genuinely lacks is filled from elsewhere and
/// reported — the checklist advises, it does not generate.
#[test]
fn checklist_gap_is_filled_from_another_precedent_and_reported() {
    let thin_nda = parsed(
        "Short NDA.docx",
        &[
            ("NON-DISCLOSURE AGREEMENT", Some(1)),
            ("entered into between the parties", None),
            ("1. CONFIDENTIALITY", Some(1)),
            ("1.1 Each party shall keep confidential all Confidential Information disclosed to it by the other party and shall not disclose it to any third party.", None),
            ("2. BREACH", Some(1)),
            ("2.1 Should either party commit a material breach, the aggrieved party may claim damages or specific performance.", None),
        ],
    );
    let (bundle, embedder) = bundle_with("precedents", &[thin_nda, service_agreement()]);
    let draft =
        draftos_assemble::assemble(&spec("Non-Disclosure Agreement"), &[bundle], &embedder).unwrap();

    // The NDA is the skeleton (it matches the contract type).
    let skeleton = draft.report.skeleton.as_ref().expect("a skeleton");
    assert_eq!(skeleton.file, "Short NDA.docx");

    // Governing Law is on the NDA checklist and absent from the precedent, so it
    // is filled from the other document and reported as such.
    let filled: Vec<&str> = draft
        .report
        .gap_filled
        .iter()
        .map(|(ct, _, _)| ct.as_str())
        .collect();
    assert!(filled.contains(&"Governing Law"), "{filled:?}");
    assert!(draft
        .document
        .clauses
        .iter()
        .any(|c| c.heading == "Governing Law"));

    // The precedent's own clauses are still all there.
    for heading in ["Confidentiality", "Breach"] {
        assert!(draft.document.clauses.iter().any(|c| c.heading == heading));
    }
}

/// Excluding a clause type removes it and says so, rather than silently.
#[test]
fn excluded_clause_types_are_removed_and_reported() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let mut s = spec("Service Agreement");
    s.exclude_clause_types = vec!["Intellectual Property".to_string()];
    let draft = draftos_assemble::assemble(&s, &[bundle], &embedder).unwrap();

    assert!(!draft
        .document
        .clauses
        .iter()
        .any(|c| c.heading == "Intellectual Property"));
    assert!(draft.report.excluded.contains(&"Intellectual Property".to_string()));
}

/// Drafting with nothing to draft from is an error, not a husk of a document.
#[test]
fn empty_corpus_is_an_error() {
    let (bundle, embedder) = bundle_with("empty", &[]);
    match draftos_assemble::assemble(&spec("Service Agreement"), &[bundle], &embedder) {
        Ok(d) => panic!("drafted {} clauses from nothing", d.document.clauses.len()),
        Err(e) => assert!(e.to_string().contains("no precedent"), "{e}"),
    }
}

/// Naming a precedent in the spec overrides the automatic choice.
#[test]
fn spec_can_name_the_skeleton_precedent() {
    let (bundle, embedder) = bundle_with("precedents", &[service_agreement()]);
    let mut s = spec("Non-Disclosure Agreement"); // deliberately the wrong type
    s.skeleton_precedent = Some("Alpha and Beta".to_string());
    let draft = draftos_assemble::assemble(&s, &[bundle], &embedder).unwrap();

    let skeleton = draft.report.skeleton.as_ref().unwrap();
    assert_eq!(skeleton.file, "Service Agreement - Alpha and Beta.docx");
    assert!(!skeleton.exact_type_match);
    // And the mismatch is surfaced rather than hidden.
    assert!(!draft.report.notes.is_empty());
}

/// Variables from the matter spec are substituted throughout, including into
/// sub-clauses.
#[test]
fn variables_are_substituted_in_sub_clauses() {
    let doc = parsed(
        "Template.docx",
        &[
            ("SERVICE AGREEMENT", Some(1)),
            ("between the parties", None),
            ("1. FEES", Some(1)),
            ("1.1 The Customer shall pay a monthly fee of {{monthly_fee}} to the Provider, payable in arrear on the last Business Day of each month.", None),
        ],
    );
    let (bundle, embedder) = bundle_with("precedents", &[doc]);
    let mut s = spec("Service Agreement");
    s.variables.insert("monthly_fee".into(), "R150 000".into());
    let draft = draftos_assemble::assemble(&s, &[bundle], &embedder).unwrap();

    let text = draft
        .document
        .walk_clauses()
        .iter()
        .flat_map(|c| c.body.iter().map(|b| b.plain_text()))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("R150 000"), "{text}");
    assert!(!text.contains("{{monthly_fee}}"), "{text}");
}
