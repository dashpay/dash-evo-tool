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
