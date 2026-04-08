//! IdentityTask backend E2E tests: TC-020 through TC-030.

use crate::framework::fixtures::shared_identity;
use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::run_task;
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

// --- TC-020: TopUpIdentity ---

/// Top up the shared identity from the framework wallet.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_020_top_up_identity() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let top_up_info = IdentityTopUpInfo {
        qualified_identity: si.qualified_identity.clone(),
        wallet: si.wallet_arc.clone(),
        identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
            500_000, 0, // identity_index
            1, // topup_index — use 1 so it doesn't collide with registration
        ),
    };

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentity(top_up_info)),
    )
    .await
    .expect("TopUpIdentity should succeed");

    match result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            tracing::info!(
                "TC-020: topped up {:?}, fee={:?}",
                qi.identity.id(),
                fee_result
            );
            assert_eq!(
                qi.identity.id(),
                si.qualified_identity.identity.id(),
                "TC-020: wrong identity returned"
            );
            assert!(fee_result.actual_fee > 0, "TC-020: fee should be > 0");
        }
        other => panic!("TC-020: expected ToppedUpIdentity, got: {:?}", other),
    }
}

// --- TC-021: TopUpIdentityFromPlatformAddresses ---

/// Top up the shared identity from a funded platform address.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_021_top_up_identity_from_platform_addresses() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    // Derive a platform address from the test wallet's receive address.
    let receive_addr = {
        let mut wallet = si.wallet_arc.write().expect("wallet lock");
        wallet
            .receive_address(Network::Testnet, false, Some(&ctx.app_context))
            .expect("TC-021: failed to derive receive address")
    };

    let platform_addr = PlatformAddress::try_from(receive_addr)
        .expect("TC-021: failed to convert to PlatformAddress");

    // Fund the platform address from the wallet UTXOs (500K duffs).
    let fund_result = run_task(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
            seed_hash: si.wallet_seed_hash,
            amount: 500_000,
            destination: platform_addr,
            fee_deduct_from_output: true,
        }),
    )
    .await
    .expect("TC-021: FundPlatformAddressFromWalletUtxos should succeed");
    assert!(
        matches!(
            fund_result,
            BackendTaskSuccessResult::PlatformAddressFunded { .. }
        ),
        "TC-021: expected PlatformAddressFunded, got: {:?}",
        fund_result
    );

    // Fetch the funded balance so we can use it.
    let balances_result = run_task(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances {
            seed_hash: si.wallet_seed_hash,
        }),
    )
    .await
    .expect("TC-021: FetchPlatformAddressBalances should succeed");

    let balance = match &balances_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => balances
            .get(&platform_addr)
            .map(|(bal, _)| *bal)
            .unwrap_or(0),
        other => panic!("TC-021: expected PlatformAddressBalances, got: {:?}", other),
    };

    tracing::info!("TC-021: platform address balance = {} credits", balance);
    assert!(balance > 0, "TC-021: platform address should have credits");

    let mut inputs = std::collections::BTreeMap::new();
    // Use half the balance to top up the identity.
    inputs.insert(platform_addr, balance / 2);

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses {
            identity: si.qualified_identity.clone(),
            inputs,
            wallet_seed_hash: si.wallet_seed_hash,
        }),
    )
    .await
    .expect("TC-021: TopUpIdentityFromPlatformAddresses should succeed");

    match result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            tracing::info!(
                "TC-021: topped up {:?} from platform address, fee={:?}",
                qi.identity.id(),
                fee_result
            );
            assert_eq!(
                qi.identity.id(),
                si.qualified_identity.identity.id(),
                "TC-021: wrong identity returned"
            );
        }
        other => panic!("TC-021: expected ToppedUpIdentity, got: {:?}", other),
    }
}

// --- TC-022: AddKeyToIdentity ---

/// Add a new authentication key to the shared identity.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_022_add_key_to_identity() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    // Generate a fresh 32-byte private key.
    let private_key_bytes: [u8; 32] = rand::rng().random();

    // Build a new ECDSA_SECP256K1 AUTHENTICATION HIGH public key.
    // Key ID 0 is a placeholder — add_key_to_identity re-assigns the ID.
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
                    .expect("TC-022: invalid secret key");
            let pk = PrivateKey::new(secret_key, Network::Testnet);
            pk.public_key(&secp).to_bytes().into()
        },
        disabled_at: None,
    });

    let new_qualified_key = QualifiedIdentityPublicKey::from(new_ipk);

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(
            si.qualified_identity.clone(),
            new_qualified_key,
            private_key_bytes,
        )),
    )
    .await
    .expect("TC-022: AddKeyToIdentity should succeed");

    match result {
        BackendTaskSuccessResult::AddedKeyToIdentity(fee_result) => {
            tracing::info!("TC-022: added key, fee={:?}", fee_result);
            assert!(fee_result.actual_fee > 0, "TC-022: fee should be > 0");
        }
        other => panic!("TC-022: expected AddedKeyToIdentity, got: {:?}", other),
    }
}

// --- TC-023: Transfer credits to another identity ---

/// Transfer credits from the shared identity to a freshly-registered second identity.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_023_transfer_credits() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    // Register a minimal second identity in-test.
    let (seed_hash_b, wallet_b) = ctx.create_funded_test_wallet(2_000_000).await;
    let (reg_info, _key_bytes_b) =
        build_identity_registration(&ctx.app_context, &wallet_b, seed_hash_b);
    let reg_result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info)),
    )
    .await
    .expect("TC-023: second identity registration should succeed");

    let recipient_id = match reg_result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => {
            tracing::info!("TC-023: registered recipient {:?}", qi.identity.id());
            qi.identity.id()
        }
        other => panic!(
            "TC-023: expected RegisteredIdentity for recipient, got: {:?}",
            other
        ),
    };

    // Transfer 10M credits from shared identity to recipient.
    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::Transfer(
            si.qualified_identity.clone(),
            recipient_id,
            10_000_000,
            None,
        )),
    )
    .await
    .expect("TC-023: Transfer should succeed");

    match result {
        BackendTaskSuccessResult::TransferredCredits(fee_result) => {
            tracing::info!("TC-023: transfer succeeded, fee={:?}", fee_result);
            assert!(fee_result.actual_fee > 0, "TC-023: fee should be > 0");
        }
        other => panic!("TC-023: expected TransferredCredits, got: {:?}", other),
    }

    // Transfer succeeded — TransferredCredits result with fee > 0 is sufficient evidence.
    tracing::info!("TC-023: transfer verified via TransferredCredits result");
}

// --- TC-024: TransferToAddresses ---

/// Transfer credits from the shared identity to a Platform address.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_024_transfer_to_addresses() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    // Derive a platform address from the test wallet.
    let receive_addr = {
        let mut wallet = si.wallet_arc.write().expect("wallet lock");
        wallet
            .receive_address(Network::Testnet, false, Some(&ctx.app_context))
            .expect("TC-024: failed to derive receive address")
    };

    let platform_addr = PlatformAddress::try_from(receive_addr)
        .expect("TC-024: failed to convert to PlatformAddress");

    let mut outputs = std::collections::BTreeMap::new();
    outputs.insert(platform_addr, 5_000_000u64);

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TransferToAddresses {
            identity: si.qualified_identity.clone(),
            outputs,
            key_id: None,
        }),
    )
    .await
    .expect("TC-024: TransferToAddresses should succeed");

    match result {
        BackendTaskSuccessResult::TransferredCredits(fee_result) => {
            tracing::info!(
                "TC-024: transfer to address succeeded, fee={:?}",
                fee_result
            );
        }
        other => panic!("TC-024: expected TransferredCredits, got: {:?}", other),
    }
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

    // The task returns either a found identity or a message indicating none found.
    // Either is acceptable — we just verify it didn't panic.
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

    // SearchIdentitiesUpToIndex may produce multiple `Progress` messages via the
    // sender channel and return a final `Message` or `LoadedIdentity`. We assert
    // only that the final return is not an error.
    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::SearchIdentitiesUpToIndex(wallet_ref, 5)),
    )
    .await
    .expect("TC-029: SearchIdentitiesUpToIndex should not error");

    tracing::info!("TC-029: result = {:?}", result);
    // Any non-error success result is valid.
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
