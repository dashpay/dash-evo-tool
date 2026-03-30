//! Unit tests for MCP layer validation.

use crate::mcp::error::McpToolError;
use crate::mcp::resolve;

// ── Amount validation ──────────────────────────────────────────

#[test]
fn zero_amount_rejected() {
    let result = resolve::validate_amount(0);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("greater than zero"),
        "got: {}",
        err
    );
}

#[test]
fn positive_amount_accepted() {
    assert!(resolve::validate_amount(1).is_ok());
    assert!(resolve::validate_amount(100_000_000).is_ok());
}

// ── Address validation ─────────────────────────────────────────

#[test]
fn empty_address_rejected() {
    let result = resolve::validate_address("");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("must not be empty")
    );
}

#[test]
fn mainnet_address_accepted() {
    assert!(resolve::validate_address("XqHiz9VVXfjBnET2z6aZ9j5LKyuGNv3byP").is_ok());
    assert!(resolve::validate_address("7abc123").is_ok());
}

#[test]
fn testnet_address_accepted() {
    assert!(resolve::validate_address("yQ9JNCT4S9zVHaKYbr1FUY4YkUMYxSzWAj").is_ok());
    assert!(resolve::validate_address("8abc123").is_ok());
    assert!(resolve::validate_address("9abc123").is_ok());
}

#[test]
fn invalid_prefix_rejected() {
    let result = resolve::validate_address("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("does not look like a valid Dash address")
    );
}

// ── McpToolError Display ───────────────────────────────────────

#[test]
fn error_display_wallet_not_found() {
    let err = McpToolError::WalletNotFound {
        id: "my-wallet".into(),
    };
    assert_eq!(err.to_string(), "Wallet not found: my-wallet");
}

#[test]
fn error_display_invalid_param() {
    let err = McpToolError::InvalidParam {
        message: "bad input".into(),
    };
    assert_eq!(err.to_string(), "Invalid parameter: bad input");
}

#[test]
fn error_display_network_mismatch() {
    let err = McpToolError::NetworkMismatch {
        expected: "testnet".into(),
        actual: "mainnet".into(),
    };
    assert_eq!(
        err.to_string(),
        "Network mismatch: expected testnet, got mainnet"
    );
}

#[test]
fn error_display_spv_sync_failed() {
    let err = McpToolError::SpvSyncFailed;
    assert!(err.to_string().contains("SPV sync incomplete"));
}

#[test]
fn error_display_internal() {
    let err = McpToolError::Internal("oops".into());
    assert_eq!(err.to_string(), "oops");
}

// ── MCP error code mapping ─────────────────────────────────────

#[test]
fn error_codes_are_distinct() {
    use rmcp::ErrorData as McpError;

    let variants: Vec<McpToolError> = vec![
        McpToolError::WalletNotFound { id: "x".into() },
        McpToolError::InvalidParam {
            message: "x".into(),
        },
        McpToolError::NetworkMismatch {
            expected: "a".into(),
            actual: "b".into(),
        },
        McpToolError::SpvSyncFailed,
        McpToolError::Internal("x".into()),
    ];

    let codes: Vec<i32> = variants
        .into_iter()
        .map(|e| {
            let mcp: McpError = e.into();
            mcp.code.0
        })
        .collect();

    // WalletNotFound, NetworkMismatch, SpvSyncFailed should have unique custom codes
    let custom_codes: Vec<i32> = vec![codes[0], codes[2], codes[3]];
    let unique: std::collections::HashSet<i32> = custom_codes.iter().copied().collect();
    assert_eq!(
        unique.len(),
        custom_codes.len(),
        "Custom error codes must be distinct: {custom_codes:?}"
    );
}
