//! Stateless input parsing for the headless masternode/evonode MCP tools.
//!
//! These helpers are the single source of truth for parsing the string
//! parameters of `masternode_identity_load` and
//! `masternode_credits_withdraw`: the node type, the withdrawal key
//! mode, the at-least-one-signing-key rule, and the ProTxHash decode. They hold
//! no state — no `AppContext`, `Sdk`, DB, or `BackendTask` — so they are
//! exhaustively unit-testable without a network. Stateful enforcement (key
//! presence on-chain, identity existence, network match) stays in the backend
//! task and the tool layer.

use std::fmt;

use crate::model::qualified_identity::IdentityType;
use crate::model::secret::Secret;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;
use thiserror::Error;

/// Why a masternode/evonode tool input failed to parse.
///
/// Model-local so these pure validators carry no dependency on the MCP layer;
/// the tool boundary converts these into `McpToolError::InvalidParam`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MasternodeInputError {
    #[error("The 'node_type' must be \"masternode\" or \"evonode\".")]
    InvalidNodeType,
    #[error("The 'key_mode' must be \"owner\" or \"transfer\".")]
    InvalidKeyMode,
    #[error(
        "Provide at least one of the owner or payout private key. \
         The owner key withdraws to the registered payout address; \
         the payout key withdraws to any address."
    )]
    NoSigningKey,
    #[error(
        "Could not read the identity ID: {input}. \
         Provide a 64-character hex ProTxHash or the Base58 identity ID \
         from masternode-identity-load."
    )]
    InvalidIdentityId { input: String },
}

/// Withdrawal key mode for the masternode credit-withdraw tool.
///
/// `Owner` signs with the OWNER-purpose key and forces the destination to the
/// registered payout address; `Transfer` signs with the TRANSFER-purpose
/// (payout) key and withdraws to any Core address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// Owner key: destination forced to the registered payout address.
    Owner,
    /// Transfer/payout key: withdraws to any caller-supplied Core address.
    Transfer,
}

impl fmt::Display for KeyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyMode::Owner => f.write_str("owner"),
            KeyMode::Transfer => f.write_str("transfer"),
        }
    }
}

/// Parse the `node_type` parameter into an [`IdentityType`].
///
/// Accepts `"masternode"` or `"evonode"`, trimmed and case-insensitive. The
/// `User` type is never produced — this tool is masternode-specific by design.
///
/// # Errors
///
/// Returns [`MasternodeInputError::InvalidNodeType`] for `"user"` or any other
/// value.
pub fn parse_node_type(node_type: &str) -> Result<IdentityType, MasternodeInputError> {
    match node_type.trim().to_ascii_lowercase().as_str() {
        "masternode" => Ok(IdentityType::Masternode),
        "evonode" => Ok(IdentityType::Evonode),
        _ => Err(MasternodeInputError::InvalidNodeType),
    }
}

/// Parse the `key_mode` parameter into a [`KeyMode`].
///
/// Accepts `"owner"` or `"transfer"`, trimmed and case-insensitive.
///
/// # Errors
///
/// Returns [`MasternodeInputError::InvalidKeyMode`] for any other value.
pub fn parse_key_mode(key_mode: &str) -> Result<KeyMode, MasternodeInputError> {
    match key_mode.trim().to_ascii_lowercase().as_str() {
        "owner" => Ok(KeyMode::Owner),
        "transfer" => Ok(KeyMode::Transfer),
        _ => Err(MasternodeInputError::InvalidKeyMode),
    }
}

/// Require at least one of the owner or payout signing keys.
///
/// A masternode identity loaded with neither signing key is watch-only and can
/// sign no withdrawal, so it is rejected. A voting key alone does not satisfy
/// the rule — it only binds the voter identity. Keys are considered present
/// when their trimmed value is non-empty (an empty string means "not supplied",
/// matching the backend's `verify_key_input`).
///
/// # Errors
///
/// Returns [`MasternodeInputError::NoSigningKey`] naming both keys and
/// explaining the two withdraw modes when neither is supplied.
pub fn require_at_least_one_signing_key(
    owner_private_key: &Secret,
    payout_private_key: &Secret,
) -> Result<(), MasternodeInputError> {
    if owner_private_key.is_blank() && payout_private_key.is_blank() {
        return Err(MasternodeInputError::NoSigningKey);
    }
    Ok(())
}

/// Decode a ProTxHash / identity ID accepting either Base58 or hex encoding.
///
/// Mirrors the backend's `load_identity` parse order: Base58 first, then a hex
/// fallback. Both the canonical hex ProTxHash encoding and a Base58 identity ID
/// resolve to the same [`Identifier`].
///
/// # Errors
///
/// Returns [`MasternodeInputError::InvalidIdentityId`] when the input parses as
/// neither.
pub fn decode_identity_id(input: &str) -> Result<Identifier, MasternodeInputError> {
    Identifier::from_string(input, Encoding::Base58)
        .or_else(|_| Identifier::from_string(input, Encoding::Hex))
        .map_err(|_| MasternodeInputError::InvalidIdentityId {
            input: input.to_owned(),
        })
}

/// Whether `input` is a syntactically valid ProTxHash / identity id — Base58 or
/// hex, shape-only (no on-chain existence check). Empty input is not valid.
///
/// Single source of truth for the Masternodes load form's inline on-blur
/// ProTxHash validation (FR-4). Uses the same Base58-then-hex decode contract as
/// [`decode_identity_id`]; the backend load task performs the authoritative
/// existence and duplicate checks.
pub fn is_valid_pro_tx_hash(input: &str) -> bool {
    // TODO: this duplicates the Base58-then-hex decode of `decode_identity_id`.
    // Fold this into `decode_identity_id(input).is_ok()` once the McpToolError
    // dependency in that function is acceptable at every call site.
    let trimmed = input.trim();
    !trimmed.is_empty()
        && (Identifier::from_string(trimmed, Encoding::Base58).is_ok()
            || Identifier::from_string(trimmed, Encoding::Hex).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;

    // ── parse_node_type (TC-MN-001/002/003/004) ──────────────────────────

    #[test]
    fn node_type_masternode_parses() {
        assert_eq!(
            parse_node_type("masternode").unwrap(),
            IdentityType::Masternode
        );
    }

    #[test]
    fn node_type_evonode_parses() {
        assert_eq!(parse_node_type("evonode").unwrap(), IdentityType::Evonode);
    }

    #[test]
    fn node_type_user_rejected() {
        let err = parse_node_type("user").unwrap_err();
        assert_eq!(err, MasternodeInputError::InvalidNodeType);
        assert_eq!(
            err.to_string(),
            "The 'node_type' must be \"masternode\" or \"evonode\"."
        );
    }

    #[test]
    fn node_type_trim_and_case_insensitive() {
        // Trailing whitespace and mixed case are normalized, not rejected.
        assert_eq!(
            parse_node_type("MASTERNODE ").unwrap(),
            IdentityType::Masternode
        );
        assert_eq!(parse_node_type(" Evonode").unwrap(), IdentityType::Evonode);
    }

    #[test]
    fn node_type_garbage_rejected() {
        for bad in ["evo", "", "node", "masternodes"] {
            assert!(
                matches!(
                    parse_node_type(bad),
                    Err(MasternodeInputError::InvalidNodeType)
                ),
                "expected {bad:?} to be rejected"
            );
        }
    }

    // ── parse_key_mode (TC-MN-030) ────────────────────────────────────────

    #[test]
    fn key_mode_owner_and_transfer_parse() {
        assert_eq!(parse_key_mode("owner").unwrap(), KeyMode::Owner);
        assert_eq!(parse_key_mode("transfer").unwrap(), KeyMode::Transfer);
    }

    #[test]
    fn key_mode_trim_and_case_insensitive() {
        assert_eq!(parse_key_mode("OWNER ").unwrap(), KeyMode::Owner);
        assert_eq!(parse_key_mode(" Transfer").unwrap(), KeyMode::Transfer);
    }

    #[test]
    fn key_mode_unknown_rejected() {
        for bad in ["foo", ""] {
            let err = parse_key_mode(bad).unwrap_err();
            assert_eq!(
                err.to_string(),
                "The 'key_mode' must be \"owner\" or \"transfer\".",
                "for input {bad:?}"
            );
        }
    }

    // ── require_at_least_one_signing_key (TC-MN-008/009) ──────────────────

    #[test]
    fn both_keys_absent_rejected_naming_both() {
        let err = require_at_least_one_signing_key(&Secret::new(""), &Secret::new("")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("owner"), "message names owner key: {msg}");
        assert!(msg.contains("payout"), "message names payout key: {msg}");
    }

    #[test]
    fn voting_key_alone_does_not_satisfy() {
        // Voting key is not a parameter here — the rule sees only owner/payout.
        // Both empty must still be rejected even when a voting key is set elsewhere.
        assert!(
            require_at_least_one_signing_key(&Secret::new("   "), &Secret::new("   ")).is_err()
        );
    }

    #[test]
    fn owner_key_alone_satisfies() {
        assert!(
            require_at_least_one_signing_key(&Secret::new("OWNER_WIF"), &Secret::new("")).is_ok()
        );
    }

    #[test]
    fn payout_key_alone_satisfies() {
        assert!(
            require_at_least_one_signing_key(&Secret::new(""), &Secret::new("PAYOUT_WIF")).is_ok()
        );
    }

    // ── decode_identity_id — Base58/hex identifier parse (TC-MN-005/006/007) ──
    //
    // Used by the withdraw tool for `identity_id`. The load tool passes
    // `pro_tx_hash` straight to the backend, which parses it identically; these
    // pin the shared Base58-then-hex contract.

    #[test]
    fn identity_id_hex_accepted() {
        let id = Identifier::random();
        let hex = id.to_string(Encoding::Hex);
        assert_eq!(decode_identity_id(&hex).unwrap(), id);
    }

    #[test]
    fn identity_id_base58_accepted_and_equals_hex_form() {
        let id = Identifier::random();
        let base58 = id.to_string(Encoding::Base58);
        let hex = id.to_string(Encoding::Hex);
        let from_base58 = decode_identity_id(&base58).unwrap();
        let from_hex = decode_identity_id(&hex).unwrap();
        // Both encodings of the same identity decode to byte-identical IDs.
        assert_eq!(from_base58, from_hex);
        assert_eq!(from_base58.as_bytes(), id.as_bytes());
    }

    #[test]
    fn identity_id_malformed_rejected() {
        // "not-a-hash" and 63/65-char hex are neither valid Base58 nor valid hex
        // identifiers. (A 64-char hex string is valid, so it is excluded here.)
        for bad in ["not-a-hash", "", &"a".repeat(63), &"b".repeat(65)] {
            assert!(
                matches!(
                    decode_identity_id(bad),
                    Err(MasternodeInputError::InvalidIdentityId { .. })
                ),
                "expected {bad:?} to be rejected"
            );
        }
    }

    // ── is_valid_pro_tx_hash — UI on-blur shape check (TC-FR4-08/09) ──────

    #[test]
    fn pro_tx_hash_accepts_hex_and_base58() {
        let id = Identifier::random();
        assert!(is_valid_pro_tx_hash(&id.to_string(Encoding::Hex)));
        assert!(is_valid_pro_tx_hash(&id.to_string(Encoding::Base58)));
    }

    #[test]
    fn pro_tx_hash_accepts_surrounding_whitespace() {
        let id = Identifier::random();
        let padded = format!("  {}  ", id.to_string(Encoding::Hex));
        assert!(is_valid_pro_tx_hash(&padded));
    }

    #[test]
    fn pro_tx_hash_rejects_empty_and_malformed() {
        for bad in ["", "   ", "not-a-hash", &"a".repeat(63), &"b".repeat(65)] {
            assert!(!is_valid_pro_tx_hash(bad), "expected {bad:?} rejected");
        }
    }

    #[test]
    fn identity_id_error_states_what_to_do() {
        // The error must carry a concrete self-resolution action: the two
        // accepted formats and the tool that produces the canonical Base58 form.
        let err = decode_identity_id("not-a-hash").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("64-character hex ProTxHash"), "got: {msg}");
        assert!(msg.contains("masternode-identity-load"), "got: {msg}");
    }
}
