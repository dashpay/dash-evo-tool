//! Helpers for shielded (ZK) operations in tests.

// TODO(production-reuse): This helper parallels `src/backend_task/shielded/mod.rs::run_warm_up_proving_key`
// and `run_initialize_shielded_wallet`.
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::shielded::ShieldedTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;

/// Check `E2E_SKIP_SHIELDED` env var and skip the calling test if set.
///
/// Call this at the top of every shielded test function. Returns `true`
/// if the test should be skipped (caller should `return` early).
pub fn skip_if_shielded_disabled() -> bool {
    if std::env::var("E2E_SKIP_SHIELDED").is_ok() {
        tracing::info!("Skipping shielded test (E2E_SKIP_SHIELDED is set)");
        true
    } else {
        false
    }
}

/// Check whether a task error indicates the platform does not support
/// shielded operations (e.g., testnet without shielded support enabled).
///
/// Returns `true` if the error message contains "not implemented" or
/// "not supported", meaning the test should be skipped gracefully.
pub fn is_platform_shielded_unsupported(
    err: &dash_evo_tool::backend_task::error::TaskError,
) -> bool {
    let msg = format!("{:?}", err).to_lowercase();
    msg.contains("not implemented") || msg.contains("not supported")
}

/// Run `WarmUpProvingKey` followed by `InitializeShieldedWallet` in sequence.
///
/// This ensures the proving key is downloaded/cached and the wallet's
/// shielded state (ZIP32 keys, commitment tree) is initialized before
/// any shielded operations.
pub async fn warm_up_and_init(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) {
    // Warm up proving key (may take 30-60s on first run)
    tracing::info!("shielded_helpers: warming up proving key...");
    let task = BackendTask::ShieldedTask(ShieldedTask::WarmUpProvingKey);
    let result = run_task(app_context, task)
        .await
        .expect("shielded_helpers: WarmUpProvingKey failed");
    assert!(
        matches!(result, BackendTaskSuccessResult::ProvingKeyReady),
        "shielded_helpers: expected ProvingKeyReady, got: {:?}",
        result
    );

    // Initialize shielded wallet
    tracing::info!("shielded_helpers: initializing shielded wallet...");
    let task = BackendTask::ShieldedTask(ShieldedTask::InitializeShieldedWallet { seed_hash });
    let result = run_task(app_context, task)
        .await
        .expect("shielded_helpers: InitializeShieldedWallet failed");
    match result {
        BackendTaskSuccessResult::ShieldedInitialized {
            seed_hash: sh,
            balance,
        } => {
            assert_eq!(sh, seed_hash);
            tracing::info!(
                "shielded_helpers: wallet initialized (balance: {})",
                balance
            );
        }
        other => panic!(
            "shielded_helpers: expected ShieldedInitialized, got: {:?}",
            other
        ),
    }
}
