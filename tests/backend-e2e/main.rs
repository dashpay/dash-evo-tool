//! Backend E2E tests for Dash Evo Tool.
//!
//! These tests exercise backend task flows directly (no GUI) against a live
//! network. They are marked `#[ignore]` and must be run explicitly:
//!
//! ```bash
//! cargo test --test backend-e2e --all-features -- --ignored --nocapture --test-threads=1
//! ```

mod cleanup;
mod funding;
mod harness;
mod identity_helpers;
mod task_runner;
mod wait;

mod fetch_contract;
mod identity_create;
mod identity_withdraw;
mod register_dpns;
mod send_funds;
mod spv_wallet;
