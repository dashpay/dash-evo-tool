//! Backend E2E: read-only `PlatformInfo` queries against live testnet.
//!
//! These cover claims that can only be settled against a real network at the
//! protocol version the network runs:
//!
//! - the current epoch is fetched live again (dashpay/platform#4231 is fixed in
//!   this repo's pin), instead of the degraded, hardcoded rendering;
//! - the completed-withdrawals filter, which sends the `FAILED` status that the
//!   protocol-13 withdrawals schema does not define, is accepted by Drive
//!   rather than rejected.
//!
//! No funds move and no state transition is broadcast.

use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::platform_info::{
    PlatformInfoTaskRequestType, PlatformInfoTaskResult,
};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};

/// Run a `PlatformInfo` request and return its text rendering.
async fn platform_info_text(request: PlatformInfoTaskRequestType) -> String {
    let ctx = ctx().await;
    let result = run_task(&ctx.app_context, BackendTask::PlatformInfo(request))
        .await
        .expect("platform info request should succeed");
    match result {
        BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::TextResult(text)) => text,
        other => panic!("Expected a text result, got: {other:?}"),
    }
}

// ── TC-PI-001 — the current epoch is read from the network, not rendered from
// the hardcoded fallback ──────────────────────────────────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_pi001_current_epoch_is_fetched_live() {
    let text = platform_info_text(PlatformInfoTaskRequestType::CurrentEpochInfo).await;
    tracing::info!("CurrentEpochInfo:\n{text}");

    assert!(
        !text.contains("could not be read from the network"),
        "the live epoch fetch must succeed, got the degraded rendering: {text}"
    );
    // Fields only the live fetch can fill.
    for field in [
        "• Epoch Index:",
        "• Start Height:",
        "• Start Core Height:",
        "(Fee multiplier cache updated:",
    ] {
        assert!(text.contains(field), "expected {field} in: {text}");
    }

    // The fetch is a network observation, so it confirms the protocol version.
    let ctx = ctx().await;
    assert_ne!(
        ctx.app_context.platform_protocol_version(),
        0,
        "a live epoch fetch must confirm the network's protocol version"
    );
}

// ── TC-PI-002 — the completed-withdrawals filter is accepted at protocol 13 ───
//
// The filter sends `FAILED` (5), which the protocol-13 withdrawals schema's
// `status` enum (0..4) does not define. A schema enum constrains documents, not
// query values, so Drive must answer the query instead of rejecting it.

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_pi002_completed_withdrawals_query_is_accepted() {
    let text = platform_info_text(PlatformInfoTaskRequestType::RecentlyCompletedWithdrawals).await;
    tracing::info!("RecentlyCompletedWithdrawals:\n{text}");

    // Either outcome proves acceptance; an unsupported where value would have
    // failed the request instead.
    assert!(
        text.contains("Recent Withdrawal History:")
            || text.contains("No recent withdrawal history"),
        "unexpected rendering: {text}"
    );

    // The structured variant shares the same filter.
    let ctx = ctx().await;
    let result = run_task(
        &ctx.app_context,
        BackendTask::PlatformInfo(PlatformInfoTaskRequestType::Withdrawals {
            completed: true,
            limit: Some(5),
            start_after: None,
        }),
    )
    .await
    .expect("the structured completed-withdrawals query should succeed");
    let BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::Withdrawals { .. }) = result
    else {
        panic!("Expected a structured withdrawals result, got: {result:?}");
    };
}

// ── TC-PI-003 — the in-queue withdrawals query still works ────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_pi003_queued_withdrawals_query_is_accepted() {
    let text = platform_info_text(PlatformInfoTaskRequestType::CurrentWithdrawalsInQueue).await;
    tracing::info!("CurrentWithdrawalsInQueue:\n{text}");
    assert!(
        !text.is_empty(),
        "the queued-withdrawals rendering is empty"
    );
}
