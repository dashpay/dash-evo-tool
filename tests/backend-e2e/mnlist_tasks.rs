//! MnListTask backend E2E tests.
//!
//! Most MnListTask variants (FetchEndDmlDiff, FetchEndQrInfo, FetchEndQrInfoWithDmls,
//! FetchDiffsChain, FetchChainLocks) require Core RPC for block hash retrieval and are
//! NOT available in SPV mode. They have been removed from this test file.
//!
//! Only the error-path test (TC-073) is retained as it uses hardcoded hashes.

use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::mnlist::MnListTask;
use dash_evo_tool::backend_task::BackendTask;
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::hashes::Hash;

// TC-068 through TC-072: REMOVED — all require Core RPC for block hash retrieval,
// not available in SPV mode.

// ─────────────────────────────────────────────────────────────────────────────
// TC-073: MnListTask error — invalid block hash
// ─────────────────────────────────────────────────────────────────────────────

/// TC-073: FetchEndDmlDiff with all-zeros block hash — must return a P2P error.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_073_fetch_dml_diff_invalid_hash() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let zero_hash = BlockHash::all_zeros();

    let task = BackendTask::MnListTask(MnListTask::FetchEndDmlDiff {
        base_block_height: 0,
        base_block_hash: zero_hash,
        block_height: 1,
        block_hash: zero_hash,
        validate_quorums: false,
    });

    let result = run_task(app_context, task).await;

    assert!(
        result.is_err(),
        "TC-073: expected error for all-zeros block hash, got: {:?}",
        result
    );
    tracing::info!("TC-073: got expected error: {:?}", result.unwrap_err());
}
