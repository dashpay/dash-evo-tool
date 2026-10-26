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
    let ctx = crate::support::wait_for_wallet_backend(&mut h);
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
        dpns_names: vec![dash_evo_tool::model::qualified_identity::DPNSNameInfo {
            name: alias.to_string(),
            acquired_at: 0,
        }],
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

/// Leaving the Token Creator through the left panel (e.g. to top up the identity)
/// and coming back through the Tokens subscreen tab keeps the whole form,
/// including the chosen preset.
#[test]
fn token_creator_form_survives_left_panel_and_tab_navigation() {
    use crate::identity_home::{key, seed_identity};
    use dash_evo_tool::ui::RootScreenType;
    use dash_sdk::dpp::identity::Purpose;
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = crate::support::mount_app(RootScreenType::RootScreenTokenCreator);
        let app_context = harness.state().current_app_context().clone();
        let auth_key = key(1, Purpose::AUTHENTICATION);
        seed_identity(&app_context, vec![auth_key.clone()], vec![auth_key]);
        harness.run_steps(5);

        let type_into = |harness: &mut Harness<'static, dash_evo_tool::app::AppState>,
                         index: usize,
                         text: &str| {
            harness
                .query_all_by_role(Role::TextInput)
                .nth(index)
                .expect("token creator text input")
                .focus();
            harness.event(egui::Event::Text(text.to_owned()));
            harness.run_steps(2);
        };
        // Simple mode inputs, top to bottom: name, description, initial supply, max supply.
        type_into(&mut harness, 0, "PresetCoin");
        type_into(&mut harness, 1, "Kept across navigation");
        type_into(&mut harness, 2, "1000");

        harness.get_by_value("Select a preset...").click();
        harness.run_steps(2);
        harness
            .get_by_label("All Allowed - All actions enabled")
            .click();
        harness.run_steps(3);

        let assert_form_kept = |harness: &Harness<'static, dash_evo_tool::app::AppState>,
                                route: &str| {
            assert!(
                harness.query_all_by_value("PresetCoin").count() >= 1,
                "the token name must survive {route}"
            );
            assert!(
                harness.query_all_by_value("Kept across navigation").count() >= 1,
                "the description must survive {route}"
            );
            assert!(
                harness.query_by_value("All Allowed").is_some(),
                "the chosen preset must survive {route}"
            );
            // The hint below "Create Token" names whatever still blocks it.
            assert!(
                harness
                    .query_all_by_label_contains("to continue.")
                    .next()
                    .is_none(),
                "\"Create Token\" must stay enabled after {route}"
            );
        };
        assert_form_kept(&harness, "filling the form");

        // Leave through the left panel (as when topping up the identity) and come back.
        harness
            .get_by_role_and_label(Role::Button, "Identities")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Tokens")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Token Creator")
            .click();
        harness.run_steps(3);
        assert_form_kept(&harness, "a left-panel round trip");

        // Leave through the Tokens subscreen tabs and come back.
        harness
            .get_by_role_and_label(Role::Button, "My Tokens")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Token Creator")
            .click();
        harness.run_steps(3);
        assert_form_kept(&harness, "a subscreen-tab round trip");
    });
}

/// Advanced mode's "Register Token Contract" is blocked, like Simple mode's
/// "Create Token", until a preset (or individually chosen "Custom" rules) is picked.
#[test]
fn advanced_register_requires_a_preset_choice() {
    use crate::identity_home::{key, seed_identity};
    use dash_evo_tool::ui::RootScreenType;
    use dash_sdk::dpp::identity::Purpose;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = crate::support::mount_app(RootScreenType::RootScreenTokenCreator);
        let app_context = harness.state().current_app_context().clone();
        let auth_key = key(1, Purpose::AUTHENTICATION);
        seed_identity(&app_context, vec![auth_key.clone()], vec![auth_key]);
        harness.run_steps(5);

        // Fill the required fields in Simple mode: name and initial supply.
        for (index, text) in [(0, "GateCoin"), (2, "1000")] {
            harness
                .query_all_by_role(Role::TextInput)
                .nth(index)
                .expect("token creator text input")
                .focus();
            harness.event(egui::Event::Text(text.to_owned()));
            harness.run_steps(2);
        }
        let toggle_advanced = |harness: &mut Harness<'static, dash_evo_tool::app::AppState>| {
            harness.get_by_label("Show Advanced Options").click();
            harness.run_steps(3);
        };
        let click_register = |harness: &mut Harness<'static, dash_evo_tool::app::AppState>| {
            harness
                .get_by_role_and_label(Role::Button, "Register Token Contract")
                .click();
            harness.run_steps(3);
        };
        const CONFIRMATION: &str = "Confirm Token Contract Registration";
        // Room for the whole Advanced form, so its bottom buttons are on screen.
        harness.set_size(egui::vec2(1280.0, 2400.0));
        harness.run_steps(2);

        toggle_advanced(&mut harness);
        assert!(
            harness
                .query_by_label("Select a token preset to continue.")
                .is_some(),
            "the blocked button must say why"
        );
        click_register(&mut harness);
        assert!(
            harness.query_by_label(CONFIRMATION).is_none(),
            "\"Register Token Contract\" must do nothing until a preset is chosen"
        );

        // Pick a preset in Simple mode, then return to Advanced mode.
        toggle_advanced(&mut harness);
        harness.get_by_value("Select a preset...").click();
        harness.run_steps(2);
        harness
            .get_by_label("All Allowed - All actions enabled")
            .click();
        harness.run_steps(3);
        toggle_advanced(&mut harness);
        assert!(
            harness
                .query_by_label("Select a token preset to continue.")
                .is_none()
        );
        // Advanced mode also asks for the plural name, right after the singular one.
        let plural_index = harness
            .query_all_by_role(Role::TextInput)
            .position(|node| node.accesskit_node().value().as_deref() == Some("GateCoin"))
            .expect("singular name input")
            + 1;
        harness
            .query_all_by_role(Role::TextInput)
            .nth(plural_index)
            .expect("plural name input")
            .focus();
        harness.event(egui::Event::Text("GateCoins".to_owned()));
        harness.run_steps(2);
        click_register(&mut harness);
        assert!(
            harness.query_by_label(CONFIRMATION).is_some(),
            "\"Register Token Contract\" must open the confirmation once a preset is chosen"
        );
    });
}
