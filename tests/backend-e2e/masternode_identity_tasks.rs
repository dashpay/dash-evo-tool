//! Masternode identity backend E2E tests: TC-084 to TC-090.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::mn_helpers::{MnCredentials, build_mn_identity_input, load_mn_credentials};
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::contested_names::ContestedResourceTask;
use dash_evo_tool::backend_task::identity::{
    IdentityInputToLoad, IdentityTask, RegisterDpnsNameInput,
};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use dash_evo_tool::model::secret::Secret;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;

/// Load a masternode identity from env credentials. Returns `None` if creds missing.
async fn load_mn_from_env(identity_type: IdentityType) -> Option<QualifiedIdentity> {
    let creds = load_mn_credentials()?;
    let ctx = ctx().await;
    let input = build_mn_identity_input(&creds, identity_type);
    let task = BackendTask::IdentityTask(IdentityTask::LoadIdentity(input));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("load_mn_from_env: LoadIdentity failed");
    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => Some(qi),
        other => panic!(
            "load_mn_from_env: expected LoadedIdentity, got: {:?}",
            other
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-084: Load Masternode Identity (all keys)
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_084_load_masternode_identity() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping TC-084: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;

    let input = build_mn_identity_input(&creds, IdentityType::Masternode);
    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("TC-084: LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                "TC-084: loaded MN identity {:?}, type={}, keys={}",
                qi.identity.id(),
                qi.identity_type,
                qi.private_keys.private_keys.len()
            );
            assert_eq!(
                qi.identity_type,
                IdentityType::Masternode,
                "TC-084: identity_type should be Masternode"
            );
            if creds.voting_key.is_some() {
                assert!(
                    qi.associated_voter_identity.is_some(),
                    "TC-084: voter identity should be present when voting key provided"
                );
                let has_voter_key = qi
                    .private_keys
                    .private_keys
                    .keys()
                    .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity);
                assert!(
                    has_voter_key,
                    "TC-084: private_keys should contain PrivateKeyOnVoterIdentity entry"
                );
            }
            if creds.owner_key.is_some() || creds.payout_key.is_some() {
                let has_main_key = qi
                    .private_keys
                    .private_keys
                    .keys()
                    .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnMainIdentity);
                assert!(
                    has_main_key,
                    "TC-084: private_keys should contain PrivateKeyOnMainIdentity entry"
                );
            }
        }
        other => panic!("TC-084: expected LoadedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-085: Load Evonode Identity
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_085_load_evonode_identity() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping TC-085: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;

    let input = build_mn_identity_input(&creds, IdentityType::Evonode);
    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("TC-085: LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                "TC-085: loaded Evonode identity {:?}, type={}",
                qi.identity.id(),
                qi.identity_type
            );
            assert_eq!(
                qi.identity_type,
                IdentityType::Evonode,
                "TC-085: identity_type should be Evonode"
            );
            if creds.voting_key.is_some() {
                assert!(
                    qi.associated_voter_identity.is_some(),
                    "TC-085: voter identity should be present when voting key provided"
                );
            }
        }
        other => panic!("TC-085: expected LoadedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-086: Load MN with Voting Key Only
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_086_load_mn_voting_key_only() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping TC-086: E2E_MN_PROTX_HASH not set");
        return;
    };
    if creds.voting_key.is_none() {
        tracing::warn!("Skipping TC-086: E2E_MN_VOTING_KEY not set");
        return;
    };
    let ctx = ctx().await;

    // Build input with only voting key — strip owner/payout to test partial load.
    let voting_only_creds = MnCredentials {
        protx_hash: creds.protx_hash.clone(),
        voting_key: creds.voting_key.clone(),
        owner_key: None,
        payout_key: None,
    };
    let input = build_mn_identity_input(&voting_only_creds, IdentityType::Masternode);

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("TC-086: LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                "TC-086: loaded MN (voting only) {:?}, voter={:?}",
                qi.identity.id(),
                qi.associated_voter_identity.as_ref().map(|(id, _)| id.id())
            );
            assert!(
                qi.associated_voter_identity.is_some(),
                "TC-086: voter identity should be present"
            );
            let has_main_key = qi
                .private_keys
                .private_keys
                .keys()
                .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnMainIdentity);
            assert!(
                !has_main_key,
                "TC-086: should have no PrivateKeyOnMainIdentity entries (no owner/payout keys)"
            );
            let has_voter_key = qi
                .private_keys
                .private_keys
                .keys()
                .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity);
            assert!(
                has_voter_key,
                "TC-086: should have PrivateKeyOnVoterIdentity entry"
            );
        }
        other => panic!("TC-086: expected LoadedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-087: Refresh MN Identity
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_087_refresh_mn_identity() {
    let Some(qi) = load_mn_from_env(IdentityType::Masternode).await else {
        tracing::warn!("Skipping TC-087: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;
    let original_id = qi.identity.id();

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi)),
    )
    .await
    .expect("TC-087: RefreshIdentity should succeed");

    match result {
        BackendTaskSuccessResult::RefreshedIdentity(refreshed) => {
            tracing::info!(
                "TC-087: refreshed MN {:?}, type={}",
                refreshed.identity.id(),
                refreshed.identity_type
            );
            assert_eq!(
                refreshed.identity.id(),
                original_id,
                "TC-087: identity ID should be unchanged after refresh"
            );
            assert_eq!(
                refreshed.identity_type,
                IdentityType::Masternode,
                "TC-087: identity_type should be preserved as Masternode"
            );
        }
        other => panic!("TC-087: expected RefreshedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-088: Load MN with Invalid ProTx (error case)
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_088_load_mn_invalid_protx() {
    let ctx = ctx().await;

    let fake_protx = "0".repeat(64);
    let input = IdentityInputToLoad {
        identity_id_input: fake_protx,
        identity_type: IdentityType::Masternode,
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
        "TC-088: loading identity with all-zeros ProTx should return an error, got: {:?}",
        result
    );
    tracing::info!("TC-088: got expected error: {}", result.unwrap_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-089: Load MN with Wrong Voting Key (error case)
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_089_load_mn_wrong_voting_key() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping TC-089: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;

    let random_bytes: [u8; 32] = rand::random();
    let fake_key = dash_sdk::dpp::dashcore::PrivateKey::from_byte_array(
        &random_bytes,
        dash_sdk::dpp::dashcore::Network::Testnet,
    )
    .expect("valid random private key");
    let fake_wif = fake_key.to_wif();

    let input = IdentityInputToLoad {
        identity_id_input: creds.protx_hash.clone(),
        identity_type: IdentityType::Masternode,
        alias_input: String::new(),
        voting_private_key_input: Secret::new(&fake_wif),
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
        "TC-089: loading MN with wrong voting key should return an error, got: {:?}",
        result
    );
    tracing::info!("TC-089: got expected error: {}", result.unwrap_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-090: Vote with MN Voter Identity
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_090_vote_with_mn_voter() {
    let Some(mn_qi) = load_mn_from_env(IdentityType::Masternode).await else {
        tracing::warn!("Skipping TC-090: E2E_MN_PROTX_HASH not set");
        return;
    };
    if mn_qi.associated_voter_identity.is_none() {
        tracing::warn!("Skipping TC-090: no voter identity (E2E_MN_VOTING_KEY not set)");
        return;
    }

    let ctx = ctx().await;

    // Step 1: Create a funded test wallet and register a User identity for DPNS.
    tracing::info!("TC-090: creating funded wallet and registering User identity...");
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(30_000_000).await;
    let (reg_info, _key_bytes) =
        build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);
    let reg_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info)),
    )
    .await
    .expect("TC-090: identity registration should succeed");

    let user_qi = match reg_result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => {
            tracing::info!("TC-090: registered User identity {:?}", qi.identity.id());
            qi
        }
        other => panic!("TC-090: expected RegisteredIdentity, got: {:?}", other),
    };

    // Step 2: Register a short contested DPNS name (< 20 chars triggers contest).
    let name = format!("e2emn{:08x}", rand::random::<u32>());
    tracing::info!("TC-090: registering contested DPNS name '{}'...", name);

    let dpns_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
            qualified_identity: user_qi,
            name_input: name.clone(),
        })),
    )
    .await
    .expect("TC-090: DPNS name registration should succeed");

    assert!(
        matches!(dpns_result, BackendTaskSuccessResult::RegisteredDpnsName(_)),
        "TC-090: expected RegisteredDpnsName, got: {:?}",
        dpns_result
    );

    // Step 3: Vote Lock on the contested name using the MN voter identity.
    tracing::info!("TC-090: voting Lock on '{}' with MN voter...", name);

    let vote_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::ContestedResourceTask(ContestedResourceTask::VoteOnDPNSNames(
            vec![(name.clone(), ResourceVoteChoice::Lock)],
            vec![mn_qi],
        )),
    )
    .await
    .expect("TC-090: VoteOnDPNSNames should succeed");

    match vote_result {
        BackendTaskSuccessResult::DPNSVoteResults(results) => {
            tracing::info!("TC-090: vote results = {:?}", results);
            assert!(
                !results.is_empty(),
                "TC-090: should have at least one vote result"
            );
            let (voted_name, choice, outcome) = &results[0];
            assert_eq!(voted_name, &name, "TC-090: voted name mismatch");
            assert_eq!(
                *choice,
                ResourceVoteChoice::Lock,
                "TC-090: vote choice mismatch"
            );
            assert!(
                outcome.is_ok(),
                "TC-090: vote should succeed, got error: {:?}",
                outcome
            );
        }
        other => panic!("TC-090: expected DPNSVoteResults, got: {:?}", other),
    }
}
