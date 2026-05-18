//! Platform-wallet migration QA harness (P3e) — standalone integration crate.
//!
//! Network-free integration coverage for the one-time platform-wallet
//! migration, exercised through the crate's PUBLIC surface only
//! (`Database::{new, initialize, recover_from_premigration_if_corrupt}` and
//! the public settings/dashpay methods). The deterministic destructive-order
//! and restore invariants are pinned by the in-crate `p3d` unit module; this
//! harness adds the integration-level lanes the QA matrix requires:
//!
//! * Stage-B crash at each sub-step + relaunch idempotency (modelled at the
//!   DB/settings boundary — the part that is network-free).
//! * Restore-from-premigration on injected new-state corruption.
//! * User-never-unlocks: marker persists, app usable, backup exists.
//! * Reentrant launch path: the marker provides cross-launch single-run.
//! * RELEASE-BLOCKING: the mandatory one-time post-migration notice — shows
//!   exactly once, gated by the one-shot
//!   `settings.platform_wallet_migration_notice_shown` flag, dismissible,
//!   jargon-free, shown to every migrated user unconditionally.
//!
//! Run: `cargo test --test migration_pw --all-features`

mod common;
mod notice;
mod recovery;
mod stage_b_idempotency;
