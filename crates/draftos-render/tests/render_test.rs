use draftos_core::lir::*;
use std::io::Read;

fn sample_doc() -> LirDocument {
    let mut doc = LirDocument::new(LirMeta {
        title: "Sample NDA".into(),
        contract_type: "Non-Disclosure Agreement".into(),
        jurisdiction: Some("South Africa".into()),
        language: "English".into(),
        matter_id: None,
        created_at: draftos_core::now_utc(),
    });
    doc.parties.push(Party {
        id: "d".into(),
        name: "Alpha (Pty) Ltd".into(),
        role: "Disclosing Party".into(),
        entity_type: None,
        reg_no: Some("2020/1/07".into()),
        address: None,
    });
    doc.definitions.push(Definition {
        term: "Confidential Information".into(),
        body: vec![Block::para("\"Confidential Information\" means all information disclosed.")],
        provenance: Provenance::default(),
        ooxml: Vec::new(),
    });
    doc.clauses.push(LirClause {
        id: draftos_core::new_id(),
        number: "1".into(),
        heading: "Confidentiality".into(),
        source_ooxml: Vec::new(),
        heading_ooxml: None,
        body: vec![
            Block::Paragraph {
                runs: vec![
                    Run::normal("The "),
                    Run::bold("Receiving Party"),
                    Run::normal(" shall keep Confidential Information secret & safe."),
                ],
            },
            Block::List {
                ordered: false,
                items: vec![
                    vec![Block::para("no disclosure to third parties")],
                    vec![Block::para("use only for the permitted purpose")],
                ],
            },
        ],
        children: Vec::new(),
        cross_refs: Vec::new(),
        defined_terms_used: vec!["Confidential Information".into()],
        provenance: Provenance::default(),
    });
    doc.execution.blocks.push(SignatureBlock {
        party_id: "d".into(),
        party_name: "Alpha (Pty) Ltd".into(),
        signatory_line: "for and on behalf of Alpha (Pty) Ltd".into(),
    });
    doc
}

#[test]
fn markdown_renders_structure() {
    let md = draftos_render::render_markdown(&sample_doc());
    assert!(md.contains("# Sample NDA"));
    assert!(md.contains("## 1. Confidentiality"));
    assert!(md.contains("**Receiving Party**"));
    assert!(md.contains("- no disclosure to third parties"));
    assert!(md.contains("_____________________________"));
}

#[test]
fn docx_is_a_valid_ooxml_package_with_escaped_text() {
    let bytes = draftos_render::render_docx(&sample_doc()).unwrap();
    assert_eq!(&bytes[..2], b"PK", "docx must be a zip");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(names.contains(&"[Content_Types].xml".to_string()));
    assert!(names.contains(&"word/document.xml".to_string()));

    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains("Sample NDA"));
    // The '&' in the clause body must be escaped, not raw.
    assert!(xml.contains("secret &amp; safe"));
    assert!(!xml.contains("secret & safe"));
    assert!(xml.contains(r#"<w:pStyle w:val="Heading1"/>"#));
}
