//! Kittest coverage for where a backend-task result goes on the real
//! `AppState`: to the screen that dispatched it, even while another screen
//! covers it, and to the screen in view only when no open screen sent it.
//!
//! No task reaches the backend: dispatches are entered through test seams and
//! their results are injected through the channel the app drains each frame.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppState, TaskResult};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::{BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::ui::identity::register_dpns_name_screen::RegisterDpnsNameSource;
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenType};
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

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

/// Open a username request page and start its status refresh, recorded as
/// that page's dispatch. Returns the dispatch.
fn refreshing_request_page(harness: &mut App) -> BackendTaskContext {
    let app_context = harness.state().current_app_context().clone();
    let page = ScreenType::UsernameRequestStatus {
        identity_id: Identifier::from([0x61; 32]),
        normalized_label: "alice".into(),
    }
    .create_screen(&app_context);
    harness.state_mut().screen_stack.push(page);
    let dispatch = request_page(harness)
        .begin_refresh_for_test()
        .expect("the page starts its refresh");
    harness.state_mut().test_record_visible_dispatch(&dispatch);
    harness.run_steps(3);
    dispatch
}

fn request_page(
    harness: &mut App,
) -> &mut dash_evo_tool::ui::identity::username_request_screen::UsernameRequestScreen {
    harness
        .state_mut()
        .screen_stack
        .iter_mut()
        .find_map(|screen| match screen {
            Screen::UsernameRequestScreen(page) => Some(page),
            _ => None,
        })
        .expect("the request page is open")
}

/// Cover the request page with the registration form, as "Register another
/// name" does.
fn cover_with_registration(harness: &mut App) {
    let app_context = harness.state().current_app_context().clone();
    let form = ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
        .create_screen(&app_context);
    harness.state_mut().screen_stack.push(form);
    harness.run_steps(3);
}

/// A request page covered by another screen still settles its own refresh,
/// whether it succeeds or fails, and ignores refreshes it did not send.
#[test]
fn a_covered_request_page_settles_its_own_refresh() {
    for fail in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
            let dispatch = refreshing_request_page(&mut harness);
            cover_with_registration(&mut harness);

            deliver(
                &mut harness,
                TaskResult::Success {
                    context: BackendTaskContext::Unknown,
                    result: Box::new(BackendTaskSuccessResult::MyUsernameRequestsRefreshed),
                },
            );
            assert_eq!(
                request_page(&mut harness).begin_refresh_for_test(),
                None,
                "a refresh the page did not send leaves its own running"
            );

            let result = if fail {
                TaskResult::Error {
                    context: dispatch,
                    error: TaskError::DataContractNotFound,
                }
            } else {
                TaskResult::Success {
                    context: dispatch,
                    result: Box::new(BackendTaskSuccessResult::MyUsernameRequestsRefreshed),
                }
            };
            deliver(&mut harness, result);
            assert_eq!(
                harness.state().test_dispatch_owner_count(),
                0,
                "a dispatch reports once, and its sender is forgotten then"
            );

            harness.state_mut().screen_stack.pop();
            harness.run_steps(3);
            assert!(
                request_page(&mut harness)
                    .begin_refresh_for_test()
                    .is_some(),
                "back in view, the page can refresh again"
            );
        });
    }
}

/// The result of a page closed before it arrived goes to the screen in view,
/// and a failure is still reported by the banner.
#[test]
fn a_closed_page_result_still_reports_its_failure() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let dispatch = refreshing_request_page(&mut harness);
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);

        let error = TaskError::DataContractNotFound;
        let message = error.to_string();
        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error,
            },
        );
        assert!(
            harness
                .query_all_by_label_contains(&message)
                .next()
                .is_some(),
            "the failure of a closed page is reported"
        );
    });
}

/// Store a username request for `identity_id` so its page has something to
/// show, Refresh button included.
fn store_request(harness: &App, identity_id: Identifier) {
    use dash_evo_tool::model::dpns::ContestDurations;
    use dash_evo_tool::model::dpns_usernames::UsernameRequest;
    let request = UsernameRequest::submitted(
        "ali",
        dash_evo_tool::utils::time::now_ms().saturating_sub(8 * 86_400_000),
        ContestDurations {
            total: std::time::Duration::from_secs(14 * 86_400),
            join: std::time::Duration::from_secs(7 * 86_400),
        },
        None,
    );
    harness
        .state()
        .current_app_context()
        .store_username_requests(&identity_id, vec![request])
        .expect("store the request");
}

/// The frame loop itself records the screen behind a task its `ui()`
/// returns: a real click on Refresh, with the task captured instead of run.
#[test]
fn a_clicked_refresh_reaches_its_page_behind_another_screen() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity_id = Identifier::from([0x62; 32]);
        store_request(&harness, identity_id);
        harness.state_mut().test_capture_dispatches();
        let app_context = harness.state().current_app_context().clone();
        let page = ScreenType::UsernameRequestStatus {
            identity_id,
            normalized_label: dash_evo_tool::model::dpns::normalize_dpns_label("ali"),
        }
        .create_screen(&app_context);
        harness.state_mut().screen_stack.push(page);
        harness.run_steps(3);

        harness.get_by_label("Refresh status").click();
        harness.run_steps(3);
        let dispatches = harness.state().test_captured_dispatches();
        assert_eq!(dispatches.len(), 1, "the click dispatches one refresh");
        let dispatch = dispatches[0].1.clone();
        assert_eq!(
            harness.state().test_dispatch_owner_count(),
            1,
            "the frame loop records the page as the refresh's sender"
        );
        assert!(harness.query_by_label("Refreshing…").is_some());

        cover_with_registration(&mut harness);
        deliver(
            &mut harness,
            TaskResult::Success {
                context: dispatch,
                result: Box::new(BackendTaskSuccessResult::MyUsernameRequestsRefreshed),
            },
        );
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Refresh status").is_some(),
            "the covered page settled its own refresh"
        );
    });
}

/// A failure that skips the general error path still reaches the covered
/// screen that dispatched the task, so it does not wait for good.
#[test]
fn a_covered_page_is_told_of_a_failure_outside_the_general_error_path() {
    use dash_evo_tool::backend_task::migration::MigrationError;
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let dispatch = refreshing_request_page(&mut harness);
        cover_with_registration(&mut harness);

        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error: TaskError::MigrationFailed {
                    source: std::sync::Arc::new(MigrationError::LegacyDataTooOld {
                        found: 1,
                        minimum_supported: 2,
                    }),
                },
            },
        );
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        assert!(
            request_page(&mut harness)
                .begin_refresh_for_test()
                .is_some(),
            "the page's refresh ended with the failure"
        );
    });
}

/// A screen that handles its own error in place keeps the banner away only
/// while it is in view. Out of view, the user would never see how it was
/// handled, so the banner still reports the failure.
#[test]
fn a_hidden_screen_handling_its_error_still_raises_the_banner() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::identity::IdentityTask;
    for hidden in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
            let dispatch = BackendTaskContext::for_dispatch_for_test(&BackendTask::IdentityTask(
                IdentityTask::RefreshMyUsernameRequests,
            ));
            harness.state_mut().test_record_visible_dispatch(&dispatch);
            if hidden {
                harness.state_mut().selected_main_screen =
                    RootScreenType::RootScreenWalletsBalances;
                harness.run_steps(3);
            }

            // The hub handles this one quietly: its refresh retries by itself.
            let error = TaskError::UsernameRequestRefreshFailed {
                source: Box::new(dash_sdk::Error::Generic("timeout".into())),
            };
            let message = error.to_string();
            deliver(
                &mut harness,
                TaskResult::Error {
                    context: dispatch,
                    error,
                },
            );
            assert_eq!(
                harness.query_all_by_label_contains(&message).count(),
                usize::from(hidden),
                "the banner appears only when the hub is out of view"
            );
        });
    }
}

/// A Send form for a fresh wallet.
fn send_form(harness: &App) -> dash_evo_tool::ui::wallets::send_screen::WalletSendScreen {
    use dash_evo_tool::model::wallet::Wallet;
    use std::sync::{Arc, RwLock};
    let app_context = harness.state().current_app_context().clone();
    let wallet = Wallet::new_from_seed([0x57; 64], app_context.network(), None, None)
        .expect("wallet fixture");
    dash_evo_tool::ui::wallets::send_screen::WalletSendScreen::new(
        &app_context,
        Arc::new(RwLock::new(wallet)),
    )
}

/// A send whose form was closed is not the next form's send. Its outcome is
/// reported by a banner and leaves the form in view as it was.
#[test]
fn a_closed_send_form_result_leaves_a_new_form_alone() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::wallet::WalletTask;
    for fail in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
            let form = Screen::WalletSendScreen(send_form(&harness));
            harness.state_mut().screen_stack.push(form);
            harness.run_steps(3);
            let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
                seed_hash: [0x57; 32],
            });
            let dispatch = match harness.state_mut().screen_stack.last_mut() {
                Some(Screen::WalletSendScreen(form)) => form.confirm_send_for_test(send),
                _ => panic!("the Send form is in view"),
            }
            .expect("the confirmed send is the form's dispatch");
            harness.state_mut().test_record_visible_dispatch(&dispatch);
            harness.run_steps(3);

            // The user closes the form while the send runs and opens a new one.
            harness.state_mut().screen_stack.pop();
            let form = Screen::WalletSendScreen(send_form(&harness));
            harness.state_mut().screen_stack.push(form);
            harness.run_steps(3);

            let (result, message) = if fail {
                let error = TaskError::DataContractNotFound;
                let message = error.to_string();
                (
                    TaskResult::Error {
                        context: dispatch,
                        error,
                    },
                    message,
                )
            } else {
                (
                    TaskResult::Success {
                        context: dispatch,
                        result: Box::new(BackendTaskSuccessResult::WalletPayment {
                            txid: "00".repeat(32),
                            recipients: vec![("yTestRecipient".into(), 100_000)],
                            total_amount: 100_000,
                        }),
                    },
                    format!(
                        "Sent {} to yTestRecipient",
                        dash_evo_tool::model::fee_estimation::format_duffs_as_dash(100_000)
                    ),
                )
            };
            deliver(&mut harness, result);
            // The form shows no outcome (below), so this one is the banner.
            assert_eq!(
                harness.query_all_by_label_contains(&message).count(),
                1,
                "a banner tells how the closed form's send ended"
            );
            assert_eq!(
                harness.query_all_by_label_contains("Send Another").count(),
                0,
                "the new form did not send it and does not claim it"
            );
        });
    }
}

/// The Documents screen in view.
fn documents(
    harness: &mut App,
) -> &mut dash_evo_tool::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen {
    match harness
        .state_mut()
        .main_screens
        .get_mut(&RootScreenType::RootScreenDocumentQuery)
    {
        Some(Screen::DocumentQueryScreen(screen)) => screen,
        _ => panic!("the Documents screen is open"),
    }
}

/// A send whose form was closed must not disturb the screen in view: a
/// document query running there keeps waiting for its own answer, and takes
/// it when it comes.
#[test]
fn a_closed_send_form_result_leaves_a_running_query_alone() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::wallet::WalletTask;
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenDocumentQuery);
        let form = Screen::WalletSendScreen(send_form(&harness));
        harness.state_mut().screen_stack.push(form);
        harness.run_steps(3);
        let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
            seed_hash: [0x57; 32],
        });
        let dispatch = match harness.state_mut().screen_stack.last_mut() {
            Some(Screen::WalletSendScreen(form)) => form.confirm_send_for_test(send),
            _ => panic!("the Send form is in view"),
        }
        .expect("the confirmed send is the form's dispatch");
        harness.state_mut().test_record_visible_dispatch(&dispatch);

        // The user closes the form and starts a query on the Documents screen.
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        let query = documents(&mut harness)
            .begin_fetch_for_test()
            .expect("the query is the screen's dispatch");
        harness.state_mut().test_record_visible_dispatch(&query);

        deliver(
            &mut harness,
            TaskResult::Success {
                context: dispatch,
                result: Box::new(BackendTaskSuccessResult::WalletPayment {
                    txid: "00".repeat(32),
                    recipients: vec![("yTestRecipient".into(), 100_000)],
                    total_amount: 100_000,
                }),
            },
        );
        assert!(
            documents(&mut harness).fetch_pending_for_test(),
            "the query still waits for its own answer"
        );

        deliver(
            &mut harness,
            TaskResult::Success {
                context: query,
                result: Box::new(BackendTaskSuccessResult::PageDocuments(
                    Default::default(),
                    None,
                )),
            },
        );
        assert!(
            !documents(&mut harness).fetch_pending_for_test(),
            "the query took its own answer"
        );
    });
}
