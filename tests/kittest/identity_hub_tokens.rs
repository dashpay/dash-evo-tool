//! Identity Hub › Tokens tab: rows, actions, controls and states for the
//! selected identity. Design: `docs/ai-design/2026-10-28-identity-tokens/`.

use crate::identity_home::{key, seed_funded_identity_with_id, seed_identity_with_id};
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
use dash_evo_tool::ui::identity::{IdentityHubScreen, IdentityHubTab, token_copy};
use dash_evo_tool::ui::tokens::tokens_screen::{TokensScreen, TokensSubscreen};
use dash_evo_tool::ui::tokens::transfer_tokens_screen::{
    CONFIRM_SEND_TITLE, send_question, sending_text, sent_text,
};
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::{
    TokenConfigurationV0Getters, TokenConfigurationV0Setters,
};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Setters;
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
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DataContract, Identifier};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::NodeT;
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
        assert!(hub.query_by_label(token_copy::SUBTITLE).is_none());
        let tokens_tab = tab_center("Tokens");
        hub.get_all_by_role_and_label(Role::Button, "Tokens")
            .find(|node| node.rect().center() == tokens_tab)
            .expect("the Tokens tab")
            .click();
        hub.run_steps(3);
        assert!(
            hub.query_by_label(token_copy::SUBTITLE).is_some(),
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
            hub.query_by_label(token_copy::STATUS_CHECKING).is_some(),
            "an unsynced balance is never shown as zero"
        );
        assert_eq!(
            hub.query_all_by_label(token_copy::SEND_LABEL).count(),
            1,
            "only the held token offers Send"
        );
        assert!(
            hub.query_by_label(token_copy::REFRESH_LABEL).is_some(),
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

        hub.get_by_label(token_copy::HIDE_ZERO_LABEL).click();
        hub.run_steps(3);
        assert!(
            hub.query_by_label("0 Grit").is_none(),
            "a fetched zero is hidden"
        );
        assert!(
            hub.query_by_label(token_copy::STATUS_CHECKING).is_some(),
            "an unknown balance is never hidden"
        );
        assert!(
            hub.query_by_label(&token_copy::hidden_count_text(1))
                .is_some()
        );
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
        assert!(hub.query_by_label(token_copy::EMPTY_HEADING).is_some());
        assert!(hub.query_by_label(token_copy::EMPTY_BODY).is_some());
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::ADD_BY_ID_LABEL).click();
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
        hub.get_by_label(token_copy::STOP_TRACKING_LABEL).click();
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
        hub.get_by_label(token_copy::STOP_TRACKING_LABEL).click();
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
            hub.query_by_label(token_copy::SEND_LABEL).is_none(),
            "nothing to send"
        );
        let tab_row_y = hub
            .get_by_role_and_label(Role::Button, "Home")
            .rect()
            .center()
            .y;
        assert!(
            hub.query_all_by_label("1")
                .any(|node| (node.rect().center().y - tab_row_y).abs() < 4.0),
            "the tab badge on the tab row counts the pending claim"
        );
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::CLAIM_LABEL).click();
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
            hub.query_by_label(token_copy::ISSUED_BY_YOU_BADGE)
                .is_some(),
            "the identity issued one of the tokens"
        );
        hub.get_by_label("More actions for Theirs").click();
        hub.run_steps(2);
        assert!(
            hub.query_by_label(token_copy::MANAGE_LABEL).is_none(),
            "a Power user without permission gets no Manage link"
        );
        hub.get_by_label(token_copy::COPY_ID_LABEL).click();
        hub.run_steps(2);

        hub.get_by_label("More actions for Mine").click();
        hub.run_steps(2);
        assert!(
            hub.query_by_label(token_copy::MANAGE_LABEL).is_some(),
            "the issuer holds admin permissions, so Manage is offered"
        );
        assert!(
            [
                "Mint",
                "Burn",
                "Freeze",
                "Unfreeze",
                "Destroy frozen funds",
                "Pause",
                "Resume",
                "Set price",
                "Purchase",
                "Update configuration",
            ]
            .iter()
            .all(|admin| hub.query_by_label(admin).is_none()),
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
        assert!(hub.query_by_label(token_copy::ERROR_REFRESH_SLOW).is_some());
        assert!(
            hub.query_all_by_label(token_copy::TRY_AGAIN_LABEL).count() >= 1,
            "the notice offers a retry"
        );
        assert!(
            hub.query_by_label("1250 DashCoffee").is_some(),
            "known balances stay"
        );
        assert!(
            hub.query_by_label(token_copy::STATUS_COULD_NOT_CHECK)
                .is_some(),
            "an unknown balance is 'could not check' after a failed refresh, not 'checking'"
        );
        assert!(
            hub.query_by_label(token_copy::FIRST_LOAD).is_none(),
            "the first-load line never shows next to a failure"
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

        for label in [token_copy::REFRESH_LABEL, token_copy::HIDE_ZERO_LABEL] {
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
            hub.query_by_label(&token_copy::tracked_count_text(12))
                .is_some(),
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
            harness.query_all_by_label(token_copy::SEND_LABEL).count() >= 1,
            "the breadcrumb and the primary button say Send"
        );
        assert!(
            harness.query_by_label_contains("Transfer").is_none(),
            "holder copy says Send everywhere"
        );
    });
}

#[test]
fn the_home_card_shows_only_with_a_balance_or_a_claim() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xB1);
        let tokens = seed_three_tokens(&app_context, identity);
        let mut hub = mount_hub_on(
            &app_context,
            egui::vec2(1280.0, 900.0),
            IdentityHubTab::Home,
        );
        assert!(
            hub.query_by_label(token_copy::SEE_ALL_TOKENS_LABEL)
                .is_some()
        );
        assert!(
            hub.query_by_label("1250 DashCoffee").is_some(),
            "the card lists what the identity holds"
        );
        assert!(
            hub.query_by_label("0 Grit").is_none(),
            "zero balances are not worth a glance"
        );
        hub.get_by_label(token_copy::SEE_ALL_TOKENS_LABEL).click();
        hub.run_steps(3);
        assert_eq!(hub.state().0.selected_tab(), IdentityHubTab::Tokens);

        // Nothing held and nothing to claim: no card.
        set_balance(&app_context, identity, tokens[0], 0);
        let hub = mount_hub_on(
            &app_context,
            egui::vec2(1280.0, 900.0),
            IdentityHubTab::Home,
        );
        assert!(
            hub.query_by_label(token_copy::SEE_ALL_TOKENS_LABEL)
                .is_none(),
            "an empty card would only advertise a feature"
        );
    });
}

#[test]
fn the_detail_page_opens_from_the_row_and_goes_back() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xB2);
        let tokens = seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);

        hub.get_by_label("More actions for DashCoffee").click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::DETAILS_LABEL).click();
        hub.run_steps(3);
        assert!(hub.query_by_label(token_copy::BACK_LABEL).is_some());
        assert!(hub.query_by_label_contains("Held by").is_some());
        assert!(hub.query_by_label("1250 DashCoffee").is_some());
        assert!(hub.query_by_label(token_copy::CAN_BE_SENT_YES).is_some());
        assert!(hub.query_by_label(token_copy::REWARDS_NONE).is_some());
        assert!(
            hub.query_by_label(
                &tokens[0]
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            )
            .is_some(),
            "the full token ID is shown for copying"
        );
        assert!(hub.query_by_label(token_copy::SEND_LABEL).is_some());
        assert!(
            hub.query_by_label(token_copy::MANAGE_LABEL).is_none(),
            "no admin permission, no Manage link"
        );
        assert!(
            hub.query_by_label(token_copy::SUBTITLE).is_none(),
            "the list is replaced by the page"
        );

        hub.get_by_label(token_copy::BACK_LABEL).click();
        hub.run_steps(3);
        assert!(hub.query_by_label(token_copy::SUBTITLE).is_some());

        // The row name opens the page too.
        hub.get_by_label("Grit").click();
        hub.run_steps(3);
        assert!(hub.query_by_label(token_copy::BACK_LABEL).is_some());
        assert!(hub.query_by_label("0 Grit").is_some());
    });
}

#[test]
fn check_balance_now_waits_for_the_masternode_list() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD2);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        app_context.connection_status().set_masternodes_ready(false);
        hub.run_steps(2);
        hub.state_mut().1.clear();
        hub.get_by_label("More actions for DashCoffee").click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::CHECK_NOW_LABEL).click();
        hub.run_steps(2);
        let dispatched = hub.state().1.iter().any(|action| {
            matches!(
                action,
                AppAction::BackendTaskWithContext { task: BackendTask::TokenTask(task), .. }
                    if matches!(**task, TokenTask::QueryIdentityTokenBalance(_))
            )
        });
        assert!(
            !dispatched,
            "a proof-verifying balance read went out before the masternode list synced"
        );
        assert!(
            hub.get_by_label(token_copy::REFRESH_LABEL)
                .accesskit_node()
                .is_disabled(),
            "Refresh stays visible, disabled until sync finishes"
        );
    });
}

#[test]
fn a_pre_programmed_distribution_to_someone_else_is_not_a_pending_claim() {
    use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::TokenPreProgrammedDistribution;
    use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::v0::TokenPreProgrammedDistributionV0;
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD3);
        let mut vest = token_config("Vest", false);
        let mut rules = match vest.distribution_rules().clone() {
            TokenDistributionRules::V0(rules) => rules,
            TokenDistributionRules::V1(_) => panic!("presets are version 0"),
        };
        // Only somebody else is a recipient.
        rules.pre_programmed_distribution = Some(TokenPreProgrammedDistribution::V0(
            TokenPreProgrammedDistributionV0 {
                distributions: BTreeMap::from([(
                    1_000u64,
                    BTreeMap::from([(Identifier::from([0x77; 32]), 500u64)]),
                )]),
            },
        ));
        vest.set_distribution_rules(TokenDistributionRules::V0(rules));
        let tokens = seed_contract(&app_context, 0xC9, Identifier::from([0xEE; 32]), vec![vest]);
        set_balance(&app_context, identity, tokens[0], 0);
        let hub = mount_hub_on(
            &app_context,
            egui::vec2(1280.0, 900.0),
            IdentityHubTab::Home,
        );
        assert!(
            hub.query_by_label(token_copy::SEE_ALL_TOKENS_LABEL)
                .is_none(),
            "nothing held and nothing to claim: no Home card"
        );
        let hub = mount_tokens_tab(&app_context);
        assert!(hub.query_by_label(token_copy::CLAIM_LABEL).is_none());
        assert!(hub.query_by_label(token_copy::STATUS_REWARDS).is_none());
    });
}

/// Eight tokens named Alpha…Hotel, every balance known.
fn seed_eight_tokens(app_context: &Arc<AppContext>, identity: Identifier) -> Vec<Identifier> {
    let names = [
        "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel",
    ];
    let tokens = seed_contract(
        app_context,
        0xC8,
        Identifier::from([0xEE; 32]),
        names.iter().map(|name| token_config(name, false)).collect(),
    );
    for (index, token) in tokens.iter().enumerate() {
        set_balance(app_context, identity, *token, (index as u64 + 1) * 100);
    }
    tokens
}

#[test]
fn the_remembered_filter_does_not_hide_rows_while_its_box_is_hidden() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD1);
        seed_three_tokens(&app_context, identity);
        // A filter typed earlier on a long list is remembered per device.
        app_context
            .save_identity_tokens_view_prefs(
                &dash_evo_tool::model::token::IdentityTokensViewPrefs {
                    filter: "Coffee".into(),
                    ..Default::default()
                },
            )
            .expect("prefs");
        let hub = mount_tokens_tab(&app_context);
        assert!(
            hub.query_by_label("0 Grit").is_some(),
            "a filter the user cannot see or clear hides nothing"
        );
        assert!(hub.query_by_label_contains("hidden.").is_none());
    });
}

#[test]
fn filter_and_sort_appear_from_eight_tokens_and_the_filter_applies() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD4);
        seed_eight_tokens(&app_context, identity);
        app_context
            .save_identity_tokens_view_prefs(
                &dash_evo_tool::model::token::IdentityTokensViewPrefs {
                    filter: "alpha".into(),
                    ..Default::default()
                },
            )
            .expect("prefs");
        let hub = mount_tokens_tab(&app_context);
        assert!(
            hub.query_by_label(&token_copy::tracked_count_text(8))
                .is_some()
        );
        assert!(
            hub.query_by_label(token_copy::SORT_LABEL).is_some(),
            "the sort control shows for a long list"
        );
        assert!(hub.query_by_label("1 Alpha").is_some());
        assert!(
            hub.query_by_label("2 Bravo").is_none(),
            "the filter applies while its box is shown"
        );
        assert!(
            hub.query_by_label(&token_copy::hidden_count_text(7))
                .is_some()
        );
    });
}

#[test]
fn a_view_only_identity_is_told_to_add_a_key_and_cannot_send() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let auth = key(1, Purpose::AUTHENTICATION);
        let identity = seed_identity_with_id(&app_context, 0xD5, vec![auth], vec![]);
        seed_three_tokens(&app_context, identity);
        let hub = mount_tokens_tab(&app_context);
        assert!(hub.query_by_label(token_copy::STATUS_ADD_KEY).is_some());
        assert!(
            hub.get_by_label(token_copy::SEND_LABEL)
                .accesskit_node()
                .is_disabled(),
            "nothing can be signed on this device"
        );
    });
}

#[test]
fn a_token_that_cannot_be_sent_shows_the_badge_and_a_disabled_send() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD6);
        let mut fixed = token_config("Fixed", false);
        fixed.set_transferable(false);
        let tokens = seed_contract(
            &app_context,
            0xC7,
            Identifier::from([0xEE; 32]),
            vec![fixed],
        );
        set_balance(&app_context, identity, tokens[0], 500);
        let hub = mount_tokens_tab(&app_context);
        assert!(hub.query_by_label(token_copy::CANT_BE_SENT_BADGE).is_some());
        assert!(
            hub.get_by_label(token_copy::SEND_LABEL)
                .accesskit_node()
                .is_disabled(),
            "Send stays visible for a held token but cannot be used"
        );
    });
}

#[test]
fn track_tokens_again_sends_one_request_this_tab_waits_on() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD7);
        let tokens = seed_three_tokens(&app_context, identity);
        let pairs: Vec<IdentityTokenIdentifier> = tokens
            .iter()
            .map(|token| IdentityTokenIdentifier {
                identity_id: identity,
                token_id: *token,
            })
            .collect();
        for pair in &pairs {
            app_context
                .mark_token_balance_untracked(*pair)
                .expect("dismiss");
        }
        let mut hub = mount_tokens_tab(&app_context);
        assert!(hub.query_by_label(token_copy::DISMISSED_HEADING).is_some());

        // The arrival refresh is still running: nothing else may ask the network.
        assert!(
            hub.get_by_label(token_copy::TRACK_AGAIN_LABEL)
                .accesskit_node()
                .is_disabled()
        );
        let arrival = hub
            .state()
            .1
            .iter()
            .find_map(|action| match action {
                AppAction::BackendTaskWithContext {
                    task: BackendTask::TokenTask(task),
                    context,
                    ..
                } if matches!(**task, TokenTask::QueryMyTokenBalances) => Some(context.clone()),
                _ => None,
            })
            .expect("the arrival refresh");
        hub.state_mut().0.display_backend_task_result(
            &arrival,
            dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedTokenBalances,
        );
        hub.run_steps(2);
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::TRACK_AGAIN_LABEL).click();
        hub.run_steps(2);
        let tracked = hub.state().1.iter().find_map(|action| match action {
            AppAction::BackendTaskWithContext {
                task: BackendTask::TokenTask(task),
                ..
            } => match &**task {
                TokenTask::TrackTokenBalancesAgain(pairs) => Some(pairs.clone()),
                _ => None,
            },
            _ => None,
        });
        let mut tracked = tracked.expect("one request this tab waits on");
        tracked.sort();
        let mut expected = pairs.clone();
        expected.sort();
        assert_eq!(tracked, expected);

        // And never before the masternode list is synced.
        app_context.connection_status().set_masternodes_ready(false);
        hub.run_steps(2);
        assert!(
            hub.get_by_label(token_copy::TRACK_AGAIN_LABEL)
                .accesskit_node()
                .is_disabled()
        );
    });
}

#[test]
fn the_receive_link_opens_receive_with_the_tokens_hint() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD8);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        hub.get_by_label(token_copy::RECEIVE_LINK).click();
        hub.run_steps(3);
        assert!(
            hub.query_by_label(token_copy::RECEIVE_TOKENS_HINT)
                .is_some()
        );
    });
}

#[test]
fn find_a_token_by_name_opens_token_search() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD9);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        hub.get_by_label(token_copy::ADD_TOKEN_LABEL).click();
        hub.run_steps(2);
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::FIND_BY_NAME_LABEL).click();
        hub.run_steps(2);
        assert!(hub.state().1.iter().any(|action| matches!(
            action,
            AppAction::SetMainScreen(RootScreenType::RootScreenTokenSearch)
        )));
    });
}

#[test]
fn the_home_card_shows_for_a_claim_alone_and_claims_right_there() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xDA);
        let mut airdrop = token_config("AirDrop", false);
        let rules = match airdrop.distribution_rules().clone() {
            TokenDistributionRules::V0(rules) => rules,
            TokenDistributionRules::V1(_) => panic!("presets are version 0"),
        };
        airdrop.set_distribution_rules(distribution_rules_with_once_per_identity(rules, Some(100)));
        let tokens = seed_contract(
            &app_context,
            0xC3,
            Identifier::from([0xEE; 32]),
            vec![airdrop],
        );
        set_balance(&app_context, identity, tokens[0], 0);
        let mut hub = mount_hub_on(
            &app_context,
            egui::vec2(1280.0, 900.0),
            IdentityHubTab::Home,
        );
        assert!(
            hub.query_by_label(token_copy::SEE_ALL_TOKENS_LABEL)
                .is_some(),
            "a pending claim alone earns the card"
        );
        assert!(
            hub.query_by_label("You can claim 1 AirDrop once.")
                .is_some()
        );
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::CLAIM_LABEL).click();
        hub.run_steps(2);
        assert!(
            hub.state()
                .1
                .iter()
                .any(|action| matches!(action, AppAction::AddScreen(Screen::ClaimTokensScreen(_)))),
            "Claim on the Home card claims; it does not merely switch tabs"
        );
        assert_eq!(hub.state().0.selected_tab(), IdentityHubTab::Home);
    });
}

#[test]
fn clicking_anywhere_on_a_row_opens_its_detail_page() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xDB);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        hub.get_by_label("1250 DashCoffee").click();
        hub.run_steps(3);
        assert!(hub.query_by_label(token_copy::BACK_LABEL).is_some());
        assert!(hub.query_by_label_contains("Held by").is_some());
    });
}

#[test]
fn manage_opens_my_tokens_on_this_token() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xDC);
        let tokens = seed_contract(
            &app_context,
            0xC6,
            identity,
            vec![token_config("Mine", true)],
        );
        set_balance(&app_context, identity, tokens[0], 10);
        let mut hub = mount_tokens_tab(&app_context);
        hub.get_by_label("More actions for Mine").click();
        hub.run_steps(2);
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::MANAGE_LABEL).click();
        hub.run_steps(2);
        assert!(hub.state().1.iter().any(|action| matches!(
            action,
            AppAction::SetMainScreen(RootScreenType::RootScreenMyTokenBalances)
        )));
        // What app.rs does on SetMainScreen: the root Tokens screen arrives.
        let mut my_tokens = TokensScreen::new(&app_context, TokensSubscreen::MyTokens);
        my_tokens.refresh_on_arrival();
        assert_eq!(
            my_tokens.selected_token,
            Some(tokens[0]),
            "My Tokens opens on this token"
        );
        assert_eq!(
            app_context.take_pending_token_selection(),
            None,
            "the request is consumed on arrival"
        );
    });
}

#[test]
fn the_send_screen_confirms_sends_and_reports_in_send_wording() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let auth = key(1, Purpose::AUTHENTICATION);
        let identity = seed_funded_identity_with_id(
            &app_context,
            0xDD,
            vec![auth.clone()],
            vec![auth],
            1_000_000_000,
        );
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
        let recipient = Identifier::from([0x55; 32]).to_string(Encoding::Base58);
        // The amount field comes first, the recipient's free-text field second.
        harness
            .get_all_by_role(Role::TextInput)
            .next()
            .expect("amount")
            .focus();
        harness.run();
        harness
            .get_all_by_role(Role::TextInput)
            .next()
            .expect("amount")
            .type_text("1");
        harness.run();
        harness
            .get_all_by_role(Role::TextInput)
            .nth(1)
            .expect("recipient")
            .focus();
        harness.run();
        harness
            .get_all_by_role(Role::TextInput)
            .nth(1)
            .expect("recipient")
            .type_text(&recipient);
        harness.run_steps(2);
        // The breadcrumb's current item is a "Send" button too; the primary
        // button comes last.
        harness
            .get_all_by_role_and_label(Role::Button, token_copy::SEND_LABEL)
            .rfind(|node| !node.accesskit_node().is_disabled())
            .expect("an enabled Send button")
            .click();
        harness.run_steps(2);
        let amount = dash_evo_tool::model::amount::Amount::new(100, 2).with_unit_name("DashCoffee");
        assert!(harness.query_by_label(CONFIRM_SEND_TITLE).is_some());
        assert!(
            harness
                .query_by_label(&send_question(&amount, &recipient))
                .is_some(),
            "the confirmation names the amount and the recipient"
        );
        assert_eq!(sending_text(&amount), "Sending 1 DashCoffee…");
        assert_eq!(
            sent_text(&amount, &recipient),
            format!("Sent 1 DashCoffee to {recipient}.")
        );
        assert!(harness.query_by_label_contains("Transfer").is_none());
    });
}

/// The filter is written to the device when its box loses focus, not on
/// every keystroke; the edit itself is never lost.
#[test]
fn the_filter_is_saved_when_its_box_loses_focus_not_while_typing() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD6);
        seed_eight_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        hub.get_by_role(Role::TextInput).click();
        hub.run_steps(2);
        hub.get_by_role(Role::TextInput).type_text("alp");
        hub.run_steps(4);
        assert!(
            hub.get_by_role(Role::TextInput).is_focused(),
            "the box keeps focus while typing"
        );
        assert_eq!(
            app_context.identity_tokens_view_prefs().filter,
            "",
            "typing must not write the filter to the database"
        );
        assert!(
            hub.query_by_label("2 Bravo").is_none() && hub.query_by_label("1 Alpha").is_some(),
            "the typed filter applies at once"
        );
        hub.key_press(egui::Key::Enter);
        hub.run_steps(4);
        assert_eq!(
            app_context.identity_tokens_view_prefs().filter,
            "alp",
            "leaving the box writes the filter once"
        );
    });
}
