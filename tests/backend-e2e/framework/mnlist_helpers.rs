//! Helpers for retrieving block info for MnList tests.

// TODO(production-reuse): This helper parallels `src/backend_task/mnlist.rs` and
// `src/ui/tools/masternode_list_diff_screen.rs::get_block_hash`.
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

use dash_evo_tool::context::AppContext;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::hashes::Hash;
use std::sync::Arc;

/// Get current block tip height and hash from Core RPC.
///
/// Returns `(height, block_hash)` of the best known block.
///
/// # Panics
///
/// Panics if Core RPC is not available (tests using this should be
/// guarded by `E2E_CORE_RPC_URL` env var check).
pub fn get_current_block_info(app_context: &Arc<AppContext>) -> (u32, BlockHash) {
    let client = app_context
        .core_client_for_wallet(None)
        .expect("mnlist_helpers: Core RPC not available (is E2E_CORE_RPC_URL set?)");
    let info = client
        .get_blockchain_info()
        .expect("mnlist_helpers: get_blockchain_info failed");
    let height = info.blocks as u32;
    let hash = client
        .get_block_hash(height)
        .expect("mnlist_helpers: get_block_hash for tip failed");
    let block_hash = BlockHash::from_byte_array(hash.to_byte_array());
    (height, block_hash)
}

/// Get block hash at a specific height from Core RPC.
///
/// # Panics
///
/// Panics if Core RPC is not available or the height is invalid.
pub fn get_block_hash_at_height(app_context: &Arc<AppContext>, height: u32) -> BlockHash {
    let client = app_context
        .core_client_for_wallet(None)
        .expect("mnlist_helpers: Core RPC not available (is E2E_CORE_RPC_URL set?)");
    let hash = client
        .get_block_hash(height)
        .unwrap_or_else(|e| panic!("mnlist_helpers: get_block_hash({}) failed: {}", height, e));
    BlockHash::from_byte_array(hash.to_byte_array())
}
