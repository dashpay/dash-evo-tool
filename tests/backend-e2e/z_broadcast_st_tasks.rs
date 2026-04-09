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
use dash_sdk::SdkBuilder;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::{CoreBlockHeight, UserFeeIncrease};
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{ContextProvider, Fetch, IdentityPublicKey};
use std::sync::Arc;

// --- No-op ContextProvider for the lightweight proof-less SDK ---

/// Minimal context provider that returns empty/error for all queries.
///
/// Sufficient for proof-less SDK operations (nonce fetch, identity fetch)
/// where quorum key verification is not performed.
struct NoopContextProvider;

impl ContextProvider for NoopContextProvider {
    fn get_data_contract(
        &self,
        _id: &dash_sdk::platform::Identifier,
        _pv: &dash_sdk::dpp::version::PlatformVersion,
    ) -> Result<Option<Arc<dash_sdk::dpp::prelude::DataContract>>, ContextProviderError> {
        Ok(None)
    }

    fn get_token_configuration(
        &self,
        _token_id: &dash_sdk::platform::Identifier,
    ) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError>
    {
        Ok(None)
    }

    fn get_quorum_public_key(
        &self,
        _quorum_type: u32,
        _quorum_hash: [u8; 32],
        _core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        Err(ContextProviderError::Config(
            "NoopContextProvider: quorum keys not available (proofs disabled)".into(),
        ))
    }

    fn get_platform_activation_height(&self) -> Result<CoreBlockHeight, ContextProviderError> {
        Err(ContextProviderError::Config(
            "NoopContextProvider: activation height not available".into(),
        ))
    }
}

// --- Helper: build a proof-less test SDK from env-configured DAPI addresses ---

/// Build an Sdk instance connected to the testnet DAPI nodes with proofs disabled.
///
/// Proofs are disabled so we don't need a full ContextProvider with quorum keys.
/// This SDK is only used for lightweight queries (nonce fetch, identity fetch).
fn build_test_sdk(
    platform_version: &'static dash_sdk::dpp::version::PlatformVersion,
) -> dash_sdk::Sdk {
    let raw = std::env::var("TESTNET_dapi_addresses")
        .expect("TESTNET_dapi_addresses env var not set — ensure .env is loaded");

    let address_list: AddressList = raw.parse().expect("invalid TESTNET_dapi_addresses");

    SdkBuilder::new(address_list)
        .with_network(Network::Testnet)
        .with_version(platform_version)
        .with_context_provider(NoopContextProvider)
        .with_proofs(false)
        .build()
        .expect("failed to build test SDK")
}

// --- TC-066 step functions ---

async fn step_broadcast_valid(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
) {
    tracing::info!("=== Step 1: broadcast valid IdentityUpdateTransition ===");

    let platform_version = ctx.app_context.platform_version();
    let identity = &si.qualified_identity.identity;
    let identity_id = identity.id();

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

    let test_sdk = build_test_sdk(platform_version);
    let nonce = test_sdk
        .get_identity_nonce(identity_id, true, None)
        .await
        .expect("failed to fetch identity nonce from Platform");
    tracing::info!("identity nonce = {}", nonce);

    let master_key = si
        .qualified_identity
        .can_sign_with_master_key()
        .expect("shared identity has no master key");
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

    let fetched = dash_sdk::platform::Identity::fetch_by_identifier(&test_sdk, identity_id)
        .await
        .expect("failed to re-fetch identity")
        .expect("identity not found on Platform after broadcast");

    let has_new_key = fetched
        .public_keys()
        .values()
        .any(|k| k.data() == new_ipk.data());
    assert!(has_new_key, "new key not found on Platform after broadcast");
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
    let refreshed_qi = ctx
        .app_context
        .load_local_qualified_identities()
        .expect("load_local_qualified_identities should succeed")
        .into_iter()
        .find(|qi| qi.identity.id() == identity_id)
        .expect("shared identity should be in local DB after refresh");
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

    let invalid_state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
        identity,
        &master_key_id,
        vec![new_ipk],
        vec![],
        invalid_nonce,
        UserFeeIncrease::default(),
        &refreshed_qi,
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
