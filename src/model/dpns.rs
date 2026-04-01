//! DPNS name normalization helpers.
//!
//! Centralizes the "trim → strip `.dash` suffix → homograph-safe normalize"
//! pipeline so every DPNS lookup uses the same logic.

use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;

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
    let trimmed = input.trim();
    let label = strip_dash_suffix(trimmed);
    convert_to_homograph_safe_chars(label)
}

/// Strip the `.dash` parent domain suffix (case-insensitive).
///
/// Returns the bare label portion, or the full input if no suffix is present.
pub fn strip_dash_suffix(input: &str) -> &str {
    if input.len() > DASH_SUFFIX.len()
        && input[input.len() - DASH_SUFFIX.len()..].eq_ignore_ascii_case(DASH_SUFFIX)
    {
        &input[..input.len() - DASH_SUFFIX.len()]
    } else {
        input
    }
}

/// Check whether the input looks like a full DPNS name (ends with `.dash`,
/// case-insensitive) rather than a bare label or identity ID.
pub fn has_dash_suffix(input: &str) -> bool {
    let trimmed = input.trim();
    trimmed.len() > DASH_SUFFIX.len()
        && trimmed[trimmed.len() - DASH_SUFFIX.len()..].eq_ignore_ascii_case(DASH_SUFFIX)
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
    }
}
