//! Contract-type, clause-type and jurisdiction labelling.
//!
//! The tables below are a *canonicalisation* layer, not the vocabulary. A pack's
//! own headings are the vocabulary: "Service Credits", "Laytime and Demurrage"
//! and "Appointment" become labels because the precedent used them, not because
//! someone added them here. The tables only exist so that headings meaning the
//! same thing ("Fees and Payment", "Consideration", "Remuneration") collapse to
//! one canonical name and can be matched across packs.
//!
//! Consequence for drafting: a clause is never unaddressable just because it
//! falls outside a fixed list. That was the bug this design replaces.

/// Synonym families → canonical clause-type label.
const CLAUSE_SYNONYMS: &[(&str, &[&str])] = &[
    ("Definitions", &["definition", "interpretation"]),
    ("Termination", &["termination", "terminate", "cancellation"]),
    ("Confidentiality", &["confidential", "non-disclosure", "secrecy"]),
    ("Payment", &["payment", "purchase price", "consideration", "fees", "remuneration"]),
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

/// Synonym families → canonical contract-type label. As above: a pack whose
/// agreements fall outside these still gets a label, derived from its own title.
const CONTRACT_SYNONYMS: &[(&str, &[&str])] = &[
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

const JURISDICTIONS: &[(&str, &[&str])] = &[
    ("South Africa", &["republic of south africa", "south africa", "laws of south africa", "popia"]),
    ("England and Wales", &["england and wales", "english law"]),
    ("United States", &["state of delaware", "state of new york", "state of california", "united states"]),
    ("Namibia", &["republic of namibia"]),
    ("Botswana", &["republic of botswana"]),
];

/// Whether `haystack` contains `needle` as whole words. Plain substring matching
/// is unusable here: "nda" occurs inside "Sunday", "spa" inside "spacing", and
/// "moi" inside "domicilium" — each of which silently mislabelled entire
/// documents. Prefix matching within a word is still allowed at the end of the
/// needle so stems like "indemnif" catch "indemnifies"/"indemnification".
fn contains_phrase(haystack: &str, needle: &str) -> bool {
    /// Lower-case, with every run of non-alphanumerics collapsed to one space
    /// and the whole thing space-padded. Separators stop mattering, so the
    /// keyword "service agreement" matches "service-agreement.docx".
    fn normalize(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push(' ');
        for c in s.chars() {
            if c.is_alphanumeric() {
                out.extend(c.to_lowercase());
            } else if !out.ends_with(' ') {
                out.push(' ');
            }
        }
        if !out.ends_with(' ') {
            out.push(' ');
        }
        out
    }

    let hay = normalize(haystack);
    let needle = normalize(needle);
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    // A needle of six characters or more may also match as a prefix, so the stem
    // "indemnif" catches "indemnification" and "terminate" catches "terminated".
    let allow_prefix = needle.len() >= 6;
    let padded = format!(" {needle}");

    let mut from = 0;
    while let Some(i) = hay[from..].find(&padded) {
        let end = from + i + padded.len();
        if hay[end..].starts_with(' ') || allow_prefix {
            return true;
        }
        from = from + i + 1;
    }
    false
}

/// Words that stay lower-case when a heading is normalised to a label.
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "in", "nor", "of", "on",
    "or", "the", "to", "up", "via", "with",
];

/// Contract type for a document. Filename and title page are the strong
/// signals; if neither matches a known family we derive a label from the
/// document's own title so unfamiliar packs are still addressable.
pub fn detect_contract_type(file_name: &str, full_text: &str) -> Option<String> {
    let name = file_name.to_ascii_lowercase();
    let head: String = full_text
        .chars()
        .take(2000)
        .collect::<String>()
        .to_ascii_lowercase();
    for (label, keywords) in CONTRACT_SYNONYMS {
        if keywords.iter().any(|k| contains_phrase(&name, k)) {
            return Some((*label).to_string());
        }
    }
    for (label, keywords) in CONTRACT_SYNONYMS {
        if keywords.iter().any(|k| contains_phrase(&head, k)) {
            return Some((*label).to_string());
        }
    }
    derive_contract_type(file_name, full_text)
}

/// Corpus-derived fallback: the first line that reads like a document title and
/// names an instrument ("SERVICE LEVEL AGREEMENT", "Charterparty").
fn derive_contract_type(file_name: &str, full_text: &str) -> Option<String> {
    const INSTRUMENTS: &[&str] = &[
        "agreement", "contract", "deed", "charter", "charterparty", "lease",
        "licence", "license", "memorandum", "mandate", "policy", "terms",
        "undertaking", "addendum", "amendment",
    ];
    let is_instrument = |s: &str| {
        let lc = s.to_ascii_lowercase();
        INSTRUMENTS.iter().any(|i| lc.contains(i))
    };
    // A bare instrument noun ("Agreement", "Contract") names nothing; it would
    // group every unrecognised document in the corpus under one useless label.
    let meaningful = |label: &str| {
        !label.is_empty()
            && !INSTRUMENTS
                .iter()
                .any(|i| label.eq_ignore_ascii_case(i))
    };

    for line in full_text.lines().take(40) {
        let line = line.trim();
        if line.is_empty() || line.len() > 90 {
            continue;
        }
        if is_instrument(line) && line.split_whitespace().count() <= 10 {
            let label = normalize_label(line);
            if meaningful(&label) {
                return Some(label);
            }
        }
    }
    // Last resort: the filename stem, when it names an instrument.
    let stem = file_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(file_name);
    let stem = stem.split(&['-', '_'][..]).next().unwrap_or(stem).trim();
    let label = normalize_label(stem);
    (is_instrument(stem) && meaningful(&label)).then_some(label)
}

/// The clause's label. Prefers a canonical family so the same concept matches
/// across packs; otherwise the precedent's own heading becomes the label.
/// Body text is consulted only for headingless clauses — matching on body text
/// misfires badly (a "Duration" clause mentioning "terminated" is not a
/// termination clause).
pub fn classify_clause_type(heading: &str, body: &str) -> Option<String> {
    let heading = heading.trim();
    if !heading.is_empty() {
        let heading_lc = heading.to_ascii_lowercase();
        for (label, keywords) in CLAUSE_SYNONYMS {
            if keywords.iter().any(|k| contains_phrase(&heading_lc, k)) {
                return Some((*label).to_string());
            }
        }
        // The pack's own vocabulary.
        let label = normalize_label(heading);
        if !label.is_empty() {
            return Some(label);
        }
    }
    let body_head: String = body
        .chars()
        .take(300)
        .collect::<String>()
        .to_ascii_lowercase();
    for (label, keywords) in CLAUSE_SYNONYMS {
        if keywords.iter().any(|k| contains_phrase(&body_head, k)) {
            return Some((*label).to_string());
        }
    }
    None
}

/// Canonical label for a heading: strip any leading number, drop trailing
/// punctuation, and title-case (leaving small words lower-case, as legal house
/// style does — "Fees and Payment", not "Fees And Payment").
pub fn normalize_label(heading: &str) -> String {
    let text = heading.trim().trim_matches(|c: char| c == '.' || c == ':' || c == ';');
    // Strip a leading clause number: "12.3 Notice of Breach" → "Notice of Breach".
    let text = text
        .split_once(char::is_whitespace)
        .filter(|(first, _)| {
            !first.is_empty()
                && first
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == ')' || c == '(')
        })
        .map(|(_, rest)| rest)
        .unwrap_or(text)
        .trim();

    let words: Vec<&str> = text.split_whitespace().collect();
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let lower = w.to_lowercase();
            if i > 0 && SMALL_WORDS.contains(&lower.as_str()) {
                lower
            } else if w.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) {
                // ALL CAPS → Title Case. Mixed case is left as the drafter wrote
                // it. Capitalise after a hyphen or slash too, as legal titles do
                // ("OFF-HIRE" → "Off-Hire", not "Off-hire").
                let mut out = String::with_capacity(lower.len());
                let mut capitalise = true;
                for ch in lower.chars() {
                    if capitalise && ch.is_alphabetic() {
                        out.extend(ch.to_uppercase());
                        capitalise = false;
                    } else {
                        out.push(ch);
                        if ch == '-' || ch == '/' {
                            capitalise = true;
                        }
                    }
                }
                out
            } else {
                (*w).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn detect_jurisdiction(full_text: &str) -> Option<String> {
    let text = full_text.to_ascii_lowercase();
    for (label, keywords) in JURISDICTIONS {
        if keywords.iter().any(|k| contains_phrase(&text, k)) {
            return Some((*label).to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_own_vocabulary_becomes_the_label() {
        // Not in any synonym family — the heading itself must survive as a label.
        assert_eq!(
            classify_clause_type("SERVICE CREDITS", "If the Provider fails…"),
            Some("Service Credits".to_string())
        );
        assert_eq!(
            classify_clause_type("Laytime and Demurrage", "…"),
            Some("Laytime and Demurrage".to_string())
        );
    }

    #[test]
    fn synonym_families_still_collapse() {
        assert_eq!(
            classify_clause_type("FEES AND PAYMENT", "…"),
            Some("Payment".to_string())
        );
        assert_eq!(
            classify_clause_type("Consideration", "…"),
            Some("Payment".to_string())
        );
    }

    #[test]
    fn heading_wins_over_misleading_body() {
        // The old body-first fallback labelled this "Termination" because the
        // body mentions "terminated"; it is a duration clause.
        let body = "This Agreement commences on the Commencement Date and \
                    endures until terminated in accordance with clause 11.";
        assert_eq!(
            classify_clause_type("DURATION", body),
            Some("Duration".to_string())
        );
    }

    #[test]
    fn small_words_stay_lower_case() {
        assert_eq!(normalize_label("LIMITATION OF LIABILITY"), "Limitation of Liability");
        assert_eq!(normalize_label("12.3 BREACH AND TERMINATION"), "Breach and Termination");
    }

    #[test]
    fn short_keywords_do_not_match_inside_words() {
        // Each of these silently mislabelled whole documents under substring
        // matching: "nda" in "Sunday", "spa" in "spacing", "moi" in "domicilium".
        let sunday = "any day other than a Saturday, Sunday or public holiday";
        assert_eq!(detect_contract_type("agreement.txt", sunday), None);
        assert!(!contains_phrase("line spacing rules", "spa"));
        assert!(!contains_phrase("their domicilium citandi", "moi"));
        // …but the real acronym still matches as a word.
        assert!(contains_phrase("this nda is entered into", "nda"));
    }

    #[test]
    fn separators_do_not_defeat_matching() {
        assert_eq!(
            detect_contract_type("service-agreement.txt", ""),
            Some("Service Agreement".to_string())
        );
        assert_eq!(
            detect_contract_type("Non_Disclosure_Agreement.docx", ""),
            Some("Non-Disclosure Agreement".to_string())
        );
    }

    #[test]
    fn stems_still_match_inflections() {
        assert_eq!(classify_clause_type("", "the Seller indemnifies the Buyer"), Some("Indemnity".into()));
        assert_eq!(classify_clause_type("", "this agreement is terminated on notice"), Some("Termination".into()));
    }

    #[test]
    fn unknown_instrument_still_gets_a_contract_type() {
        let text = "TIME CHARTERPARTY\n\nentered into between…";
        assert_eq!(
            detect_contract_type("charter-2024.docx", text),
            Some("Time Charterparty".to_string())
        );
    }
}
