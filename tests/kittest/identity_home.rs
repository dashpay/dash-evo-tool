use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::identity::home::{self, HomeState};
use dash_evo_tool::ui::identity::profile_cache::ProfileCache;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose, SecurityLevel};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;
use std::sync::Arc;

fn key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
    let mut key = IdentityPublicKey::random_key(id, Some(id as u64), PlatformVersion::latest());
    key.set_id(id);
    key.set_purpose(purpose);
    key.set_security_level(SecurityLevel::CRITICAL);
    key
}

fn seed_identity(
    app_context: &Arc<AppContext>,
    on_chain: Vec<IdentityPublicKey>,
    with_private: Vec<IdentityPublicKey>,
) {
    let public_keys = on_chain.into_iter().map(|key| (key.id(), key)).collect();
    let identity = Identity::new_with_id_and_keys(
        Identifier::from([0xA5; 32]),
        public_keys,
        PlatformVersion::latest(),
    )
    .expect("identity");
    let identity_id = identity.id();

    let mut private_keys = BTreeMap::new();
    for key in with_private {
        private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key),
                PrivateKeyData::InVault,
            ),
        );
    }

    let qualified_identity = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("Home test identity".to_string()),
        private_keys: KeyStorage { private_keys },
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    };

    app_context
        .insert_local_qualified_identity(&qualified_identity, &None)
        .expect("seed identity");
    app_context.set_selected_identity(Some(identity_id));
}

fn mount_home(app_context: Arc<AppContext>) -> Harness<'static, (HomeState, ProfileCache)> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(
            move |ui, state: &mut (HomeState, ProfileCache)| {
                let _ = home::render(ui, &app_context, &state.0, &mut state.1);
            },
            (HomeState::default(), ProfileCache::default()),
        );
    harness.run();
    harness
}

#[test]
fn home_send_to_wallet_disabled_for_everyday_without_local_withdrawal_key() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        let button = harness.get_by_label("Send to wallet");
        assert!(button.accesskit_node().is_disabled());

        button.hover();
        harness.run();
        assert!(
            harness
                .query_by_label_contains(
                    "no key available for withdrawal in the current interface mode"
                )
                .is_some(),
            "the disabled tooltip must explain the role-aware capability requirement"
        );
    });
}

#[test]
fn home_send_to_wallet_enabled_for_developer_with_on_chain_key_only() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Developer);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let harness = mount_home(app_context);
        let button = harness.get_by_label("Send to wallet");
        assert!(!button.accesskit_node().is_disabled());
    });
}

#[test]
fn home_send_to_wallet_enabled_for_everyday_with_local_transfer_key() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let harness = mount_home(app_context);
        let button = harness.get_by_label("Send to wallet");
        assert!(!button.accesskit_node().is_disabled());
    });
}
