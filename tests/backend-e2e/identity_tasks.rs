//! IdentityTask backend E2E tests: TC-020 through TC-030.

use crate::framework::fixtures::shared_identity;
use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::identity::{
    IdentityInputToLoad, IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod,
};
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::IdentityType;
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::wallet::WalletArcRef;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use rand::prelude::*;

// --- TC-020: Identity mutation lifecycle ---

async fn step_top_up(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 1: TopUpIdentity from wallet ===");

    let top_up_info = IdentityTopUpInfo {
        qualified_identity: si.qualified_identity.clone(),
        wallet: si.wallet_arc.clone(),
        identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
            500_000, 0, // identity_index
            1, // topup_index — use 1 so it doesn't collide with registration
        ),
    };

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentity(top_up_info)),
    )
    .await
    .expect("TopUpIdentity should succeed");

    match result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            tracing::info!("topped up {:?}, fee={:?}", qi.identity.id(), fee_result);
            assert_eq!(
                qi.identity.id(),
                si.qualified_identity.identity.id(),
                "wrong identity returned"
            );
            assert!(fee_result.actual_fee > 0, "fee should be > 0");
        }
        other => panic!("expected ToppedUpIdentity, got: {:?}", other),
    }
}

async fn step_top_up_from_platform_addresses(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 2: TopUpIdentity from platform addresses ===");

    let receive_addr = {
        let mut wallet = si.wallet_arc.write().expect("wallet lock");
        wallet
            .receive_address(Network::Testnet, false, Some(&ctx.app_context))
            .expect("failed to derive receive address")
    };

    let platform_addr =
        PlatformAddress::try_from(receive_addr).expect("failed to convert to PlatformAddress");

    let fund_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
            seed_hash: si.wallet_seed_hash,
            amount: 200_000,
            destination: platform_addr,
            fee_deduct_from_output: true,
        }),
    )
    .await
    .expect("FundPlatformAddressFromWalletUtxos should succeed");
    assert!(
        matches!(
            fund_result,
            BackendTaskSuccessResult::PlatformAddressFunded { .. }
        ),
        "expected PlatformAddressFunded, got: {:?}",
        fund_result
    );

    // Platform needs time to process the funding tx in a block. On testnet,
    // blocks are ~2.5 min so allow up to 360s (two full block intervals plus
    // propagation/processing margin).
    let poll_timeout = std::time::Duration::from_secs(360);
    let poll_interval = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();

    // Reset platform sync state so incremental sync doesn't skip the newly
    // funded address (the previous sync checkpoint may be past the funding tx).
    if let Err(e) = ctx
        .app_context
        .db()
        .set_platform_sync_info(&si.wallet_seed_hash, 0, 0)
    {
        tracing::warn!("Failed to reset platform sync info: {}", e);
    }

    let balance = loop {
        let balances_result = run_task(
            &ctx.app_context,
            BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances {
                seed_hash: si.wallet_seed_hash,
            }),
        )
        .await
        .expect("FetchPlatformAddressBalances should succeed");

        let bal = match &balances_result {
            BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
                balances.get(&platform_addr).map(|(b, _)| *b).unwrap_or(0)
            }
            other => panic!("expected PlatformAddressBalances, got: {:?}", other),
        };

        if bal > 0 {
            tracing::info!("platform address balance = {} credits", bal);
            break bal;
        }

        if start.elapsed() > poll_timeout {
            panic!(
                "platform address should have credits (waited {:?})",
                poll_timeout
            );
        }

        tracing::info!("balance still 0, retrying in {:?}...", poll_interval);
        tokio::time::sleep(poll_interval).await;
    };

    let mut inputs = std::collections::BTreeMap::new();
    inputs.insert(platform_addr, balance / 2);

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses {
            identity: si.qualified_identity.clone(),
            inputs,
            wallet_seed_hash: si.wallet_seed_hash,
        }),
    )
    .await
    .expect("TopUpIdentityFromPlatformAddresses should succeed");

    match result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            tracing::info!(
                "topped up {:?} from platform address, fee={:?}",
                qi.identity.id(),
                fee_result
            );
            assert_eq!(
                qi.identity.id(),
                si.qualified_identity.identity.id(),
                "wrong identity returned"
            );
        }
        other => panic!("expected ToppedUpIdentity, got: {:?}", other),
    }
}

async fn step_add_key(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 3: AddKeyToIdentity ===");

    let private_key_bytes: [u8; 32] = rand::rng().random();

    let new_ipk = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::HIGH,
        contract_bounds: None,
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: {
            use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
            use dash_sdk::dpp::dashcore::PrivateKey;
            let secp = Secp256k1::new();
            let secret_key =
                dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&private_key_bytes)
                    .expect("invalid secret key");
            let pk = PrivateKey::new(secret_key, Network::Testnet);
            pk.public_key(&secp).to_bytes().into()
        },
        disabled_at: None,
    });

    let new_qualified_key = QualifiedIdentityPublicKey::from(new_ipk);

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(
            si.qualified_identity.clone(),
            new_qualified_key,
            private_key_bytes,
        )),
    )
    .await
    .expect("AddKeyToIdentity should succeed");

    match result {
        BackendTaskSuccessResult::AddedKeyToIdentity(fee_result) => {
            tracing::info!("added key, fee={:?}", fee_result);
            assert!(fee_result.actual_fee > 0, "fee should be > 0");
        }
        other => panic!("expected AddedKeyToIdentity, got: {:?}", other),
    }
}

async fn step_transfer_credits(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 4: Transfer credits to another identity ===");

    let (seed_hash_b, wallet_b) = ctx.create_funded_test_wallet(30_000_000).await;
    let (reg_info, _key_bytes_b) =
        build_identity_registration(&ctx.app_context, &wallet_b, seed_hash_b);
    let reg_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info)),
    )
    .await
    .expect("second identity registration should succeed");

    let recipient_id = match reg_result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => {
            tracing::info!("registered recipient {:?}", qi.identity.id());
            qi.identity.id()
        }
        other => panic!(
            "expected RegisteredIdentity for recipient, got: {:?}",
            other
        ),
    };

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::Transfer(
            si.qualified_identity.clone(),
            recipient_id,
            10_000_000,
            None,
        )),
    )
    .await
    .expect("Transfer should succeed");

    match result {
        BackendTaskSuccessResult::TransferredCredits(fee_result) => {
            tracing::info!(
                "transfer succeeded, estimated_fee={}, actual_fee={}",
                fee_result.estimated_fee,
                fee_result.actual_fee
            );
        }
        other => panic!("expected TransferredCredits, got: {:?}", other),
    }

    tracing::info!("transfer verified via TransferredCredits result");
}

async fn step_transfer_to_addresses(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 5: TransferToAddresses ===");

    let receive_addr = {
        let mut wallet = si.wallet_arc.write().expect("wallet lock");
        wallet
            .receive_address(Network::Testnet, false, Some(&ctx.app_context))
            .expect("failed to derive receive address")
    };

    let platform_addr =
        PlatformAddress::try_from(receive_addr).expect("failed to convert to PlatformAddress");

    let mut outputs = std::collections::BTreeMap::new();
    outputs.insert(platform_addr, 5_000_000u64);

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TransferToAddresses {
            identity: si.qualified_identity.clone(),
            outputs,
            key_id: None,
        }),
    )
    .await
    .expect("TransferToAddresses should succeed");

    match result {
        BackendTaskSuccessResult::TransferredCredits(fee_result) => {
            tracing::info!("transfer to address succeeded, fee={:?}", fee_result);
        }
        other => panic!("expected TransferredCredits, got: {:?}", other),
    }
}

/// Identity mutation lifecycle: top-up from wallet, top-up from platform addresses,
/// add key, transfer credits, transfer to addresses.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_020_identity_mutation_lifecycle() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    step_top_up(ctx, si).await;
    step_top_up_from_platform_addresses(ctx, si).await;
    step_add_key(ctx, si).await;
    step_transfer_credits(ctx, si).await;
    step_transfer_to_addresses(ctx, si).await;
}

// --- TC-025: RefreshIdentity ---

/// Refresh the shared identity from Platform and verify balance > 0.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_025_refresh_identity() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(si.qualified_identity.clone())),
    )
    .await
    .expect("TC-025: RefreshIdentity should succeed");

    match result {
        BackendTaskSuccessResult::RefreshedIdentity(qi) => {
            tracing::info!(
                "TC-025: refreshed {:?}, balance={}",
                qi.identity.id(),
                qi.identity.balance()
            );
            assert_eq!(
                qi.identity.id(),
                si.qualified_identity.identity.id(),
                "TC-025: wrong identity returned"
            );
            assert!(qi.identity.balance() > 0, "TC-025: balance should be > 0");
        }
        other => panic!("TC-025: expected RefreshedIdentity, got: {:?}", other),
    }
}

// --- TC-026: RefreshLoadedIdentitiesOwnedDPNSNames ---

/// Refresh DPNS names owned by loaded identities (no DPNS name required — just no error).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_026_refresh_loaded_identities_dpns_names() {
    let ctx = ctx().await;
    // Ensure shared identity is in the DB before refreshing names.
    let _si = shared_identity().await;

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames),
    )
    .await
    .expect("TC-026: RefreshLoadedIdentitiesOwnedDPNSNames should succeed");

    assert!(
        matches!(result, BackendTaskSuccessResult::RefreshedOwnedDpnsNames),
        "TC-026: expected RefreshedOwnedDpnsNames, got: {:?}",
        result
    );
    tracing::info!("TC-026: RefreshedOwnedDpnsNames OK");
}

// --- TC-027: LoadIdentity ---

/// Load the shared identity from Platform by its Base58 ID.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_027_load_identity() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let identity_id_base58 = si
        .qualified_identity
        .identity
        .id()
        .to_string(Encoding::Base58);

    let input = IdentityInputToLoad {
        identity_id_input: identity_id_base58,
        identity_type: IdentityType::User,
        alias_input: String::new(),
        voting_private_key_input: Secret::new(""),
        owner_private_key_input: Secret::new(""),
        payout_address_private_key_input: Secret::new(""),
        keys_input: vec![],
        derive_keys_from_wallets: true,
        selected_wallet_seed_hash: Some(si.wallet_seed_hash),
    };

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("TC-027: LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                "TC-027: loaded identity {:?}, keys={}",
                qi.identity.id(),
                qi.identity.public_keys().len()
            );
            assert_eq!(
                qi.identity.id(),
                si.qualified_identity.identity.id(),
                "TC-027: wrong identity loaded"
            );
            assert!(
                !qi.identity.public_keys().is_empty(),
                "TC-027: loaded identity should have keys"
            );
        }
        other => panic!("TC-027: expected LoadedIdentity, got: {:?}", other),
    }
}

// --- TC-028: SearchIdentityFromWallet ---

/// Search for an identity at index 0 from the shared identity's wallet.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_028_search_identity_from_wallet() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let wallet_ref = WalletArcRef::from(si.wallet_arc.clone());

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::SearchIdentityFromWallet(wallet_ref, 0)),
    )
    .await
    .expect("TC-028: SearchIdentityFromWallet should not error");

    match &result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!("TC-028: found identity {:?}", qi.identity.id());
        }
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => {
            tracing::info!("TC-028: found identity (registered) {:?}", qi.identity.id());
        }
        BackendTaskSuccessResult::Message(msg) => {
            tracing::info!("TC-028: no identity at index 0: {}", msg);
        }
        other => panic!("TC-028: unexpected result: {:?}", other),
    }
}

// --- TC-029: SearchIdentitiesUpToIndex ---

/// Search for identities up to index 5 in the shared identity's wallet.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_029_search_identities_up_to_index() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let wallet_ref = WalletArcRef::from(si.wallet_arc.clone());

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::SearchIdentitiesUpToIndex(wallet_ref, 5)),
    )
    .await
    .expect("TC-029: SearchIdentitiesUpToIndex should not error");

    tracing::info!("TC-029: result = {:?}", result);
}

// --- TC-030: LoadIdentity error — nonexistent identity ---

/// Attempt to load a randomly-generated (nonexistent) identity — expect an error.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_030_load_nonexistent_identity() {
    let ctx = ctx().await;

    let nonexistent_id = Identifier::random();
    let id_str = nonexistent_id.to_string(Encoding::Base58);
    tracing::info!("TC-030: attempting to load nonexistent identity {}", id_str);

    let input = IdentityInputToLoad {
        identity_id_input: id_str,
        identity_type: IdentityType::User,
        alias_input: String::new(),
        voting_private_key_input: Secret::new(""),
        owner_private_key_input: Secret::new(""),
        payout_address_private_key_input: Secret::new(""),
        keys_input: vec![],
        derive_keys_from_wallets: false,
        selected_wallet_seed_hash: None,
    };

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await;

    assert!(
        result.is_err(),
        "TC-030: loading a nonexistent identity should return an error, got: {:?}",
        result
    );
    tracing::info!("TC-030: got expected error: {:?}", result.unwrap_err());
}
