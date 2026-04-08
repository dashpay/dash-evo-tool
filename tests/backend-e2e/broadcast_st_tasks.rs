//! BroadcastStateTransition backend E2E tests: TC-066 and TC-067.
//!
//! TC-066: Build a valid IdentityUpdateTransition, sign it, and broadcast via
//!         BackendTask::BroadcastStateTransition. Assert BroadcastedStateTransition.
//!
//! TC-067: Build an invalid state transition (wrong nonce) and assert Err(TaskError::...).

use crate::framework::fixtures::shared_identity;
use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::SdkBuilder;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::platform::{Fetch, IdentityPublicKey};

// --- Helper: build a proof-less test SDK from env-configured DAPI addresses ---

/// Build an Sdk instance connected to the testnet DAPI nodes with proofs disabled.
///
/// Proofs are disabled so we don't need a full ContextProvider with quorum keys.
/// This SDK is only used for lightweight queries (nonce fetch, identity fetch).
fn build_test_sdk(
    platform_version: &'static dash_sdk::dpp::version::PlatformVersion,
) -> dash_sdk::Sdk {
    let raw = std::env::var("TESTNET_dapi_addresses")
        .expect("TC-066: TESTNET_dapi_addresses env var not set — ensure .env is loaded");

    let address_list: AddressList = raw.parse().expect("TC-066: invalid TESTNET_dapi_addresses");

    SdkBuilder::new(address_list)
        .with_network(Network::Testnet)
        .with_version(platform_version)
        .with_proofs(false)
        .build()
        .expect("TC-066: failed to build test SDK")
}

// --- TC-066: BroadcastStateTransition — identity update ---

/// Build a valid IdentityUpdateTransition adding a new key, sign it, and broadcast
/// via BackendTask::BroadcastStateTransition. Verifies the result and confirms the
/// new key is visible on Platform after broadcast.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_066_broadcast_valid_identity_update() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let platform_version = ctx.app_context.platform_version();
    let identity = &si.qualified_identity.identity;
    let identity_id = identity.id();

    // Build a fresh ECDSA_SECP256K1 key to add to the identity.
    let new_private_key_bytes: [u8; 32] = rand::random();

    let new_public_key_data = {
        use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
        use dash_sdk::dpp::dashcore::PrivateKey;
        let secp = Secp256k1::new();
        let secret_key =
            dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&new_private_key_bytes)
                .expect("TC-066: invalid secret key bytes");
        let pk = PrivateKey::new(secret_key, Network::Testnet);
        pk.public_key(&secp).to_bytes()
    };

    // ID 0 is a placeholder; set_id below assigns the correct next ID.
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

    // Fetch the current identity nonce from Platform using a proof-less test SDK.
    let test_sdk = build_test_sdk(platform_version);
    let nonce = test_sdk
        .get_identity_nonce(identity_id, true, None)
        .await
        .expect("TC-066: failed to fetch identity nonce from Platform");
    tracing::info!("TC-066: identity nonce = {}", nonce);

    // Build and sign the state transition.
    let master_key = si
        .qualified_identity
        .can_sign_with_master_key()
        .expect("TC-066: shared identity has no master key");
    let master_key_id = master_key.identity_public_key.id();

    let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
        identity,
        &master_key_id,
        vec![new_ipk.clone()],
        vec![],
        nonce,
        UserFeeIncrease::default(),
        &si.qualified_identity,
        platform_version,
        None,
    )
    .expect("TC-066: failed to build IdentityUpdateTransition");

    tracing::info!("TC-066: state transition built and signed, broadcasting...");

    // Dispatch via BroadcastStateTransition.
    let result = run_task(
        &ctx.app_context,
        BackendTask::BroadcastStateTransition(state_transition),
    )
    .await
    .expect("TC-066: BroadcastStateTransition should succeed");

    assert!(
        matches!(result, BackendTaskSuccessResult::BroadcastedStateTransition),
        "TC-066: expected BroadcastedStateTransition, got: {:?}",
        result
    );
    tracing::info!("TC-066: broadcast succeeded");

    // Verify: re-fetch identity from Platform and confirm the new key is present.
    let fetched = dash_sdk::platform::Identity::fetch_by_identifier(&test_sdk, identity_id)
        .await
        .expect("TC-066: failed to re-fetch identity")
        .expect("TC-066: identity not found on Platform after broadcast");

    let has_new_key = fetched
        .public_keys()
        .values()
        .any(|k| k.data() == new_ipk.data());
    assert!(
        has_new_key,
        "TC-066: new key not found on Platform after broadcast"
    );
    tracing::info!("TC-066: new key confirmed on Platform");

    tracing::info!("TC-066: complete");
}

// --- TC-067: BroadcastStateTransition error — invalid state transition ---

/// Build a state transition with a deliberately wrong nonce (u64::MAX) and assert
/// that BackendTask::BroadcastStateTransition returns a typed Err(TaskError::...).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_067_broadcast_invalid_state_transition() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let platform_version = ctx.app_context.platform_version();
    let identity = &si.qualified_identity.identity;

    // Generate a fresh key to add (content doesn't matter — nonce makes it invalid).
    let new_private_key_bytes: [u8; 32] = rand::random();
    let new_public_key_data = {
        use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
        use dash_sdk::dpp::dashcore::PrivateKey;
        let secp = Secp256k1::new();
        let secret_key =
            dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&new_private_key_bytes)
                .expect("TC-067: invalid secret key bytes");
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

    // Use an intentionally invalid nonce (u64::MAX) to force a Platform rejection.
    let invalid_nonce: u64 = u64::MAX;

    let master_key = si
        .qualified_identity
        .can_sign_with_master_key()
        .expect("TC-067: shared identity has no master key");
    let master_key_id = master_key.identity_public_key.id();

    let invalid_state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
        identity,
        &master_key_id,
        vec![new_ipk],
        vec![],
        invalid_nonce,
        UserFeeIncrease::default(),
        &si.qualified_identity,
        platform_version,
        None,
    )
    .expect("TC-067: failed to build (invalid-nonce) IdentityUpdateTransition");

    tracing::info!("TC-067: broadcasting invalid state transition (nonce=u64::MAX)...");

    let result = run_task(
        &ctx.app_context,
        BackendTask::BroadcastStateTransition(invalid_state_transition),
    )
    .await;

    assert!(
        result.is_err(),
        "TC-067: expected Err(TaskError::...) for invalid state transition, got Ok({:?})",
        result.ok()
    );
    tracing::info!("TC-067: broadcast correctly rejected: {:?}", result.err());

    // Also verify using the identity task path just to refresh the identity state
    // (ensures the invalid broadcast did not corrupt Platform state).
    let refresh_result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(si.qualified_identity.clone())),
    )
    .await
    .expect("TC-067: RefreshIdentity should succeed after failed broadcast");
    assert!(
        matches!(
            refresh_result,
            BackendTaskSuccessResult::RefreshedIdentity(_)
        ),
        "TC-067: expected RefreshedIdentity after failed broadcast, got: {:?}",
        refresh_result
    );
    tracing::info!("TC-067: complete");
}
