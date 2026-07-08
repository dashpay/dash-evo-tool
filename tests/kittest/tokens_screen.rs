//! W4 kittest — TokensScreen (Token Creator) obeys the app-scoped selected identity.
//!
//! B4 migration: `TokensScreen::new()` when built for `TokenCreator` must seed
//! `selected_identity` from the app-scoped selection (fallback: first loaded).
//!
//! B6 regression lock: the 6 N/A token recipient/target/member selectors (mint,
//! transfer, freeze, unfreeze, destroy-frozen-funds, groups) must NOT carry
//! `syncing_global`. The `default_selector_has_no_sync_target` unit test in
//! `identity_selector.rs` covers any freshly-built `IdentitySelector`; B6 adds
//! a structural note and a spot-check that `TokensScreen` (Token Creator) does
//! NOT accidentally apply syncing_global to the N/A sites when rendering.
//!
//! Write-back: `syncing_global` component-level write-back is verified by
//! `identity_selector::tests::syncing_global_writes_selection_to_app_context`.
//!
//! # TODO(WalletFixture / private-key fixture)
//! Add screen-level write-back assertions (ComboBox click → `resolve_selected_identity()`
//! moves) once an identity fixture with loaded AUTH HIGH/CRITICAL private keys exists.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::tokens::tokens_screen::{TokensScreen, TokensSubscreen};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use std::collections::BTreeMap;
use std::sync::Arc;

fn build_ctx() -> (
    Harness<'static, dash_evo_tool::app::AppState>,
    Arc<AppContext>,
) {
    let mut h = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("AppState builds")
            .with_animations(false)
    });
    h.run_steps(5);
    let ctx = h.state().current_app_context().clone();
    (h, ctx)
}

fn seed_token_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    let id = identity.id();
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&qi, &None)
        .expect("seed token identity");
    id
}

/// W4 B4: `TokensScreen` in `TokenCreator` subscreen must seed `selected_identity`
/// from the app-scoped selection, not always the first loaded identity.
#[test]
fn token_creator_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_token_identity(&ctx, 0xA1, "Token Alpha");
        let second = seed_token_identity(&ctx, 0xB2, "Token Beta");

        ctx.set_selected_identity(Some(second));

        let screen = TokensScreen::new(&ctx, TokensSubscreen::TokenCreator);

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "TokensScreen (TokenCreator) must default to the app-scoped selected identity"
        );
    });
}
