//! Helpers for creating DashPay-capable identities in tests.

// TODO(production-reuse): This helper parallels `src/backend_task/identity/mod.rs::default_identity_key_specs`
// and `src/backend_task/dashpay.rs` key selection logic.
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::sync::{Arc, RwLock};

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;

/// Register an identity with DashPay encryption/decryption keys.
///
/// The default identity key specs already include DashPay contract-bound
/// encryption and decryption keys, so this simply delegates to the standard
/// identity registration flow.
///
/// Returns the QualifiedIdentity and the raw master authentication private key
/// bytes captured during registration (before the wallet encrypts them).
pub async fn create_dashpay_identity(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    wallet_seed_hash: WalletSeedHash,
) -> (QualifiedIdentity, Vec<u8>) {
    let (reg_info, master_key_bytes) =
        build_identity_registration(app_context, wallet_arc, wallet_seed_hash);

    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info));
    let result = run_task(app_context, task)
        .await
        .expect("create_dashpay_identity: identity registration failed");

    match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, fee) => {
            tracing::info!(
                "create_dashpay_identity: registered {:?} (fee: {:?})",
                qi.identity.id(),
                fee
            );
            (qi, master_key_bytes)
        }
        other => panic!(
            "create_dashpay_identity: expected RegisteredIdentity, got: {:?}",
            other
        ),
    }
}
