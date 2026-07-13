//! Atomic snapshot of the legacy-data migration progress.
//!
//! [`MigrationStatus`] is the single source of truth the UI hot path
//! reads to decide whether to show a "your data is being migrated"
//! banner, an empty-state placeholder, or normal wallet content. The
//! [`MigrationTask`](crate::backend_task::migration::MigrationTask)
//! orchestrator writes state transitions as the migration walks each
//! legacy table; everything else is read-only.
//!
//! Backed by [`ArcSwap`] so each frame can `load()` the current state
//! without taking a lock — the UI calls this from `update()`.

use std::sync::Arc;

use arc_swap::ArcSwap;

/// Which legacy domain the migration is currently working on.
///
/// Drives banner text granularity ("Migrating your imported keys…" vs
/// "Migrating your shielded data…").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStep {
    /// Sniffing `data.db` for legacy rows.
    Detecting,
    /// Importing DET-owned rows the wallet drain never touched: scheduled
    /// DPNS votes and top-up history.
    AppData,
    /// Copying `single_key_wallet` rows into the upstream `SecretStore`.
    SingleKey,
    /// Mirroring legacy shielded rows + cursor into the per-wallet sidecar.
    Shielded,
    /// Copying HD wallet seed envelopes from the legacy `wallet` table
    /// into the upstream `SecretStore` vault. The full envelope
    /// (ciphertext + salt + nonce + flags) travels verbatim — the
    /// migrator never decrypts.
    WalletSeeds,
    /// Copying legacy `wallet` rows (alias / `is_main` / `core_wallet_name`
    /// / master xpub) into the DET wallet-metadata sidecar in
    /// `det-app.sqlite`.
    WalletMeta,
    /// Writing the completion sentinel and cleaning up.
    Finalize,
}

/// High-level state of the legacy migration.
///
/// The `Failed` variant carries an `Arc<MigrationError>` rather than a
/// stringified copy: the UI banner formats the error via `Display` at
/// render time, which keeps the typed error chain reachable for the
/// log path and the details panel without a lossy `to_string()`
/// round-trip. `MigrationError` is not `Clone`, but `Arc` cheaply
/// satisfies the `Clone` bound `MigrationStatus` needs to publish a
/// state across the per-frame `load_full()` boundary.
#[derive(Debug, Clone)]
pub enum MigrationState {
    /// No migration in progress and none required.
    Idle,
    /// Migration is currently executing the given step.
    Running { step: MigrationStep },
    /// Migration completed successfully (or no legacy data was present).
    Success,
    /// Migration failed. The wrapped error is rendered for the user via
    /// its `Display` impl at banner-render time; the typed chain is
    /// preserved for the details panel and logs.
    Failed {
        error: Arc<crate::backend_task::migration::MigrationError>,
    },
}

impl PartialEq for MigrationState {
    /// Compare states for the per-frame reconciler. Stateless variants
    /// (`Idle`, `Success`, `Running { step }`) compare structurally;
    /// `Failed` compares the wrapped error by `Arc::ptr_eq`. The
    /// reconciler treats every newly-published `Failed` state as a
    /// transition, so a retry that fails with a fresh error correctly
    /// refreshes the banner even when the user-visible text is
    /// identical.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (MigrationState::Idle, MigrationState::Idle) => true,
            (MigrationState::Success, MigrationState::Success) => true,
            (MigrationState::Running { step: a }, MigrationState::Running { step: b }) => a == b,
            (MigrationState::Failed { error: a }, MigrationState::Failed { error: b }) => {
                Arc::ptr_eq(a, b)
            }
            _ => false,
        }
    }
}

impl Eq for MigrationState {}

impl MigrationState {
    /// Returns `true` while the migration task is mid-flight.
    pub fn is_running(&self) -> bool {
        matches!(self, MigrationState::Running { .. })
    }
}

/// Atomic, cheaply-readable migration status.
///
/// Cloned by `AppContext` and read once per UI frame. Writers (the
/// migration task) call [`set_state`](Self::set_state) to publish a
/// transition; readers call [`state`](Self::state) and dereference the
/// returned `Arc`.
#[derive(Debug)]
pub struct MigrationStatus {
    state: ArcSwap<MigrationState>,
}

impl MigrationStatus {
    /// Construct an idle status. Used by `AppContext` and by tests.
    pub fn new_idle() -> Self {
        Self {
            state: ArcSwap::from_pointee(MigrationState::Idle),
        }
    }

    /// Load the current state. Cheap — no lock, just a single atomic load.
    pub fn state(&self) -> Arc<MigrationState> {
        self.state.load_full()
    }

    /// Publish a new state. Idempotent — repeated identical writes are
    /// allowed and cheap.
    pub fn set_state(&self, new_state: MigrationState) {
        self.state.store(Arc::new(new_state));
    }
}

impl Default for MigrationStatus {
    fn default() -> Self {
        Self::new_idle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TC-MIG-001 (supporting) — MigrationStatus state transitions on the
    /// success path. Drives Idle → Running{Detecting} → Running{SingleKey}
    /// → Running{Shielded} → Running{Finalize} → Success. This is the
    /// state machine the banner reads via `migration_running_text`; the
    /// kittest in `tests/kittest/migration_banner.rs` verifies the banner
    /// surface for each step.
    #[test]
    fn state_transitions_success_path() {
        let status = MigrationStatus::new_idle();
        assert_eq!(*status.state(), MigrationState::Idle);
        assert!(!status.state().is_running());

        status.set_state(MigrationState::Running {
            step: MigrationStep::Detecting,
        });
        assert!(status.state().is_running());
        assert_eq!(
            *status.state(),
            MigrationState::Running {
                step: MigrationStep::Detecting
            }
        );

        for step in [
            MigrationStep::SingleKey,
            MigrationStep::Shielded,
            MigrationStep::WalletSeeds,
            MigrationStep::WalletMeta,
            MigrationStep::Finalize,
        ] {
            status.set_state(MigrationState::Running { step });
            assert_eq!(*status.state(), MigrationState::Running { step });
            assert!(status.state().is_running());
        }

        status.set_state(MigrationState::Success);
        assert_eq!(*status.state(), MigrationState::Success);
        assert!(!status.state().is_running());
    }

    /// Failure transitions carry a typed error and clear the running
    /// flag. The wrapped `MigrationError` reaches the UI banner via
    /// `Display` at render time — `MigrationState` itself never owns a
    /// stringified copy.
    #[test]
    fn failed_state_clears_running_and_carries_typed_error() {
        use crate::backend_task::migration::MigrationError;

        let status = MigrationStatus::new_idle();
        status.set_state(MigrationState::Failed {
            error: Arc::new(MigrationError::WalletBackendUnavailable),
        });
        assert!(!status.state().is_running());
        assert!(matches!(*status.state(), MigrationState::Failed { .. }));
    }
}
