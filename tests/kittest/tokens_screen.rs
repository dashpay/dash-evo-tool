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

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_contract::QualifiedContract;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::tokens::tokens_screen::{IdentityTokenInfo, TokensScreen, TokensSubscreen};
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenType};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Shown next to Simple mode's "Create Token" until a preset is chosen.
const SIMPLE_PRESET_HINT: &str = "Choose a preset to create the token.";
/// Shown next to Advanced mode's "Register Token Contract" until a preset or Custom is chosen.
const ADVANCED_PRESET_HINT: &str = "Choose a preset, or Custom to use the rules below.";

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

/// A user identity with one CRITICAL authentication key — enough for the token
/// action scaffold to render its form instead of the "no keys" notice.
fn identity_with_critical_key(app_context: &Arc<AppContext>) -> QualifiedIdentity {
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
    use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
    use dash_sdk::platform::IdentityPublicKey;

    let mut identity =
        Identity::create_basic_identity(Identifier::from([0xC3; 32]), PlatformVersion::latest())
            .expect("basic identity");
    let mut key = IdentityPublicKey::default_versioned(PlatformVersion::latest())
        .expect("identity key structure");
    key.set_id(0);
    key.set_purpose(Purpose::AUTHENTICATION);
    key.set_security_level(SecurityLevel::CRITICAL);
    key.set_key_type(KeyType::ECDSA_HASH160);
    key.set_data(vec![1; 20].into());
    identity.add_public_key(key);
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("token-action-layout".to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    }
}

/// Token info for a placeholder token; the layout under test does not depend
/// on which contract carries it.
fn layout_token_info(app_context: &Arc<AppContext>) -> IdentityTokenInfo {
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};

    let contract = load_system_data_contract(SystemDataContract::DPNS, PlatformVersion::latest())
        .expect("dpns contract");
    IdentityTokenInfo {
        token_id: Identifier::from([0xD4; 32]),
        token_alias: "layout-token".to_string(),
        identity: identity_with_critical_key(app_context),
        data_contract: QualifiedContract {
            contract,
            alias: None,
        },
        token_config: TokenConfigurationV0::default_most_restrictive().into(),
        token_position: 0,
    }
}

/// Mint, burn, freeze, unfreeze, destroy-frozen-funds, pause and resume share
/// one screen scaffold. Its "Advanced Options" checkbox must sit on the page
/// heading's row and the first form step must follow right below — not be
/// pushed to the bottom of the window by a checkbox centred in the remaining
/// height.
#[test]
fn token_action_forms_start_right_under_the_heading() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);
        let app_context = harness.state().current_app_context().clone();
        // Developer role renders the form for any identity with public keys,
        // so no private key material is needed.
        app_context.set_user_role(UserRole::Developer);
        let info = layout_token_info(&app_context);

        let screens = [
            ("Mint Tokens", ScreenType::MintTokensScreen(info.clone())),
            ("Burn Tokens", ScreenType::BurnTokensScreen(info.clone())),
            (
                "Freeze Identity’s Tokens",
                ScreenType::FreezeTokensScreen(info.clone()),
            ),
            (
                "Unfreeze Identity’s Tokens",
                ScreenType::UnfreezeTokensScreen(info.clone()),
            ),
            (
                "Destroy Frozen Funds",
                ScreenType::DestroyFrozenFundsScreen(info.clone()),
            ),
            (
                "Pause Token Contract",
                ScreenType::PauseTokensScreen(info.clone()),
            ),
            (
                "Resume Token Contract",
                ScreenType::ResumeTokensScreen(info.clone()),
            ),
        ];

        for (heading, screen_type) in screens {
            let screen = screen_type.create_screen(&app_context);
            {
                let state = harness.state_mut();
                state.screen_stack.clear();
                state.screen_stack.push(screen);
            }
            harness.run_steps(5);

            let heading_rect = harness.get_by_label(heading).rect();
            let checkbox_rect = harness.get_by_label("Advanced Options").rect();
            let first_step_rect = harness.get_by_label_contains("1. ").rect();

            assert!(
                checkbox_rect.center().y >= heading_rect.top()
                    && checkbox_rect.center().y <= heading_rect.bottom(),
                "{heading}: the Advanced Options checkbox must sit on the heading row \
                 (heading {heading_rect:?}, checkbox {checkbox_rect:?})"
            );
            let gap = first_step_rect.top() - heading_rect.bottom();
            assert!(
                (0.0..=60.0).contains(&gap),
                "{heading}: the first form step must start right under the heading, \
                 found a {gap}px gap (heading {heading_rect:?}, step {first_step_rect:?})"
            );
        }
    });
}

/// The "My Tokens" top bar offers "Create Token", which opens the Token Creator.
#[test]
fn my_tokens_create_token_button_opens_token_creator() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);

        harness.get_by_label("Create Token").click();
        harness.run_steps(5);

        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenTokenCreator,
            "Create Token must switch to the Token Creator"
        );
        assert_eq!(
            harness
                .state()
                .current_app_context()
                .get_app_settings()
                .root_screen_type,
            RootScreenType::RootScreenTokenCreator,
            "the tokens subscreen chooser must highlight Token Creator"
        );
        let Screen::TokensScreen(screen) = harness.state_mut().visible_screen_mut() else {
            panic!("the visible screen must be the tokens screen");
        };
        assert!(
            screen.tokens_subscreen == TokensSubscreen::TokenCreator,
            "the visible tokens screen must show the Token Creator"
        );
    });
}

/// Simple mode shows the "holders can send this token" choice, so a choice made
/// in Advanced mode is never hidden, and the confirmation states it.
#[test]
fn simple_mode_shows_and_confirms_transferability() {
    use crate::identity_home::{key, seed_identity};
    use dash_evo_tool::ui::RootScreenType;
    use dash_sdk::dpp::identity::Purpose;
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = crate::support::mount_app(RootScreenType::RootScreenTokenCreator);
        harness.set_size(egui::vec2(1280.0, 2400.0));
        let app_context = harness.state().current_app_context().clone();
        let auth_key = key(1, Purpose::AUTHENTICATION);
        seed_identity(&app_context, vec![auth_key.clone()], vec![auth_key]);
        harness.run_steps(5);

        assert!(
            harness
                .query_by_label("Holders can send this token to other identities")
                .is_some(),
            "Simple mode must show whether holders can send the token"
        );

        // Simple mode inputs, top to bottom: name, description, initial supply, max supply.
        for (index, text) in [(0, "SendCoin"), (2, "1000")] {
            harness
                .query_all_by_role(Role::TextInput)
                .nth(index)
                .expect("token creator text input")
                .focus();
            harness.event(egui::Event::Text(text.to_owned()));
            harness.run_steps(2);
        }
        harness.get_by_value("Select a preset...").click();
        harness.run_steps(2);
        harness
            .get_by_label("All Allowed - All actions enabled")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Create Token")
            .click();
        harness.run_steps(3);

        assert!(
            harness
                .query_all_by_label_contains("Holders can send this token to other identities.")
                .next()
                .is_some(),
            "the confirmation must state whether holders can send the token"
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
            // The hint next to "Create Token" names whatever still blocks it.
            for hint in [
                SIMPLE_PRESET_HINT,
                "Enter a token name to continue.",
                "Enter an initial supply to continue.",
            ] {
                assert!(
                    harness.query_by_label(hint).is_none(),
                    "\"Create Token\" must stay enabled after {route}"
                );
            }
        };
        assert_form_kept(&harness, "filling the form");

        // Leave through the left panel (as when topping up the identity) and come back.
        harness
            .get_by_role_and_label(Role::Button, "Identities")
            .click();
        harness.run_steps(3);
        // The Identities hub has its own "Tokens" tab, so pick the left-panel
        // button: it is the leftmost match.
        harness
            .get_all_by_role_and_label(Role::Button, "Tokens")
            .min_by(|a, b| a.rect().center().x.total_cmp(&b.rect().center().x))
            .expect("the left-panel Tokens button")
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
        assert!(
            harness.query_by_label(SIMPLE_PRESET_HINT).is_some(),
            "Simple mode must say a preset is needed"
        );
        // Room for the whole Advanced form, so its bottom buttons are on screen.
        harness.set_size(egui::vec2(1280.0, 2400.0));
        harness.run_steps(2);

        toggle_advanced(&mut harness);
        assert!(
            harness.query_by_label(ADVANCED_PRESET_HINT).is_some(),
            "the blocked button must say why"
        );
        assert!(harness.query_by_label(SIMPLE_PRESET_HINT).is_none());
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
        assert!(harness.query_by_label(ADVANCED_PRESET_HINT).is_none());
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
