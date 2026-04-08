//! Shared fixtures for backend E2E tests.
//!
//! Each fixture uses `tokio::sync::OnceCell` for lazy, one-time initialization
//! within the shared runtime. The first test that calls an accessor initializes
//! the fixture; subsequent tests reuse it.

use crate::framework::dashpay_helpers;
use crate::framework::harness;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::run_task;
use crate::framework::token_helpers;
use dash_evo_tool::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::PrivateKeyTarget;
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use std::sync::{Arc, RwLock};

// --- SharedIdentity ---

// TODO(production-reuse): This fixture parallels `src/backend_task/identity/mod.rs::run_register_identity_task`.
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

static SHARED_IDENTITY: tokio::sync::OnceCell<SharedIdentity> = tokio::sync::OnceCell::const_new();

/// A single registered identity reused across identity/token/broadcast tests.
pub struct SharedIdentity {
    pub qualified_identity: dash_evo_tool::model::qualified_identity::QualifiedIdentity,
    pub wallet_arc: Arc<RwLock<Wallet>>,
    pub wallet_seed_hash: WalletSeedHash,
    pub signing_key: IdentityPublicKey,
    pub signing_key_bytes: Vec<u8>,
}

/// Get (or initialize) the shared identity fixture.
///
/// Registers an identity from a freshly funded test wallet (2M duffs).
pub async fn shared_identity() -> &'static SharedIdentity {
    SHARED_IDENTITY
        .get_or_init(|| async {
            let ctx = harness::ctx().await;

            tracing::info!("SharedIdentity: creating funded test wallet (10M duffs)...");
            let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(10_000_000).await;

            let (reg_info, master_key_bytes) =
                build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);

            let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info));
            let result = run_task(&ctx.app_context, task)
                .await
                .expect("SharedIdentity: identity registration failed");

            let qi = match result {
                BackendTaskSuccessResult::RegisteredIdentity(qi, fee) => {
                    tracing::info!(
                        "SharedIdentity: registered {:?} (fee: {:?})",
                        qi.identity.id(),
                        fee
                    );
                    qi
                }
                other => panic!(
                    "SharedIdentity: expected RegisteredIdentity, got: {:?}",
                    other
                ),
            };

            let signing_key = find_authentication_public_key(&qi);

            SharedIdentity {
                qualified_identity: qi,
                wallet_arc,
                wallet_seed_hash: seed_hash,
                signing_key,
                signing_key_bytes: master_key_bytes,
            }
        })
        .await
}

// --- SharedToken ---

// TODO(production-reuse): This fixture parallels `src/backend_task/tokens/mod.rs::run_token_task` (RegisterTokenContract).
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

static SHARED_TOKEN: tokio::sync::OnceCell<SharedToken> = tokio::sync::OnceCell::const_new();

/// Token contract + position registered by SHARED_IDENTITY.
pub struct SharedToken {
    pub data_contract: Arc<DataContract>,
    pub token_position: TokenContractPosition,
    pub token_id: Identifier,
}

/// Get (or initialize) the shared token fixture.
///
/// Deploys a token contract with permissive rules using the shared identity.
pub async fn shared_token() -> &'static SharedToken {
    SHARED_TOKEN
        .get_or_init(|| async {
            let ctx = harness::ctx().await;
            let si = shared_identity().await;

            tracing::info!("SharedToken: registering token contract...");
            let task =
                token_helpers::build_register_token_task(&si.qualified_identity, &si.signing_key);
            let result = run_task(&ctx.app_context, task)
                .await
                .expect("SharedToken: token contract registration failed");

            match result {
                BackendTaskSuccessResult::RegisteredTokenContract => {
                    tracing::info!("SharedToken: contract registered, fetching from Platform...");
                }
                other => panic!(
                    "SharedToken: expected RegisteredTokenContract, got: {:?}",
                    other
                ),
            }

            // The registered contract is saved to the local DB by the registration
            // task. Retrieve it by scanning the DB for token contracts.
            let qualified_contracts = ctx
                .app_context
                .db()
                .get_contracts(&ctx.app_context, None, None)
                .expect("SharedToken: failed to query contracts from DB");

            let contract = qualified_contracts
                .into_iter()
                .find(|qc| !qc.contract.tokens().is_empty())
                .expect("SharedToken: no token contract found in DB after registration")
                .contract;

            let token_position: TokenContractPosition = 0;
            let token_id = contract
                .token_id(token_position)
                .expect("SharedToken: token at position 0 not found");

            let data_contract = Arc::new(contract);

            tracing::info!(
                "SharedToken: contract_id={:?}, token_id={:?}",
                data_contract.id(),
                token_id
            );

            SharedToken {
                data_contract,
                token_position,
                token_id,
            }
        })
        .await
}

// --- SharedDashPayPair ---

// TODO(production-reuse): This fixture parallels `src/backend_task/identity/mod.rs::run_register_identity_task`
// and `src/backend_task/identity/register_dpns_name.rs`.
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

static SHARED_DASHPAY_PAIR: tokio::sync::OnceCell<SharedDashPayPair> =
    tokio::sync::OnceCell::const_new();

/// Two identities (A, B) with DashPay keys and DPNS names.
pub struct SharedDashPayPair {
    pub identity_a: dash_evo_tool::model::qualified_identity::QualifiedIdentity,
    pub identity_b: dash_evo_tool::model::qualified_identity::QualifiedIdentity,
    pub username_a: String,
    pub username_b: String,
    pub signing_key_a: (IdentityPublicKey, Vec<u8>),
    pub signing_key_b: (IdentityPublicKey, Vec<u8>),
    pub wallet_a: Arc<RwLock<Wallet>>,
    pub wallet_b: Arc<RwLock<Wallet>>,
}

/// Get (or initialize) the shared DashPay pair fixture.
///
/// Creates two identities with DashPay keys and DPNS names (3M duffs each).
pub async fn shared_dashpay_pair() -> &'static SharedDashPayPair {
    SHARED_DASHPAY_PAIR
        .get_or_init(|| async {
            let ctx = harness::ctx().await;

            tracing::info!(
                "SharedDashPayPair: creating two funded test wallets (3M duffs each)..."
            );
            let (seed_hash_a, wallet_a) = ctx.create_funded_test_wallet(3_000_000).await;
            let (seed_hash_b, wallet_b) = ctx.create_funded_test_wallet(3_000_000).await;

            // Register identities with DashPay keys
            tracing::info!("SharedDashPayPair: registering identity A...");
            let (qi_a, key_bytes_a) =
                dashpay_helpers::create_dashpay_identity(&ctx.app_context, &wallet_a, seed_hash_a)
                    .await;

            tracing::info!("SharedDashPayPair: registering identity B...");
            let (qi_b, key_bytes_b) =
                dashpay_helpers::create_dashpay_identity(&ctx.app_context, &wallet_b, seed_hash_b)
                    .await;

            // Register DPNS names
            let username_a = format!("e2epair-a-{}", hex::encode(&seed_hash_a[..4]));
            let username_b = format!("e2epair-b-{}", hex::encode(&seed_hash_b[..4]));

            tracing::info!(
                "SharedDashPayPair: registering DPNS name '{}' for identity A...",
                username_a
            );
            let task_a =
                BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
                    qualified_identity: qi_a.clone(),
                    name_input: username_a.clone(),
                }));
            let result_a = run_task(&ctx.app_context, task_a)
                .await
                .expect("SharedDashPayPair: DPNS registration for A failed");
            assert!(
                matches!(result_a, BackendTaskSuccessResult::RegisteredDpnsName(_)),
                "SharedDashPayPair: expected RegisteredDpnsName for A, got: {:?}",
                result_a
            );

            tracing::info!(
                "SharedDashPayPair: registering DPNS name '{}' for identity B...",
                username_b
            );
            let task_b =
                BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
                    qualified_identity: qi_b.clone(),
                    name_input: username_b.clone(),
                }));
            let result_b = run_task(&ctx.app_context, task_b)
                .await
                .expect("SharedDashPayPair: DPNS registration for B failed");
            assert!(
                matches!(result_b, BackendTaskSuccessResult::RegisteredDpnsName(_)),
                "SharedDashPayPair: expected RegisteredDpnsName for B, got: {:?}",
                result_b
            );

            // Wait for both DPNS names to propagate before returning.
            // Platform needs time to make names queryable after registration.
            tracing::info!("SharedDashPayPair: waiting for DPNS names to propagate...");
            let propagation_timeout = std::time::Duration::from_secs(60);
            let poll_interval = std::time::Duration::from_secs(5);
            let start = std::time::Instant::now();
            loop {
                let search_a = run_task(
                    &ctx.app_context,
                    BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
                        username_a.clone(),
                        None,
                    )),
                )
                .await;
                let search_b = run_task(
                    &ctx.app_context,
                    BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
                        username_b.clone(),
                        None,
                    )),
                )
                .await;

                let a_found = matches!(search_a, Ok(BackendTaskSuccessResult::LoadedIdentity(_)));
                let b_found = matches!(search_b, Ok(BackendTaskSuccessResult::LoadedIdentity(_)));

                if a_found && b_found {
                    tracing::info!(
                        "SharedDashPayPair: both DPNS names propagated after {:?}",
                        start.elapsed()
                    );
                    break;
                }

                if start.elapsed() > propagation_timeout {
                    panic!(
                        "SharedDashPayPair: DPNS names did not propagate within {:?} \
                         (a_found={}, b_found={})",
                        propagation_timeout, a_found, b_found
                    );
                }

                tracing::info!(
                    "SharedDashPayPair: DPNS not yet queryable (a={}, b={}), retrying in {:?}...",
                    a_found,
                    b_found,
                    poll_interval
                );
                tokio::time::sleep(poll_interval).await;
            }

            let signing_key_a = (find_authentication_public_key(&qi_a), key_bytes_a);
            let signing_key_b = (find_authentication_public_key(&qi_b), key_bytes_b);

            tracing::info!(
                "SharedDashPayPair: ready — A={:?} ({}), B={:?} ({})",
                qi_a.identity.id(),
                username_a,
                qi_b.identity.id(),
                username_b,
            );

            SharedDashPayPair {
                identity_a: qi_a,
                identity_b: qi_b,
                username_a,
                username_b,
                signing_key_a,
                signing_key_b,
                wallet_a,
                wallet_b,
            }
        })
        .await
}

// --- Helpers ---

/// Find the first AUTHENTICATION public key in a QualifiedIdentity.
///
/// Tries HIGH first, then CRITICAL. MASTER is skipped because Platform
/// rejects MASTER-level keys for most state transitions (tokens, data
/// contracts, etc.) — only HIGH or CRITICAL are accepted.
pub fn find_authentication_public_key(
    qi: &dash_evo_tool::model::qualified_identity::QualifiedIdentity,
) -> IdentityPublicKey {
    for target_level in [
        SecurityLevel::HIGH,
        SecurityLevel::CRITICAL,
        SecurityLevel::MASTER,
    ] {
        for ((target, _key_id), (qualified_key, _)) in qi.private_keys.private_keys.iter() {
            if *target != PrivateKeyTarget::PrivateKeyOnMainIdentity {
                continue;
            }
            let ipk = &qualified_key.identity_public_key;
            if ipk.purpose() == Purpose::AUTHENTICATION && ipk.security_level() == target_level {
                return ipk.clone();
            }
        }
    }
    panic!("find_authentication_public_key: no AUTHENTICATION key found in QualifiedIdentity");
}
