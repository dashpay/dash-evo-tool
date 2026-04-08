//! MnListTask backend E2E tests: TC-068 to TC-073.
//!
//! These tests exercise read-only P2P masternode list queries against a live
//! testnet. They require SPV to be synced and (for block hash retrieval) Core
//! RPC to be reachable.

use crate::framework::harness::ctx;
use crate::framework::mnlist_helpers::{get_block_hash_at_height, get_current_block_info};
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::mnlist::MnListTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::hashes::Hash;

/// Guard: MnList tests need Core RPC for block hash retrieval.
/// Skip gracefully when Core RPC is unavailable.
fn require_core_rpc() -> bool {
    if std::env::var("E2E_CORE_RPC_URL").is_err() {
        println!("Skipping: E2E_CORE_RPC_URL not set — MnList tests require Core RPC for block hash retrieval");
        false
    } else {
        true
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-068: FetchEndDmlDiff
// ─────────────────────────────────────────────────────────────────────────────

/// TC-068: FetchEndDmlDiff — fetch masternode list diff between two blocks.
///
/// Uses current tip and (tip - 100) as base, obtained at runtime via Core RPC.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_068_fetch_end_dml_diff() {
    if !require_core_rpc() { return; }
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let (tip_height, tip_hash) = get_current_block_info(app_context);
    let base_height = tip_height - 100;
    let base_hash = get_block_hash_at_height(app_context, base_height);

    let task = BackendTask::MnListTask(MnListTask::FetchEndDmlDiff {
        base_block_height: base_height,
        base_block_hash: base_hash,
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
            assert_eq!(got_base, base_height, "base_height mismatch");
            assert_eq!(got_tip, tip_height, "height mismatch");
            // The diff's block_hash must match the tip hash we requested
            assert_eq!(
                diff.block_hash, tip_hash,
                "diff block_hash should match requested tip"
            );
            assert_eq!(
                diff.base_block_hash, base_hash,
                "diff base_block_hash should match requested base"
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
    if !require_core_rpc() { return; }
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let genesis_hash = get_block_hash_at_height(app_context, 0);
    let (_, tip_hash) = get_current_block_info(app_context);

    let task = BackendTask::MnListTask(MnListTask::FetchEndQrInfo {
        known_block_hashes: vec![genesis_hash],
        block_hash: tip_hash,
    });

    let result = run_task(app_context, task)
        .await
        .expect("TC-069: FetchEndQrInfo should succeed");

    match result {
        BackendTaskSuccessResult::MnListFetchedQrInfo { qr_info } => {
            // The tip diff must reference the requested block hash
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
    if !require_core_rpc() { return; }
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let genesis_hash = get_block_hash_at_height(app_context, 0);
    let (_, tip_hash) = get_current_block_info(app_context);

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

/// TC-071: FetchDiffsChain — fetch a sequence of two consecutive MNList diffs.
///
/// Chain: [(tip-200, tip-100), (tip-100, tip)]
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_071_fetch_diffs_chain() {
    if !require_core_rpc() { return; }
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let (tip_height, tip_hash) = get_current_block_info(app_context);
    let h100 = tip_height - 100;
    let h200 = tip_height - 200;
    let hash100 = get_block_hash_at_height(app_context, h100);
    let hash200 = get_block_hash_at_height(app_context, h200);

    let chain = vec![
        (h200, hash200, h100, hash100),
        (h100, hash100, tip_height, tip_hash),
    ];

    let task = BackendTask::MnListTask(MnListTask::FetchDiffsChain { chain });

    let result = run_task(app_context, task)
        .await
        .expect("TC-071: FetchDiffsChain should succeed");

    match result {
        BackendTaskSuccessResult::MnListFetchedDiffs { items } => {
            assert_eq!(items.len(), 2, "expected 2 diff items in chain");
            let ((b0, h0), _) = &items[0];
            let ((b1, h1), _) = &items[1];
            assert_eq!(*b0, h200, "first diff base height mismatch");
            assert_eq!(*h0, h100, "first diff end height mismatch");
            assert_eq!(*b1, h100, "second diff base height mismatch");
            assert_eq!(*h1, tip_height, "second diff end height mismatch");
        }
        other => panic!("TC-071: expected MnListFetchedDiffs, got: {:?}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TC-072: FetchChainLocks (conditional on E2E_CORE_RPC_URL)
// ─────────────────────────────────────────────────────────────────────────────

/// TC-072: FetchChainLocks — fetch block-level chainlock signatures via Core RPC.
///
/// Skipped when `E2E_CORE_RPC_URL` is not set, as the task requires an
/// accessible Core RPC node with block data.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_072_fetch_chain_locks() {
    if std::env::var("E2E_CORE_RPC_URL").is_err() {
        tracing::info!("TC-072: skipped — E2E_CORE_RPC_URL not set");
        return;
    }

    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let (tip_height, _) = get_current_block_info(app_context);
    let base_height = tip_height - 10;

    let task = BackendTask::MnListTask(MnListTask::FetchChainLocks {
        base_block_height: base_height,
        block_height: tip_height,
    });

    let result = run_task(app_context, task)
        .await
        .expect("TC-072: FetchChainLocks should succeed");

    match result {
        BackendTaskSuccessResult::MnListChainLockSigs { entries } => {
            assert!(
                !entries.is_empty(),
                "expected at least one chainlock entry in the range"
            );
            // Each entry should have a valid block height in the requested range
            for ((height, _hash), _sig_opt) in &entries {
                assert!(
                    *height >= base_height && *height < tip_height,
                    "chainlock entry height {} outside expected range [{}, {})",
                    height,
                    base_height,
                    tip_height
                );
            }
        }
        other => panic!("TC-072: expected MnListChainLockSigs, got: {:?}", other),
    }
}

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
        "TC-073: expected Err for all-zeros block hash, got: {:?}",
        result
    );
    tracing::info!("TC-073: received expected error: {:?}", result.unwrap_err());
}
