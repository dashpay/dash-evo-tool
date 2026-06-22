//! Stateless input parsing for the headless masternode/evonode MCP tools.
//!
//! These helpers are the single source of truth for parsing the string
//! parameters of `masternode_identity_load` and
//! `masternode_withdraw`: the node type, the withdrawal key
//! mode, the at-least-one-signing-key rule, and the ProTxHash decode. They hold
//! no state — no `AppContext`, `Sdk`, DB, or `BackendTask` — so they are
//! exhaustively unit-testable without a network. Stateful enforcement (key
//! presence on-chain, identity existence, network match) stays in the backend
//! task and the tool layer.

use std::fmt;

use crate::mcp::error::McpToolError;
use crate::model::qualified_identity::IdentityType;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;

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
/// Returns [`McpToolError::InvalidParam`] for `"user"` or any other value.
pub fn parse_node_type(node_type: &str) -> Result<IdentityType, McpToolError> {
    match node_type.trim().to_ascii_lowercase().as_str() {
        "masternode" => Ok(IdentityType::Masternode),
        "evonode" => Ok(IdentityType::Evonode),
        _ => Err(McpToolError::InvalidParam {
            message: "The 'node_type' must be \"masternode\" or \"evonode\".".to_owned(),
        }),
    }
}

/// Parse the `key_mode` parameter into a [`KeyMode`].
///
/// Accepts `"owner"` or `"transfer"`, trimmed and case-insensitive.
///
/// # Errors
///
/// Returns [`McpToolError::InvalidParam`] for any other value.
pub fn parse_key_mode(key_mode: &str) -> Result<KeyMode, McpToolError> {
    match key_mode.trim().to_ascii_lowercase().as_str() {
        "owner" => Ok(KeyMode::Owner),
        "transfer" => Ok(KeyMode::Transfer),
        _ => Err(McpToolError::InvalidParam {
            message: "The 'key_mode' must be \"owner\" or \"transfer\".".to_owned(),
        }),
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
/// Returns [`McpToolError::InvalidParam`] naming both keys and explaining the
/// two withdraw modes when neither is supplied.
pub fn require_at_least_one_signing_key(
    owner_private_key: &str,
    payout_private_key: &str,
) -> Result<(), McpToolError> {
    if owner_private_key.trim().is_empty() && payout_private_key.trim().is_empty() {
        return Err(McpToolError::InvalidParam {
            message: "Provide at least one of the owner or payout private key. \
                      The owner key withdraws to the registered payout address; \
                      the payout key withdraws to any address."
                .to_owned(),
        });
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
/// Returns [`McpToolError::InvalidParam`] when the input parses as neither.
pub fn decode_identity_id(input: &str) -> Result<Identifier, McpToolError> {
    Identifier::from_string(input, Encoding::Base58)
        .or_else(|_| Identifier::from_string(input, Encoding::Hex))
        .map_err(|_| McpToolError::InvalidParam {
            message: format!(
                "Could not read the identity ID: {input}. \
                 Provide a 64-character hex ProTxHash or the Base58 identity ID \
                 from identity-masternode-load."
            ),
        })
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
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert_eq!(
            err.to_string(),
            "Invalid parameter: The 'node_type' must be \"masternode\" or \"evonode\"."
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
                matches!(parse_node_type(bad), Err(McpToolError::InvalidParam { .. })),
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
                "Invalid parameter: The 'key_mode' must be \"owner\" or \"transfer\".",
                "for input {bad:?}"
            );
        }
    }

    // ── require_at_least_one_signing_key (TC-MN-008/009) ──────────────────

    #[test]
    fn both_keys_absent_rejected_naming_both() {
        let err = require_at_least_one_signing_key("", "").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("owner"), "message names owner key: {msg}");
        assert!(msg.contains("payout"), "message names payout key: {msg}");
    }

    #[test]
    fn voting_key_alone_does_not_satisfy() {
        // Voting key is not a parameter here — the rule sees only owner/payout.
        // Both empty must still be rejected even when a voting key is set elsewhere.
        assert!(require_at_least_one_signing_key("   ", "   ").is_err());
    }

    #[test]
    fn owner_key_alone_satisfies() {
        assert!(require_at_least_one_signing_key("OWNER_WIF", "").is_ok());
    }

    #[test]
    fn payout_key_alone_satisfies() {
        assert!(require_at_least_one_signing_key("", "PAYOUT_WIF").is_ok());
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
                    Err(McpToolError::InvalidParam { .. })
                ),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn identity_id_error_states_what_to_do() {
        // M-01 — the error must carry a concrete self-resolution action: the two
        // accepted formats and the tool that produces the canonical Base58 form.
        let err = decode_identity_id("not-a-hash").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("64-character hex ProTxHash"), "got: {msg}");
        assert!(msg.contains("identity-masternode-load"), "got: {msg}");
    }
}
