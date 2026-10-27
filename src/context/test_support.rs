//! Shared [`AppContext`] fixtures for unit tests in the `context` module tree.
//!
//! Follows the `kv_test_support` pattern: one fixture every test wires against
//! instead of hand-rolling a copy of the ~15-line construction dance.

use std::path::Path;
use std::sync::Arc;

use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::document::DocumentV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::{Document, Identifier};

use super::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::user_role::UserRoleCell;
use crate::wallet_backend::DetKv;

/// A user identity with no names, alias, keys or wallet.
pub(crate) fn bare_user_identity(id: Identifier, network: Network) -> QualifiedIdentity {
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    QualifiedIdentity {
        identity: dash_sdk::platform::Identity::create_basic_identity(
            id,
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("identity"),
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: None,
        private_keys: KeyStorage::default(),
        dpns_names: Vec::new(),
        associated_wallets: Default::default(),
        secret_access: None,
        wallet_index: None,
        top_ups: Default::default(),
        status: IdentityStatus::Active,
        network,
    }
}

/// A user identity publishing one key that can register usernames (key id 1),
/// held on this device only when `held`.
pub(crate) fn user_identity_with_signing_key(
    id: Identifier,
    network: Network,
    held: bool,
) -> QualifiedIdentity {
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
    use dash_sdk::dpp::identity::{Purpose, SecurityLevel};

    let mut key = dash_sdk::platform::IdentityPublicKey::random_key(
        1,
        Some(7),
        dash_sdk::dpp::version::PlatformVersion::latest(),
    );
    key.set_id(1);
    key.set_purpose(Purpose::AUTHENTICATION);
    key.set_security_level(SecurityLevel::HIGH);
    // Username registrations are signed with ECDSA keys only.
    key.set_key_type(dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1);
    let mut identity = bare_user_identity(id, network);
    identity.identity.add_public_key(key.clone());
    if held {
        identity.private_keys = KeyStorage::from(std::collections::BTreeMap::from([(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 1),
            (key.into(), PrivateKeyData::InVault),
        )]));
    }
    identity
}

/// Hold published key `key_id` of `identity` only in the per-key encrypted
/// format of early versions, which this version keeps but cannot open.
pub(crate) fn hold_as_legacy_encrypted(
    identity: &mut QualifiedIdentity,
    key_id: dash_sdk::dpp::identity::KeyID,
) {
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

    let key = identity.identity.public_keys()[&key_id].clone();
    identity.private_keys.insert_at(
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
        (key.into(), PrivateKeyData::Encrypted(vec![0x5A; 48])),
    );
}

/// Hold published key `key_id` of `identity` as a reference into the wallet
/// with `seed_hash`, the way a key found by a wallet scan is kept.
pub(crate) fn hold_as_wallet_reference(
    identity: &mut QualifiedIdentity,
    key_id: dash_sdk::dpp::identity::KeyID,
    seed_hash: crate::model::wallet::WalletSeedHash,
) {
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::model::qualified_identity::encrypted_key_storage::{
        PrivateKeyData, WalletDerivationPath,
    };
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

    let key = identity.identity.public_keys()[&key_id].clone();
    identity.private_keys.insert_at(
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
        (
            key.into(),
            PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                wallet_seed_hash: seed_hash,
                derivation_path: Default::default(),
            }),
        ),
    );
}

/// [`test_app_context`] with its wallet backend wired, so identities can be
/// stored and read back the way screens read them.
pub(crate) async fn wired_test_app_context(dir: &Path) -> Arc<AppContext> {
    let context = test_app_context(dir);
    let (tx, _rx) = tokio::sync::mpsc::channel(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());
    context
        .ensure_wallet_backend(sender)
        .await
        .expect("wire wallet backend");
    context
}

/// Build a network-free [`AppContext`] backed by throwaway temp storage — enough
/// to exercise the settings read-modify-write and feature-gate paths.
pub(crate) fn test_app_context(dir: &Path) -> Arc<AppContext> {
    let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
    test_app_context_with_kv_and_network(dir, app_kv, Network::Testnet)
}

/// Build a throwaway [`AppContext`] for a specific network.
pub(crate) fn test_app_context_for_network(dir: &Path, network: Network) -> Arc<AppContext> {
    let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
    test_app_context_with_kv_and_network(dir, app_kv, network)
}

/// [`test_app_context`] with a caller-supplied app k/v store, so a test can
/// inject a fault-injecting backing store (see `kv_test_support::FailingKv`).
pub(crate) fn test_app_context_with_kv(dir: &Path, app_kv: Arc<DetKv>) -> Arc<AppContext> {
    test_app_context_with_kv_and_network(dir, app_kv, Network::Testnet)
}

fn test_app_context_with_kv_and_network(
    dir: &Path,
    app_kv: Arc<DetKv>,
    network: Network,
) -> Arc<AppContext> {
    crate::app_dir::ensure_env_file(dir);
    let db = Arc::new(crate::database::Database::new(dir.join("data.db")).expect("db"));
    db.create_tables(true).expect("create tables");
    db.set_default_version().expect("set version");
    let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
    AppContext::new(
        dir.to_path_buf(),
        network,
        db,
        Default::default(),
        Default::default(),
        egui::Context::default(),
        app_kv,
        secret_store,
        UserRoleCell::default(),
    )
    .expect("AppContext")
}

/// Put `ctx` at `protocol_version`, as its SDK stands once a proven response
/// ratcheted it there. The replacement SDK is a mock: nothing reaches a network.
pub(crate) fn set_sdk_protocol_version(ctx: &AppContext, protocol_version: u32) {
    let version = dash_sdk::dpp::version::PlatformVersion::get(protocol_version)
        .expect("a protocol version this build knows");
    let sdk = dash_sdk::SdkBuilder::new_mock()
        .with_initial_version(version)
        .build()
        .expect("mock sdk");
    ctx.sdk.store(Arc::new(sdk));
}

/// Have `ctx`'s SDK learn `protocol_version` as it does from a response whose
/// proof and quorum signature were verified. Only raises it, as the network
/// does; an unknown version is recorded as proven, as upstream records it.
pub(crate) fn prove_protocol_version(ctx: &AppContext, protocol_version: u32) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    ctx.sdk
        .load()
        .verify_response_metadata(
            "test",
            &dash_sdk::dapi_grpc::platform::v0::ResponseMetadata {
                height: 1,
                time_ms: now_ms,
                protocol_version,
                ..Default::default()
            },
        )
        .expect("fresh proved metadata");
}

/// A mock SDK that reads data contracts through `ctx`, as the app's own SDK does.
pub(crate) fn mock_sdk_for(ctx: &AppContext) -> Sdk {
    let sdk = Sdk::new_mock();
    let provider = ctx
        .spv_context_provider
        .read()
        .expect("context provider lock")
        .clone();
    sdk.set_context_provider(provider);
    sdk
}

/// A registered DPNS name document for `label`, owned by `owner`, whose record
/// points at `identity`; a [`Value`] so a test picks how the id is encoded.
pub(crate) fn dpns_name_document(owner: Identifier, label: &str, identity: Value) -> Document {
    let text = |value: &str| Value::Text(value.to_string());
    Document::V0(DocumentV0 {
        moderated_at: None,
        moderated_by: None,
        id: Identifier::from([0x5D; 32]),
        owner_id: owner,
        creator_id: None,
        properties: [
            ("label".to_string(), text(label)),
            (
                "normalizedLabel".to_string(),
                text(&crate::model::dpns::normalize_dpns_label(label)),
            ),
            ("normalizedParentDomainName".to_string(), text("dash")),
            (
                "records".to_string(),
                Value::Map(vec![(text("identity"), identity)]),
            ),
        ]
        .into(),
        revision: None,
        created_at: Some(100),
        updated_at: None,
        transferred_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        transferred_at_block_height: None,
        created_at_core_block_height: None,
        updated_at_core_block_height: None,
        transferred_at_core_block_height: None,
        contract_version: None,
    })
}

/// An ECDSA AUTHENTICATION key `id` at `level`, unbound and unlimited.
pub(crate) fn auth_key(
    id: u32,
    level: dash_sdk::dpp::identity::SecurityLevel,
) -> dash_sdk::platform::IdentityPublicKey {
    dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0 {
        id,
        purpose: dash_sdk::dpp::identity::Purpose::AUTHENTICATION,
        security_level: level,
        contract_bounds: None,
        key_type: dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: vec![2; 33].into(),
        disabled_at: None,
    }
    .into()
}

/// `key`, a version 0 key such as [`auth_key`] builds, bound to `bounds`.
pub(crate) fn bound_to(
    key: dash_sdk::platform::IdentityPublicKey,
    bounds: dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds,
) -> dash_sdk::platform::IdentityPublicKey {
    let dash_sdk::platform::IdentityPublicKey::V0(mut v0) = key else {
        panic!("bound_to takes a version 0 key");
    };
    v0.contract_bounds = Some(bounds);
    v0.into()
}

/// A testnet user identity publishing `keys` and holding the private half of
/// each.
pub(crate) fn identity_holding(
    keys: &[dash_sdk::platform::IdentityPublicKey],
) -> QualifiedIdentity {
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

    let mut qi = bare_user_identity(Identifier::new([0xD0; 32]), Network::Testnet);
    qi.identity
        .set_public_keys(keys.iter().map(|key| (key.id(), key.clone())).collect());
    for key in keys {
        qi.private_keys
            .insert_non_encrypted(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
                (
                    QualifiedIdentityPublicKey {
                        identity_public_key: key.clone(),
                        in_wallet_at_derivation_path: None,
                    },
                    [key.id() as u8; 32],
                ),
            )
            .expect("a free slot");
    }
    qi
}

/// The DashPay system contract, whose document types require a HIGH key.
pub(crate) fn dashpay_contract() -> dash_sdk::platform::DataContract {
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
    load_system_data_contract(
        SystemDataContract::Dashpay,
        dash_sdk::dpp::version::PlatformVersion::latest(),
    )
    .expect("the DashPay contract")
}

/// A user identity whose only key, key 1 (an ECDSA HIGH AUTHENTICATION key
/// bound to `bounds`), is held as a reference into an open test wallet, and
/// that wallet.
pub(crate) fn identity_with_bound_wallet_key(
    bounds: dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds,
) -> (
    QualifiedIdentity,
    Arc<std::sync::RwLock<crate::model::wallet::Wallet>>,
) {
    let wallet = crate::model::wallet::test_support::open_wallet();
    let seed_hash = wallet.seed_hash();
    let key = bound_to(
        auth_key(1, dash_sdk::dpp::identity::SecurityLevel::HIGH),
        bounds,
    );
    use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    let mut identity = bare_user_identity(Identifier::new([0xD1; 32]), Network::Testnet);
    identity
        .identity
        .set_public_keys(std::collections::BTreeMap::from([(key.id(), key)]));
    hold_as_wallet_reference(&mut identity, 1, seed_hash);
    let wallet = Arc::new(std::sync::RwLock::new(wallet));
    identity
        .associated_wallets
        .insert(seed_hash, Arc::clone(&wallet));
    (identity, wallet)
}
