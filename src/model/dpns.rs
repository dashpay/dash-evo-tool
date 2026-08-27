//! DPNS name normalization and document helpers.
//!
//! Centralizes the "trim → strip `.dash` suffix → homograph-safe normalize"
//! pipeline so every DPNS lookup uses the same logic, and provides shared
//! extraction utilities for DPNS domain documents.

use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeV0Methods;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::voting::vote_polls::VotePoll;
use dash_sdk::platform::{Document, Identifier};

/// The immediate result of submitting a DPNS registration document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsRegistrationOutcome {
    /// The submitted username was registered without a community vote.
    Registered,
    /// The submitted username entered a community vote before it can be awarded.
    PendingCommunityVote,
}

/// Classify a submitted DPNS document using its document type's contest rules.
pub fn classify_dpns_registration_outcome(
    document_type: &impl DocumentTypeV0Methods,
    document: &Document,
    platform_version: &PlatformVersion,
) -> Result<DpnsRegistrationOutcome, Box<ProtocolError>> {
    match document_type
        .contested_vote_poll_for_document(document, platform_version)
        .map_err(Box::new)?
    {
        Some(VotePoll::ContestedDocumentResourceVotePoll(_)) => {
            Ok(DpnsRegistrationOutcome::PendingCommunityVote)
        }
        None => Ok(DpnsRegistrationOutcome::Registered),
    }
}

/// The `.dash` parent domain suffix (case-insensitive match target).
const DASH_SUFFIX: &str = ".dash";

/// Result of validating a bare label for DPNS registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsNameValidationResult {
    Valid,
    TooShort,
    TooLong,
    InvalidCharacter(char),
    StartsWithHyphen,
    EndsWithHyphen,
}

/// Validate the format of a bare DPNS label before registration.
pub fn validate_dpns_name(name: &str) -> DpnsNameValidationResult {
    if name.len() < 3 {
        return DpnsNameValidationResult::TooShort;
    }
    if name.len() > 63 {
        return DpnsNameValidationResult::TooLong;
    }
    if name.starts_with('-') {
        return DpnsNameValidationResult::StartsWithHyphen;
    }
    if name.ends_with('-') {
        return DpnsNameValidationResult::EndsWithHyphen;
    }
    for character in name.chars() {
        if !character.is_ascii_alphanumeric() && character != '-' {
            return DpnsNameValidationResult::InvalidCharacter(character);
        }
    }
    DpnsNameValidationResult::Valid
}

impl DpnsNameValidationResult {
    /// Return user guidance for an invalid label.
    pub fn error_message(self) -> Option<String> {
        match self {
            Self::Valid => None,
            Self::TooShort => Some("Name must be at least 3 characters long.".to_string()),
            Self::TooLong => Some("Name must be no more than 63 characters long.".to_string()),
            Self::InvalidCharacter(character) => Some(format!(
                "The character '{character}' is not allowed. Use only letters, numbers, and hyphens."
            )),
            Self::StartsWithHyphen => Some("Name cannot start with a hyphen.".to_string()),
            Self::EndsWithHyphen => Some("Name cannot end with a hyphen.".to_string()),
        }
    }
}

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
    if has_dash_suffix(trimmed) {
        // `has_dash_suffix` guarantees a `.dash` tail longer than the suffix, so
        // this ASCII-boundary slice is always valid.
        &trimmed[..trimmed.len() - DASH_SUFFIX.len()]
    } else {
        trimmed
    }
}

/// The DPNS input carries a domain suffix other than `.dash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("A DPNS name must end in .dash or be a plain label.")]
pub struct NonDashDomainError;

/// Validate that a DPNS username-or-ID input has an acceptable format.
///
/// Returns `Ok(())` for:
/// - Bare labels: `"alice"`, `"olivia22"`
/// - `.dash` names (case-insensitive): `"alice.dash"`, `"Alice.DASH"`
/// - Identity IDs (no dots): `"4EfA..."`
///
/// Returns [`NonDashDomainError`] for inputs with non-`.dash` domains:
/// `"alice.foo"`, `"alice.com"`.
pub fn validate_dpns_input(input: &str) -> Result<(), NonDashDomainError> {
    let trimmed = input.trim();
    if trimmed.contains('.') && !has_dash_suffix(trimmed) {
        Err(NonDashDomainError)
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
    use std::collections::BTreeMap;

    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::data_contracts::SystemDataContract;
    use dash_sdk::dpp::document::DocumentV0;
    use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
    use dash_sdk::dpp::version::PlatformVersion;

    fn domain_document(label: &str) -> Document {
        let owner_id = Identifier::from([1; 32]);
        Document::V0(DocumentV0 {
            id: Identifier::from([2; 32]),
            owner_id,
            creator_id: None,
            properties: BTreeMap::from([
                ("parentDomainName".to_string(), "dash".into()),
                ("normalizedParentDomainName".to_string(), "dash".into()),
                ("label".to_string(), label.into()),
                (
                    "normalizedLabel".to_string(),
                    convert_to_homograph_safe_chars(label).into(),
                ),
                ("preorderSalt".to_string(), [0_u8; 32].into()),
                (
                    "records".to_string(),
                    BTreeMap::from([("identity".to_string(), Value::from(owner_id))]).into(),
                ),
                (
                    "subdomainRules".to_string(),
                    BTreeMap::from([("allowSubdomains".to_string(), Value::Bool(false))]).into(),
                ),
            ]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        })
    }

    #[test]
    fn registration_outcome_is_pending_for_contested_dpns_document() {
        let platform_version = PlatformVersion::latest();
        let contract = load_system_data_contract(SystemDataContract::DPNS, platform_version)
            .expect("bundled DPNS contract");
        let document_type = contract
            .document_type_for_name("domain")
            .expect("domain document type");

        assert_eq!(
            classify_dpns_registration_outcome(
                &document_type,
                &domain_document("alice"),
                platform_version,
            )
            .expect("classification succeeds"),
            DpnsRegistrationOutcome::PendingCommunityVote
        );
    }

    #[test]
    fn registration_outcome_is_registered_for_non_contested_dpns_document() {
        let platform_version = PlatformVersion::latest();
        let contract = load_system_data_contract(SystemDataContract::DPNS, platform_version)
            .expect("bundled DPNS contract");
        let document_type = contract
            .document_type_for_name("domain")
            .expect("domain document type");

        assert_eq!(
            classify_dpns_registration_outcome(
                &document_type,
                &domain_document("alice2"),
                platform_version,
            )
            .expect("classification succeeds"),
            DpnsRegistrationOutcome::Registered
        );
    }

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
        assert_eq!(validate_dpns_input("alice.foo"), Err(NonDashDomainError));
        assert_eq!(validate_dpns_input("alice.com"), Err(NonDashDomainError));
        assert_eq!(
            validate_dpns_input("  alice.xyz  "),
            Err(NonDashDomainError)
        );
    }

    #[test]
    fn dpns_registration_name_accepts_boundary_lengths() {
        assert_eq!(validate_dpns_name("abc"), DpnsNameValidationResult::Valid);
        assert_eq!(
            validate_dpns_name(&"a".repeat(63)),
            DpnsNameValidationResult::Valid
        );
    }

    #[test]
    fn dpns_registration_name_rejects_invalid_formats() {
        assert_eq!(validate_dpns_name("ab"), DpnsNameValidationResult::TooShort);
        assert_eq!(
            validate_dpns_name(&"a".repeat(64)),
            DpnsNameValidationResult::TooLong
        );
        assert_eq!(
            validate_dpns_name("-alice"),
            DpnsNameValidationResult::StartsWithHyphen
        );
        assert_eq!(
            validate_dpns_name("alice-"),
            DpnsNameValidationResult::EndsWithHyphen
        );
        assert_eq!(
            validate_dpns_name("ali_ce"),
            DpnsNameValidationResult::InvalidCharacter('_')
        );
    }
}
