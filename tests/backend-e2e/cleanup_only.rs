//! Standalone cleanup test.
//!
//! Initializing the shared `BackendTestContext` sweeps orphaned test wallets
//! (see `cleanup::cleanup_test_wallets`). This noop test triggers that
//! initialization so it can be run as a dedicated CI step after the E2E suite.
//!
//! ```bash
//! cargo test --test backend-e2e --all-features -- --ignored --nocapture cleanup_only
//! ```

use crate::framework::harness::ctx;

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn cleanup_only() {
    // Initialization performs cleanup_test_wallets() as its final step.
    let _ctx = ctx().await;
    tracing::info!("Cleanup-only run complete.");
}
