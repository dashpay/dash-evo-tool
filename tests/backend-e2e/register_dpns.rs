//! Tests: Register a DPNS name for an identity.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::{run_on_large_stack, run_task};
use dash_evo_tool::backend_task::identity::{
    IdentityRegistrationInfo, IdentityTask, RegisterDpnsNameInput, RegisterIdentityFundingMethod,
    build_identity_registration as build_identity_registration_at_index,
};
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::dpns::DpnsRegistrationOutcome;
use dash_evo_tool::model::wallet::Wallet;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::platform::types::identity::PublicKeyHash;
use dash_sdk::platform::{Fetch, Identity};
use rand::prelude::*;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, RwLock};

/// Identity indices the platform-funded test draws from. The app offers
/// indices below 30 and every fixture uses 0, so nothing else reaches here.
const PLATFORM_FUNDED_IDENTITY_INDICES: Range<u32> = 1_000_000..2_000_000;
/// Candidate indices tried before the test gives up.
const IDENTITY_INDEX_ATTEMPTS: usize = 5;

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

/// Create an identity from credits already on the framework wallet's platform
/// addresses, register a DPNS name for it, verify by searching.
///
/// Spends no Core funds: no funded test wallet and no asset lock. The identity
/// receives exactly the balance the app requires for one non-contested name,
/// so a pass also shows that this estimate satisfies the network.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_register_dpns_name_funded_from_platform_addresses() {
    let ctx = ctx().await;
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

    // 20+ characters, so the name is registered at once instead of opening a
    // community vote.
    let random_suffix: u64 = rand::rng().random();
    let dpns_name = format!("e2e-platform-{:016x}", random_suffix);

    let estimator = app_context.fee_estimator();
    let creation_fee = estimator.estimate_identity_create_from_addresses(1, false, key_count);
    let funding = estimator.estimate_username_registration(dpns_name.len(), false);
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

    let qualified_identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };
    tracing::info!(
        identity_id = %qualified_identity.identity.id(),
        identity_index,
        balance = qualified_identity.identity.balance(),
        "Identity registered from platform address credits"
    );

    tracing::info!("Registering DPNS name: {}", dpns_name);

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
