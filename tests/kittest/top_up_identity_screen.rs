//! Kittest coverage for an identity top-up on the real `AppState`: the blocking
//! dialog, its "Continue in background" exit, and where the outcome of a top-up
//! goes once the screen that sent it is no longer the one in view.
//!
//! These run the real frame loop, so a result travels the same channel and the
//! same routing as in the app. No task ever reaches the backend: the top-up is
//! entered through a test seam and its result is injected.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppAction, AppState, BootPhase, TaskResult};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{
    BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult,
};
use dash_evo_tool::model::address::ValidatedAddress;
use dash_evo_tool::model::amount::Amount;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::identity::top_up_identity_screen::TopUpIdentityScreen;
use dash_evo_tool::ui::wallets::send_screen::{
    IdentitySendMode, SourceSelection, WalletSendScreen,
};
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenType};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::{Address, Network, PublicKey};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui::accesskit::Role;
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
const WALLET_SEND_PROGRESS: &str = "Sending transaction";
const WALLET_SEND_COMPLETE: &str = "Identity topped up successfully!";
const TOP_UP_ALREADY_RUNNING: &str = "Funds are already being added to this identity. Wait for that transfer to finish, then try again.";

/// How long the dialog stays a hard block before it offers the background.
const BACKGROUND_OFFER_AFTER: Duration = Duration::from_secs(30);

type App = Harness<'static, AppState>;

/// How many labels on screen contain `text`.
fn count(harness: &App, text: &str) -> usize {
    harness.query_all_by_label_contains(text).count()
}

fn test_identity(network: Network) -> QualifiedIdentity {
    identity_with_id([0x54; 32], network)
}

fn identity_with_id(id: [u8; 32], network: Network) -> QualifiedIdentity {
    QualifiedIdentity {
        identity: Identity::create_basic_identity(Identifier::from(id), PlatformVersion::latest())
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

/// A top-up of `identity`, as a send form prepares it. It is never run.
fn top_up_of(identity: &QualifiedIdentity) -> BackendTask {
    BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses {
        identity: identity.clone(),
        inputs: Default::default(),
        wallet_seed_hash: Default::default(),
    })
}

/// A Wallet Send screen that confirmed a top-up of `recipient` and waits on
/// it, with the dispatch it waits on.
fn waiting_wallet_send(
    harness: &App,
    recipient: &QualifiedIdentity,
) -> (WalletSendScreen, BackendTaskContext) {
    let app_context = harness.state().current_app_context().clone();
    let wallet = Wallet::new_from_seed([0x54; 64], app_context.network(), None, None)
        .expect("wallet fixture");
    let mut wallet_send = WalletSendScreen::new(&app_context, Arc::new(RwLock::new(wallet)));
    let dispatch = wallet_send
        .confirm_send_for_test(top_up_of(recipient))
        .expect("a confirmed send records its dispatch");
    (wallet_send, dispatch)
}

/// What [`wallet_send_waits_while_a_top_up_runs_in_background`] leaves running.
struct TwoTopUps {
    harness: App,
    /// The identity Add Funds tops up in the background, and that top-up.
    add_funds: (QualifiedIdentity, BackendTaskContext),
    /// The identity Wallet Send tops up, and the dispatch it waits on.
    wallet_send: (QualifiedIdentity, BackendTaskContext),
}

/// The user sends a top-up to the background, leaves Add Funds and starts a
/// top-up of another identity on Wallet Send, which now waits on its own result.
fn wallet_send_waits_while_a_top_up_runs_in_background() -> TwoTopUps {
    let (mut harness, identity) = mount_add_funds();
    let dispatch = begin_top_up(&mut harness);
    continue_in_background(&mut harness);

    let other = identity_with_id([0x55; 32], identity.network);
    let (wallet_send, own_dispatch) = waiting_wallet_send(&harness, &other);
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
    TwoTopUps {
        harness,
        add_funds: (identity, dispatch),
        wallet_send: (other, own_dispatch),
    }
}

/// Wallet Send completes on a top-up result, so a background top-up ending
/// while it waits must not read as the end of its own transfer.
#[test]
fn background_top_up_success_leaves_wallet_send_waiting() {
    in_app(|| {
        let TwoTopUps {
            mut harness,
            add_funds: (identity, dispatch),
            wallet_send: (other, own_dispatch),
        } = wallet_send_waits_while_a_top_up_runs_in_background();

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

        deliver(&mut harness, topped_up(own_dispatch, &other));
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
        let TwoTopUps {
            mut harness,
            add_funds: (_identity, dispatch),
            ..
        } = wallet_send_waits_while_a_top_up_runs_in_background();
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

/// Home starts its own loads, and they can still be running when the user
/// confirms a send from it. One of them failing is reported, but it is not the
/// answer Wallet Send waits on: handing the form back would let the user send,
/// and pay, a second time.
#[test]
fn a_failed_load_of_another_screen_leaves_wallet_send_waiting() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::dashpay::DashPayTask;
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let app_context = harness.state().current_app_context().clone();
        let identity = test_identity(app_context.network());
        let (wallet_send, own_send) = waiting_wallet_send(&harness, &identity);
        harness
            .state_mut()
            .screen_stack
            .push(Screen::WalletSendScreen(wallet_send));
        // Results are drained at the start of a frame, before it renders.
        harness.step();
        assert_eq!(count(&harness, WALLET_SEND_WAITING), 1);

        let home_load = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
            identity: identity.clone(),
        }));
        let load_failure = TaskError::NoIdentitiesFound;
        let load_message = load_failure.to_string();
        assert!(
            harness
                .state()
                .task_result_sender
                .try_send(TaskResult::Error {
                    context: BackendTaskContext::from(&home_load),
                    error: load_failure,
                })
                .is_ok()
        );
        harness.step();
        assert_eq!(
            count(&harness, WALLET_SEND_WAITING),
            1,
            "Wallet Send keeps waiting on the send it dispatched"
        );
        assert_eq!(
            count(&harness, &load_message),
            1,
            "the failed load is still reported"
        );

        assert!(
            harness
                .state()
                .task_result_sender
                .try_send(TaskResult::Error {
                    context: own_send,
                    error: TaskError::DocumentNotFound,
                })
                .is_ok()
        );
        harness.step();
        assert_eq!(
            count(&harness, WALLET_SEND_WAITING),
            0,
            "the failure of its own send ends the wait"
        );
    });
}

/// The identity with a wallet of its own, stored where the send form reads it.
fn stored_identity_with_wallet(harness: &App) -> QualifiedIdentity {
    let app_context = harness.state().current_app_context().clone();
    let mut identity = test_identity(app_context.network());
    let wallet = Wallet::new_from_seed([0x54; 64], app_context.network(), None, None)
        .expect("wallet fixture");
    identity
        .associated_wallets
        .insert(wallet.seed_hash(), Arc::new(RwLock::new(wallet)));
    app_context
        .insert_local_qualified_identity(&identity, &None)
        .expect("store the identity");
    identity
}

/// Open Home's Top up form for `identity` on top of the mounted app.
fn open_top_up_form(harness: &mut App, identity: &QualifiedIdentity) {
    let app_context = harness.state().current_app_context().clone();
    let screen = ScreenType::IdentitySendScreen(identity.clone(), IdentitySendMode::TopUp)
        .create_screen(&app_context);
    harness.state_mut().screen_stack.push(screen);
    harness.run_steps(3);
}

fn top_up_form(harness: &mut App) -> &mut WalletSendScreen {
    match harness.state_mut().screen_stack.last_mut() {
        Some(Screen::WalletSendScreen(screen)) => screen,
        _ => panic!("the Top up form must be the visible screen"),
    }
}

/// Confirm a top-up on the visible form the way its dialog does, and return
/// the dispatch. The task itself is not run: no funds may move in a test.
fn confirm_top_up(harness: &mut App, identity: &QualifiedIdentity) -> BackendTaskContext {
    let dispatch = top_up_form(harness)
        .confirm_send_for_test(top_up_of(identity))
        .expect("a confirmed send records its dispatch");
    harness.run_steps(3);
    assert_eq!(count(harness, WALLET_SEND_WAITING), 1);
    assert_eq!(
        count(harness, WALLET_SEND_PROGRESS),
        1,
        "a confirmed send shows its progress banner"
    );
    dispatch
}

/// Fill the visible Top up form and ask it for the send it would confirm.
fn prepare_top_up(harness: &mut App, identity: &QualifiedIdentity) -> Result<AppAction, String> {
    let core = Address::p2pkh(
        &PublicKey::new(
            SecretKey::from_slice(&[0x31; 32])
                .expect("secret key")
                .public_key(&Secp256k1::new()),
        ),
        identity.network,
    );
    let source = SourceSelection::PlatformAddresses(vec![(
        PlatformAddress::try_from(core.clone()).expect("platform source"),
        core,
        3 * CREDITS_PER_DUFF * 100_000_000,
    )]);
    let destination = ValidatedAddress::Identity {
        id: identity.identity.id(),
        dpns_name: None,
    };
    let form = top_up_form(harness);
    form.set_simple_send_input_for_test(
        Some(source),
        Some(destination),
        Some(Amount::new_dash(1.0)),
    );
    form.validate_and_send_for_test()
}

/// Home's Top up used to open Add Funds, which refuses a second top-up of an
/// identity while one runs, wherever the user went in between. The shared form
/// must keep that: two top-ups prepared before either is saved would be paid
/// twice and recorded under one index.
#[test]
fn a_top_up_left_running_blocks_another_from_a_reopened_form() {
    in_app(|| {
        // The Wallets root asks for nothing of its own while it is in view.
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity = stored_identity_with_wallet(&harness);
        open_top_up_form(&mut harness, &identity);
        assert!(
            prepare_top_up(&mut harness, &identity).is_ok(),
            "with nothing running the form offers the top-up"
        );
        let dispatch = confirm_top_up(&mut harness, &identity);

        leave_by_breadcrumb(&mut harness);

        open_top_up_form(&mut harness, &identity);
        assert_eq!(
            prepare_top_up(&mut harness, &identity).err().as_deref(),
            Some(TOP_UP_ALREADY_RUNNING),
            "a reopened form must not offer a second top-up while the first runs"
        );

        // Another transfer to the same identity ending is not this top-up ending.
        let other_transfer = BackendTaskContext::IdentityTopUp(identity.identity.id());
        deliver(&mut harness, topped_up(other_transfer, &identity));
        assert_eq!(
            prepare_top_up(&mut harness, &identity).err().as_deref(),
            Some(TOP_UP_ALREADY_RUNNING),
            "only the result of the running top-up lets the next one start"
        );

        deliver(&mut harness, topped_up(dispatch, &identity));
        assert!(
            prepare_top_up(&mut harness, &identity).is_ok(),
            "the form offers a top-up again once the first has ended"
        );
    });
}

/// A failed top-up frees the identity for the next one just as a finished one
/// does, and the form that waited on it is told and handed back.
#[test]
fn the_top_up_form_is_told_how_its_own_top_up_ended() {
    in_app(|| {
        // The Wallets root asks for nothing of its own while it is in view.
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity = stored_identity_with_wallet(&harness);

        open_top_up_form(&mut harness, &identity);
        let dispatch = confirm_top_up(&mut harness, &identity);
        deliver(&mut harness, topped_up(dispatch, &identity));
        assert_eq!(
            count(&harness, WALLET_SEND_COMPLETE),
            1,
            "the form that sent the top-up shows that it succeeded"
        );
        assert_eq!(
            count(&harness, DONE_BANNER),
            0,
            "a form in view says so itself; no banner repeats it"
        );

        harness.state_mut().screen_stack.clear();
        open_top_up_form(&mut harness, &identity);
        let dispatch = confirm_top_up(&mut harness, &identity);
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
            0,
            "the form that sent the top-up is handed back when it fails"
        );
        assert_eq!(count(&harness, &message), 1);
        assert!(
            prepare_top_up(&mut harness, &identity).is_ok(),
            "a failed top-up does not keep the identity blocked"
        );
    });
}

/// Leave a form that waits on its top-up by the breadcrumb back to Identities.
/// It sits above the navigation entry of the same name and stays enabled while
/// the form waits. The form's own progress banner has nothing left to end it,
/// so it goes with the form and the top-up is followed in the background.
fn leave_by_breadcrumb(harness: &mut App) {
    harness
        .query_all_by_role_and_label(Role::Button, "Identities")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("the breadcrumb back to Identities")
        .click();
    harness.run_steps(3);
    assert!(
        harness.state().screen_stack.is_empty(),
        "the breadcrumb closes the waiting form"
    );
    assert_eq!(
        count(harness, WALLET_SEND_PROGRESS),
        0,
        "the closed form's progress banner does not stay behind"
    );
    assert_eq!(
        count(harness, BACKGROUND_BANNER),
        1,
        "the top-up that keeps running is followed in the background"
    );
}

/// A top-up keeps running after its form is closed. With nobody left to show
/// how it ended, its success is confirmed the way Add Funds confirms one
/// continued in the background.
#[test]
fn a_top_up_that_succeeds_after_its_form_was_left_is_confirmed() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity = stored_identity_with_wallet(&harness);
        open_top_up_form(&mut harness, &identity);
        let dispatch = confirm_top_up(&mut harness, &identity);
        leave_by_breadcrumb(&mut harness);

        deliver(&mut harness, topped_up(dispatch, &identity));
        assert_eq!(
            count(&harness, DONE_BANNER),
            1,
            "the top-up the user walked away from is confirmed"
        );
        assert_eq!(count(&harness, BACKGROUND_BANNER), 0);
        assert_eq!(count(&harness, WALLET_SEND_PROGRESS), 0);
    });
}

/// A top-up that fails after its form was closed leaves no progress behind
/// either: only the failure is reported.
#[test]
fn a_top_up_that_fails_after_its_form_was_left_leaves_no_progress_banner() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity = stored_identity_with_wallet(&harness);
        open_top_up_form(&mut harness, &identity);
        let dispatch = confirm_top_up(&mut harness, &identity);
        leave_by_breadcrumb(&mut harness);

        let error = TaskError::NoIdentitiesFound;
        let message = error.to_string();
        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error,
            },
        );
        assert_eq!(count(&harness, &message), 1, "the failure is reported");
        assert_eq!(count(&harness, BACKGROUND_BANNER), 0);
        assert_eq!(count(&harness, WALLET_SEND_PROGRESS), 0);
    });
}

/// A form covered by another screen is not told how its top-up ended. Back in
/// view it must not go on waiting for a result that already came.
#[test]
fn a_top_up_form_covered_while_its_top_up_ends_stops_waiting() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity = stored_identity_with_wallet(&harness);
        open_top_up_form(&mut harness, &identity);
        let dispatch = confirm_top_up(&mut harness, &identity);

        let other = identity_with_id([0x55; 32], identity.network);
        open_add_funds(&mut harness, &other);
        assert_eq!(count(&harness, WALLET_SEND_WAITING), 0);
        deliver(&mut harness, topped_up(dispatch, &identity));

        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        assert_eq!(
            count(&harness, WALLET_SEND_WAITING),
            0,
            "the top-up it waited on has ended"
        );
        assert_eq!(count(&harness, WALLET_SEND_PROGRESS), 0);
        assert!(
            prepare_top_up(&mut harness, &identity).is_ok(),
            "the form is usable again"
        );
        assert_eq!(
            count(&harness, DONE_BANNER),
            1,
            "the covered form could not say how it ended, so a banner did"
        );
    });
}

/// Ask for `network` and step the app until it runs on it. A switch ends in
/// whichever frame its result arrives, so nothing here counts frames.
fn switch_network(harness: &mut App, network: Network) {
    harness.state_mut().change_network(network);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        harness.step();
        let state = harness.state();
        if state.current_app_context().network() == network
            && state.boot_phase() == BootPhase::Ready
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the switch to {network:?} did not finish in time"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.run_steps(3);
}

/// A first visit to a network finishes switching while the app stays usable,
/// and every switch drops the stacked screens, dialogs and banners. A top-up
/// still running keeps running, so it must stay in flight and in view on the
/// other network and after the way back to its own.
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

        // The switch runs as a backend task on its own clock and ends in
        // whichever frame its result arrives, so the top-up is sent before the
        // switch is asked for. The switch finds the same state either way.
        open_add_funds(&mut harness, &identity);
        let dispatch = begin_top_up(&mut harness);
        switch_network(&mut harness, second);
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

        // Back to the identity's own network, where Add Funds can be opened.
        switch_network(&mut harness, first);
        assert_eq!(
            harness.state().current_app_context().network(),
            identity.network,
            "Add Funds is opened on the network the identity lives on"
        );
        assert_eq!(
            count(&harness, BACKGROUND_BANNER),
            1,
            "the banner follows the top-up through the switch back as well"
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
