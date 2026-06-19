//! TS-SIGN-E2E-01 — broadcast a state transition signed by a MIGRATED
//! `InVault` identity key, proving the per-use JIT free-rider path end-to-end.
//!
//! The shared identity's signing keys are migrated to the vault as raw bytes
//! (`PrivateKeyData::InVault`), exactly as the eager load-path migration does,
//! then an IdentityUpdateTransition is built + signed + broadcast. Signing
//! routes through the async `QualifiedIdentity` `Signer` →
//! `resolve_private_key_bytes` → `with_secret(SecretScope::IdentityKey{..})`,
//! which fetches the raw key from the vault per-use (prompt-free). A successful
//! broadcast proves the key was never resident yet still signed live.
//!
//! `#[ignore]` — requires `E2E_WALLET_MNEMONIC` + live DAPI/SPV. Run with:
//! ```bash
//! RUST_MIN_STACK=16777216 cargo test --test backend-e2e --all-features -- \
//!   --ignored --nocapture ts_sign_e2e_01_in_vault_identity_signs_and_broadcasts
//! ```

use crate::framework::fixtures::shared_identity;
use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task_with_nonce_retry;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use dash_evo_tool::wallet_backend::IdentityKeyView;
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

/// TS-SIGN-E2E-01.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn ts_sign_e2e_01_in_vault_identity_signs_and_broadcasts() {
    let ctx = ctx().await;
    let si = shared_identity().await;

    let platform_version = ctx.app_context.platform_version();
    let identity_id = si.qualified_identity.identity.id();

    // Fetch the live identity (latest keys + revision).
    let sdk = ctx.app_context.sdk();
    let mut identity = dash_sdk::platform::Identity::fetch_by_identifier(&sdk, identity_id)
        .await
        .expect("fetch identity")
        .expect("identity present");

    // Build the qualified identity and MIGRATE its plaintext signing keys into
    // the vault as InVault — exactly what the eager load-path migration does.
    let mut qi = si.qualified_identity.clone();
    qi.identity = identity.clone();

    let taken = qi.private_keys.take_plaintext_for_vault();
    assert!(
        !taken.is_empty(),
        "the shared identity must have carried plaintext signing keys to migrate"
    );
    IdentityKeyView::new(&ctx.app_context.secret_store(), identity_id.to_buffer())
        .store_all(&taken)
        .expect("store identity keys raw in the vault");

    // Residency: after migration the keystore must hold ONLY InVault for the
    // migrated keys — no resident plaintext.
    assert!(
        qi.private_keys
            .private_keys
            .values()
            .all(|(_, d)| !matches!(
                d,
                PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_)
            )),
        "no plaintext identity key may remain resident after migration"
    );
    assert!(
        qi.private_keys
            .private_keys
            .values()
            .any(|(_, d)| matches!(d, PrivateKeyData::InVault)),
        "migrated keys must be InVault placeholders"
    );

    // Wire the chokepoint so the resolver can fetch the raw key per-use.
    qi.secret_access = Some(ctx.app_context.wallet_backend().unwrap().secret_access());

    // Build a new key to add, and sign the IdentityUpdate with the (now InVault)
    // MASTER key via the JIT free-rider path.
    let new_private_key_bytes: [u8; 32] = rand::random();
    let new_public_key_data = {
        use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
        use dash_sdk::dpp::dashcore::PrivateKey;
        let secp = Secp256k1::new();
        let secret_key =
            dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&new_private_key_bytes)
                .expect("valid secret");
        PrivateKey::new(secret_key, Network::Testnet)
            .public_key(&secp)
            .to_bytes()
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
    identity.bump_revision();

    let nonce = sdk
        .get_identity_nonce(identity_id, true, None)
        .await
        .expect("fetch nonce");
    let master_key_id = identity
        .public_keys()
        .values()
        .find(|k| {
            k.purpose() == Purpose::AUTHENTICATION && k.security_level() == SecurityLevel::MASTER
        })
        .expect("identity has a MASTER AUTHENTICATION key")
        .id();

    // The new key's plaintext is registered so the ST can sign the key-add proof
    // of possession; the MASTER signer key is the InVault one we just migrated.
    qi.private_keys.insert_non_encrypted(
        (
            dash_evo_tool::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity,
            new_ipk.id(),
        ),
        (
            dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey::from(new_ipk.clone()),
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
    .await
    .expect("build + sign IdentityUpdateTransition via the InVault JIT path");

    let result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::BroadcastStateTransition(state_transition),
    )
    .await
    .expect("broadcast should succeed");
    assert!(
        matches!(result, BackendTaskSuccessResult::BroadcastedStateTransition),
        "expected BroadcastedStateTransition, got {result:?}"
    );

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let fetched = dash_sdk::platform::Identity::fetch_by_identifier(&sdk, identity_id)
        .await
        .expect("re-fetch identity")
        .expect("identity present after broadcast");
    assert!(
        fetched
            .public_keys()
            .values()
            .any(|k| k.data() == new_ipk.data()),
        "the new key must be visible on Platform — the InVault MASTER key signed the ST"
    );
}
