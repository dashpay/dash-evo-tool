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

use crate::model::wallet::WalletSeedHash;

/// Which legacy domain the migration is currently working on.
///
/// Drives banner text granularity ("Migrating your imported keys…" vs
/// "Migrating your shielded data…").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStep {
    /// Sniffing `data.db` for legacy rows.
    Detecting,
    /// Copying `single_key_wallet` rows into the upstream `SecretStore`.
    SingleKey,
    /// Mirroring legacy shielded rows + cursor into the per-wallet sidecar.
    Shielded,
    /// Copying HD wallet seed envelopes from the legacy `wallet` table
    /// into the upstream `SecretStore` vault. Password-protected rows
    /// are deferred — see
    /// [`MigrationStatus::pending_seed_passwords`].
    WalletSeeds,
    /// Copying legacy `wallet` rows (alias / `is_main` / `core_wallet_name`
    /// / master xpub) into the DET wallet-metadata sidecar in
    /// `det-app.sqlite`.
    WalletMeta,
    /// Writing the completion sentinel and cleaning up.
    Finalize,
}

impl MigrationStep {
    /// Short user-facing label for the current step. Each variant is a
    /// single complete sentence — safe for i18n extraction.
    pub fn label(self) -> &'static str {
        match self {
            MigrationStep::Detecting => "Checking your existing data.",
            MigrationStep::SingleKey => "Migrating your imported keys.",
            MigrationStep::Shielded => "Migrating your shielded data.",
            MigrationStep::WalletSeeds => "Moving your wallets into the new vault.",
            MigrationStep::WalletMeta => "Updating wallet names.",
            MigrationStep::Finalize => "Finishing up.",
        }
    }
}

/// High-level state of the legacy migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationState {
    /// No migration in progress and none required.
    Idle,
    /// Migration is currently executing the given step.
    Running { step: MigrationStep },
    /// Migration completed successfully (or no legacy data was present).
    Success,
    /// Migration failed. The displayed reason is kept short and actionable.
    Failed { reason: String },
}

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
///
/// [`pending_seed_passwords`](Self::pending_seed_passwords) is an
/// orthogonal channel: the `WalletSeeds` migration step records seed
/// hashes whose seed envelope is password-protected and could not be
/// migrated silently. T-W-00.6 will surface a per-wallet password
/// prompt to drain this set; until then the migration still progresses
/// to `Success` so password-free wallets are not held hostage.
#[derive(Debug)]
pub struct MigrationStatus {
    state: ArcSwap<MigrationState>,
    pending_seed_passwords: ArcSwap<Vec<WalletSeedHash>>,
}

impl MigrationStatus {
    /// Construct an idle status. Used by `AppContext` and by tests.
    pub fn new_idle() -> Self {
        Self {
            state: ArcSwap::from_pointee(MigrationState::Idle),
            pending_seed_passwords: ArcSwap::from_pointee(Vec::new()),
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

    /// Snapshot of the seed hashes whose seed envelope still needs the
    /// user's password before it can be moved into the vault. Empty
    /// when there is no pending work; T-W-00.6 will own the UI that
    /// drains this set.
    pub fn pending_seed_passwords(&self) -> Arc<Vec<WalletSeedHash>> {
        self.pending_seed_passwords.load_full()
    }

    /// Publish the seed-hash set the password-collection dialog needs
    /// to drive. An empty vector clears the pending work; otherwise
    /// last-write-wins, mirroring [`Self::set_state`].
    pub fn set_pending_seed_passwords(&self, hashes: Vec<WalletSeedHash>) {
        self.pending_seed_passwords.store(Arc::new(hashes));
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

    /// TC-MIG-011 — MigrationStatus state transitions on the success path.
    /// Drives Idle → Running{Detecting} → Running{SingleKey} →
    /// Running{Shielded} → Running{Finalize} → Success.
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

    /// Failure transitions carry a short reason and clear the running flag.
    #[test]
    fn failed_state_is_terminal() {
        let status = MigrationStatus::new_idle();
        status.set_state(MigrationState::Failed {
            reason: "test reason".into(),
        });
        assert!(!status.state().is_running());
        assert!(matches!(*status.state(), MigrationState::Failed { .. }));
    }

    /// Each migration step exposes a non-empty, sentence-shaped label.
    #[test]
    fn step_labels_are_non_empty_sentences() {
        for step in [
            MigrationStep::Detecting,
            MigrationStep::SingleKey,
            MigrationStep::Shielded,
            MigrationStep::WalletSeeds,
            MigrationStep::WalletMeta,
            MigrationStep::Finalize,
        ] {
            let label = step.label();
            assert!(!label.is_empty(), "step {step:?} has empty label");
            assert!(
                label.ends_with('.'),
                "step {step:?} label `{label}` is not a complete sentence"
            );
        }
    }

    /// TC-W-002 storage half — pending seed passwords default to empty
    /// and round-trip through the dedicated channel without touching
    /// the main `state` field.
    #[test]
    fn pending_seed_passwords_round_trip() {
        let status = MigrationStatus::new_idle();
        assert!(status.pending_seed_passwords().is_empty());
        let a: WalletSeedHash = [0xAA; 32];
        let b: WalletSeedHash = [0xBB; 32];
        status.set_pending_seed_passwords(vec![a, b]);
        let snap = status.pending_seed_passwords();
        assert_eq!(snap.as_slice(), &[a, b]);
        assert_eq!(*status.state(), MigrationState::Idle);
        status.set_pending_seed_passwords(Vec::new());
        assert!(status.pending_seed_passwords().is_empty());
    }
}
