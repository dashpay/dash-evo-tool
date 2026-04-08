//! MnListTask backend E2E tests: TC-068 to TC-073.
//!
//! These tests exercise read-only P2P masternode list queries against a live
//! testnet. They require SPV to be synced. Block info is obtained via DAPI
//! (Platform gRPC) and the well-known genesis hash — no Core RPC needed.

use crate::framework::harness::ctx;
use crate::framework::mnlist_helpers::{get_current_block_info, get_genesis_hash};
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::mnlist::MnListTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::hashes::Hash;

// ─────────────────────────────────────────────────────────────────────────────
// TC-068: FetchEndDmlDiff
// ─────────────────────────────────────────────────────────────────────────────

/// TC-068: FetchEndDmlDiff — fetch masternode list diff between genesis and tip.
///
/// Uses genesis hash (compile-time constant) as base and DAPI-reported tip as
/// target. Production code uses the same P2P protocol (`CoreP2PHandler`).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_068_fetch_end_dml_diff() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let (tip_height, tip_hash) = get_current_block_info(app_context).await;
    let genesis_hash = get_genesis_hash(app_context);

    let task = BackendTask::MnListTask(MnListTask::FetchEndDmlDiff {
        base_block_height: 0,
        base_block_hash: genesis_hash,
        block_height: tip_height,
        block_hash: tip_hash,
        validate_quorums: false,
    });

    let result = run_task(app_context, task)
        .await
        .expect("TC-068: FetchEndDmlDiff should succeed");

    match result {
        BackendTaskSuccessResult::MnListFetchedDiff {
            base_height: got_base,
            height: got_tip,
            diff,
        } => {
            assert_eq!(got_base, 0, "base_height mismatch");
            assert_eq!(got_tip, tip_height, "height mismatch");
            assert_eq!(
                diff.block_hash, tip_hash,
                "diff block_hash should match requested tip"
            );
            assert_eq!(
                diff.base_block_hash, genesis_hash,
                "diff base_block_hash should match genesis"
            );
        }
        other => panic!("TC-068: expected MnListFetchedDiff, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-069: FetchEndQrInfo
// ─────────────────────────────────────────────────────────────────────────────

/// TC-069: FetchEndQrInfo — fetch quorum rotation info using genesis as known block.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_069_fetch_end_qr_info() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let genesis_hash = get_genesis_hash(app_context);
    let (_, tip_hash) = get_current_block_info(app_context).await;

    let task = BackendTask::MnListTask(MnListTask::FetchEndQrInfo {
        known_block_hashes: vec![genesis_hash],
        block_hash: tip_hash,
    });

    let result = run_task(app_context, task)
        .await
        .expect("TC-069: FetchEndQrInfo should succeed");

    match result {
        BackendTaskSuccessResult::MnListFetchedQrInfo { qr_info } => {
            assert_eq!(
                qr_info.mn_list_diff_tip.block_hash, tip_hash,
                "TC-069: mn_list_diff_tip block_hash should match requested tip"
            );
        }
        other => panic!("TC-069: expected MnListFetchedQrInfo, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-070: FetchEndQrInfoWithDmls
// ─────────────────────────────────────────────────────────────────────────────

/// TC-070: FetchEndQrInfoWithDmls — same as TC-069 but via the DML-supplemented variant.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_070_fetch_end_qr_info_with_dmls() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let genesis_hash = get_genesis_hash(app_context);
    let (_, tip_hash) = get_current_block_info(app_context).await;

    let task = BackendTask::MnListTask(MnListTask::FetchEndQrInfoWithDmls {
        known_block_hashes: vec![genesis_hash],
        block_hash: tip_hash,
    });

    let result = run_task(app_context, task)
        .await
        .expect("TC-070: FetchEndQrInfoWithDmls should succeed");

    match result {
        BackendTaskSuccessResult::MnListFetchedQrInfo { qr_info } => {
            assert_eq!(
                qr_info.mn_list_diff_tip.block_hash, tip_hash,
                "TC-070: mn_list_diff_tip block_hash should match requested tip"
            );
        }
        other => panic!("TC-070: expected MnListFetchedQrInfo, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-071: FetchDiffsChain
// ─────────────────────────────────────────────────────────────────────────────

/// TC-071: FetchDiffsChain — fetch a single-segment diff chain from genesis to tip.
///
/// Without Core RPC, we cannot look up block hashes at arbitrary heights.
/// DAPI provides only the tip hash, and genesis is a compile-time constant.
/// This limits us to a single chain segment, but it still exercises the
/// `FetchDiffsChain` code path (P2P loop, result accumulation).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_071_fetch_diffs_chain() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let (tip_height, tip_hash) = get_current_block_info(app_context).await;
    let genesis_hash = get_genesis_hash(app_context);

    let chain = vec![(0, genesis_hash, tip_height, tip_hash)];

    let task = BackendTask::MnListTask(MnListTask::FetchDiffsChain { chain });

    let result = run_task(app_context, task)
        .await
        .expect("TC-071: FetchDiffsChain should succeed");

    match result {
        BackendTaskSuccessResult::MnListFetchedDiffs { items } => {
            assert_eq!(items.len(), 1, "expected 1 diff item in chain");
            let ((b0, h0), _) = &items[0];
            assert_eq!(*b0, 0, "diff base height mismatch");
            assert_eq!(*h0, tip_height, "diff end height mismatch");
        }
        other => panic!("TC-071: expected MnListFetchedDiffs, got: {:?}", other),
    }
}

// TC-072: FetchChainLocks — REMOVED. Genuinely requires Core RPC for
// `client.get_block_hash()` and `client.get_block()` calls.

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
