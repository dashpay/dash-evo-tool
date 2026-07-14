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
use tokio::sync::Notify;

use crate::model::wallet::WalletSeedHash;

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
    /// Importing legacy `identity` rows — and the owner / voting / payout
    /// keys they carry — into the modern identity store. Runs after the
    /// wallet drain so each identity's key lands against a wallet that
    /// already exists.
    Identities,
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
    /// Migration copied and hydrated protected wallets but must collect their
    /// passwords before registration and completion can continue.
    AwaitingWalletPasswords { wallets: Vec<WalletSeedHash> },
    /// Migration completed successfully (or no legacy data was present).
    Success,
    /// The wallet drain completed — seeds, metadata and registration all
    /// landed, so funds are reachable — but `count` legacy scheduled votes
    /// could not be decoded and did not come across. Terminal and non-fatal:
    /// the legacy rows are never deleted, and a retry cannot decode a corrupt
    /// row, so the user is told once rather than offered a futile retry.
    SucceededWithUnreadableVotes { count: u32 },
    /// The wallet drain completed, but `count` legacy identities could not be
    /// decoded and did not come across — so the keys they held (a masternode's
    /// owner / voting key, say) are not loaded. Terminal and non-fatal: the
    /// legacy rows are never deleted, so they remain recoverable. Re-published
    /// from a durable record on every launch until the user acknowledges it, and
    /// separate from [`Self::SucceededWithUnreadableVotes`] because the remedy
    /// differs — re-import a key, not re-schedule a vote.
    SucceededWithUnreadableIdentities { count: u32 },
    /// The wallet drain completed, but both DET-owned passes left undecodable
    /// rows behind on the same launch: `identities` legacy identities and
    /// `votes` legacy scheduled votes did not come across. Terminal and
    /// non-fatal — like the two single-signal variants it combines, and for the
    /// same reason: a corrupt row decodes no better on a retry.
    ///
    /// It exists because neither signal may eat the other. Both warnings are
    /// durable and re-published on every launch until acknowledged, so with only
    /// the single-signal variants available the identity half would permanently
    /// outrank the vote half and cost the user a vote whose deadline is still
    /// live. One banner names both remedies instead: load the identities again,
    /// re-schedule the votes.
    ///
    /// Because that single banner names both problems, its single acknowledge
    /// action retires *both* durable records (see
    /// [`acknowledge_unreadable_votes`](crate::backend_task::migration::finish_unwire::acknowledge_unreadable_votes)
    /// and
    /// [`acknowledge_unreadable_identities`](crate::backend_task::migration::finish_unwire::acknowledge_unreadable_identities)).
    SucceededWithUnreadableIdentitiesAndVotes { identities: u32, votes: u32 },
    /// Both DET-owned passes are damaged on the same launch: the wallet drain
    /// landed, but `count` legacy identities could not be decoded AND the
    /// app-data import hit a hard failure. Rendered as a single retryable error
    /// banner naming both problems, so neither masks the other — the plain
    /// [`Self::SucceededWithUnreadableIdentities`] would have swallowed the
    /// app-data failure and left the user no retry. The app-data sentinel stays
    /// unwritten, so a retry (or the next launch) re-attempts that import; the
    /// undecodable identity rows stay in the previous version's storage, named by
    /// a durable warning. Terminal until then.
    ///
    /// A *hard* app-data failure is the only producer. An app-data pass that
    /// completed and merely left a later notice-record read failing is not one:
    /// its sentinel is written, so this state's "we did not finish updating the
    /// rest of your data — retry" copy would be false and its retry a no-op. That
    /// path publishes [`Self::SucceededWithUnreadableIdentities`] instead.
    FailedWithUnreadableIdentities {
        count: u32,
        error: Arc<crate::backend_task::migration::MigrationError>,
    },
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
            (
                MigrationState::SucceededWithUnreadableVotes { count: a },
                MigrationState::SucceededWithUnreadableVotes { count: b },
            ) => a == b,
            (
                MigrationState::SucceededWithUnreadableIdentities { count: a },
                MigrationState::SucceededWithUnreadableIdentities { count: b },
            ) => a == b,
            (
                MigrationState::SucceededWithUnreadableIdentitiesAndVotes {
                    identities: ia,
                    votes: va,
                },
                MigrationState::SucceededWithUnreadableIdentitiesAndVotes {
                    identities: ib,
                    votes: vb,
                },
            ) => ia == ib && va == vb,
            (MigrationState::Running { step: a }, MigrationState::Running { step: b }) => a == b,
            (
                MigrationState::AwaitingWalletPasswords { wallets: a },
                MigrationState::AwaitingWalletPasswords { wallets: b },
            ) => a == b,
            (
                MigrationState::FailedWithUnreadableIdentities {
                    count: a,
                    error: ea,
                },
                MigrationState::FailedWithUnreadableIdentities {
                    count: b,
                    error: eb,
                },
            ) => a == b && Arc::ptr_eq(ea, eb),
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
        matches!(
            self,
            MigrationState::Running { .. } | MigrationState::AwaitingWalletPasswords { .. }
        )
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
    wallet_password_submitted: Notify,
}

impl MigrationStatus {
    /// Construct an idle status. Used by `AppContext` and by tests.
    pub fn new_idle() -> Self {
        Self {
            state: ArcSwap::from_pointee(MigrationState::Idle),
            wallet_password_submitted: Notify::new(),
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

    /// Wait until the UI submits a migrated wallet's password.
    pub async fn wait_for_wallet_password(&self) {
        self.wallet_password_submitted.notified().await;
    }

    /// Resume the migration task after a migrated wallet was unlocked.
    pub fn notify_wallet_password_submitted(&self) {
        self.wallet_password_submitted.notify_one();
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
            MigrationStep::Identities,
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

    #[test]
    fn awaiting_wallet_passwords_remains_running_and_preserves_wallet_order() {
        let status = MigrationStatus::new_idle();
        let wallets = vec![[0x11; 32], [0x22; 32]];

        status.set_state(MigrationState::AwaitingWalletPasswords {
            wallets: wallets.clone(),
        });

        assert!(status.state().is_running());
        assert_eq!(
            *status.state(),
            MigrationState::AwaitingWalletPasswords { wallets },
        );
    }

    #[tokio::test]
    async fn wallet_password_notification_wakes_the_waiting_migration() {
        let status = MigrationStatus::new_idle();

        status.notify_wallet_password_submitted();
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            status.wait_for_wallet_password(),
        )
        .await
        .expect("a submitted wallet password must wake the migration task");
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

    /// The combined failure state compares by `count` AND error identity, like
    /// `Failed`: two combined states with the same count but distinct error
    /// `Arc`s are unequal, so the per-frame reconciler treats a fresh failure as
    /// a transition and re-renders instead of suppressing it as a duplicate.
    #[test]
    fn combined_failure_state_compares_count_and_error_identity() {
        use crate::backend_task::migration::MigrationError;

        let shared = Arc::new(MigrationError::WalletBackendUnavailable);
        let a = MigrationState::FailedWithUnreadableIdentities {
            count: 2,
            error: Arc::clone(&shared),
        };
        let same = MigrationState::FailedWithUnreadableIdentities {
            count: 2,
            error: Arc::clone(&shared),
        };
        assert_eq!(a, same, "same count and same error Arc compare equal");

        let different_error = MigrationState::FailedWithUnreadableIdentities {
            count: 2,
            error: Arc::new(MigrationError::WalletBackendUnavailable),
        };
        assert_ne!(
            a, different_error,
            "a fresh error Arc is a new transition even at the same count",
        );

        let different_count = MigrationState::FailedWithUnreadableIdentities {
            count: 3,
            error: Arc::clone(&shared),
        };
        assert_ne!(a, different_count, "a changed count is a transition");
    }
}
