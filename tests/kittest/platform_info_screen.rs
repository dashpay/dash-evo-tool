//! Tools ▸ Platform info: a fetch and its result outlive refresh notifications.
//!
//! `AppState::update` calls `refresh()` on the visible screen for every
//! `TaskResult::Refresh`, and background work sends those several times a
//! second for minutes. The test drives that real path: the button click, the
//! notifications and the result all go through `AppState`.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppState, TaskResult};
use dash_evo_tool::backend_task::platform_info::PlatformInfoTaskResult;
use dash_evo_tool::backend_task::{BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const EPOCH_RESULT: &str = "Current epoch: 42";

/// Queue one refresh notification and let `AppState` deliver it.
fn deliver_refresh(harness: &mut Harness<'static, AppState>) {
    harness
        .state()
        .task_result_sender
        .try_send(TaskResult::Refresh)
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
        // No network: the click's backend task is never spawned, and the result
        // it would have produced is delivered by hand below.
        rt.block_on(harness.state().subtasks.shutdown_async())
            .expect("stop backend dispatch");

        harness.get_by_label("Fetch Current Epoch Info").click();
        harness.run_steps(2);
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
                context: BackendTaskContext::Unknown,
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
