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
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::platform::IdentityPublicKey;
use std::sync::{Arc, RwLock};

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;

/// Register an identity with DashPay encryption/decryption keys.
///
/// The default identity key specs already include DashPay contract-bound
/// encryption and decryption keys, so this simply delegates to the standard
/// identity registration flow.
pub async fn create_dashpay_identity(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    wallet_seed_hash: WalletSeedHash,
) -> QualifiedIdentity {
    let reg_info = build_identity_registration(app_context, wallet_arc, wallet_seed_hash);

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
            qi
        }
        other => panic!(
            "create_dashpay_identity: expected RegisteredIdentity, got: {:?}",
            other
        ),
    }
}

/// Extract the DashPay contract-bound signing key from a QualifiedIdentity.
///
/// Returns the first AUTHENTICATION key suitable for signing DashPay
/// state transitions (HIGH or CRITICAL security level).
pub fn get_dashpay_signing_key(qi: &QualifiedIdentity) -> (IdentityPublicKey, Vec<u8>) {
    for target_level in [SecurityLevel::HIGH, SecurityLevel::CRITICAL] {
        for ((_target, _key_id), (qualified_key, private_key_data)) in
            qi.private_keys.private_keys.iter()
        {
            let ipk = &qualified_key.identity_public_key;
            if ipk.purpose() == Purpose::AUTHENTICATION && ipk.security_level() == target_level {
                let bytes = match private_key_data {
                    PrivateKeyData::Clear(b) | PrivateKeyData::AlwaysClear(b) => b.to_vec(),
                    _ => continue,
                };
                return (ipk.clone(), bytes);
            }
        }
    }
    panic!("get_dashpay_signing_key: no AUTHENTICATION key found");
}

/// Extract the encryption key from a QualifiedIdentity.
///
/// Returns the ENCRYPTION key bound to the DashPay contactRequest document type.
pub fn get_encryption_key(qi: &QualifiedIdentity) -> (IdentityPublicKey, Vec<u8>) {
    for ((_target, _key_id), (qualified_key, private_key_data)) in
        qi.private_keys.private_keys.iter()
    {
        let ipk = &qualified_key.identity_public_key;
        if ipk.purpose() == Purpose::ENCRYPTION {
            let bytes = match private_key_data {
                PrivateKeyData::Clear(b) | PrivateKeyData::AlwaysClear(b) => b.to_vec(),
                _ => continue,
            };
            return (ipk.clone(), bytes);
        }
    }
    panic!("get_encryption_key: no ENCRYPTION key found in QualifiedIdentity");
}
