//! Deterministic rule engine: a **checklist** of clause types a valid document
//! of a given type ought to contain, and where a missing one belongs.
//!
//! This crate does **not** decide a document's structure. That comes from the
//! precedents in the knowledge sources (see draftos-assemble) — otherwise
//! DraftOS would impose the same hardcoded skeleton on every legal corpus it is
//! pointed at, which is exactly the defect this design replaced. The tables
//! below are advisory: they let the assembler *report* that a drafted document
//! has no governing-law clause, and decide where to insert one if a precedent
//! for it exists. An unknown contract type is not an error — it simply has no
//! checklist, and the precedent's own structure stands unaudited.
//!
//! Clause-type strings match the canonical labels produced by
//! draftos-extract::metadata; a pack's own vocabulary passes through unaudited.

/// Ordered clause requirements for one contract type.
pub struct ContractRule {
    pub contract_type: &'static str,
    /// Clause types that must appear, in canonical drafting order.
    pub required: &'static [&'static str],
    /// Clause types commonly included; assembled if a precedent exists.
    pub optional: &'static [&'static str],
}

/// Clause types that must never coexist in one document. Empty: real conflicts
/// are rare and each needs a documented legal reason before it is added. The
/// mechanism is wired (see `conflicts`) so a rule can be added without touching
/// callers.
const CONFLICTS: &[(&str, &str)] = &[];

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

/// Clause types a document of this type is expected to contain. Advisory: used
/// to audit a drafted document, never to generate one. An unknown contract type
/// has no checklist — callers must treat the empty result as "nothing to audit",
/// not as "draft nothing".
pub fn required_clause_types(contract_type: &str) -> Vec<String> {
    match rule_for(contract_type) {
        Some(rule) => rule.required.iter().map(|s| s.to_string()).collect(),
        None => Vec::new(),
    }
}

/// Full expected set (required + optional) for a contract type; empty when the
/// contract type is unknown.
pub fn all_clause_types(contract_type: &str) -> Vec<String> {
    match rule_for(contract_type) {
        Some(rule) => rule
            .required
            .iter()
            .chain(rule.optional.iter())
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}

/// Where a clause type belongs in canonical drafting order. Used only to decide
/// where to *insert* a clause the precedent lacked; the precedent's own order is
/// otherwise left alone. Falls back to a generic ordering for contract types
/// with no explicit rule, and sorts unknown labels to the end.
pub fn order_index(contract_type: &str, clause_type: &str) -> usize {
    let order = all_clause_types(contract_type);
    let order: &[String] = if order.is_empty() {
        &[]
    } else {
        &order
    };
    if let Some(i) = order.iter().position(|t| t.eq_ignore_ascii_case(clause_type)) {
        return i;
    }
    // Generic fallback keeps boilerplate at the back wherever it comes from.
    GENERIC_ORDER
        .iter()
        .position(|t| t.eq_ignore_ascii_case(clause_type))
        .map(|i| order.len() + i)
        .unwrap_or(order.len() + GENERIC_ORDER.len())
}

/// Whether a clause type is on this contract type's required checklist.
pub fn is_required(contract_type: &str, clause_type: &str) -> bool {
    required_clause_types(contract_type)
        .iter()
        .any(|t| t.eq_ignore_ascii_case(clause_type))
}

/// Audit a drafted document: which required clause types are absent from the
/// labels actually present. Empty when the contract type has no checklist.
pub fn missing_required(contract_type: &str, present: &[String]) -> Vec<String> {
    required_clause_types(contract_type)
        .into_iter()
        .filter(|req| !present.iter().any(|p| p.eq_ignore_ascii_case(req)))
        .collect()
}

/// Conflicting clause-type pairs present in a set.
pub fn conflicts(clause_types: &[String]) -> Vec<(String, String)> {
    let has = |name: &str| clause_types.iter().any(|t| t.eq_ignore_ascii_case(name));
    CONFLICTS
        .iter()
        .filter(|(a, b)| has(a) && has(b))
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect()
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
    fn unknown_contract_type_has_no_checklist() {
        // Critically: *not* a fallback skeleton. An unfamiliar instrument is
        // drafted from its own precedents; we simply have nothing to audit.
        assert!(required_clause_types("Aircraft Wet Lease").is_empty());
        assert!(missing_required("Aircraft Wet Lease", &[]).is_empty());
    }

    #[test]
    fn audit_reports_absent_required_types() {
        let present = vec!["Definitions".to_string(), "Confidentiality".to_string()];
        let missing = missing_required("Non-Disclosure Agreement", &present);
        assert!(missing.contains(&"Governing Law".to_string()));
        assert!(!missing.contains(&"Confidentiality".to_string()));
    }

    #[test]
    fn unknown_label_sorts_after_known_ones() {
        let known = order_index("Non-Disclosure Agreement", "Governing Law");
        let unknown = order_index("Non-Disclosure Agreement", "Laytime and Demurrage");
        assert!(unknown > known);
    }
}
