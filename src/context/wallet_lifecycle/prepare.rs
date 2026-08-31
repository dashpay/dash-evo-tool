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

impl AppContext {
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
        let _gate = self.prepare_gate.lock().await;
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
            return Err(error);
        }

        crate::backend_task::migration::finish_unwire::run_gated(self).await?;
        Ok(())
    }
}
