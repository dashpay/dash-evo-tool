//! Unit tests for MCP layer validation.

use crate::mcp::error::McpToolError;
use crate::mcp::resolve;

pub(super) fn legacy_wallet_context(
    data_dir: &std::path::Path,
) -> std::sync::Arc<crate::context::AppContext> {
    use crate::context::AppContext;
    use dash_sdk::dpp::dashcore::Network;

    crate::app_dir::ensure_env_file(data_dir);
    let db = std::sync::Arc::new(
        crate::database::test_helpers::create_database_at_path(&data_dir.join("data.db"))
            .expect("create legacy database"),
    );
    let app_kv = AppContext::open_app_kv(data_dir).expect("open app k/v");
    let secret_store = AppContext::open_secret_store(data_dir).expect("open secret store");
    AppContext::new(
        data_dir.to_path_buf(),
        Network::Testnet,
        db,
        Default::default(),
        Default::default(),
        egui::Context::default(),
        app_kv,
        secret_store,
        crate::model::user_role::UserRoleCell::default(),
    )
    .expect("create app context")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_wallets_hydrated_finishes_pending_legacy_migration() {
    use crate::context::migration_status::MigrationState;
    use dash_sdk::dpp::dashcore::Network;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let ctx = legacy_wallet_context(temp_dir.path());
    let seed = [0x91u8; 64];
    let seed_hash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
    let xpub = crate::database::test_helpers::legacy_master_epk_bytes(&seed, Network::Testnet);
    crate::database::test_helpers::seed_legacy_unprotected_hd_wallet_row(
        &ctx.db,
        &seed_hash,
        &seed,
        &xpub,
        "Legacy savings",
        Network::Testnet,
    )
    .expect("seed legacy wallet");

    assert!(
        ctx.wallets.read().expect("wallet map").is_empty(),
        "precondition: the legacy wallet is not hydrated yet"
    );

    resolve::ensure_wallets_hydrated(&ctx)
        .await
        .expect("hydrate and migrate wallets");

    let alias = ctx
        .wallets
        .read()
        .expect("wallet map")
        .get(&seed_hash)
        .map(|wallet| wallet.read().expect("wallet").alias.clone());
    let migration_state = ctx.migration_status().state();
    let backend = ctx.wallet_backend().expect("backend wired");
    let spv_started = backend.is_started();
    backend.shutdown().await;

    assert_eq!(alias, Some(Some("Legacy savings".to_owned())));
    assert_eq!(*migration_state, MigrationState::Success);
    assert!(!spv_started, "wallet hydration must not start chain sync");
}

// ── Amount validation ──────────────────────────────────────────

#[test]
fn zero_amount_rejected() {
    let result = resolve::validate_positive_amount(0, "duffs");
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
    assert!(resolve::validate_positive_amount(1, "duffs").is_ok());
    assert!(resolve::validate_positive_amount(100_000_000, "duffs").is_ok());
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
        McpToolError::StorageNotReady,
        McpToolError::Internal("x".into()),
    ];

    let codes: Vec<i32> = variants
        .into_iter()
        .map(|e| {
            let mcp: McpError = e.into();
            mcp.code.0
        })
        .collect();

    // WalletNotFound, NetworkMismatch, SpvSyncFailed, StorageNotReady should have unique custom codes
    let custom_codes: Vec<i32> = vec![codes[0], codes[2], codes[3], codes[4]];
    let unique: std::collections::HashSet<i32> = custom_codes.iter().copied().collect();
    assert_eq!(
        unique.len(),
        custom_codes.len(),
        "Custom error codes must be distinct: {custom_codes:?}"
    );
}

#[test]
fn storage_not_ready_display_is_actionable() {
    let err = McpToolError::StorageNotReady;
    let msg = err.to_string();
    assert!(
        msg.contains("starting up") || msg.contains("wait") || msg.contains("retry"),
        "StorageNotReady message should direct the user to wait/retry; got: {msg}"
    );
}

#[test]
fn storage_not_ready_has_dedicated_error_code() {
    use rmcp::ErrorData as McpError;

    let spv: McpError = McpToolError::SpvSyncFailed.into();
    let storage: McpError = McpToolError::StorageNotReady.into();
    assert_ne!(
        spv.code.0, storage.code.0,
        "StorageNotReady must have a different code from SpvSyncFailed"
    );
    // Custom range: must not collide with standard JSON-RPC codes
    assert!(
        storage.code.0 < -32000 || storage.code.0 == -32005,
        "StorageNotReady code should be in the custom range"
    );
}
