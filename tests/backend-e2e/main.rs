//! Backend E2E tests for Dash Evo Tool.
//!
//! These tests exercise backend task flows directly (no GUI) against a live
//! network. They are marked `#[ignore]` and must be run explicitly:
//!
//! ```bash
//! cargo test --test backend-e2e --all-features -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The Dash Platform SDK recurses deeply during proof verification, exceeding the
//! default 8 MB thread stack. The harness drives every SDK-bearing task on a
//! dedicated 32 MB-stack runtime (see [`framework::task_runner`]), so callers no
//! longer need to set `RUST_MIN_STACK`.

mod framework;

mod cleanup_only;
mod fetch_contract;
mod identity_create;
mod identity_masternode_withdraw;
mod identity_withdraw;
mod register_dpns;
mod send_funds;
mod spv_wallet;
mod tx_is_ours;

mod identity_cold_boot;
mod spv_reconnect;

mod core_tasks;
// TODO(dashpay-e2e): deferred — dashpay backend depends on upstream platform-wallet dashpay completion. Re-enable once dashpay/platform#3841 ("complete dashpay", shumkov) lands and the platform-wallet dep is bumped. Tests: tc_032/033/036/037/041/043/044/045/046.
// mod dashpay_tasks;
mod event_bridge_live;
mod identity_in_vault_sign;
mod identity_tasks;
mod shielded_tasks;
mod token_tasks;
mod wallet_reregistration;
mod wallet_tasks;
mod z_broadcast_st_tasks;
