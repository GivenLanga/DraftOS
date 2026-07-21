//! Shared fixtures: build a real source bundle from a precedent and run the
//! genuine extract → index → retrieve → assemble path over it. Nothing is
//! stubbed, so these tests fail if any stage in the chain regresses.

use draftos_core::{MatterSpec, Paragraph, ParsedDocument, PartyInput, SourceManifest};
use draftos_embed::{EmbeddingProvider, HashEmbedder};
use draftos_index::SourceBundle;

/// A precedent document from `(text, heading_level)` pairs.
pub fn parsed(file_name: &str, lines: &[(&str, Option<u8>)]) -> ParsedDocument {
    ParsedDocument {
        file_name: file_name.to_string(),
        paragraphs: lines
            .iter()
            .map(|(text, level)| Paragraph {
                text: (*text).to_string(),
                heading_level: *level,
                ooxml: None,
            })
            .collect(),
    }
}

/// A source bundle containing the given precedents, ingested for real.
pub fn bundle_with(name: &str, docs: &[ParsedDocument]) -> (SourceBundle, HashEmbedder) {
    let tmp = std::env::temp_dir().join(format!("draftos-test-{}", draftos_core::new_id()));
    let embedder = HashEmbedder::new(HashEmbedder::default_dims());
    let manifest = SourceManifest {
        id: draftos_core::new_id(),
        name: name.to_string(),
        folder: tmp.clone(),
        embed_model: embedder.id(),
        embed_dims: embedder.dims(),
        created_at: draftos_core::now_utc(),
        schema_version: SourceManifest::CURRENT_SCHEMA,
    };
    let mut bundle = SourceBundle::create(&tmp.join("bundle"), manifest).unwrap();

    for doc in docs {
        let extracted = draftos_extract::extract(doc, &doc.file_name);
        let texts: Vec<String> = extracted
            .iter()
            .map(|c| format!("{} {}", c.heading.clone().unwrap_or_default(), c.body))
            .collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let embeddings = embedder.embed(&refs).unwrap();
        let meta = draftos_index::DocumentMeta {
            contract_type: extracted.iter().find_map(|c| c.metadata.contract_type.clone()),
            jurisdiction: extracted.iter().find_map(|c| c.metadata.jurisdiction.clone()),
            title: doc.paragraphs.first().map(|p| draftos_extract::normalize_label(&p.text)),
        };
        bundle
            .upsert_document(&doc.file_name, "hash", &meta, &extracted, &embeddings)
            .unwrap();
    }
    (bundle, embedder)
}

pub fn spec(contract_type: &str) -> MatterSpec {
    MatterSpec {
        matter_id: draftos_core::new_id(),
        title: format!("{contract_type} - Gamma and Delta"),
        contract_type: contract_type.to_string(),
        jurisdiction: Some("South Africa".to_string()),
        language: "English".to_string(),
        parties: vec![
            PartyInput {
                id: "p1".into(),
                name: "Gamma (Pty) Ltd".into(),
                role: "Customer".into(),
                entity_type: None,
                reg_no: None,
                address: None,
            },
            PartyInput {
                id: "p2".into(),
                name: "Delta Services (Pty) Ltd".into(),
                role: "Provider".into(),
                entity_type: None,
                reg_no: None,
                address: None,
            },
        ],
        variables: Default::default(),
        source_scope: Vec::new(),
        skeleton_precedent: None,
        approved_only: false,
        include_clause_types: Vec::new(),
        exclude_clause_types: Vec::new(),
    }
}

/// An ordinary commercial precedent: numbered clauses, sub-clauses, a
/// definitions clause, internal cross-references and two schedules.
pub fn service_agreement() -> ParsedDocument {
    parsed(
        "Service Agreement - Alpha and Beta.docx",
        &[
        ("SERVICE LEVEL AGREEMENT", Some(1)),
        ("entered into between Alpha (Pty) Ltd and Beta Holdings Ltd", None),
        ("1. DEFINITIONS AND INTERPRETATION", Some(1)),
        ("1.1 In this Agreement, unless inconsistent with the context:", None),
        ("\"Business Day\" means any day other than a Saturday, Sunday or public holiday in the Republic of South Africa.", None),
        ("\"Services\" means the services described in Schedule 1.", None),
        ("\"Service Credit\" means the rebate calculated in accordance with clause 6.", None),
        ("2. APPOINTMENT", Some(1)),
        ("2.1 The Customer appoints the Provider to render the Services with effect from the Commencement Date, and the Provider accepts the appointment on the terms set out in this Agreement.", None),
        ("2.2 The appointment is not exclusive and the Customer may appoint third parties to render comparable services.", None),
        ("3. DURATION", Some(1)),
        ("3.1 This Agreement commences on the Commencement Date and endures for an initial period of 24 months, whereafter it continues indefinitely until terminated in accordance with clause 11.", None),
        ("4. SERVICE LEVELS", Some(1)),
        ("4.1 The Provider shall render the Services in accordance with the service levels set out in Schedule 2.", None),
        ("4.2 The Provider shall measure and report on its performance against the service levels monthly, within five Business Days of month end.", None),
        ("5. FEES AND PAYMENT", Some(1)),
        ("5.1 The Customer shall pay the Provider the fees set out in Schedule 1, monthly in arrear.", None),
        ("5.2 All amounts are exclusive of value-added tax, which shall be added at the prevailing rate.", None),
        ("5.3 Invoices are payable within 30 days of date of invoice, without deduction or set-off.", None),
        ("6. SERVICE CREDITS", Some(1)),
        ("6.1 If the Provider fails to meet a service level, the Customer is entitled to a Service Credit calculated in accordance with Schedule 2.", None),
        ("6.2 Service Credits are the Customer's sole financial remedy for a failure to meet a service level.", None),
        ("7. WARRANTIES", Some(1)),
        ("7.1 The Provider warrants that it has the skill, capacity and resources to render the Services and that it will do so with reasonable care and skill.", None),
        ("8. LIMITATION OF LIABILITY", Some(1)),
        ("8.1 Neither party is liable to the other for any indirect or consequential loss, including loss of profit, howsoever arising.", None),
        ("8.2 Each party's aggregate liability under this Agreement is limited to the fees paid in the 12 months preceding the claim.", None),
        ("9. CONFIDENTIALITY", Some(1)),
        ("9.1 Each party shall keep confidential all Confidential Information of the other party and shall not disclose it to any third party without prior written consent.", None),
        ("9.2 This clause 9 survives termination of this Agreement for a period of five years.", None),
        ("10. INTELLECTUAL PROPERTY", Some(1)),
        ("10.1 All intellectual property developed by the Provider in rendering the Services vests in the Customer on creation.", None),
        ("11. BREACH AND TERMINATION", Some(1)),
        ("11.1 Should either party commit a material breach and fail to remedy it within 14 days of written notice, the aggrieved party may cancel this Agreement.", None),
        ("11.2 Either party may terminate this Agreement on 90 days written notice to the other.", None),
        ("12. DISPUTE RESOLUTION", Some(1)),
        ("12.1 Any dispute arising out of this Agreement shall be referred to arbitration in Johannesburg under the rules of AFSA.", None),
        ("13. GOVERNING LAW", Some(1)),
        ("13.1 This Agreement is governed by and construed in accordance with the laws of the Republic of South Africa.", None),
        ("14. NOTICES", Some(1)),
        ("14.1 The parties choose the addresses set out in Schedule 1 as their domicilium citandi et executandi.", None),
        ("15. GENERAL", Some(1)),
        ("15.1 This Agreement constitutes the whole agreement between the parties and no variation is of force unless reduced to writing and signed by both parties.", None),
        ("SCHEDULE 1", Some(1)),
        ("The Services, the fees payable and the parties' addresses are set out in this Schedule 1 as agreed between the parties in writing.", None),
        ("SCHEDULE 2", Some(1)),
        ("The service levels and the Service Credit calculation applicable to the Services are set out in this Schedule 2.", None),
        ],
    )
}
