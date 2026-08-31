//! The storage-preparation gate: one ordering for everything a network's
//! storage needs before it can be used.
//!
//! Backend wiring, the upstream schema ladder, wallet hydration and the legacy
//! `data.db` drain used to race each other — a fire-and-forget wiring task, a
//! frame-loop-dispatched drain, and chain sync, sequenced by nothing. Each was
//! individually guarded. [`AppContext::prepare_storage`] replaces those guards
//! with a data dependency: every step is an `await` inside one function, under
//! one lock, so nothing downstream can observe a half-prepared network.
//!
//! Chain sync is deliberately *not* part of the sequence — no migration in the
//! tree waits on it. It starts as a continuation of a completed preparation in
//! [`AppContext::ensure_wallet_backend_and_start_spv`].

use super::*;
use crate::context::migration_status::{MigrationState, MigrationStep};

/// Proof that the storage-preparation gate is held.
///
/// A `&PrepareGateGuard` parameter is a fact the compiler checks: only
/// [`AppContext::lock_prepare_gate`] and [`AppContext::try_lock_prepare_gate`]
/// can produce one. A bare `&MutexGuard<'_, ()>` would not be — every
/// `Mutex<()>` in the crate hands out the same type, and there is another.
pub struct PrepareGateGuard<'a> {
    /// Held for its `Drop`; the gate is released when this value is.
    _guard: tokio::sync::MutexGuard<'a, ()>,
}

impl AppContext {
    /// Take the storage-preparation gate, waiting for whoever holds it.
    pub(crate) async fn lock_prepare_gate(&self) -> PrepareGateGuard<'_> {
        PrepareGateGuard {
            _guard: self.prepare_gate.lock().await,
        }
    }

    /// Take the storage-preparation gate if it is free right now.
    ///
    /// # Errors
    ///
    /// [`tokio::sync::TryLockError`] when a preparation, migration or another
    /// gate-guarded operation holds it.
    pub(crate) fn try_lock_prepare_gate(
        &self,
    ) -> Result<PrepareGateGuard<'_>, tokio::sync::TryLockError> {
        self.prepare_gate
            .try_lock()
            .map(|guard| PrepareGateGuard { _guard: guard })
    }

    /// Prepare this network's storage: wire the wallet backend (which runs the
    /// upstream schema ladder and rehydrates the wallets it finds), then drain
    /// the previous version's `data.db`.
    ///
    /// Idempotent and safe to call on every entry path — a second call on a
    /// prepared network fast-paths through wiring and short-circuits the drain
    /// on its completion sentinel. Concurrent callers serialize on
    /// [`AppContext::prepare_gate`]; the loser observes the winner's finished
    /// work rather than repeating it.
    ///
    /// Progress is published on [`MigrationStatus`](crate::context::migration_status::MigrationStatus)
    /// so the one existing overlay/banner surface covers the whole sequence.
    /// Only a launch that finds the status still [`MigrationState::Idle`]
    /// announces [`MigrationStep::Wiring`]; a later call on an already prepared
    /// network stays silent rather than re-raising a progress surface over a
    /// working app.
    ///
    /// The drain's best-effort DAPI node refresh is detached and never awaited.
    /// It queues for the gate behind this call and holds it while it runs, so a
    /// gate-guarded operation still waits it out — unchanged from before.
    ///
    /// # Errors
    ///
    /// [`TaskError`] from wiring (storage open, an incompatible on-disk layout)
    /// or from the drain — see [`finish_unwire::run`](crate::backend_task::migration::finish_unwire::run)
    /// for that half. A wiring failure resets the status to
    /// [`MigrationState::Idle`] so the readiness check is re-attempted rather
    /// than left stuck mid-step; the caller owns the user-facing surface.
    pub async fn prepare_storage(
        self: &Arc<Self>,
        task_result_sender: crate::utils::egui_mpsc::SenderAsync<crate::app::TaskResult>,
    ) -> Result<(), TaskError> {
        let gate = self.lock_prepare_gate().await;
        let status = self.migration_status();
        let announce = matches!(*status.state(), MigrationState::Idle);
        if announce {
            status.set_state(MigrationState::Running {
                step: MigrationStep::Wiring,
            });
        }

        if let Err(error) = self.ensure_wallet_backend(task_result_sender).await {
            if announce {
                status.set_state(MigrationState::Idle);
            }
            // The sweep is deliberately NOT attempted here: it reads through the
            // backend's k/v store, which is the thing that just failed to open,
            // so it could only bail on its own "k/v store not ready" branch.
            return Err(error);
        }

        let drain = crate::backend_task::migration::finish_unwire::run_gated(self, &gate).await;

        // Run the sweep on the drain's failure path too: a deterministic drain
        // failure would otherwise postpone the only recovery path for orphaned
        // vault keys forever. A partially applied import cannot fake the roster
        // absence the sweep's irreversible delete acts on — it only ever adds
        // roster entries, and it declines every identity a manifest belongs to,
        // because removal records the deletion and sets the Global unload
        // marker that `purge_identity_scope` (Identity scope only) leaves
        // standing.
        self.run_pending_vault_cleanup_sweep(&gate);

        drain?;
        Ok(())
    }

    /// Test seam: take the storage-preparation gate and hold it, pinning any
    /// [`Self::prepare_storage`] a test dispatches at its first line. Lets a
    /// test observe what a dispatch site left behind before the run it spawned
    /// can overwrite it.
    #[cfg(feature = "testing")]
    pub async fn test_hold_prepare_gate(&self) -> PrepareGateGuard<'_> {
        self.lock_prepare_gate().await
    }

    /// Drive the pending vault-cleanup sweep once, under the held gate.
    ///
    /// The sweep is the only recovery path for vault keys an interrupted
    /// identity removal orphaned, and it is silent when it does not run. It
    /// normally rides `bootstrap_loaded_wallets` inside
    /// [`Self::ensure_wallet_backend`], where the gate guarantees it skips: its
    /// `try_lock` loses to the gate this call chain holds, and its in-progress
    /// check sees the [`MigrationStep::Wiring`] this sequence published. Both
    /// would fail identically on the next boot, and the one after — so it is
    /// driven explicitly here rather than left to a call the gate disabled.
    ///
    /// Run under the gate already held rather than after releasing it: the
    /// drain's detached DAPI refresh is queued on this same gate and would very
    /// likely win the handoff, so a post-release `try_lock` would fail and
    /// reproduce the bug — while waiting the refresh out would hold the gate up
    /// across a network call. The sweep's guard exists to exclude a *concurrent*
    /// migration; holding the gate satisfies that strictly harder than racing
    /// for it. No other lock is held across it, and its own per-identity record
    /// guards are taken in the documented `prepare_gate` → record order.
    fn run_pending_vault_cleanup_sweep(&self, gate: &PrepareGateGuard<'_>) {
        // By here the drain has published a terminal state either way, so an
        // in-progress status means someone published a step this function does
        // not know about — the sweep would skip silently on it, which is the
        // exact failure being fixed. Say so instead of letting it pass.
        if self.migration_status().state().is_in_progress() {
            tracing::warn!(
                state = ?self.migration_status().state(),
                "Storage preparation finished on a non-terminal status; the pending \
                 vault-cleanup sweep will skip and orphaned identity keys stay unrecovered"
            );
        }
        self.resume_pending_vault_cleanups_gated(gate);
    }
}
