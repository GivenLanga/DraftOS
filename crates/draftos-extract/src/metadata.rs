//! Keyword heuristics for contract type, clause type, and jurisdiction.
//! Deliberately boring and auditable. Expandable tables, not clever models.

const CONTRACT_TYPES: &[(&str, &[&str])] = &[
    ("Share Purchase Agreement", &["share purchase", "sale of shares", "spa"]),
    ("Non-Disclosure Agreement", &["non-disclosure", "nondisclosure", "confidentiality agreement", "nda"]),
    ("Loan Agreement", &["loan agreement", "facility agreement", "credit agreement"]),
    ("Lease Agreement", &["lease agreement", "agreement of lease", "rental agreement"]),
    ("Employment Agreement", &["employment agreement", "contract of employment", "employment contract"]),
    ("Service Agreement", &["services agreement", "service agreement", "master services", "consulting agreement"]),
    ("Shareholders Agreement", &["shareholders agreement", "shareholders' agreement"]),
    ("Sale Agreement", &["sale agreement", "purchase agreement", "sale of business"]),
    ("Memorandum of Incorporation", &["memorandum of incorporation", "moi"]),
    ("Joint Venture Agreement", &["joint venture"]),
    ("Supply Agreement", &["supply agreement"]),
    ("Licence Agreement", &["licence agreement", "license agreement", "licensing agreement"]),
];

const CLAUSE_TYPES: &[(&str, &[&str])] = &[
    ("Definitions", &["definition", "interpretation"]),
    ("Termination", &["termination", "terminate", "cancellation"]),
    ("Confidentiality", &["confidential", "non-disclosure", "secrecy"]),
    ("Payment", &["payment", "purchase price", "consideration", "fees", "remuneration", "interest rate"]),
    ("Intellectual Property", &["intellectual property", "copyright", "trade mark", "trademark", "patent"]),
    ("Indemnity", &["indemnif", "indemnity", "hold harmless"]),
    ("Warranties", &["warrant", "representation"]),
    ("Force Majeure", &["force majeure", "act of god", "vis major", "casus fortuitus"]),
    ("Dispute Resolution", &["dispute", "arbitration", "mediation"]),
    ("Governing Law", &["governing law", "governed by", "applicable law"]),
    ("Notices", &["notice", "domicilium", "domicilia"]),
    ("Breach", &["breach", "default", "remedies"]),
    ("Restraint of Trade", &["restraint", "non-compete", "non-solicit"]),
    ("Assignment", &["assignment", "cession", "delegation"]),
    ("Limitation of Liability", &["limitation of liability", "liability cap", "exclusion of liability"]),
    ("Data Protection", &["popia", "data protection", "personal information", "gdpr", "privacy"]),
    ("Insurance", &["insurance", "insure"]),
    ("General", &["whole agreement", "entire agreement", "severability", "counterparts", "variation"]),
];

const JURISDICTIONS: &[(&str, &[&str])] = &[
    ("South Africa", &["republic of south africa", "south africa", "laws of south africa", "popia"]),
    ("England and Wales", &["england and wales", "english law"]),
    ("United States", &["state of delaware", "state of new york", "state of california", "united states"]),
    ("Namibia", &["republic of namibia"]),
    ("Botswana", &["republic of botswana"]),
];

pub fn detect_contract_type(file_name: &str, full_text: &str) -> Option<String> {
    // Filename first (strongest signal), then the first ~2000 chars (title page).
    let name = file_name.to_ascii_lowercase();
    let head: String = full_text.chars().take(2000).collect::<String>().to_ascii_lowercase();
    for (label, keywords) in CONTRACT_TYPES {
        if keywords.iter().any(|k| name.contains(k)) {
            return Some((*label).to_string());
        }
    }
    for (label, keywords) in CONTRACT_TYPES {
        if keywords.iter().any(|k| head.contains(k)) {
            return Some((*label).to_string());
        }
    }
    None
}

pub fn classify_clause_type(heading: &str, body: &str) -> Option<String> {
    let heading_lc = heading.to_ascii_lowercase();
    for (label, keywords) in CLAUSE_TYPES {
        if keywords.iter().any(|k| heading_lc.contains(k)) {
            return Some((*label).to_string());
        }
    }
    // Fall back to the opening of the body — weaker signal, first match wins.
    let body_head: String = body.chars().take(300).collect::<String>().to_ascii_lowercase();
    for (label, keywords) in CLAUSE_TYPES {
        if keywords.iter().any(|k| body_head.contains(k)) {
            return Some((*label).to_string());
        }
    }
    None
}

pub fn detect_jurisdiction(full_text: &str) -> Option<String> {
    let text = full_text.to_ascii_lowercase();
    for (label, keywords) in JURISDICTIONS {
        if keywords.iter().any(|k| text.contains(k)) {
            return Some((*label).to_string());
        }
    }
    None
}
