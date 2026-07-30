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
//! cargo test --test backend-e2e --all-features -- \
//!   --ignored --nocapture ts_sign_e2e_01_in_vault_identity_signs_and_broadcasts
//! ```

use crate::framework::fixtures::shared_identity;
use crate::framework::harness::ctx;
use crate::framework::task_runner::{run_on_large_stack, run_task_with_nonce_retry};
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
    let mut identity =
        run_on_large_stack({
            let sdk = ctx.app_context.sdk();
            move || async move {
                dash_sdk::platform::Identity::fetch_by_identifier(&sdk, identity_id).await
            }
        })
        .await
        .expect("fetch identity")
        .expect("identity present");

    // Build the qualified identity and MIGRATE its plaintext signing keys into
    // the vault as InVault — exactly what the eager load-path migration does.
    let mut qi = si.qualified_identity.clone();
    qi.identity = identity.clone();

    // The fixture registers from an HD wallet, so its keys are stored
    // `AtWalletDerivationPath` (never plaintext-at-rest) and the migration would
    // find nothing. Materialize the MASTER signing key to `Clear` first —
    // mirroring the non-wallet load path (`load_identity.rs`) that yields
    // `PrivateKeyData::Clear` — by deriving its raw bytes from the HD seed at
    // the same identity-auth path production registered it at (index 0, key 0).
    // Those bytes match the on-chain MASTER key, so the InVault signature
    // verifies after migration.
    materialize_master_key_as_clear(ctx, &si.wallet_seed_hash, &mut qi).await;

    let taken = qi.private_keys.take_plaintext_for_vault();
    assert!(
        !taken.is_empty(),
        "the migrated MASTER key must have been materialized as plaintext to carry into the vault"
    );
    IdentityKeyView::new(&ctx.app_context.secret_store(), identity_id.to_buffer())
        .store_all(&taken)
        .expect("store identity keys raw in the vault");

    // Residency: after migration the keystore must hold ONLY InVault for the
    // migrated keys — no resident plaintext.
    assert!(
        qi.private_keys
            .values()
            .all(|(_, d)| !matches!(d, PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_))),
        "no plaintext identity key may remain resident after migration"
    );
    assert!(
        qi.private_keys
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

    // Poll for the new key to become visible rather than assuming a fixed
    // propagation delay: re-fetch the identity until the key appears or the
    // ~10s deadline passes. A single fixed sleep is racy — it can re-fetch
    // before the broadcast has propagated and fail spuriously.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let key_visible = loop {
        let fetched = run_on_large_stack({
            let sdk = ctx.app_context.sdk();
            move || async move {
                dash_sdk::platform::Identity::fetch_by_identifier(&sdk, identity_id).await
            }
        })
        .await
        .expect("re-fetch identity")
        .expect("identity present after broadcast");
        if fetched
            .public_keys()
            .values()
            .any(|k| k.data() == new_ipk.data())
        {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    };
    assert!(
        key_visible,
        "the new key must be visible on Platform within 10s — the InVault MASTER key signed the ST"
    );
}

/// Rewrite the MASTER AUTHENTICATION key of `qi` from `AtWalletDerivationPath`
/// to a resident `PrivateKeyData::Clear`, deriving its raw bytes from the HD
/// seed at the same identity-auth path production registered it at.
///
/// The fixture identity is wallet-derived, so the migration under test
/// (`take_plaintext_for_vault`) only acts on `Clear`/`AlwaysClear` keys. This
/// reproduces the load-path state in which a MASTER key is plaintext-at-rest,
/// so the migration → InVault → JIT-sign chain has a key to operate on. The
/// derived bytes are byte-identical to the on-chain MASTER key (same BIP-32
/// path), so the signature it later produces verifies.
async fn materialize_master_key_as_clear(
    ctx: &crate::framework::harness::BackendTestContext,
    wallet_seed_hash: &dash_evo_tool::model::wallet::WalletSeedHash,
    qi: &mut dash_evo_tool::model::qualified_identity::QualifiedIdentity,
) {
    use dash_evo_tool::model::qualified_identity::PrivateKeyTarget;
    use dash_evo_tool::wallet_backend::SecretScope;
    use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};

    let network = ctx.app_context.network();

    let (map_key, master_pub) = qi
        .private_keys
        .iter()
        .find_map(|(map_key, (pub_key, _))| {
            let ipk = &pub_key.identity_public_key;
            (map_key.0 == PrivateKeyTarget::PrivateKeyOnMainIdentity
                && ipk.purpose() == Purpose::AUTHENTICATION
                && ipk.security_level() == SecurityLevel::MASTER)
                .then(|| (map_key.clone(), pub_key.clone()))
        })
        .expect("qualified identity must carry a MASTER AUTHENTICATION key");

    // Production registers the MASTER key at identity index 0, key index 0.
    let master_path =
        DerivationPath::identity_authentication_path(network, KeyDerivationType::ECDSA, 0, 0);

    let master_bytes = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .secret_access()
        .with_secret(
            &SecretScope::HdSeed {
                seed_hash: *wallet_seed_hash,
            },
            |plaintext| {
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(dash_evo_tool::backend_task::error::TaskError::WalletLocked)?;
                let xprv = master_path
                    .derive_priv_ecdsa_for_master_seed(seed, network)
                    .expect("derive master private key from seed");
                Ok(xprv.to_priv().inner.secret_bytes())
            },
        )
        .await
        .expect("resolve HD seed and derive MASTER private key");

    qi.private_keys
        .insert_non_encrypted(map_key, (master_pub, master_bytes));
}
