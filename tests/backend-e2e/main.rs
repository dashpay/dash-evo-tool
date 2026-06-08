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
mod identity_withdraw;
mod register_dpns;
mod send_funds;
mod spv_wallet;
mod tx_is_ours;

mod core_tasks;
mod dashpay_tasks;
mod event_bridge_live;
mod identity_tasks;
mod mnlist_tasks;
mod shielded_tasks;
mod token_tasks;
mod wallet_tasks;
mod z_broadcast_st_tasks;
