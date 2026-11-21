//! Identity Hub › Tokens tab: rows, actions, controls and states for the
//! selected identity. Design: `docs/ai-design/2026-10-28-identity-tokens/`.

use crate::identity_home::{key, seed_identity_with_id};
use crate::identity_hub_layout::{HubHarness, hub_harness};
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
use dash_evo_tool::ui::identity::{IdentityHubScreen, IdentityHubTab, token_copy};
use dash_evo_tool::ui::tokens::tokens_screen::{TokensScreen, TokensSubscreen};
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
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
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DataContract, Identifier};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::NodeT;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

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
    mount_hub_as_is(app_context, size, tab)
}

/// Mounts the hub on whatever sync state the network is in.
fn mount_hub_as_is(
    app_context: &Arc<AppContext>,
    size: egui::Vec2,
    tab: IdentityHubTab,
) -> HubHarness {
    let mut hub = hub_harness(app_context, size);
    hub.state_mut().0.select_tab(tab);
    hub.run_steps(4);
    hub
}

fn mount_tokens_tab(app_context: &Arc<AppContext>) -> HubHarness {
    mount_hub(app_context, egui::vec2(1280.0, 900.0))
}

/// A pair check: a track-again request for exactly one pair.
fn is_pair_check(task: &TokenTask) -> bool {
    matches!(task, TokenTask::TrackTokenBalancesAgain(pairs) if pairs.len() == 1)
}

/// Part of the timeout error's text, which the failed refresh shows inline.
const TIMEOUT_TEXT: &str = "the network took too long to respond";

/// The error a timed-out network request carries.
fn elapsed_timeout(runtime: &tokio::runtime::Runtime) -> tokio::time::error::Elapsed {
    runtime
        .block_on(async {
            tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>()).await
        })
        .expect_err("elapsed")
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
fn rows_show_balance_zero_and_checking_states_without_send() {
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
            hub.query_all_by_label("Send").count(),
            0,
            "the hub does not offer Send"
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
        // Every input on the tab is named for screen readers.
        hub.check_accessibility();
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
        // Every input on the tab is named for screen readers.
        hub.check_accessibility();
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
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::FIND_BY_NAME_LABEL).click();
        hub.run_steps(2);
        assert!(
            hub.state().1.iter().any(|action| matches!(
                action,
                AppAction::SetMainScreen(RootScreenType::RootScreenTokenSearch)
            )),
            "Find a token by name opens token search"
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
fn a_once_per_identity_claim_is_shown_without_a_claim_action() {
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
        let hub = mount_tokens_tab(&app_context);

        assert!(
            hub.query_by_label("You can claim 1 AirDrop once in Tokens › My Tokens.")
                .is_some(),
            "the once-per-identity amount is shown in the token's decimals"
        );
        assert!(hub.query_by_label("Send").is_none(), "nothing to send");
        assert!(hub.query_by_label("1").is_none());
        assert!(hub.query_by_label("Claim").is_none());
    });
}

/// A refresh that finds no balance keeps the reward lines: an open
/// once-per-identity claim and a scheduled distribution to this identity.
#[test]
fn reward_information_shows_when_a_refresh_finds_no_balance() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xA9);
        let mut airdrop = token_config("AirDrop", false);
        let rules = match airdrop.distribution_rules().clone() {
            TokenDistributionRules::V0(rules) => rules,
            TokenDistributionRules::V1(_) => panic!("presets are version 0"),
        };
        airdrop.set_distribution_rules(distribution_rules_with_once_per_identity(rules, Some(100)));
        seed_contract(
            &app_context,
            0xCA,
            Identifier::from([0xEE; 32]),
            vec![airdrop],
        );
        let tomorrow = chrono::Utc::now().timestamp_millis() as u64 + 86_400_000;
        seed_contract(
            &app_context,
            0xCB,
            Identifier::from([0xEE; 32]),
            vec![pre_programmed_token("Future", false, tomorrow, identity)],
        );
        let mut hub = mount_tokens_tab(&app_context);
        let arrival = balance_refreshes(&hub).pop().expect("arrival refresh");
        hub.state_mut().0.display_backend_task_result(
            &arrival,
            dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedTokenBalances,
        );
        hub.run_steps(2);

        assert_eq!(
            hub.query_all_by_label(token_copy::STATUS_NO_BALANCE_FOUND)
                .count(),
            2
        );
        assert!(
            hub.query_by_label("You can claim 1 AirDrop once in Tokens › My Tokens.")
                .is_some()
        );
        assert!(hub.query_by_label(token_copy::STATUS_REWARDS).is_some());
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
        let error = TaskError::TokenBalanceRefreshTimeout {
            source: elapsed_timeout(&runtime),
        };
        hub.state_mut()
            .0
            .display_backend_task_error(&context, &error);
        assert!(
            !hub.state_mut().0.display_task_error(&error),
            "a real failure always reaches the banner, with the inline notice's text"
        );
        hub.run_steps(2);
        assert!(hub.query_by_label_contains(TIMEOUT_TEXT).is_some());
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
        let foreign = BackendTaskContext::TokenBalanceRefresh;
        assert!(
            !hub.state()
                .0
                .should_suppress_backend_task_error(&foreign, &busy),
            "an error of another screen's refresh is not this tab's to swallow"
        );
        hub.state_mut()
            .0
            .display_backend_task_error(&foreign, &busy);
        // Every input on the tab is named for screen readers.
        hub.check_accessibility();
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
    });
}

/// Every full balance refresh the hub has asked for, oldest first.
fn balance_refreshes(hub: &HubHarness) -> Vec<BackendTaskContext> {
    hub.state()
        .1
        .iter()
        .filter_map(|action| match action {
            AppAction::BackendTaskWithContext {
                task: BackendTask::TokenTask(task),
                context,
            } if matches!(**task, TokenTask::QueryMyTokenBalances) => Some(context.clone()),
            _ => None,
        })
        .collect()
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
        assert!(
            !asked_for_a_pair_check(&hub),
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
fn check_balance_now_says_why_it_waits_for_a_running_refresh() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD3);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        let arrival = balance_refreshes(&hub)
            .pop()
            .expect("the arrival refresh is running");

        hub.get_by_label("More actions for DashCoffee").click();
        hub.run_steps(2);
        assert_refresh_blocks(&mut hub, token_copy::CHECK_NOW_LABEL);

        hub.state_mut().0.display_backend_task_result(
            &arrival,
            dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedTokenBalances,
        );
        hub.run_steps(2);
        hub.state_mut().1.clear();
        // The completed refresh changes row contents and can close the menu.
        hub.key_press(egui::Key::Escape);
        hub.run_steps(2);
        hub.get_by_label("More actions for DashCoffee").click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::CHECK_NOW_LABEL).click();
        hub.run_steps(2);
        assert!(
            asked_for_a_pair_check(&hub),
            "the check works again once the refresh ended"
        );
    });
}

/// Every `label` control is disabled and its tooltip explains the running refresh.
fn assert_refresh_blocks(hub: &mut HubHarness, label: &str) {
    let status_lines = hub
        .query_all_by_label(token_copy::REFRESHING_TOOLTIP)
        .count();
    assert!(
        hub.query_all_by_label(label)
            .all(|node| node.accesskit_node().is_disabled()),
        "{label} cannot run while a refresh does, so it must not look clickable"
    );
    hub.query_all_by_label(label)
        .next()
        .expect("the control is shown")
        .hover();
    // Tooltips appear after a short delay; step a few frames past it.
    hub.run_steps(30);
    assert!(
        hub.query_all_by_label(token_copy::REFRESHING_TOOLTIP)
            .count()
            > status_lines,
        "the disabled control explains why it is disabled"
    );
}

fn asked_for_a_pair_check(hub: &HubHarness) -> bool {
    hub.state().1.iter().any(|action| {
        matches!(
            action,
            AppAction::BackendTaskWithContext { task: BackendTask::TokenTask(task), .. }
                if is_pair_check(task)
        )
    })
}

#[test]
fn try_again_on_a_failed_row_says_why_it_waits_for_a_running_refresh() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xD5);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        let arrival = balance_refreshes(&hub)
            .pop()
            .expect("the arrival refresh is running");
        let timed_out = || TaskError::TokenBalanceRefreshTimeout {
            source: elapsed_timeout(&runtime),
        };
        hub.state_mut()
            .0
            .display_backend_task_error(&arrival, &timed_out());
        hub.run_steps(2);
        assert!(
            hub.query_by_label(token_copy::STATUS_COULD_NOT_CHECK)
                .is_some(),
            "precondition: a row whose balance could not be checked"
        );

        hub.get_by_label(token_copy::REFRESH_LABEL).click();
        hub.run_steps(2);
        assert_eq!(balance_refreshes(&hub).len(), 2, "a second refresh runs");
        assert!(
            hub.query_by_label(token_copy::STATUS_COULD_NOT_CHECK)
                .is_some(),
            "the row still offers its retry while the refresh runs"
        );
        assert_refresh_blocks(&mut hub, token_copy::TRY_AGAIN_LABEL);

        let running = balance_refreshes(&hub)
            .pop()
            .expect("the second refresh is running");
        hub.state_mut()
            .0
            .display_backend_task_error(&running, &timed_out());
        hub.run_steps(2);
        hub.state_mut().1.clear();
        hub.query_all_by_label(token_copy::TRY_AGAIN_LABEL)
            .last()
            .expect("the row's retry")
            .click();
        hub.run_steps(2);
        assert!(
            asked_for_a_pair_check(&hub),
            "the retry works again once the refresh ended"
        );
    });
}

type RoutedApp = Harness<'static, dash_evo_tool::app::AppState>;

fn routed_hub(harness: &mut RoutedApp) -> &mut IdentityHubScreen {
    match harness
        .state_mut()
        .main_screens
        .get_mut(&RootScreenType::RootScreenIdentityHub)
    {
        Some(Screen::IdentityHubScreen(hub)) => hub,
        _ => panic!("the hub is open"),
    }
}

fn deliver_to_app(harness: &mut RoutedApp, result: dash_evo_tool::app::TaskResult) {
    assert!(harness.state().task_result_sender.try_send(result).is_ok());
    harness.run_steps(3);
}

fn routed_balance_refreshes(harness: &RoutedApp) -> Vec<BackendTaskContext> {
    harness
        .state()
        .test_captured_dispatches()
        .iter()
        .filter(|(task, _)| {
            matches!(task, BackendTask::TokenTask(task)
                if matches!(**task, TokenTask::QueryMyTokenBalances))
        })
        .map(|(_, context)| context.clone())
        .collect()
}

/// On the real `AppState`: the hub's own refresh failed, then My Tokens came
/// into view and `task` finished there. Returns the app back on the hub, with
/// how many refreshes were asked for after that arrival began.
fn return_to_the_hub_after_my_tokens_ran(
    runtime: &tokio::runtime::Runtime,
    task: BackendTask,
) -> (RoutedApp, usize) {
    let _guard = runtime.enter();
    let mut harness = crate::support::mount_app(RootScreenType::RootScreenIdentityHub);
    let app_context = harness.state().current_app_context().clone();
    app_context.connection_status().set_masternodes_ready(true);
    let identity = seed_signing_identity(&app_context, 0xD8);
    seed_three_tokens(&app_context, identity);
    harness.state_mut().test_capture_dispatches();
    *routed_hub(&mut harness) = IdentityHubScreen::new(&app_context);
    routed_hub(&mut harness).select_tab(IdentityHubTab::Tokens);
    harness.run_steps(4);

    let arrival = routed_balance_refreshes(&harness)
        .pop()
        .expect("the hub's arrival refresh");
    let source = elapsed_timeout(runtime);
    deliver_to_app(
        &mut harness,
        dash_evo_tool::app::TaskResult::Error {
            context: arrival,
            error: TaskError::TokenBalanceRefreshTimeout { source },
        },
    );
    assert!(
        harness.query_all_by_label_contains(TIMEOUT_TEXT).count() == 2,
        "the hub shows its failed refresh beside the banner"
    );

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenMyTokenBalances;
    harness.run_steps(3);
    let context = BackendTaskContext::for_dispatch_for_test(&task);
    harness.state_mut().test_record_visible_dispatch(&context);
    deliver_to_app(
        &mut harness,
        dash_evo_tool::app::TaskResult::Success {
            context,
            result: Box::new(
                dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedTokenBalances,
            ),
        },
    );

    let before = routed_balance_refreshes(&harness).len();
    harness.state_mut().selected_main_screen = RootScreenType::RootScreenIdentityHub;
    routed_hub(&mut harness).refresh_on_arrival();
    harness.run_steps(3);
    let asked = routed_balance_refreshes(&harness).len() - before;
    (harness, asked)
}

/// A full refresh on My Tokens reaches the hidden hub through its observer,
/// so coming back shows no stale failure and asks the network nothing.
#[test]
fn a_full_refresh_on_my_tokens_settles_the_hidden_hub() {
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
        let (harness, asked) = return_to_the_hub_after_my_tokens_ran(&runtime, task);
        assert!(
            harness.query_all_by_label_contains(TIMEOUT_TEXT).count() == 1,
            "the full refresh ended the hub's failure; the banner stays"
        );
        assert_eq!(asked, 0, "the balances are fresh, so no refresh starts");
    });
}

/// A pair check on My Tokens reaches the hidden hub too, but read one pair:
/// the failure stays and the arrival still asks for every balance.
#[test]
fn a_pair_check_on_my_tokens_leaves_the_hidden_hubs_failure() {
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let task = BackendTask::TokenTask(Box::new(TokenTask::TrackTokenBalancesAgain(vec![
            IdentityTokenIdentifier {
                identity_id: Identifier::from([0x01; 32]),
                token_id: Identifier::from([0x02; 32]),
            },
        ])));
        let (harness, asked) = return_to_the_hub_after_my_tokens_ran(&runtime, task);
        assert!(
            harness.query_all_by_label_contains(TIMEOUT_TEXT).count() == 2,
            "one pair's answer says nothing about the failed refresh"
        );
        assert_eq!(asked, 1, "the arrival still asks for every balance");
    });
}

fn pre_programmed_token(
    name: &str,
    admin: bool,
    scheduled_at: u64,
    recipient: Identifier,
) -> TokenConfiguration {
    use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::TokenPreProgrammedDistribution;
    use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::v0::TokenPreProgrammedDistributionV0;

    let mut token = token_config(name, admin);
    let TokenDistributionRules::V0(mut rules) = token.distribution_rules().clone() else {
        panic!("presets are version 0");
    };
    rules.pre_programmed_distribution = Some(TokenPreProgrammedDistribution::V0(
        TokenPreProgrammedDistributionV0 {
            distributions: BTreeMap::from([(scheduled_at, BTreeMap::from([(recipient, 500)]))]),
        },
    ));
    token.set_distribution_rules(TokenDistributionRules::V0(rules));
    token
}

#[test]
fn hub_token_regression_hidden_hub_retries_only_its_deferred_requests() {
    use dash_evo_tool::app::TaskResult;
    use dash_evo_tool::backend_task::BackendTaskSuccessResult;

    for track_again in [false, true] {
        with_isolated_data_dir(|| {
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let _guard = runtime.enter();
            let mut app = crate::support::mount_app(RootScreenType::RootScreenIdentityHub);
            let ctx = app.state().current_app_context().clone();
            ctx.connection_status().set_masternodes_ready(true);
            let identity = seed_signing_identity(&ctx, 0xA2);
            let token = seed_contract(&ctx, 0xC1, identity, vec![token_config("Coffee", false)])[0];
            let pair = IdentityTokenIdentifier {
                identity_id: identity,
                token_id: token,
            };
            if track_again {
                ctx.mark_token_balance_untracked(pair)
                    .expect("dismiss the token");
            }
            let task =
                BackendTask::TokenTask(Box::new(TokenTask::TrackTokenBalancesAgain(vec![pair])));
            app.state_mut().test_capture_dispatches();
            *routed_hub(&mut app) = IdentityHubScreen::new(&ctx);
            routed_hub(&mut app).select_tab(IdentityHubTab::Tokens);
            app.run_steps(4);
            let arrival = routed_balance_refreshes(&app).pop().expect("arrival");
            deliver_to_app(
                &mut app,
                TaskResult::Success {
                    context: arrival,
                    result: Box::new(BackendTaskSuccessResult::FetchedTokenBalances),
                },
            );
            if track_again {
                app.get_by_label(token_copy::TRACK_AGAIN_LABEL).click();
            } else {
                app.get_by_label("More actions for Coffee").click();
                app.run_steps(2);
                app.get_by_label(token_copy::CHECK_NOW_LABEL).click();
            }
            app.run_steps(3);
            let own = app
                .state()
                .test_captured_dispatches()
                .iter()
                .find_map(|(dispatched, context)| (dispatched == &task).then(|| context.clone()))
                .expect("the hub dispatched the request");
            let foreign = BackendTaskContext::for_dispatch_for_test(&task);
            app.state_mut().test_record_visible_dispatch(&foreign);

            app.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
            app.run_steps(3);
            let busy = TaskError::TokenBalanceRefreshInProgress;
            let message = busy.to_string();
            deliver_to_app(
                &mut app,
                TaskResult::Error {
                    context: own,
                    error: busy,
                },
            );
            assert!(app.query_by_label_contains(&message).is_none());

            let before = app.state().test_captured_dispatches().len();
            app.state_mut().selected_main_screen = RootScreenType::RootScreenIdentityHub;
            app.run_steps(3);
            let retry = app.state().test_captured_dispatches()[before..]
                .iter()
                .find_map(|(retry, context)| {
                    (if track_again {
                        retry == &task
                    } else {
                        matches!(retry, BackendTask::TokenTask(task)
                        if matches!(**task, TokenTask::QueryMyTokenBalances))
                    })
                    .then(|| context.clone())
                })
                .expect("the hub retries after returning");
            app.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
            app.run_steps(3);
            let error = TaskError::NoIdentitiesFound;
            let genuine_message = error.to_string();
            deliver_to_app(
                &mut app,
                TaskResult::Error {
                    context: retry,
                    error,
                },
            );
            assert!(app.query_by_label_contains(&genuine_message).is_some());

            deliver_to_app(
                &mut app,
                TaskResult::Error {
                    context: foreign,
                    error: TaskError::TokenBalanceRefreshInProgress,
                },
            );
            assert!(app.query_by_label_contains(&message).is_some());
        });
    }
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
        assert!(hub.query_by_label("Send").is_none());
    });
}

#[test]
fn a_token_that_cannot_be_sent_shows_the_badge_without_send() {
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
        assert!(hub.query_by_label("Send").is_none());
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
fn absent_balance_after_success_shows_no_balance_found_and_no_send() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_signing_identity(&app_context, 0xA1);
        seed_contract(
            &app_context,
            0xC1,
            Identifier::from([0xEE; 32]),
            vec![token_config("TKN", false)],
        );
        let mut hub = mount_tokens_tab(&app_context);
        assert!(hub.query_by_label(token_copy::FIRST_LOAD).is_some());
        let arrival = balance_refreshes(&hub).pop().expect("arrival refresh");
        hub.state_mut().0.display_backend_task_result(
            &arrival,
            dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedTokenBalances,
        );
        hub.run_steps(2);
        assert!(hub.query_by_label("No balance found.").is_some());
        assert!(hub.query_by_label("0 TKN").is_none());
        assert!(hub.query_by_role_and_label(Role::Button, "Send").is_none());
        hub.get_by_label(token_copy::REFRESH_LABEL).click();
        hub.run_steps(2);
        assert!(hub.query_by_label(token_copy::FIRST_LOAD).is_none());
        assert!(hub.query_by_label(token_copy::REFRESHING_TOOLTIP).is_some());
    });
}

/// The context of the pair check the last click dispatched.
fn pair_check_context(hub: &HubHarness) -> BackendTaskContext {
    hub.state()
        .1
        .iter()
        .find_map(|action| match action {
            AppAction::BackendTaskWithContext {
                task: BackendTask::TokenTask(task),
                context,
            } if is_pair_check(task) => Some(context.clone()),
            _ => None,
        })
        .expect("the pair check was dispatched")
}

/// Settle the arrival refresh, then check `name`'s balance from its row menu.
fn check_row_now(hub: &mut HubHarness, name: &str) -> BackendTaskContext {
    let arrival = balance_refreshes(hub)
        .pop()
        .expect("the arrival refresh is running");
    hub.state_mut().0.display_backend_task_result(
        &arrival,
        dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedTokenBalances,
    );
    hub.run_steps(2);
    hub.state_mut().1.clear();
    hub.get_by_label(&format!("More actions for {name}"))
        .click();
    hub.run_steps(2);
    hub.get_by_label(token_copy::CHECK_NOW_LABEL).click();
    hub.run_steps(2);
    pair_check_context(hub)
}

/// A check that fails on a row in plain view still reaches the banner; a
/// check refused because a refresh is running stays quiet.
#[test]
fn a_pair_check_failure_on_a_visible_row_reaches_the_banner_but_a_refusal_does_not() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_signing_identity(&app_context, 0xE8);
        seed_three_tokens(&app_context, identity);
        let mut hub = mount_tokens_tab(&app_context);
        let check = check_row_now(&mut hub, "Tester");
        let error = TaskError::NoIdentitiesFound;
        hub.state_mut().0.display_backend_task_error(&check, &error);
        assert!(
            !hub.state_mut().0.display_task_error(&error),
            "the failure reaches the banner though its row shows it too"
        );

        hub.state_mut().1.clear();
        hub.get_by_label("More actions for Tester").click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::CHECK_NOW_LABEL).click();
        hub.run_steps(2);
        let check = pair_check_context(&hub);
        let busy = TaskError::TokenBalanceRefreshInProgress;
        assert!(
            hub.state()
                .0
                .should_suppress_backend_task_error(&check, &busy),
            "a refusal is no failure: the check is sent again later"
        );
        hub.state_mut().0.display_backend_task_error(&check, &busy);
    });
}

fn regression_claim_hub(app_context: &Arc<AppContext>) -> HubHarness {
    let identity = seed_signing_identity(app_context, 0xA6);
    let mut token = token_config("AirDrop", false);
    let TokenDistributionRules::V0(rules) = token.distribution_rules().clone() else {
        panic!("version 0 preset");
    };
    token.set_distribution_rules(distribution_rules_with_once_per_identity(rules, Some(100)));
    let tokens = seed_contract(app_context, 0xC2, Identifier::from([0xEE; 32]), vec![token]);
    set_balance(app_context, identity, tokens[0], 100);
    mount_tokens_tab(app_context)
}

#[test]
fn hub_token_surfaces_offer_neither_send_nor_claim() {
    with_isolated_data_dir(|| {
        let (_runtime, ctx) = fresh_app_context();
        let mut hub = regression_claim_hub(&ctx);
        // Open the row menu too, so its items are checked as well.
        hub.get_by_label(&token_copy::more_actions_label("AirDrop"))
            .click();
        hub.run_steps(3);
        assert_no_send_or_claim(&hub);
    });
}

/// No button or link starts with Send or Claim, whatever follows.
fn assert_no_send_or_claim(hub: &HubHarness) {
    for role in [Role::Button, Role::Link] {
        for node in hub.query_all_by_role(role) {
            let label = node.accesskit_node().label().unwrap_or_default();
            assert!(
                !label.starts_with("Send") && !label.starts_with("Claim"),
                "the hub offers {label:?}"
            );
        }
    }
}

/// A disconnect marks the connection as stopping before the masternode list
/// is cleared; no balance read goes out until that teardown ends.
#[test]
fn a_disconnected_refresh_cannot_be_retried_while_the_connection_stops() {
    use dash_evo_tool::model::spv_status::SpvStatus;

    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_signing_identity(&app_context, 0xB8);
        seed_contract(
            &app_context,
            0xC8,
            Identifier::from([0xEE; 32]),
            vec![token_config("DashCoffee", false)],
        );
        let mut hub = mount_tokens_tab(&app_context);
        let arrival = balance_refreshes(&hub);
        app_context
            .connection_status()
            .set_spv_status(SpvStatus::Stopping);
        hub.state_mut()
            .0
            .display_backend_task_error(&arrival[0], &TaskError::TokenBalanceRefreshDisconnected);
        hub.run_steps(6);
        assert!(
            hub.query_by_label(&TaskError::TokenBalanceRefreshDisconnected.to_string())
                .is_some()
        );
        // The notice's retry is a link; the failed row has a retry button too.
        // Only their state is read here: a click would leave a disabled
        // tooltip over the controls below.
        let off = |hub: &HubHarness, role: Role, label: &str| {
            hub.get_by_role_and_label(role, label)
                .accesskit_node()
                .is_disabled()
        };
        for (role, label) in [
            (Role::Link, token_copy::TRY_AGAIN_LABEL),
            (Role::Button, token_copy::TRY_AGAIN_LABEL),
            (Role::Button, token_copy::REFRESH_LABEL),
        ] {
            assert!(off(&hub, role, label), "{label:?} waits for the teardown");
        }

        app_context
            .connection_status()
            .set_spv_status(SpvStatus::Running);
        hub.run_steps(2);
        assert!(!off(&hub, Role::Link, token_copy::TRY_AGAIN_LABEL));
        // The link wraps inside the long notice, so its rect centre can miss
        // the text: activate it from the keyboard instead of the pointer.
        hub.get_by_role_and_label(Role::Link, token_copy::TRY_AGAIN_LABEL)
            .focus();
        hub.run_steps(1);
        hub.key_press(egui::Key::Enter);
        hub.run_steps(2);
        assert_eq!(
            balance_refreshes(&hub).len(),
            2,
            "the retry works once connected"
        );
    });
}

#[test]
fn regression_import_by_id_restores_a_dismissed_token() {
    with_isolated_data_dir(|| {
        let (runtime, ctx) = fresh_app_context();
        let identity = seed_signing_identity(&ctx, 0xA2);
        let tokens = seed_three_tokens(&ctx, identity);
        let mut hub = mount_tokens_tab(&ctx);
        hub.get_by_label("More actions for DashCoffee").click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::STOP_TRACKING_LABEL).click();
        hub.run_steps(2);
        hub.state_mut().1.clear();
        hub.get_by_label(token_copy::STOP_TRACKING_LABEL).click();
        hub.run_steps(3);
        let pair = hub
            .state()
            .1
            .iter()
            .find_map(|action| match action {
                AppAction::BackendTaskWithContext {
                    task: BackendTask::TokenTask(task),
                    ..
                } => {
                    if let TokenTask::StopTrackingTokenBalance(pair) = **task {
                        Some(pair)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .expect("stop-tracking request");
        ctx.mark_token_balance_untracked(pair)
            .expect("persist dismissal");
        assert!(
            ctx.get_all_known_tokens()
                .expect("global registry")
                .contains_key(&tokens[0])
        );
        hub.get_by_label(token_copy::ADD_TOKEN_LABEL).click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::ADD_BY_ID_LABEL).click();
        hub.run_steps(2);
        let screen = hub
            .state_mut()
            .1
            .drain(..)
            .find_map(|action| match action {
                AppAction::AddScreen(Screen::AddTokenById(screen)) => Some(screen),
                _ => None,
            })
            .expect("hub opens import-by-ID");
        let contract = ctx.get_contract_by_token_id(&tokens[0]).unwrap().unwrap();
        let mut import = crate::support::builder_with_app_fonts()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui,
                 state: &mut (
                    dash_evo_tool::ui::tokens::add_token_by_id_screen::AddTokenByIdScreen,
                    Vec<AppAction>,
                )| {
                    let action = state.0.ui(ui);
                    state.1.push(action);
                },
                (screen, Vec::new()),
            );
        import.state_mut().0.display_task_result(
            dash_evo_tool::backend_task::BackendTaskSuccessResult::FetchedContractWithTokenPosition(
                contract.contract,
                0,
            ),
        );
        import.run_steps(3);
        import
            .get_all_by_role_and_label(Role::Button, "Import Token")
            .last()
            .expect("import form button")
            .click();
        import.run_steps(2);
        let task = import
            .state_mut()
            .1
            .drain(..)
            .find_map(|action| match action {
                AppAction::BackendTasks(tasks, _) => {
                    tasks.into_iter().find_map(|task| match task {
                        BackendTask::ContractTask(task) => Some(*task),
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("import saves contract");
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        runtime
            .block_on(ctx.run_contract_task(
                task,
                &ctx.sdk(),
                dash_evo_tool::utils::egui_mpsc::SenderAsync::new(sender, egui::Context::default()),
            ))
            .unwrap();
        hub.state_mut().0.refresh_on_arrival();
        hub.run_steps(3);
        assert!(
            hub.query_by_label("DashCoffee").is_some(),
            "import restores the identity's row"
        );
    });
}

#[test]
fn regression_search_can_reimport_a_dismissed_known_token() {
    with_isolated_data_dir(|| {
        let (runtime, ctx) = fresh_app_context();
        let identity = seed_signing_identity(&ctx, 0xA2);
        let tokens = seed_three_tokens(&ctx, identity);
        let pair = IdentityTokenIdentifier {
            identity_id: identity,
            token_id: tokens[0],
        };
        ctx.mark_token_balance_untracked(pair).unwrap();
        let mut hub = mount_tokens_tab(&ctx);
        hub.get_by_label(token_copy::ADD_TOKEN_LABEL).click();
        hub.run_steps(2);
        hub.get_by_label(token_copy::FIND_BY_NAME_LABEL).click();
        hub.run_steps(2);
        assert!(hub.state().1.iter().any(|action| matches!(
            action,
            AppAction::SetMainScreen(RootScreenType::RootScreenTokenSearch)
        )));
        let contract = ctx.get_contract_by_token_id(&tokens[0]).unwrap().unwrap();
        let contract_id = contract.contract.id();
        let token = dash_evo_tool::ui::tokens::tokens_screen::TokenInfo {
            token_id: tokens[0],
            token_name: "DashCoffee".into(),
            data_contract_id: contract_id,
            token_position: 0,
            token_configuration: contract.contract.tokens()[&0].clone(),
            description: None,
        };
        let mut screen = TokensScreen::new(&ctx, TokensSubscreen::SearchTokens);
        screen.selected_contract_id = Some(contract_id);
        screen.display_backend_task_result(
            &BackendTaskContext::Other,
            dash_evo_tool::backend_task::BackendTaskSuccessResult::ContractsWithDescriptions(
                BTreeMap::from([(contract_id, (None, vec![token]))]),
            ),
        );
        let mut search = crate::support::builder_with_app_fonts()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, state: &mut (TokensScreen, Vec<AppAction>)| {
                    let action = state.0.ui(ui);
                    state.1.push(action);
                },
                (screen, Vec::new()),
            );
        search.run_steps(3);
        search.get_by_label("Add to My Tokens").click();
        search.run_steps(2);
        let task = search
            .state_mut()
            .1
            .drain(..)
            .find_map(|action| match action {
                AppAction::BackendTasks(tasks, _) => {
                    tasks.into_iter().find_map(|task| match task {
                        BackendTask::TokenTask(task)
                            if matches!(*task, TokenTask::SaveTokenLocally(_)) =>
                        {
                            Some(*task)
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("a dismissed known token can be imported again");
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        runtime
            .block_on(ctx.run_token_task(
                task,
                &ctx.sdk(),
                dash_evo_tool::utils::egui_mpsc::SenderAsync::new(sender, egui::Context::default()),
            ))
            .unwrap();
        hub.state_mut().0.refresh_on_arrival();
        hub.run_steps(3);
        assert!(hub.query_by_label("DashCoffee").is_some());
    });
}
