//! Identity Hub › Tokens tab: rows, actions, controls and states for the
//! selected identity. Design: `docs/ai-design/2026-10-28-identity-tokens/`.

use crate::identity_home::{key, seed_identity_with_id};
use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::tokens::TokenTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_contract::InsertTokensToo;
use dash_evo_tool::model::token::{
    IdentityTokenIdentifier, distribution_rules_with_once_per_identity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::identity::{IdentityHubScreen, IdentityHubTab, tokens_tab};
use dash_evo_tool::ui::{Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::{
    TokenConfigurationV0Getters, TokenConfigurationV0Setters,
};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{
    TokenConfigurationPreset, TokenConfigurationPresetFeatures,
};
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_localization::TokenConfigurationLocalization;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_localization::v0::TokenConfigurationLocalizationV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::config::DataContractConfig;
use dash_sdk::dpp::data_contract::v1::DataContractV1;
use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DataContract, Identifier};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

type HubHarness = Harness<'static, (IdentityHubScreen, Vec<AppAction>)>;

/// A token whose admin actions, if any, belong to the contract owner.
fn token_config(name: &str, admin: bool) -> TokenConfiguration {
    let preset = TokenConfigurationPreset {
        features: if admin {
            TokenConfigurationPresetFeatures::WithExtremeActions
        } else {
            TokenConfigurationPresetFeatures::MostRestrictive
        },
        action_taker: if admin {
            AuthorizedActionTakers::ContractOwner
        } else {
            AuthorizedActionTakers::NoOne
        },
    };
    TokenConfiguration::V0(preset.token_configuration_v0(
        TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
            localizations: BTreeMap::from([(
                "en".to_string(),
                TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                    should_capitalize: true,
                    singular_form: name.to_string(),
                    plural_form: format!("{name}s"),
                }),
            )]),
            decimals: 2,
        }),
        1_000_000,
        None,
        true,
        false,
    ))
}

/// Seeds a contract owned by `owner` carrying `tokens`, registered with every
/// token tracked; returns the token IDs by position.
fn seed_contract(
    app_context: &Arc<AppContext>,
    contract_byte: u8,
    owner: Identifier,
    tokens: Vec<TokenConfiguration>,
) -> Vec<Identifier> {
    let contract = DataContract::V1(DataContractV1 {
        id: Identifier::from([contract_byte; 32]),
        version: 1,
        owner_id: owner,
        document_types: BTreeMap::new(),
        config: DataContractConfig::default_for_version(PlatformVersion::latest())
            .expect("contract config"),
        schema_defs: None,
        groups: BTreeMap::new(),
        tokens: tokens
            .into_iter()
            .enumerate()
            .map(|(position, config)| (position as u16, config))
            .collect(),
        keywords: vec![],
        created_at: None,
        updated_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        created_at_epoch: None,
        updated_at_epoch: None,
        description: None,
    });
    app_context
        .insert_contract_if_not_exists(&contract, None, InsertTokensToo::AllTokensShouldBeAdded)
        .expect("seed contract");
    (0..contract.tokens().len() as u16)
        .map(|position| contract.token_id(position).expect("token id"))
        .collect()
}

/// Seeds and selects an identity that can sign on this device.
fn seed_signing_identity(app_context: &Arc<AppContext>, byte: u8) -> Identifier {
    let auth = key(1, Purpose::AUTHENTICATION);
    seed_identity_with_id(app_context, byte, vec![auth.clone()], vec![auth])
}

fn set_balance(
    app_context: &Arc<AppContext>,
    identity: Identifier,
    token: Identifier,
    balance: u64,
) {
    app_context
        .insert_token_identity_balance(&token, &identity, balance)
        .expect("seed balance");
}

fn mount_hub(app_context: &Arc<AppContext>, size: egui::Vec2) -> HubHarness {
    mount_hub_on(app_context, size, IdentityHubTab::Tokens)
}

fn mount_hub_on(
    app_context: &Arc<AppContext>,
    size: egui::Vec2,
    tab: IdentityHubTab,
) -> HubHarness {
    // The hub holds its proof-verifying reads until the masternode list is
    // synced; these tests start on a synced network.
    app_context.connection_status().set_masternodes_ready(true);
    let mut hub = Harness::builder().with_size(size).build_ui_state(
        |ui, state: &mut (IdentityHubScreen, Vec<AppAction>)| {
            let action = state.0.ui(ui);
            if !matches!(action, AppAction::None) {
                state.1.push(action);
            }
        },
        (IdentityHubScreen::new(app_context), Vec::new()),
    );
    hub.state_mut().0.select_tab(tab);
    hub.run_steps(4);
    hub
}

fn mount_tokens_tab(app_context: &Arc<AppContext>) -> HubHarness {
    mount_hub(app_context, egui::vec2(1280.0, 900.0))
}

/// The DashCoffee (held), Grit (zero) and Tester (unchecked) tokens of `identity`.
fn seed_three_tokens(app_context: &Arc<AppContext>, identity: Identifier) -> Vec<Identifier> {
    let tokens = seed_contract(
        app_context,
        0xC1,
        Identifier::from([0xEE; 32]),
        vec![
            token_config("DashCoffee", false),
            token_config("Grit", false),
            token_config("Tester", false),
        ],
    );
    set_balance(app_context, identity, tokens[0], 125_000);
    set_balance(app_context, identity, tokens[1], 0);
    tokens
}

#[test]
fn the_tokens_tab_sits_between_contacts_and_profile() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_signing_identity(&app_context, 0xA1);
        let mut hub = mount_hub_on(
            &app_context,
            egui::vec2(1280.0, 900.0),
            IdentityHubTab::Home,
        );
        // The left nav also has a Tokens button; the tab strip is the row of
        // buttons level with the Home tab.
        let tab_row_y = hub
            .get_by_role_and_label(Role::Button, "Home")
            .rect()
            .center()
            .y;
        let tab_center = |label: &str| {
            hub.get_all_by_role_and_label(Role::Button, label)
                .map(|node| node.rect().center())
                .find(|center| (center.y - tab_row_y).abs() < 2.0)
                .unwrap_or_else(|| panic!("no {label} tab on the tab row"))
        };
        let xs: Vec<f32> = ["Home", "Contacts", "Tokens", "Profile"]
            .map(|label| tab_center(label).x)
            .to_vec();
        assert!(
            xs.windows(2).all(|pair| pair[0] < pair[1]),
            "tabs must read Home · Contacts · Tokens · Profile, got {xs:?}"
        );
        assert!(hub.query_by_label(tokens_tab::SUBTITLE).is_none());
        let tokens_tab = tab_center("Tokens");
        hub.get_all_by_role_and_label(Role::Button, "Tokens")
            .find(|node| node.rect().center() == tokens_tab)
            .expect("the Tokens tab")
            .click();
        hub.run_steps(3);
        assert!(
            hub.query_by_label(tokens_tab::SUBTITLE).is_some(),
            "clicking the tab shows the Tokens tab"
        );
    });
}

#[test]
fn rows_show_balance_zero_and_checking_states_with_send_only_where_held() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA2);
        seed_three_tokens(&app_context, identity);
        let hub = mount_tokens_tab(&app_context);

        assert!(hub.query_by_label("DashCoffee").is_some());
        assert!(
            hub.query_by_label("1250 DashCoffee").is_some(),
            "the balance uses the token's decimals and name"
        );
        assert!(
            hub.query_by_label("0 Grit").is_some(),
            "a fetched zero is a normal row"
        );
        assert!(hub.query_by_label("Tester").is_some());
        assert!(
            hub.query_by_label(tokens_tab::STATUS_CHECKING).is_some(),
            "an unsynced balance is never shown as zero"
        );
        assert_eq!(
            hub.query_all_by_label(tokens_tab::SEND_LABEL).count(),
            1,
            "only the held token offers Send"
        );
        assert!(
            hub.query_by_label(tokens_tab::REFRESH_LABEL).is_some(),
            "the Refresh button is present (disabled while the arrival refresh runs)"
        );
        assert!(
            hub.state().1.iter().any(|action| matches!(
                action,
                AppAction::BackendTaskWithContext {
                    task: BackendTask::TokenTask(task),
                    ..
                } if matches!(**task, TokenTask::QueryMyTokenBalances)
            )),
            "arriving on the tab refreshes balances"
        );
    });
}

#[test]
fn hide_zero_hides_only_fetched_zeros_and_is_remembered() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA3);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);

        hub.get_by_label(tokens_tab::HIDE_ZERO_LABEL).click();
        hub.run_steps(3);
        assert!(
            hub.query_by_label("0 Grit").is_none(),
            "a fetched zero is hidden"
        );
        assert!(
            hub.query_by_label(tokens_tab::STATUS_CHECKING).is_some(),
            "an unknown balance is never hidden"
        );
        assert!(hub.query_by_label("1 hidden").is_some());
        assert!(app_context.identity_tokens_view_prefs().hide_zero_balances);

        let again = mount_tokens_tab(&app_context);
        assert!(
            again.query_by_label("0 Grit").is_none(),
            "the choice is remembered on this device"
        );
    });
}

#[test]
fn an_empty_registry_offers_both_ways_to_add_a_token() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_signing_identity(&app_context, 0xA4);
        let mut hub = mount_tokens_tab(&app_context);
        assert!(hub.query_by_label(tokens_tab::EMPTY_HEADING).is_some());
        assert!(hub.query_by_label(tokens_tab::EMPTY_BODY).is_some());
        hub.state_mut().1.clear();
        hub.get_by_label(tokens_tab::ADD_BY_ID_LABEL).click();
        hub.run_steps(2);
        assert!(
            hub.state()
                .1
                .iter()
                .any(|action| matches!(action, AppAction::AddScreen(Screen::AddTokenById(_)))),
            "Add by ID opens the import screen"
        );
    });
}

#[test]
fn stop_tracking_asks_first_then_dispatches_and_drops_the_row() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA5);
        let tokens = seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);

        hub.get_by_label("More actions for DashCoffee").click();
        hub.run_steps(2);
        hub.get_by_label(tokens_tab::STOP_TRACKING_LABEL).click();
        hub.run_steps(2);
        assert!(
            hub.query_by_label_contains("Stop tracking DashCoffee for")
                .is_some(),
            "a confirmation names the token and the identity"
        );
        assert!(
            hub.query_by_label("1250 DashCoffee").is_some(),
            "nothing changes before confirming"
        );
        hub.state_mut().1.clear();
        hub.get_by_label(tokens_tab::STOP_TRACKING_LABEL).click();
        hub.run_steps(3);

        let pair = IdentityTokenIdentifier {
            identity_id: identity,
            token_id: tokens[0],
        };
        assert!(
            hub.state().1.iter().any(|action| matches!(
                action,
                AppAction::BackendTaskWithContext {
                    task: BackendTask::TokenTask(task),
                    ..
                } if matches!(**task, TokenTask::StopTrackingTokenBalance(p) if p == pair)
            )),
            "confirming dispatches the stop-tracking task"
        );
        assert!(
            hub.query_by_label("1250 DashCoffee").is_none(),
            "the row leaves at once"
        );
        assert!(hub.query_by_label("0 Grit").is_some(), "other rows stay");
    });
}

#[test]
fn a_once_per_identity_claim_opens_the_claim_screen() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA6);
        let mut airdrop = token_config("AirDrop", false);
        let rules = match airdrop.distribution_rules().clone() {
            TokenDistributionRules::V0(rules) => rules,
            TokenDistributionRules::V1(_) => panic!("presets are version 0"),
        };
        airdrop.set_distribution_rules(distribution_rules_with_once_per_identity(rules, Some(100)));
        let tokens = seed_contract(
            &app_context,
            0xC2,
            Identifier::from([0xEE; 32]),
            vec![airdrop],
        );
        set_balance(&app_context, identity, tokens[0], 0);
        let mut hub = mount_tokens_tab(&app_context);

        assert!(
            hub.query_by_label("You can claim 1 AirDrop once.")
                .is_some(),
            "the once-per-identity amount is shown in the token's decimals"
        );
        assert!(
            hub.query_by_label(tokens_tab::SEND_LABEL).is_none(),
            "nothing to send"
        );
        assert!(
            hub.query_by_label("1").is_some(),
            "the tab badge counts the pending claim"
        );
        hub.state_mut().1.clear();
        hub.get_by_label(tokens_tab::CLAIM_LABEL).click();
        hub.run_steps(2);
        assert!(
            hub.state()
                .1
                .iter()
                .any(|action| matches!(action, AppAction::AddScreen(Screen::ClaimTokensScreen(_)))),
            "Claim pushes the claim screen scoped to this identity"
        );
    });
}

#[test]
fn the_manage_link_needs_a_real_admin_permission_not_a_role() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA7);
        app_context
            .set_and_persist_user_role(UserRole::Power)
            .expect("role");
        // Somebody else's token with owner-only admin actions, and our own.
        let theirs = seed_contract(
            &app_context,
            0xC3,
            Identifier::from([0xEE; 32]),
            vec![token_config("Theirs", true)],
        );
        let mine = seed_contract(
            &app_context,
            0xC4,
            identity,
            vec![token_config("Mine", true)],
        );
        set_balance(&app_context, identity, theirs[0], 500);
        set_balance(&app_context, identity, mine[0], 500);
        let mut hub = mount_tokens_tab(&app_context);

        assert!(
            hub.query_by_label(tokens_tab::ISSUED_BY_YOU_BADGE)
                .is_some(),
            "the identity issued one of the tokens"
        );
        hub.get_by_label("More actions for Theirs").click();
        hub.run_steps(2);
        assert!(
            hub.query_by_label(tokens_tab::MANAGE_LABEL).is_none(),
            "a Power user without permission gets no Manage link"
        );
        hub.get_by_label(tokens_tab::COPY_ID_LABEL).click();
        hub.run_steps(2);

        hub.get_by_label("More actions for Mine").click();
        hub.run_steps(2);
        assert!(
            hub.query_by_label(tokens_tab::MANAGE_LABEL).is_some(),
            "the issuer holds admin permissions, so Manage is offered"
        );
        assert!(
            hub.query_by_label("Mint").is_none() && hub.query_by_label("Burn").is_none(),
            "no admin action is rendered in the hub"
        );
    });
}

#[test]
fn a_failed_refresh_shows_inline_and_keeps_the_rows() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA8);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);

        let context = hub
            .state()
            .1
            .iter()
            .find_map(|action| match action {
                AppAction::BackendTaskWithContext {
                    task: BackendTask::TokenTask(task),
                    context,
                } if matches!(**task, TokenTask::QueryMyTokenBalances) => Some(context.clone()),
                _ => None,
            })
            .expect("the arrival refresh was dispatched");
        let elapsed = runtime
            .block_on(async {
                tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>()).await
            })
            .expect_err("elapsed");
        let error = TaskError::TokenBalanceRefreshTimeout { source: elapsed };
        hub.state_mut()
            .0
            .display_backend_task_error(&context, &error);
        assert!(
            hub.state_mut().0.display_task_error(&error),
            "the tab shows the failure itself, not a global banner"
        );
        hub.run_steps(2);
        assert!(hub.query_by_label(tokens_tab::ERROR_REFRESH_SLOW).is_some());
        assert!(hub.query_by_label(tokens_tab::TRY_AGAIN_LABEL).is_some());
        assert!(
            hub.query_by_label("1250 DashCoffee").is_some(),
            "known balances stay"
        );

        // A refresh already running elsewhere is not a failure to the user.
        let busy = TaskError::TokenBalanceRefreshInProgress;
        hub.state_mut()
            .0
            .display_backend_task_error(&BackendTaskContext::TokenBalanceRefresh, &busy);
        assert!(
            !hub.state_mut().0.display_task_error(&busy),
            "an error of another screen's refresh is not this tab's to swallow"
        );
    });
}

#[test]
fn the_list_scrolls_under_a_fixed_header_on_a_short_window() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA9);
        let tokens = seed_contract(
            &app_context,
            0xC5,
            Identifier::from([0xEE; 32]),
            (0..12)
                .map(|i| token_config(&format!("Token{i:02}"), false))
                .collect(),
        );
        for (i, token) in tokens.iter().enumerate() {
            set_balance(&app_context, identity, *token, 100 * (i as u64 + 1));
        }
        let hub = mount_hub(&app_context, egui::vec2(1280.0, 800.0));

        for label in [tokens_tab::REFRESH_LABEL, tokens_tab::HIDE_ZERO_LABEL] {
            let rect = hub.get_by_label(label).rect();
            assert!(
                rect.max.y < 800.0,
                "{label} must stay on screen, got {rect:?}"
            );
        }
        let first = hub.get_by_label("Token11").rect();
        assert!(
            first.max.y < 800.0,
            "the first row is visible, got {first:?}"
        );
        assert!(
            hub.query_by_label("12 tokens").is_some(),
            "a long list shows its count"
        );
    });
}

#[test]
fn the_send_screen_uses_send_wording() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xAA);
        seed_three_tokens(&app_context, identity);
        let balance = app_context
            .identity_token_balances()
            .expect("balances")
            .into_values()
            .find(|balance| balance.token_alias == "DashCoffee")
            .expect("the held token");
        let screen = ScreenType::TransferTokensScreen(balance).create_screen(&app_context);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut Screen| {
                    let _ = screen.ui(ui);
                },
                screen,
            );
        harness.run();
        assert!(harness.query_by_label("Send DashCoffee").is_some());
        assert!(
            harness.query_all_by_label(tokens_tab::SEND_LABEL).count() >= 1,
            "the breadcrumb and the primary button say Send"
        );
        assert!(
            harness.query_by_label_contains("Transfer").is_none(),
            "holder copy says Send everywhere"
        );
    });
}
