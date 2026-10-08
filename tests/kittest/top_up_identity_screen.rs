//! Kittest coverage for an identity top-up on the real `AppState`: the blocking
//! dialog, its "Continue in background" exit, and where the outcome of a top-up
//! goes once the screen that sent it is no longer the one in view.
//!
//! These run the real frame loop, so a result travels the same channel and the
//! same routing as in the app. No task ever reaches the backend: the top-up is
//! entered through a test seam and its result is injected.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppState, BootPhase, TaskResult};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::{BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::identity::top_up_identity_screen::TopUpIdentityScreen;
use dash_evo_tool::ui::wallets::send_screen::WalletSendScreen;
use dash_evo_tool::ui::{RootScreenType, Screen};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const DIALOG: &str = "Adding funds to your identity.";
const CONTINUE_IN_BACKGROUND: &str = "Continue in background";
const BACKGROUND_BANNER: &str = "Adding funds to your identity in the background.";
const DONE_BANNER: &str = "The funds were added to";
const FORM: &str = "Follow these steps to add funds to your identity:";
const FORM_PAUSED: &str = "You can add more funds when this transfer finishes.";
const WALLET_SEND_WAITING: &str = "Sending...";
const WALLET_SEND_COMPLETE: &str = "Identity topped up successfully!";

/// How long the dialog stays a hard block before it offers the background.
const BACKGROUND_OFFER_AFTER: Duration = Duration::from_secs(30);

type App = Harness<'static, AppState>;

/// How many labels on screen contain `text`.
fn count(harness: &App, text: &str) -> usize {
    harness.query_all_by_label_contains(text).count()
}

fn test_identity(network: Network) -> QualifiedIdentity {
    QualifiedIdentity {
        identity: Identity::create_basic_identity(
            Identifier::from([0x54; 32]),
            PlatformVersion::latest(),
        )
        .expect("basic identity"),
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("Top-up fixture".to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network,
    }
}

/// Open Add Funds for `identity` on top of the mounted app.
fn open_add_funds(harness: &mut App, identity: &QualifiedIdentity) {
    let app_context = harness.state().current_app_context().clone();
    harness
        .state_mut()
        .screen_stack
        .push(Screen::TopUpIdentityScreen(TopUpIdentityScreen::new(
            identity.clone(),
            &app_context,
        )));
    harness.run_steps(3);
}

/// The mounted app with Add Funds open on its form.
fn mount_add_funds() -> (App, QualifiedIdentity) {
    let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
    let identity = test_identity(harness.state().current_app_context().network());
    open_add_funds(&mut harness, &identity);
    assert_eq!(count(&harness, FORM), 1, "Add Funds opens on its form");
    (harness, identity)
}

fn add_funds_screen(harness: &mut App) -> &mut TopUpIdentityScreen {
    match harness.state_mut().screen_stack.last_mut() {
        Some(Screen::TopUpIdentityScreen(screen)) => screen,
        _ => panic!("Add Funds must be the visible screen"),
    }
}

/// Send the top-up the way "Add funds" does and return its dispatch.
fn begin_top_up(harness: &mut App) -> BackendTaskContext {
    let dispatch = add_funds_screen(harness)
        .begin_top_up_for_test()
        .expect("a dispatched top-up records its dispatch");
    harness.run_steps(3);
    assert_eq!(
        count(harness, DIALOG),
        1,
        "a dispatched top-up blocks the app behind its dialog"
    );
    dispatch
}

/// Let the top-up run long and leave its dialog through the offered button.
fn continue_in_background(harness: &mut App) {
    add_funds_screen(harness).backdate_top_up_for_test(BACKGROUND_OFFER_AFTER);
    // The centred card moves until it has measured itself; settle before clicking.
    harness.run_steps(3);
    harness.get_by_label(CONTINUE_IN_BACKGROUND).click();
    harness.run_steps(3);
    assert_eq!(count(harness, BACKGROUND_BANNER), 1);
}

/// Deliver `result` through the channel backend tasks report on.
fn deliver(harness: &mut App, result: TaskResult) {
    assert!(
        harness.state().task_result_sender.try_send(result).is_ok(),
        "the task result channel accepts the result"
    );
    harness.run_steps(3);
}

fn topped_up(context: BackendTaskContext, identity: &QualifiedIdentity) -> TaskResult {
    TaskResult::Success {
        context,
        result: Box::new(BackendTaskSuccessResult::ToppedUpIdentity(
            identity.clone(),
            FeeResult::new(1, 1),
        )),
    }
}

/// Run `test` against an isolated data dir with a tokio runtime entered.
fn in_app(test: impl FnOnce()) {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        test();
    });
}

/// The dialog is the only thing between the user and an app blocked for as long
/// as the network takes, so its exit has to work from the real button and key.
fn top_up_leaves_its_dialog(activate: impl FnOnce(&mut App)) {
    in_app(|| {
        let (mut harness, _identity) = mount_add_funds();
        begin_top_up(&mut harness);
        assert_eq!(
            count(&harness, CONTINUE_IN_BACKGROUND),
            0,
            "a top-up that just started stays a hard block"
        );

        add_funds_screen(&mut harness).backdate_top_up_for_test(BACKGROUND_OFFER_AFTER);
        harness.run_steps(3);
        assert_eq!(count(&harness, CONTINUE_IN_BACKGROUND), 1);
        activate(&mut harness);
        harness.run_steps(3);

        assert_eq!(
            count(&harness, DIALOG),
            0,
            "the app is usable again once the top-up continues in the background"
        );
        assert_eq!(
            count(&harness, BACKGROUND_BANNER),
            1,
            "one banner follows the top-up, and the status under it does not repeat it"
        );
        assert_eq!(
            count(&harness, FORM_PAUSED),
            1,
            "the form stays paused while the top-up runs"
        );
        assert_eq!(count(&harness, FORM), 0);
    });
}

#[test]
fn top_up_continues_in_background_from_the_dialog_button() {
    top_up_leaves_its_dialog(|harness| harness.get_by_label(CONTINUE_IN_BACKGROUND).click());
}

#[test]
fn top_up_continues_in_background_on_enter() {
    top_up_leaves_its_dialog(|harness| harness.key_press(egui::Key::Enter));
}

#[test]
fn top_up_continues_in_background_on_space() {
    top_up_leaves_its_dialog(|harness| harness.key_press(egui::Key::Space));
}

/// The user sends a top-up to the background, leaves Add Funds and starts a
/// transfer on Wallet Send, which now waits on its own result.
fn wallet_send_waits_while_a_top_up_runs_in_background()
-> (App, BackendTaskContext, QualifiedIdentity) {
    let (mut harness, identity) = mount_add_funds();
    let dispatch = begin_top_up(&mut harness);
    continue_in_background(&mut harness);

    let app_context = harness.state().current_app_context().clone();
    let wallet = Wallet::new_from_seed([0x54; 64], app_context.network(), None, None)
        .expect("wallet fixture");
    let mut wallet_send = WalletSendScreen::new(&app_context, Arc::new(RwLock::new(wallet)));
    wallet_send.mark_sending_for_test();
    let state = harness.state_mut();
    state.screen_stack.clear();
    state
        .screen_stack
        .push(Screen::WalletSendScreen(wallet_send));
    harness.run_steps(3);
    assert_eq!(
        count(&harness, WALLET_SEND_WAITING),
        1,
        "Wallet Send waits on its own transfer"
    );
    (harness, dispatch, identity)
}

/// Wallet Send completes on a top-up result, so a background top-up ending
/// while it waits must not read as the end of its own transfer.
#[test]
fn background_top_up_success_leaves_wallet_send_waiting() {
    in_app(|| {
        let (mut harness, dispatch, identity) =
            wallet_send_waits_while_a_top_up_runs_in_background();

        deliver(&mut harness, topped_up(dispatch, &identity));
        assert_eq!(
            count(&harness, WALLET_SEND_COMPLETE),
            0,
            "the background top-up is not the transfer Wallet Send waits on"
        );
        assert_eq!(count(&harness, WALLET_SEND_WAITING), 1);
        assert_eq!(
            count(&harness, DONE_BANNER),
            1,
            "the background top-up is confirmed by its own banner"
        );
        assert_eq!(count(&harness, BACKGROUND_BANNER), 0);

        // Wallet Send's own top-up reports without a dispatch from Add Funds.
        let own_transfer = BackendTaskContext::IdentityTopUp(identity.identity.id());
        deliver(&mut harness, topped_up(own_transfer, &identity));
        assert_eq!(
            count(&harness, WALLET_SEND_COMPLETE),
            1,
            "Wallet Send still completes on the result of its own transfer"
        );
    });
}

/// An Add Funds screen opened while the identity's top-up runs did not send
/// it, so it is not handed the result. It still has to show what the top-up
/// left behind.
#[test]
fn reopened_add_funds_shows_the_balance_its_top_up_left() {
    in_app(|| {
        let (mut harness, identity) = mount_add_funds();
        let dispatch = begin_top_up(&mut harness);
        continue_in_background(&mut harness);

        harness.state_mut().screen_stack.clear();
        open_add_funds(&mut harness, &identity);
        assert_eq!(count(&harness, FORM_PAUSED), 1);
        assert_eq!(count(&harness, "0.0000 DASH"), 1);

        // The backend stores the new balance before it reports the result.
        let mut topped_up_identity = identity.clone();
        topped_up_identity.identity.set_balance(200_000_000_000);
        harness
            .state()
            .current_app_context()
            .insert_local_qualified_identity(&topped_up_identity, &None)
            .expect("store the topped-up identity");
        deliver(&mut harness, topped_up(dispatch, &topped_up_identity));

        assert_eq!(count(&harness, FORM), 1);
        assert_eq!(
            count(&harness, "2.0000 DASH"),
            1,
            "the form comes back with the balance the top-up left"
        );
    });
}

/// A failed background top-up must not hand Wallet Send its form back while
/// the transfer it sent is still running: sending again would pay twice.
#[test]
fn background_top_up_failure_leaves_wallet_send_waiting() {
    in_app(|| {
        let (mut harness, dispatch, _identity) =
            wallet_send_waits_while_a_top_up_runs_in_background();
        let error = TaskError::NoIdentitiesFound;
        let message = error.to_string();

        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error,
            },
        );
        assert_eq!(
            count(&harness, WALLET_SEND_WAITING),
            1,
            "Wallet Send keeps waiting on its own transfer"
        );
        assert_eq!(
            count(&harness, &message),
            1,
            "the failed top-up is still reported"
        );
        assert_eq!(count(&harness, BACKGROUND_BANNER), 0);
    });
}

/// A first visit to a network finishes switching while the app stays usable,
/// and the switch drops every stacked screen, dialog and banner. The top-up
/// sent in the meantime keeps running, so it must stay in flight and in view.
#[test]
fn top_up_survives_a_network_switch_that_drops_its_screen() {
    in_app(|| {
        // The switch target's context is built by a backend task, which needs
        // that network configured. The shipped example config has every network.
        let data_dir = std::env::var("DASH_EVO_DATA_DIR").expect("the isolated data dir");
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.env.example"),
            std::path::Path::new(&data_dir).join(".env"),
        )
        .expect("seed the isolated data dir with a full network config");

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let first = harness.state().current_app_context().network();
        // The switch task awaits the start of chain sync, which never returns offline.
        harness
            .state()
            .current_app_context()
            .update_auto_start_spv(false)
            .expect("turn chain sync off for the switch");
        let second = if first == Network::Testnet {
            Network::Mainnet
        } else {
            Network::Testnet
        };
        let identity = test_identity(first);

        harness.state_mut().change_network(second);
        open_add_funds(&mut harness, &identity);
        let dispatch = begin_top_up(&mut harness);

        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            harness.step();
            let state = harness.state();
            if state.current_app_context().network() == second
                && state.boot_phase() == BootPhase::Ready
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the switch to {second:?} did not finish in time"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        harness.run_steps(3);
        assert!(
            harness.state().screen_stack.is_empty(),
            "the switch drops the screen that sent the top-up"
        );
        assert_eq!(count(&harness, DIALOG), 0);
        assert_eq!(
            count(&harness, BACKGROUND_BANNER),
            1,
            "a top-up that outlives its screen is followed by the banner"
        );

        open_add_funds(&mut harness, &identity);
        assert_eq!(
            count(&harness, FORM_PAUSED),
            1,
            "a new Add Funds screen must not offer the form while the top-up runs"
        );
        assert_eq!(count(&harness, FORM), 0);

        deliver(&mut harness, topped_up(dispatch, &identity));
        assert_eq!(
            count(&harness, DONE_BANNER),
            1,
            "the top-up is confirmed although its screen is gone"
        );
        assert_eq!(count(&harness, BACKGROUND_BANNER), 0);
        assert_eq!(
            count(&harness, FORM),
            1,
            "the form comes back when the top-up ends"
        );
    });
}

/// Some failures are handled before the app's general error path. A top-up
/// that ends in one of them has ended all the same.
#[test]
fn top_up_ends_on_a_failure_handled_outside_the_general_error_path() {
    in_app(|| {
        let (mut harness, _identity) = mount_add_funds();
        let dispatch = begin_top_up(&mut harness);

        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error: TaskError::CoreWalletAutoDetected {
                    wallet_name: "fixture".to_string(),
                },
            },
        );
        assert_eq!(
            count(&harness, DIALOG),
            0,
            "an ended top-up must not keep the app blocked"
        );
        assert_eq!(count(&harness, FORM_PAUSED), 0);
        assert_eq!(count(&harness, FORM), 1);
    });
}

/// The screen that sent a top-up and is still in view is the one told how it
/// ended: it shows the success itself, so no banner repeats it.
#[test]
fn watched_top_up_ends_on_its_success_view() {
    in_app(|| {
        let (mut harness, identity) = mount_add_funds();
        let dispatch = begin_top_up(&mut harness);

        deliver(&mut harness, topped_up(dispatch, &identity));
        assert_eq!(count(&harness, DIALOG), 0);
        assert_eq!(count(&harness, "Identity Topped Up Successfully!"), 1);
        assert_eq!(
            count(&harness, DONE_BANNER),
            0,
            "a top-up watched to its end is confirmed by its screen alone"
        );
    });
}

/// A failure the user watched brings the form back next to the error.
#[test]
fn watched_top_up_failure_brings_the_form_back() {
    in_app(|| {
        let (mut harness, _identity) = mount_add_funds();
        let dispatch = begin_top_up(&mut harness);
        let error = TaskError::NoIdentitiesFound;
        let message = error.to_string();

        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error,
            },
        );
        assert_eq!(count(&harness, DIALOG), 0);
        assert_eq!(count(&harness, &message), 1);
        assert_eq!(count(&harness, FORM_PAUSED), 0);
        assert_eq!(count(&harness, FORM), 1);
    });
}
