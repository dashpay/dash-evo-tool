//! Tools ▸ Platform info: a fetch and its result outlive refresh notifications.
//!
//! Scoped cache notifications can arrive during a Platform Info request.
//! The button click, notifications, and captured request's result all go through
//! `AppState`; backend execution is replaced with a deterministic completion.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppState, TaskResult};
use dash_evo_tool::backend_task::platform_info::{
    PlatformInfoTaskRequestType, PlatformInfoTaskResult,
};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const EPOCH_RESULT: &str = "Current epoch: 42";

/// Queue one refresh notification and let `AppState` deliver it.
fn deliver_refresh(harness: &mut Harness<'static, AppState>) {
    harness
        .state()
        .task_result_sender
        .try_send(TaskResult::CacheUpdated(
            dash_evo_tool::backend_task::CacheUpdate {
                network: harness.state().current_app_context().network(),
                domain: dash_evo_tool::backend_task::CacheDomain::Identities,
            },
        ))
        .expect("queue the refresh notification");
    harness.run_steps(2);
}

#[test]
fn a_fetch_started_during_a_refresh_burst_shows_progress_and_then_its_result() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenToolsPlatformInfoScreen);
        // Capture an accepted request without executing its network work.
        harness.state_mut().test_capture_dispatches();

        harness.get_by_label("Fetch Current Epoch Info").click();
        harness.run_steps(2);
        let context = harness
            .state()
            .test_captured_dispatches()
            .iter()
            .find_map(|(task, context)| {
                matches!(
                    task,
                    BackendTask::PlatformInfo(PlatformInfoTaskRequestType::CurrentEpochInfo)
                )
                .then(|| context.clone())
            })
            .expect("the button dispatches a current-epoch request");
        assert!(
            harness.query_by_label("Loading...").is_some(),
            "the premise: a started fetch shows its progress"
        );

        for _ in 0..3 {
            deliver_refresh(&mut harness);
        }
        assert!(
            harness.query_by_label("Loading...").is_some(),
            "a refresh notification must not hide a fetch that is still running"
        );
        assert!(
            harness.query_by_label("No results yet").is_none(),
            "a running fetch must not look as if it was never started"
        );

        harness
            .state()
            .task_result_sender
            .try_send(TaskResult::Success {
                context,
                result: Box::new(BackendTaskSuccessResult::PlatformInfo(
                    PlatformInfoTaskResult::TextResult(EPOCH_RESULT.to_owned()),
                )),
            })
            .expect("queue the fetch result");
        harness.run_steps(2);
        for _ in 0..3 {
            deliver_refresh(&mut harness);
        }

        assert!(
            harness.query_by_label(EPOCH_RESULT).is_some(),
            "the fetched result must stay on screen while refresh notifications keep arriving"
        );
        assert!(
            harness
                .query_by_label("Current Epoch Information")
                .is_some(),
            "the result must keep the title of the fetch that produced it"
        );
        assert!(
            harness.query_by_label("Loading...").is_none(),
            "a finished fetch must stop showing progress"
        );
    });
}
