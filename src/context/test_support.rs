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

/// An identity public key built by dpp's versioned factory: writable, enabled
/// and not bound to a contract. Tests that need a read-only or disabled key set
/// that on the result.
pub(crate) fn identity_public_key(
    id: dash_sdk::dpp::identity::KeyID,
    purpose: dash_sdk::dpp::identity::Purpose,
    security_level: dash_sdk::dpp::identity::SecurityLevel,
    key_type: dash_sdk::dpp::identity::KeyType,
    data: impl Into<dash_sdk::dpp::platform_value::BinaryData>,
) -> dash_sdk::platform::IdentityPublicKey {
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;

    let mut key = dash_sdk::platform::IdentityPublicKey::default_versioned(
        dash_sdk::dpp::version::PlatformVersion::latest(),
    )
    .expect("identity key structure");
    key.set_id(id);
    key.set_purpose(purpose);
    key.set_security_level(security_level);
    key.set_key_type(key_type);
    key.set_data(data.into());
    key
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

/// Protocol versions tests declare proven, per SDK instance.
///
/// Upstream records a verified protocol version only from a response whose
/// proof and quorum signature it checked itself, so a test cannot make an
/// offline SDK prove one. [`AppContext::protocol_version_info`] folds this
/// override into the SDK's answer in test builds only. It is bound to the SDK
/// it was set for: a rebuilt connection is a new SDK and has proven nothing.
///
/// Reaches only reads through `AppContext`; code that asks an `Sdk` directly
/// for `protocol_version_info()` sees the SDK's own answer.
// TODO(upstream): replace with an SDK-level mock seam once one exists:
// mock expectations raise only the stored version, never the verified one
// (https://github.com/dashpay/platform/issues/5622).
#[derive(Debug, Default)]
pub(crate) struct ProvenProtocolVersionOverride(std::sync::Mutex<Option<(Arc<Sdk>, u32)>>);

impl ProvenProtocolVersionOverride {
    /// Record `protocol_version` as proven for `sdk`. Only raises it, as the
    /// network does.
    fn prove(&self, sdk: Arc<Sdk>, protocol_version: u32) {
        let mut slot = self.0.lock().expect("override lock");
        let proven = match slot.as_ref() {
            Some((current, proven)) if Arc::ptr_eq(current, &sdk) => {
                (*proven).max(protocol_version)
            }
            _ => protocol_version,
        };
        *slot = Some((sdk, proven));
    }

    /// `info` with the version proven for `sdk` folded in, as upstream
    /// reports a version it verified itself.
    pub(crate) fn apply(
        &self,
        sdk: &Arc<Sdk>,
        mut info: dash_sdk::ProtocolVersionInfo,
    ) -> dash_sdk::ProtocolVersionInfo {
        let slot = self.0.lock().expect("override lock");
        if let Some((current, proven)) = slot.as_ref()
            && Arc::ptr_eq(current, sdk)
        {
            let verified = info.verified_version.unwrap_or(0).max(*proven);
            info.verified_version = Some(verified);
            info.newer_protocol_available = verified > info.maximum_supported_version;
        }
        info
    }
}

/// Have `ctx`'s current SDK report `protocol_version` as proven by the
/// network, as it does after a response whose proof and quorum signature were
/// verified. Only raises it; an unknown version is recorded as proven, as
/// upstream records it. Moves only the proven version
/// ([`ProvenProtocolVersionOverride`]); use [`set_sdk_protocol_version`] for
/// the version transitions are built at.
pub(crate) fn prove_protocol_version(ctx: &AppContext, protocol_version: u32) {
    ctx.proven_protocol_version_for_tests
        .prove(ctx.sdk.load_full(), protocol_version);
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

/// [`mock_sdk_for`], starting at `protocol_version` as the app's SDK starts at
/// its network's version.
pub(crate) fn mock_sdk_at_for(ctx: &AppContext, protocol_version: u32) -> Sdk {
    let version = dash_sdk::dpp::version::PlatformVersion::get(protocol_version)
        .expect("a protocol version this build knows");
    let sdk = dash_sdk::SdkBuilder::new_mock()
        .with_initial_version(version)
        .build()
        .expect("mock sdk");
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
