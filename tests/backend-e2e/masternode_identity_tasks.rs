//! Masternode identity backend E2E tests: TC-084 to TC-090.
//!
//! Tests run in parallel — each is fully independent. Tracing spans
//! (`#[tracing::instrument]`) tag every log line with the test name,
//! making interleaved output easy to filter.

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
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::query_types::ContestedResource;

/// Load a masternode identity from env credentials. Returns `None` if creds missing.
#[tracing::instrument(skip_all, fields(identity_type = %identity_type))]
async fn load_mn_from_env(identity_type: IdentityType) -> Option<QualifiedIdentity> {
    let creds = load_mn_credentials()?;
    let ctx = ctx().await;
    let input = build_mn_identity_input(&creds, identity_type);
    let task = BackendTask::IdentityTask(IdentityTask::LoadIdentity(input));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("LoadIdentity failed");
    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => Some(qi),
        other => panic!("expected LoadedIdentity, got: {:?}", other),
    }
}

/// Generate a DPNS label that is guaranteed to match the contested-resource
/// regex `^[a-zA-Z01-]{3,19}$` — i.e. 3–19 chars drawn only from
/// `[a-zA-Z01-]`. Only names matching this regex trigger a vote contest;
/// length alone (< 20 chars) is not sufficient.
///
/// Prefix is a recognisable tag so humans browsing testnet state can tell
/// these are test-generated names; suffix is random from `[a-z01]`.
fn generate_contested_dpns_name() -> String {
    // `e0emn` replaces the previous `e2emn` — `2` is not in the contested
    // alphabet, so the old prefix slipped names past the regex.
    const PREFIX: &str = "e0emn";
    const POOL: &[u8] = b"abcdefghijklmnopqrstuvwxyz01";
    const SUFFIX_LEN: usize = 8;
    let suffix: String = (0..SUFFIX_LEN)
        .map(|_| POOL[rand::random::<u8>() as usize % POOL.len()] as char)
        .collect();
    // 5 + 8 = 13 chars, comfortably within the 3..=19 bound.
    format!("{PREFIX}{suffix}")
}

/// Poll Platform until the vote poll for `(dash, normalized_label)` exists on
/// the DPNS contract, up to `timeout`. Panics with a diagnostic message if the
/// contest does not appear in time — this catches regressions where the
/// generator produces a name that silently registers as an uncontested domain
/// document, which would otherwise surface as an opaque voting timeout.
async fn wait_for_dpns_contest(sdk: &dash_sdk::Sdk, label: &str, timeout: std::time::Duration) {
    let platform_version = sdk.version();
    let dpns_contract = load_system_data_contract(SystemDataContract::DPNS, platform_version)
        .expect("load DPNS system contract");
    let normalized = convert_to_homograph_safe_chars(label);
    let poll_interval = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();

    loop {
        let query = VotePollsByDocumentTypeQuery {
            contract_id: dpns_contract.id(),
            document_type_name: "domain".to_string(),
            index_name: "parentNameAndLabel".to_string(),
            // Start exactly at our normalized label (inclusive) so a single
            // returned row is enough to confirm the contest exists.
            start_at_value: Some((Value::Text(normalized.clone()), true)),
            start_index_values: vec!["dash".into()],
            end_index_values: vec![],
            limit: Some(1),
            order_ascending: true,
        };

        match ContestedResource::fetch_many(sdk, query).await {
            Ok(resources) => {
                let found = resources
                    .0
                    .iter()
                    .any(|r| r.0.as_str() == Some(&normalized));
                if found {
                    tracing::info!(
                        elapsed_ms = start.elapsed().as_millis(),
                        label = label,
                        normalized = normalized.as_str(),
                        "contested vote poll is live",
                    );
                    return;
                }
            }
            Err(e) => {
                tracing::warn!("contested-resource query failed (will retry): {}", e);
            }
        }

        if start.elapsed() >= timeout {
            panic!(
                "DPNS name {label} registered but did not enter contest state within {:?} — \
                 check that the name matches regex ^[a-zA-Z01-]{{3,19}}$",
                timeout,
            );
        }
        tokio::time::sleep(poll_interval).await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-084: Load Masternode Identity (all keys)
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn tc_084_load_masternode_identity() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;

    let input = build_mn_identity_input(&creds, IdentityType::Masternode);
    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                id = ?qi.identity.id(),
                identity_type = %qi.identity_type,
                key_count = qi.private_keys.private_keys.len(),
                "loaded MN identity"
            );
            assert_eq!(
                qi.identity_type,
                IdentityType::Masternode,
                "identity_type should be Masternode"
            );
            if creds.voting_key.is_some() {
                assert!(
                    qi.associated_voter_identity.is_some(),
                    "voter identity should be present when voting key provided"
                );
                let has_voter_key = qi
                    .private_keys
                    .private_keys
                    .keys()
                    .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity);
                assert!(
                    has_voter_key,
                    "private_keys should contain PrivateKeyOnVoterIdentity entry"
                );
            }
            if creds.owner_key.is_some() {
                let has_main_key = qi
                    .private_keys
                    .private_keys
                    .keys()
                    .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnMainIdentity);
                assert!(
                    has_main_key,
                    "private_keys should contain PrivateKeyOnMainIdentity entry"
                );
            }
        }
        other => panic!("expected LoadedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-085: Load Evonode Identity
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn tc_085_load_evonode_identity() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;

    let input = build_mn_identity_input(&creds, IdentityType::Evonode);
    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                id = ?qi.identity.id(),
                identity_type = %qi.identity_type,
                "loaded Evonode identity"
            );
            assert_eq!(
                qi.identity_type,
                IdentityType::Evonode,
                "identity_type should be Evonode"
            );
            if creds.voting_key.is_some() {
                assert!(
                    qi.associated_voter_identity.is_some(),
                    "voter identity should be present when voting key provided"
                );
            }
        }
        other => panic!("expected LoadedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-086: Load MN with Voting Key Only
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn tc_086_load_mn_voting_key_only() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping: E2E_MN_PROTX_HASH not set");
        return;
    };
    if creds.voting_key.is_none() {
        tracing::warn!("Skipping: E2E_MN_VOTING_KEY not set");
        return;
    };
    let ctx = ctx().await;

    // Build input with only voting key — strip owner to test partial load.
    let voting_only_creds = MnCredentials {
        protx_hash: creds.protx_hash.clone(),
        voting_key: creds.voting_key.clone(),
        owner_key: None,
    };
    let input = build_mn_identity_input(&voting_only_creds, IdentityType::Masternode);

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
    )
    .await
    .expect("LoadIdentity should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(qi) => {
            tracing::info!(
                id = ?qi.identity.id(),
                voter = ?qi.associated_voter_identity.as_ref().map(|(id, _)| id.id()),
                "loaded MN (voting only)"
            );
            assert!(
                qi.associated_voter_identity.is_some(),
                "voter identity should be present"
            );
            let has_main_key = qi
                .private_keys
                .private_keys
                .keys()
                .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnMainIdentity);
            assert!(
                !has_main_key,
                "should have no PrivateKeyOnMainIdentity entries (no owner keys)"
            );
            let has_voter_key = qi
                .private_keys
                .private_keys
                .keys()
                .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity);
            assert!(has_voter_key, "should have PrivateKeyOnVoterIdentity entry");
        }
        other => panic!("expected LoadedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-087: Refresh MN Identity
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn tc_087_refresh_mn_identity() {
    let Some(qi) = load_mn_from_env(IdentityType::Masternode).await else {
        tracing::warn!("Skipping: E2E_MN_PROTX_HASH not set");
        return;
    };
    let ctx = ctx().await;
    let original_id = qi.identity.id();

    let result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi)),
    )
    .await
    .expect("RefreshIdentity should succeed");

    match result {
        BackendTaskSuccessResult::RefreshedIdentity(refreshed) => {
            tracing::info!(
                id = ?refreshed.identity.id(),
                identity_type = %refreshed.identity_type,
                "refreshed MN identity"
            );
            assert_eq!(
                refreshed.identity.id(),
                original_id,
                "identity ID should be unchanged after refresh"
            );
            assert_eq!(
                refreshed.identity_type,
                IdentityType::Masternode,
                "identity_type should be preserved as Masternode"
            );
        }
        other => panic!("expected RefreshedIdentity, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-088: Load MN with Invalid ProTx (error case)
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
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
        "loading identity with all-zeros ProTx should return an error, got: {:?}",
        result
    );
    tracing::info!(error = %result.unwrap_err(), "got expected error");
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-089: Load MN with Wrong Voting Key (error case)
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn tc_089_load_mn_wrong_voting_key() {
    let Some(creds) = load_mn_credentials() else {
        tracing::warn!("Skipping: E2E_MN_PROTX_HASH not set");
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
        "loading MN with wrong voting key should return an error, got: {:?}",
        result
    );
    tracing::info!(error = %result.unwrap_err(), "got expected error");
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-090: Vote with MN Voter Identity
// ─────────────────────────────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn tc_090_vote_with_mn_voter() {
    let Some(mn_qi) = load_mn_from_env(IdentityType::Masternode).await else {
        tracing::warn!("Skipping: E2E_MN_PROTX_HASH not set");
        return;
    };
    if mn_qi.associated_voter_identity.is_none() {
        tracing::warn!("Skipping: no voter identity (E2E_MN_VOTING_KEY not set)");
        return;
    }

    let ctx = ctx().await;

    // Step 1: Create a funded test wallet and register a User identity for DPNS.
    tracing::info!("creating funded wallet and registering User identity...");
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(30_000_000).await;
    let (reg_info, _key_bytes) =
        build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);
    let reg_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info)),
    )
    .await
    .expect("identity registration should succeed");

    let user_qi = match reg_result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => {
            tracing::info!(id = ?qi.identity.id(), "registered User identity");
            qi
        }
        other => panic!("expected RegisteredIdentity, got: {:?}", other),
    };

    // Step 2: Register a short DPNS name composed only of [a-zA-Z01-] so the
    // normalizedLabel matches the contested regex ^[a-zA-Z01-]{3,19}$ and a
    // masternode vote contest is triggered. Both length AND alphabet must
    // match — < 20 chars alone is not sufficient.
    let name = generate_contested_dpns_name();
    tracing::info!(name = %name, "registering contested DPNS name");

    let dpns_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
            qualified_identity: user_qi,
            name_input: name.clone(),
        })),
    )
    .await
    .expect("DPNS name registration should succeed");

    assert!(
        matches!(dpns_result, BackendTaskSuccessResult::RegisteredDpnsName(_)),
        "expected RegisteredDpnsName, got: {:?}",
        dpns_result
    );

    // Step 2b: Precheck — wait until the vote poll actually exists on
    // Platform before attempting to vote. DPNS registration → contest state
    // can take a block or two; failing fast here (instead of waiting out the
    // opaque vote timeout) makes regressions in the name generator or
    // registration path obvious.
    tracing::info!(name = %name, "waiting for DPNS contest to go live");
    wait_for_dpns_contest(
        &ctx.app_context.sdk(),
        &name,
        std::time::Duration::from_secs(30),
    )
    .await;

    // Step 3: Vote Lock on the contested name using the MN voter identity.
    tracing::info!(name = %name, "voting Lock with MN voter");

    let vote_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::ContestedResourceTask(ContestedResourceTask::VoteOnDPNSNames(
            vec![(name.clone(), ResourceVoteChoice::Lock)],
            vec![mn_qi],
        )),
    )
    .await
    .expect("VoteOnDPNSNames should succeed");

    match vote_result {
        BackendTaskSuccessResult::DPNSVoteResults(results) => {
            tracing::info!(?results, "vote completed");
            assert!(!results.is_empty(), "should have at least one vote result");
            let (voted_name, choice, outcome) = &results[0];
            assert_eq!(voted_name, &name, "voted name mismatch");
            assert_eq!(*choice, ResourceVoteChoice::Lock, "vote choice mismatch");
            assert!(
                outcome.is_ok(),
                "vote should succeed, got error: {:?}",
                outcome
            );
        }
        other => panic!("expected DPNSVoteResults, got: {:?}", other),
    }
}
