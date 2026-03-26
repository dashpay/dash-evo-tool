//! Backend E2E tests for Dash Evo Tool.
//!
//! These tests exercise backend task flows directly (no GUI) against a live
//! network. They are marked `#[ignore]` and must be run explicitly:
//!
//! ```bash
//! cargo test --test backend-e2e --all-features -- --ignored --nocapture --test-threads=1
//! ```

mod framework;

mod cleanup_only;
mod fetch_contract;
mod identity_create;
mod identity_withdraw;
mod register_dpns;
mod send_funds;
mod spv_wallet;
mod tx_is_ours;
