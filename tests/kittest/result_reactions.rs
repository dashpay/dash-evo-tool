//! Kittest coverage for the domain reactions to a backend result on the real
//! `AppState`: persistent roots observe outcomes they did not send, exactly
//! once; a root settles its own task while hidden; and a result of a network
//! no longer active reaches none of the new network's screens.
//!
//! No task reaches the backend: dispatches are entered through test seams and
//! their results are injected through the channel the app drains each frame.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppState, BootPhase, TaskResult};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::model::legacy_restore::LegacyRestoreSummary;
use dash_evo_tool::ui::identity::IDENTITY_REMOVED;
use dash_evo_tool::ui::network_chooser_screen::NetworkChooserScreen;
use dash_evo_tool::ui::{RootScreenType, Screen};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, Instant};

type App = Harness<'static, AppState>;

fn in_app(test: impl FnOnce()) {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        test();
    });
}

fn deliver(harness: &mut App, result: TaskResult) {
    assert!(
        harness.state().task_result_sender.try_send(result).is_ok(),
        "the task result channel accepts the result"
    );
    harness.run_steps(3);
}

fn count(harness: &App, text: &str) -> usize {
    harness.query_all_by_label_contains(text).count()
}

fn removed(network: Network, identity_id: Identifier) -> BackendTaskSuccessResult {
    BackendTaskSuccessResult::RemovedIdentities {
        network,
        identity_ids: vec![identity_id],
        associated_cleanup_failed: false,
        local_data_cleanup_failed: false,
        cleanup_deferred: false,
    }
}

/// Switch to the other of Testnet and Mainnet and wait until it is ready.
fn switch_network(harness: &mut App) {
    let first = harness.state().current_app_context().network();
    let second = if first == Network::Testnet {
        Network::Mainnet
    } else {
        Network::Testnet
    };
    harness.state_mut().change_network(second);
    let deadline = Instant::now() + Duration::from_secs(60);
    while harness.state().current_app_context().network() != second
        || harness.state().boot_phase() != BootPhase::Ready
    {
        assert!(
            Instant::now() < deadline,
            "the switch did not finish in time"
        );
        harness.step();
    }
}

/// An identity removed while the Masternodes screen is in view is also
/// dropped by the hidden hub as the app-wide selection, and the removal is
/// reported by one banner.
#[test]
fn a_removal_seen_from_masternodes_reaches_the_hidden_hub_once() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenMasternodes);
        let identity_id = Identifier::from([0x52; 32]);
        let app_context = harness.state().current_app_context().clone();
        app_context.set_selected_identity(Some(identity_id));
        harness.run_steps(3);

        deliver(
            &mut harness,
            TaskResult::Success {
                context: BackendTaskContext::Unknown,
                result: Box::new(removed(app_context.network(), identity_id)),
            },
        );
        assert_eq!(
            app_context.selected_identity_id(),
            None,
            "the hidden hub drops the removed identity as the selection"
        );
        assert_eq!(
            count(&harness, IDENTITY_REMOVED),
            1,
            "one banner reports the removal"
        );
    });
}

/// A removal dispatched on one network whose result arrives after a switch
/// to another reaches none of the new network's screens: the hub keeps the
/// selection it holds there, even for an identity of the same id. The
/// dispatch still reports once, by the banner alone.
#[test]
fn a_removal_from_the_previous_network_leaves_the_new_networks_hub_alone() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let identity_id = Identifier::from([0x53; 32]);
        let dispatch = BackendTaskContext::for_dispatch_for_test(&BackendTask::IdentityTask(
            IdentityTask::RemoveIdentity { identity_id },
        ));
        harness.state_mut().test_record_visible_dispatch(&dispatch);

        switch_network(&mut harness);
        let app_context = harness.state().current_app_context().clone();
        app_context.set_selected_identity(Some(identity_id));
        harness.run_steps(3);

        deliver(
            &mut harness,
            TaskResult::Success {
                context: dispatch,
                result: Box::new(removed(app_context.network(), identity_id)),
            },
        );
        assert_eq!(
            app_context.selected_identity_id(),
            Some(identity_id),
            "the previous network's removal does not touch the new network's selection"
        );
        assert_eq!(
            count(&harness, IDENTITY_REMOVED),
            1,
            "the banner alone reports the removal"
        );
        assert_eq!(
            harness.state().test_dispatch_owner_count(),
            0,
            "the dispatch reported once"
        );
    });
}

fn chooser(harness: &mut App) -> &mut NetworkChooserScreen {
    match harness
        .state_mut()
        .main_screens
        .get_mut(&RootScreenType::RootScreenNetworkChooser)
    {
        Some(Screen::NetworkChooserScreen(screen)) => screen,
        _ => panic!("the network chooser is open"),
    }
}

/// A restore from the previous version started on the Settings screen is
/// settled there even while another screen is in view, whether it succeeds
/// or fails: the Restore button is freed, the dispatch is forgotten, and one
/// banner reports the outcome.
#[test]
fn a_hidden_chooser_settles_its_own_restore() {
    for fail in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenNetworkChooser);
            harness.state_mut().test_capture_dispatches();
            let action = chooser(&mut harness).begin_restore_for_test();
            harness.state_mut().test_dispatch_screen_action(action);
            let restore = harness
                .state()
                .test_captured_dispatches()
                .last()
                .map(|(_, context)| context.clone())
                .expect("the restore is dispatched");

            harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
            harness.run_steps(3);

            let (result, message) = if fail {
                let error = TaskError::TaskManagerShuttingDown;
                let message = error.to_string();
                (
                    TaskResult::Error {
                        context: restore,
                        error,
                    },
                    message,
                )
            } else {
                let summary = LegacyRestoreSummary::default();
                let message = summary.user_message();
                (
                    TaskResult::Success {
                        context: restore,
                        result: Box::new(BackendTaskSuccessResult::PreviousVersionRestored(
                            summary,
                        )),
                    },
                    message,
                )
            };
            deliver(&mut harness, result);
            assert!(
                !chooser(&mut harness).restore_in_progress_for_test(),
                "the hidden chooser's own result frees its button"
            );
            assert_eq!(harness.state().test_dispatch_owner_count(), 0);
            assert_eq!(
                count(&harness, &message),
                1,
                "one banner reports the outcome"
            );
        });
    }
}

/// A dispatch the chooser started, captured instead of run.
fn captured_dispatch(
    harness: &mut App,
    action: dash_evo_tool::app::AppAction,
) -> BackendTaskContext {
    harness.state_mut().test_dispatch_screen_action(action);
    harness
        .state()
        .test_captured_dispatches()
        .last()
        .map(|(_, context)| context.clone())
        .expect("the action is dispatched")
}

/// A failure of one Settings task settles that task alone: a failed restore
/// leaves a running DAPI endpoint discovery running, and the discovery's own
/// failure frees its button.
#[test]
fn a_settings_task_failure_settles_only_its_own_task() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenNetworkChooser);
        harness.state_mut().test_capture_dispatches();
        let discovery_action = chooser(&mut harness).begin_discovery_for_test();
        let discovery = captured_dispatch(&mut harness, discovery_action);
        let restore_action = chooser(&mut harness).begin_restore_for_test();
        let restore = captured_dispatch(&mut harness, restore_action);

        deliver(
            &mut harness,
            TaskResult::Error {
                context: restore,
                error: TaskError::TaskManagerShuttingDown,
            },
        );
        assert!(!chooser(&mut harness).restore_in_progress_for_test());
        assert!(
            chooser(&mut harness).discovery_in_progress_for_test(),
            "a failed restore leaves the running discovery alone"
        );

        deliver(
            &mut harness,
            TaskResult::Error {
                context: discovery,
                error: TaskError::TaskManagerShuttingDown,
            },
        );
        assert!(
            !chooser(&mut harness).discovery_in_progress_for_test(),
            "the discovery's own failure frees its button"
        );
        assert_eq!(harness.state().test_dispatch_owner_count(), 0);
    });
}

/// A scheduled-vote sweep that fails is reported by one banner, unless the
/// sweep found no voting identity on this device, which is no news: that
/// failure is quiet.
#[test]
fn a_sweep_failure_is_reported_unless_it_is_quiet() {
    fn no_voting_identity() -> TaskError {
        TaskError::NoVotingIdentity {
            identity_id: "voter-id".to_owned(),
        }
    }
    fn vote_unavailable() -> TaskError {
        TaskError::DpnsCurrentVoteUnavailable
    }
    for (source, reported) in [
        (no_voting_identity as fn() -> TaskError, false),
        (vote_unavailable as fn() -> TaskError, true),
    ] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenDocumentQuery);
            let network = harness.state().current_app_context().network();
            let error = TaskError::ScheduledVoteSweepFailed {
                network,
                source: Box::new(source()),
            };
            let message = error.to_string();
            deliver(
                &mut harness,
                TaskResult::Error {
                    context: BackendTaskContext::ScheduledVoteSweep { network },
                    error,
                },
            );
            assert_eq!(
                count(&harness, &message),
                usize::from(reported),
                "a sweep failing with {:?} is reported: {reported}",
                source()
            );
        });
    }
}
