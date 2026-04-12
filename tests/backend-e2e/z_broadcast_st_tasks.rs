//! BroadcastStateTransition backend E2E tests: TC-066 and TC-067.
//!
//! TC-066: Build a valid IdentityUpdateTransition, sign it, and broadcast via
//!         BackendTask::BroadcastStateTransition. Assert BroadcastedStateTransition.
//!         Then build an invalid state transition (wrong nonce) and assert Err(TaskError::...).

use crate::framework::fixtures::shared_identity;
use crate::framework::harness::ctx;
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity;
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::platform::{Fetch, IdentityPublicKey};

// --- TC-066 step functions ---

async fn step_broadcast_valid(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 1: broadcast valid IdentityUpdateTransition ===");

    let platform_version = ctx.app_context.platform_version();
    let identity_id = si.qualified_identity.identity.id();

    // Fetch the current identity from Platform so we have the latest public
    // keys and revision (other tests may have added keys since registration).
    let sdk = ctx.app_context.sdk();
    let mut identity = dash_sdk::platform::Identity::fetch_by_identifier(&sdk, identity_id)
        .await
        .expect("failed to fetch identity from Platform")
        .expect("identity not found on Platform");

    let new_private_key_bytes: [u8; 32] = rand::random();

    let new_public_key_data = {
        use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
        use dash_sdk::dpp::dashcore::PrivateKey;
        let secp = Secp256k1::new();
        let secret_key =
            dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&new_private_key_bytes)
                .expect("invalid secret key bytes");
        let pk = PrivateKey::new(secret_key, Network::Testnet);
        pk.public_key(&secp).to_bytes()
    };

    let mut new_ipk = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::HIGH,
        contract_bounds: None,
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: new_public_key_data.into(),
        disabled_at: None,
    });
    new_ipk.set_id(identity.get_public_key_max_id() + 1);

    // Bump revision to match what Platform expects after the update.
    identity.bump_revision();

    let nonce = sdk
        .get_identity_nonce(identity_id, true, None)
        .await
        .expect("failed to fetch identity nonce from Platform");
    tracing::info!("identity nonce = {}", nonce);

    let master_key_id = identity
        .public_keys()
        .values()
        .find(|k| {
            k.purpose() == Purpose::AUTHENTICATION && k.security_level() == SecurityLevel::MASTER
        })
        .expect("shared identity has no MASTER AUTHENTICATION key")
        .id();

    // Build a mutable copy of the qualified identity with the fresh Platform
    // state and the new key's private key registered in the key storage.
    // The signer implementation looks up private keys from `private_keys` when
    // signing the new key, so it must be present before calling
    // `try_from_identity_with_signer`.
    let mut qi = si.qualified_identity.clone();
    qi.identity = identity.clone();
    qi.private_keys.insert_non_encrypted(
        (PrivateKeyOnMainIdentity, new_ipk.id()),
        (
            QualifiedIdentityPublicKey::from(new_ipk.clone()),
            new_private_key_bytes,
        ),
    );

    let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
        &identity,
        &master_key_id,
        vec![new_ipk.clone()],
        vec![],
        nonce,
        UserFeeIncrease::default(),
        &qi,
        platform_version,
        None,
    )
    .expect("failed to build IdentityUpdateTransition");

    tracing::info!("state transition built and signed, broadcasting...");

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::BroadcastStateTransition(state_transition),
    )
    .await
    .expect("BroadcastStateTransition should succeed");

    assert!(
        matches!(result, BackendTaskSuccessResult::BroadcastedStateTransition),
        "expected BroadcastedStateTransition, got: {:?}",
        result
    );
    tracing::info!("broadcast succeeded");

    // Brief delay for DAPI propagation — broadcast confirms on one node but
    // a different node may serve the re-fetch before processing the same block.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let fetched = dash_sdk::platform::Identity::fetch_by_identifier(&sdk, identity_id)
        .await
        .expect("failed to re-fetch identity")
        .expect("identity not found on Platform after broadcast");

    let has_new_key = fetched
        .public_keys()
        .values()
        .any(|k| k.data() == new_ipk.data());

    assert!(
        has_new_key,
        "New key NOT found on Platform after broadcast. \
         Fetched {} keys, expected new key with id {}. \
         The broadcast succeeded, so the key should be visible.",
        fetched.public_keys().len(),
        new_ipk.id(),
    );
    tracing::info!("new key confirmed on Platform");
}

async fn step_broadcast_invalid(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 2: broadcast invalid IdentityUpdateTransition (bad nonce) ===");

    let platform_version = ctx.app_context.platform_version();

    let refresh_result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(si.qualified_identity.clone())),
    )
    .await
    .expect("RefreshIdentity should succeed");

    assert!(
        matches!(
            refresh_result,
            BackendTaskSuccessResult::RefreshedIdentity(_)
        ),
        "expected RefreshedIdentity, got: {:?}",
        refresh_result
    );

    let identity_id = si.qualified_identity.identity.id();
    let mut refreshed_qi = ctx
        .app_context
        .load_local_qualified_identities()
        .expect("load_local_qualified_identities should succeed")
        .into_iter()
        .find(|qi| qi.identity.id() == identity_id)
        .expect("shared identity should be in local DB after refresh");
    refreshed_qi.identity.bump_revision();
    let identity = &refreshed_qi.identity;

    let new_private_key_bytes: [u8; 32] = rand::random();
    let new_public_key_data = {
        use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
        use dash_sdk::dpp::dashcore::PrivateKey;
        let secp = Secp256k1::new();
        let secret_key =
            dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&new_private_key_bytes)
                .expect("invalid secret key bytes");
        let pk = PrivateKey::new(secret_key, Network::Testnet);
        pk.public_key(&secp).to_bytes()
    };

    let mut new_ipk = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::HIGH,
        contract_bounds: None,
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: new_public_key_data.into(),
        disabled_at: None,
    });
    new_ipk.set_id(identity.get_public_key_max_id() + 1);

    let invalid_nonce: u64 = u64::MAX;

    let master_key_id = identity
        .public_keys()
        .values()
        .find(|k| {
            k.purpose() == Purpose::AUTHENTICATION && k.security_level() == SecurityLevel::MASTER
        })
        .expect("refreshed identity has no MASTER AUTHENTICATION key")
        .id();

    // Register the new key's private key in the signer so
    // try_from_identity_with_signer can sign it.
    let mut signer_qi = refreshed_qi.clone();
    signer_qi.private_keys.insert_non_encrypted(
        (PrivateKeyOnMainIdentity, new_ipk.id()),
        (
            QualifiedIdentityPublicKey::from(new_ipk.clone()),
            new_private_key_bytes,
        ),
    );

    let invalid_state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
        identity,
        &master_key_id,
        vec![new_ipk],
        vec![],
        invalid_nonce,
        UserFeeIncrease::default(),
        &signer_qi,
        platform_version,
        None,
    )
    .expect("failed to build (invalid-nonce) IdentityUpdateTransition");

    tracing::info!("broadcasting invalid state transition (nonce=u64::MAX)...");

    let result = run_task(
        &ctx.app_context,
        BackendTask::BroadcastStateTransition(invalid_state_transition),
    )
    .await;

    assert!(
        result.is_err(),
        "expected Err(TaskError::...) for invalid state transition, got Ok({:?})",
        result.ok()
    );
    tracing::info!("broadcast correctly rejected: {:?}", result.err());

    let refresh_result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(si.qualified_identity.clone())),
    )
    .await
    .expect("RefreshIdentity should succeed after failed broadcast");
    assert!(
        matches!(
            refresh_result,
            BackendTaskSuccessResult::RefreshedIdentity(_)
        ),
        "expected RefreshedIdentity after failed broadcast, got: {:?}",
        refresh_result
    );
}

/// Broadcast state transitions lifecycle: valid identity update then invalid (bad nonce) rejection.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_066_broadcast_state_transitions() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    step_broadcast_valid(ctx, si).await;
    step_broadcast_invalid(ctx, si).await;
}
