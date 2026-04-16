//! Backend E2E tests for Dash Evo Tool.
//!
//! These tests exercise backend task flows directly (no GUI) against a live
//! network. They are marked `#[ignore]` and must be run explicitly:
//!
//! ```bash
//! RUST_MIN_STACK=16777216 cargo test --test backend-e2e --all-features -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The `RUST_MIN_STACK=16777216` (16 MB) is required because the Dash Platform SDK
//! uses deep call stacks during state transition construction, exceeding the
//! default 8 MB thread stack size.

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
mod identity_tasks;
mod mnlist_tasks;
mod shielded_tasks;
mod token_tasks;
mod wallet_tasks;
mod z_broadcast_st_tasks;
