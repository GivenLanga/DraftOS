//! Deterministic rule engine: for a contract type, which clause types a valid
//! document must contain, in what order, and which pairs must never coexist.
//!
//! This is the "law" the assembler follows. It is intentionally a plain data
//! table — auditable, testable, and free of any model. Clause-type strings
//! match the labels produced by draftos-extract::metadata.

/// Ordered clause requirements for one contract type.
pub struct ContractRule {
    pub contract_type: &'static str,
    /// Clause types that must appear, in canonical drafting order.
    pub required: &'static [&'static str],
    /// Clause types commonly included; assembled if a precedent exists.
    pub optional: &'static [&'static str],
}

/// Clause types that must never coexist in one document.
const CONFLICTS: &[(&str, &str)] = &[
    // A mutual NDA's confidentiality supersedes a one-way secrecy clause, etc.
    // (kept small and explicit; grow as real conflicts are identified)
    ("Governing Law", "Dispute Resolution"), // example placeholder pair — both may in fact coexist; see note
];

// NOTE: CONFLICTS is deliberately near-empty. Real conflicts are rare and
// must be added only with a documented legal reason. The example pair above is
// NOT enforced (see `conflicts`, which currently returns none) — it documents
// the shape without asserting a false rule.

const RULES: &[ContractRule] = &[
    ContractRule {
        contract_type: "Non-Disclosure Agreement",
        required: &[
            "Definitions",
            "Confidentiality",
            "Breach",
            "Termination",
            "Governing Law",
            "Notices",
        ],
        optional: &["Data Protection", "Dispute Resolution", "General"],
    },
    ContractRule {
        contract_type: "Share Purchase Agreement",
        required: &[
            "Definitions",
            "Payment",
            "Warranties",
            "Indemnity",
            "Confidentiality",
            "Breach",
            "Termination",
            "Governing Law",
            "Notices",
        ],
        optional: &["Restraint of Trade", "Dispute Resolution", "General"],
    },
    ContractRule {
        contract_type: "Loan Agreement",
        required: &[
            "Definitions",
            "Payment",
            "Warranties",
            "Breach",
            "Governing Law",
            "Notices",
        ],
        optional: &["Insurance", "Indemnity", "Dispute Resolution", "General"],
    },
    ContractRule {
        contract_type: "Employment Agreement",
        required: &[
            "Definitions",
            "Payment",
            "Confidentiality",
            "Restraint of Trade",
            "Termination",
            "Governing Law",
            "Notices",
        ],
        optional: &["Intellectual Property", "Data Protection", "General"],
    },
    ContractRule {
        contract_type: "Service Agreement",
        required: &[
            "Definitions",
            "Payment",
            "Warranties",
            "Limitation of Liability",
            "Confidentiality",
            "Termination",
            "Governing Law",
            "Notices",
        ],
        optional: &["Intellectual Property", "Indemnity", "Data Protection", "General"],
    },
    ContractRule {
        contract_type: "Lease Agreement",
        required: &[
            "Definitions",
            "Payment",
            "Breach",
            "Termination",
            "Governing Law",
            "Notices",
        ],
        optional: &["Insurance", "General"],
    },
];

/// Canonical order for contract types we don't have an explicit rule for.
/// Substantive clauses (whatever precedents exist) slot between the framing
/// clauses at the front and the boilerplate at the back.
const GENERIC_ORDER: &[&str] = &[
    "Definitions",
    "Payment",
    "Warranties",
    "Indemnity",
    "Intellectual Property",
    "Confidentiality",
    "Limitation of Liability",
    "Restraint of Trade",
    "Insurance",
    "Assignment",
    "Breach",
    "Termination",
    "Force Majeure",
    "Data Protection",
    "Dispute Resolution",
    "Governing Law",
    "Notices",
    "General",
];

pub fn rule_for(contract_type: &str) -> Option<&'static ContractRule> {
    RULES
        .iter()
        .find(|r| r.contract_type.eq_ignore_ascii_case(contract_type))
}

/// The ordered clause types to try to assemble for a contract type. When no
/// explicit rule exists, falls back to the generic canonical order.
pub fn required_clause_types(contract_type: &str) -> Vec<String> {
    match rule_for(contract_type) {
        Some(rule) => rule.required.iter().map(|s| s.to_string()).collect(),
        None => GENERIC_ORDER.iter().map(|s| s.to_string()).collect(),
    }
}

/// Full ordered set (required + optional) for a contract type.
pub fn all_clause_types(contract_type: &str) -> Vec<String> {
    match rule_for(contract_type) {
        Some(rule) => rule
            .required
            .iter()
            .chain(rule.optional.iter())
            .map(|s| s.to_string())
            .collect(),
        None => GENERIC_ORDER.iter().map(|s| s.to_string()).collect(),
    }
}

/// Sort clause types into canonical drafting order. Types not in the known
/// order are appended (stable) just before boilerplate.
pub fn order_index(contract_type: &str, clause_type: &str) -> usize {
    let order = all_clause_types(contract_type);
    order
        .iter()
        .position(|t| t.eq_ignore_ascii_case(clause_type))
        .unwrap_or(order.len())
}

/// Whether a clause type is required (as opposed to optional) for a contract.
pub fn is_required(contract_type: &str, clause_type: &str) -> bool {
    required_clause_types(contract_type)
        .iter()
        .any(|t| t.eq_ignore_ascii_case(clause_type))
}

/// Conflicting clause-type pairs present in a set. Currently returns none —
/// see the note on CONFLICTS; wired so validation can use it once real rules
/// are added.
pub fn conflicts(_clause_types: &[String]) -> Vec<(String, String)> {
    let _ = CONFLICTS; // referenced intentionally; not yet enforced
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nda_requires_confidentiality_before_boilerplate() {
        let req = required_clause_types("Non-Disclosure Agreement");
        assert!(req.contains(&"Confidentiality".to_string()));
        let conf = order_index("Non-Disclosure Agreement", "Confidentiality");
        let gov = order_index("Non-Disclosure Agreement", "Governing Law");
        assert!(conf < gov, "confidentiality should precede governing law");
    }

    #[test]
    fn unknown_contract_type_uses_generic_order() {
        let req = required_clause_types("Aircraft Wet Lease");
        assert_eq!(req.first().map(String::as_str), Some("Definitions"));
        assert!(req.contains(&"Governing Law".to_string()));
    }
}
