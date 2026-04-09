//! DPNS name normalization and document helpers.
//!
//! Centralizes the "trim → strip `.dash` suffix → homograph-safe normalize"
//! pipeline so every DPNS lookup uses the same logic, and provides shared
//! extraction utilities for DPNS domain documents.

use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::platform::{Document, Identifier};

/// The `.dash` parent domain suffix (case-insensitive match target).
const DASH_SUFFIX: &str = ".dash";

/// Extract the bare label from a DPNS input and apply homograph-safe normalization.
///
/// Handles all common user inputs:
/// - `"alice"` → `"a11ce"`
/// - `"alice.dash"` → `"a11ce"`
/// - `"  Alice.DASH  "` → `"a11ce"`
/// - `"Alice"` → `"a11ce"`
///
/// The returned string is ready for use as a `normalizedLabel` query value.
pub fn normalize_dpns_label(input: &str) -> String {
    let label = strip_dash_suffix(input);
    convert_to_homograph_safe_chars(label)
}

/// Strip the `.dash` parent domain suffix (case-insensitive).
///
/// Returns the bare label portion, or the full input if no suffix is present.
pub fn strip_dash_suffix(input: &str) -> &str {
    let trimmed = input.trim();
    let suffix_start = trimmed.len().saturating_sub(DASH_SUFFIX.len());
    if trimmed.len() > DASH_SUFFIX.len()
        && trimmed
            .get(suffix_start..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(DASH_SUFFIX))
    {
        &trimmed[..suffix_start]
    } else {
        trimmed
    }
}

/// Validate that a DPNS username-or-ID input has an acceptable format.
///
/// Returns `Ok(())` for:
/// - Bare labels: `"alice"`, `"olivia22"`
/// - `.dash` names (case-insensitive): `"alice.dash"`, `"Alice.DASH"`
/// - Identity IDs (no dots): `"4EfA..."`
///
/// Returns `Err(input)` for inputs with non-`.dash` domains: `"alice.foo"`, `"alice.com"`
pub fn validate_dpns_input(input: &str) -> Result<(), String> {
    let trimmed = input.trim();
    if trimmed.contains('.') && !has_dash_suffix(trimmed) {
        Err(trimmed.to_string())
    } else {
        Ok(())
    }
}

/// Check whether the input looks like a full DPNS name (ends with `.dash`,
/// case-insensitive) rather than a bare label or identity ID.
pub fn has_dash_suffix(input: &str) -> bool {
    let trimmed = input.trim();
    let suffix_start = trimmed.len().saturating_sub(DASH_SUFFIX.len());
    trimmed.len() > DASH_SUFFIX.len()
        && trimmed
            .get(suffix_start..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(DASH_SUFFIX))
}

/// Extract the identity ID from a DPNS domain document's `records.identity` field.
///
/// This is the authoritative identity reference for a DPNS name — unlike
/// `document.owner_id()`, it correctly reflects ownership after name transfers.
///
/// Returns `None` if the document lacks the `records` map or the `identity` entry.
pub fn extract_identity_id_from_dpns_document(document: &Document) -> Option<Identifier> {
    document
        .get("records")
        .and_then(|records| {
            if let Value::Map(map) = records {
                map.iter()
                    .find(|(k, _)| matches!(k, Value::Text(key) if key == "identity"))
                    .map(|(_, v)| v.clone())
            } else {
                None
            }
        })
        .and_then(|id_value| {
            if let Value::Identifier(id_bytes) = id_value {
                Some(Identifier::from(id_bytes))
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_bare_label() {
        assert_eq!(normalize_dpns_label("alice"), "a11ce");
    }

    #[test]
    fn normalize_with_dash_suffix() {
        assert_eq!(normalize_dpns_label("alice.dash"), "a11ce");
    }

    #[test]
    fn normalize_case_insensitive_suffix() {
        assert_eq!(normalize_dpns_label("Alice.DASH"), "a11ce");
        assert_eq!(normalize_dpns_label("alice.Dash"), "a11ce");
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_dpns_label("  alice.dash  "), "a11ce");
    }

    #[test]
    fn normalize_homograph_chars() {
        // o→0, i→1, l→1
        assert_eq!(normalize_dpns_label("olivia"), "011v1a");
        assert_eq!(
            normalize_dpns_label("supertestingnameabc123"),
            "supertest1ngnameabc123"
        );
    }

    #[test]
    fn has_suffix_detection() {
        assert!(has_dash_suffix("alice.dash"));
        assert!(has_dash_suffix("Alice.DASH"));
        assert!(has_dash_suffix("alice.Dash"));
        assert!(!has_dash_suffix("alice"));
        assert!(!has_dash_suffix("dash")); // too short
        assert!(!has_dash_suffix(".dash")); // just the suffix, no label
    }

    #[test]
    fn strip_suffix_cases() {
        assert_eq!(strip_dash_suffix("alice.dash"), "alice");
        assert_eq!(strip_dash_suffix("alice.DASH"), "alice");
        assert_eq!(strip_dash_suffix("alice"), "alice");
        assert_eq!(strip_dash_suffix("a.dash"), "a"); // valid: label "a"
        assert_eq!(strip_dash_suffix(".dash"), ".dash"); // no label, len == 5
        // Non-ASCII: must not panic even though byte offset isn't a char boundary
        assert_eq!(strip_dash_suffix("ünïcödë"), "ünïcödë");
        assert_eq!(strip_dash_suffix("  alice  "), "alice"); // trims whitespace
    }

    #[test]
    fn validate_dpns_input_accepts_valid_inputs() {
        assert!(validate_dpns_input("alice").is_ok());
        assert!(validate_dpns_input("olivia22").is_ok());
        assert!(validate_dpns_input("alice.dash").is_ok());
        assert!(validate_dpns_input("Alice.DASH").is_ok());
        assert!(validate_dpns_input("alice.Dash").is_ok());
        assert!(validate_dpns_input("  alice.dash  ").is_ok());
        // Identity IDs (no dots)
        assert!(validate_dpns_input("4EfA78bC").is_ok());
    }

    #[test]
    fn validate_dpns_input_rejects_non_dash_domains() {
        assert_eq!(validate_dpns_input("alice.foo"), Err("alice.foo".into()));
        assert_eq!(validate_dpns_input("alice.com"), Err("alice.com".into()));
        assert_eq!(
            validate_dpns_input("  alice.xyz  "),
            Err("alice.xyz".into())
        );
    }
}
