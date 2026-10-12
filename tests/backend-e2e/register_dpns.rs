//! Tests: Register a DPNS name for an identity.

use crate::framework::harness::{BackendTestContext, ctx};
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::{run_on_large_stack, run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::identity::{
    IdentityRegistrationInfo, IdentityTask, RegisterDpnsNameInput, RegisterIdentityFundingMethod,
    build_identity_registration as build_identity_registration_at_index,
};
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::dpns::DpnsRegistrationOutcome;
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
use dash_evo_tool::model::wallet::Wallet;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::platform::types::identity::PublicKeyHash;
use dash_sdk::platform::{Fetch, Identifier, Identity};
use futures::FutureExt;
use rand::prelude::*;
use std::collections::BTreeMap;
use std::ops::Range;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Identity indices the platform-funded test draws from. The app offers
/// indices below 30 and every fixture uses 0, so nothing else reaches here.
const PLATFORM_FUNDED_IDENTITY_INDICES: Range<u32> = 1_000_000..2_000_000;
/// Candidate indices tried before the test gives up.
const IDENTITY_INDEX_ATTEMPTS: usize = 5;
/// Credits kept on the identity to pay for sending the rest back, as multiples
/// of the app's transfer fee estimate. A larger one is tried when the network
/// refuses the transfer.
const REFUND_FEE_RESERVES: [u64; 3] = [1, 3, 10];
/// Below this, what is left on the identity is not worth a transfer.
const SMALLEST_REFUND: u64 = 1_000_000;
/// Reads of an identity balance tried before giving up, and the pause between them.
const BALANCE_READ_ATTEMPTS: u32 = 5;
const BALANCE_READ_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Registration data for an identity index of `wallet_arc` that no identity
/// on the network uses yet.
///
/// Draws a random index from [`PLATFORM_FUNDED_IDENTITY_INDICES`] and asks the
/// network whether any key derived at that index already belongs to an
/// identity. Platform keeps those keys unique, so "none found" means no
/// identity was ever created from this wallet at that index.
///
/// # Panics
///
/// Panics if key derivation or a lookup fails, or if every candidate is taken.
async fn unused_identity_registration(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
) -> IdentityRegistrationInfo {
    for _ in 0..IDENTITY_INDEX_ATTEMPTS {
        let identity_index = rand::rng().random_range(PLATFORM_FUNDED_IDENTITY_INDICES);
        // The funding amount is unused: the caller replaces the funding method.
        let registration =
            build_identity_registration_at_index(app_context, wallet_arc, identity_index, 0)
                .await
                .expect("Failed to build identity registration");
        let key_hashes: Vec<[u8; 20]> = registration
            .keys
            .to_public_keys_map()
            .expect("identity keys should convert to public keys")
            .values()
            .map(|key| key.public_key_hash().expect("identity key hash"))
            .collect();
        let index_in_use = run_on_large_stack({
            let sdk = app_context.sdk();
            move || async move {
                for key_hash in key_hashes {
                    if Identity::fetch(&sdk, PublicKeyHash(key_hash))
                        .await?
                        .is_some()
                    {
                        return Ok::<_, dash_sdk::Error>(true);
                    }
                }
                Ok(false)
            }
        })
        .await
        .expect("identity lookup by key hash should succeed");
        if !index_in_use {
            return registration;
        }
        tracing::info!(
            identity_index,
            "Identity index already in use, drawing another"
        );
    }
    panic!("No unused identity index found in {IDENTITY_INDEX_ATTEMPTS} attempts");
}

/// Create identity, register a DPNS name, verify by searching.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_register_dpns_name() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Create funded test wallet (needs enough for identity + DPNS registration)
    let (_, wallet_arc) = ctx.create_funded_test_wallet(30_000_000).await;

    // Register identity on Platform
    let reg_info = build_identity_registration(app_context, &wallet_arc).await;
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info));
    let result = run_task(app_context, task)
        .await
        .expect("Identity registration should succeed");

    let qualified_identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };

    // Generate a unique DPNS name >= 20 chars to avoid contest voting period.
    // Contested names (< 20 chars) go through masternode voting and don't
    // appear as regular domain documents immediately.
    let random_suffix: u64 = rand::rng().random();
    let dpns_name = format!("e2etest-{:016x}", random_suffix);
    tracing::info!("Registering DPNS name: {}", dpns_name);

    // TODO: DAPI propagation delay on identity registration
    // Expected: RegisterDpnsName succeeds immediately after RegisterIdentity
    // Actual: occasionally fails with "not found" because the identity hasn't
    //         propagated to the DAPI node that processes the DPNS registration
    let task = BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
        qualified_identity: qualified_identity.clone(),
        name_input: dpns_name.clone(),
        approved_contest_fee: 0,
        signing_key_id: None,
    }));
    let result = run_task(app_context, task)
        .await
        .expect("DPNS registration should succeed");

    match result {
        BackendTaskSuccessResult::RegisteredDpnsName { fee_result, .. } => {
            tracing::info!("DPNS name registered, fee: {:?}", fee_result);
        }
        other => panic!("Expected RegisteredDpnsName, got: {:?}", other),
    }

    // Search for the name to verify
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
        dpns_name.clone(),
        None,
    ));

    let result = run_task(app_context, task)
        .await
        .expect("DPNS search should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(found_identity) => {
            assert_eq!(
                found_identity.identity.id(),
                qualified_identity.identity.id(),
                "Found identity should match registered identity"
            );
            tracing::info!("DPNS name '{}' verified via search", dpns_name);
        }
        other => panic!("Expected LoadedIdentity from DPNS search, got: {:?}", other),
    }
}

/// An identity created from platform address credits, and where they came from.
pub(crate) struct PlatformFunded {
    pub identity: QualifiedIdentity,
    pub funding_address: PlatformAddress,
}

/// Create an identity on the framework wallet holding `funding` credits taken
/// from the wallet's richest platform address. Spends no Core funds.
///
/// # Panics
///
/// Panics when no single address covers `funding` plus the creation fee, naming
/// available and needed credits, or when the identity cannot be created.
pub(crate) async fn platform_funded_identity(
    ctx: &BackendTestContext,
    funding: u64,
) -> PlatformFunded {
    let app_context = &ctx.app_context;
    let seed_hash = ctx.framework_wallet_hash;
    let wallet_arc = app_context
        .wallet_context()
        .wallets()
        .get(&seed_hash)
        .cloned()
        .expect("framework wallet missing");

    // Credits already sitting on the framework wallet's platform addresses.
    let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let balances = match run_task(app_context, task)
        .await
        .expect("FetchPlatformAddressBalances should succeed")
    {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => balances,
        other => panic!("Expected PlatformAddressBalances, got: {:?}", other),
    };
    let total_credits: u64 = balances.values().map(|(credits, _)| *credits).sum();
    let richest = balances
        .iter()
        .map(|(address, (credits, _))| (*address, *credits))
        .max_by_key(|(_, credits)| *credits);

    let mut reg_info = unused_identity_registration(app_context, &wallet_arc).await;
    let identity_index = reg_info.wallet_identity_index;
    let key_count = reg_info
        .keys
        .to_public_keys_map()
        .expect("identity keys should convert to public keys")
        .len();

    let creation_fee = app_context
        .fee_estimator()
        .estimate_identity_create_from_addresses(1, false, key_count);
    // The creation fee is charged to what stays on the funding address, not to
    // the amount moved (as observed on testnet), so the address must hold both.
    let needed = funding.saturating_add(creation_fee);

    let (funding_address, richest_credits) = richest.unwrap_or_else(|| {
        panic!(
            "The framework wallet has no funded platform address: 0 credits available, \
             {needed} credits needed on one address"
        )
    });
    assert!(
        richest_credits >= needed,
        "The framework wallet's platform addresses do not cover this run: {needed} credits \
         needed on one address, the richest holds {richest_credits}, all {} addresses hold \
         {total_credits}",
        balances.len(),
    );
    tracing::info!(
        identity_index,
        funding,
        creation_fee,
        richest_credits,
        "Registering an identity from platform address credits"
    );

    reg_info.identity_funding_method = RegisterIdentityFundingMethod::FundWithPlatformAddresses {
        inputs: BTreeMap::from([(funding_address, funding)]),
        wallet_seed_hash: seed_hash,
    };
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info));
    let result = run_task(app_context, task)
        .await
        .expect("Identity registration from platform addresses should succeed");

    let identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };
    tracing::info!(
        identity_id = %identity.identity.id(),
        identity_index,
        balance = identity.identity.balance(),
        "Identity registered from platform address credits"
    );
    PlatformFunded {
        identity,
        funding_address,
    }
}

/// The identity's balance as the network reports it now.
///
/// A node that has not caught up may not know a new identity yet, so an
/// unanswered read is repeated a few times; `None` if it stays unreadable.
pub(crate) async fn network_balance(app_context: &Arc<AppContext>, id: Identifier) -> Option<u64> {
    for attempt in 1..=BALANCE_READ_ATTEMPTS {
        let fetched = run_on_large_stack({
            let sdk = app_context.sdk();
            move || async move { Identity::fetch_by_identifier(&sdk, id).await }
        })
        .await;
        match fetched {
            Ok(Some(identity)) => return Some(identity.balance()),
            Ok(None) => tracing::warn!(%id, attempt, "The network does not show the identity yet"),
            Err(error) => {
                tracing::warn!(%id, attempt, ?error, "Identity balance could not be read")
            }
        }
        tokio::time::sleep(BALANCE_READ_RETRY_DELAY).await;
    }
    None
}

/// Run `body`, then send what is left on the identity back to the address
/// that funded it, also when `body` panicked.
pub(crate) async fn returning_credits<T>(
    app_context: &Arc<AppContext>,
    funded: &PlatformFunded,
    body: impl Future<Output = T>,
) -> T {
    let outcome = AssertUnwindSafe(body).catch_unwind().await;
    return_credits(app_context, funded).await;
    outcome.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Send the identity's remaining credits back to its funding address.
///
/// Best effort: a failure is logged and leaves the credits on the identity.
async fn return_credits(app_context: &Arc<AppContext>, funded: &PlatformFunded) {
    let id = funded.identity.identity.id();
    let Some(balance) = network_balance(app_context, id).await else {
        tracing::warn!(%id, "Credits not returned: the identity balance is unknown");
        return;
    };
    let transfer_fee = app_context
        .fee_estimator()
        .estimate_credit_transfer_to_addresses(1);
    let mut identity = funded.identity.clone();
    identity.identity.set_balance(balance);
    for reserve in REFUND_FEE_RESERVES.map(|times| transfer_fee.saturating_mul(times)) {
        let amount = balance.saturating_sub(reserve);
        if amount < SMALLEST_REFUND {
            break;
        }
        let task = BackendTask::IdentityTask(IdentityTask::TransferToAddresses {
            identity: identity.clone(),
            outputs: BTreeMap::from([(funded.funding_address, amount)]),
            key_id: None,
        });
        match run_task_with_nonce_retry(app_context, task).await {
            Ok(result) => {
                tracing::info!(
                    %id,
                    returned = amount,
                    kept_for_fee = reserve,
                    ?result,
                    "Returned identity credits to the funding address"
                );
                return;
            }
            Err(error) => tracing::warn!(
                %id,
                amount,
                kept_for_fee = reserve,
                ?error,
                "Returning identity credits was refused"
            ),
        }
    }
    tracing::warn!(%id, balance, "Credits not returned: they stay on the identity");
}

/// Create an identity from credits already on the framework wallet's platform
/// addresses, register a DPNS name for it, verify by searching.
///
/// Spends no Core funds: no funded test wallet and no asset lock. The identity
/// receives exactly the balance the app requires for one non-contested name,
/// so a pass also shows that this estimate satisfies the network. What is left
/// afterwards goes back to the funding address.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_register_dpns_name_funded_from_platform_addresses() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // 20+ characters, so the name is registered at once instead of opening a
    // community vote.
    let random_suffix: u64 = rand::rng().random();
    let dpns_name = format!("e2e-platform-{:016x}", random_suffix);
    let funding = app_context
        .fee_estimator()
        .estimate_username_registration(dpns_name.len(), false);

    let funded = platform_funded_identity(ctx, funding).await;
    returning_credits(
        app_context,
        &funded,
        register_and_verify(app_context, &funded.identity, &dpns_name),
    )
    .await;
}

/// Register `dpns_name` for `qualified_identity` through the app's task and
/// verify it by searching.
async fn register_and_verify(
    app_context: &Arc<AppContext>,
    qualified_identity: &QualifiedIdentity,
    dpns_name: &str,
) {
    tracing::info!("Registering DPNS name: {}", dpns_name);

    let task = BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
        qualified_identity: qualified_identity.clone(),
        name_input: dpns_name.to_owned(),
        approved_contest_fee: 0,
        signing_key_id: None,
    }));
    let result = run_task(app_context, task)
        .await
        .expect("DPNS registration should succeed");

    match result {
        BackendTaskSuccessResult::RegisteredDpnsName {
            outcome,
            fee_result,
        } => {
            assert_eq!(
                outcome,
                DpnsRegistrationOutcome::Registered,
                "A 20+ character name should be registered without a vote"
            );
            tracing::info!("DPNS name registered, fee: {:?}", fee_result);
        }
        other => panic!("Expected RegisteredDpnsName, got: {:?}", other),
    }

    // Search for the name to verify
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
        dpns_name.to_owned(),
        None,
    ));
    let result = run_task(app_context, task)
        .await
        .expect("DPNS search should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(found_identity) => {
            assert_eq!(
                found_identity.identity.id(),
                qualified_identity.identity.id(),
                "Found identity should match registered identity"
            );
            tracing::info!("DPNS name '{}' verified via search", dpns_name);
        }
        other => panic!("Expected LoadedIdentity from DPNS search, got: {:?}", other),
    }
}
