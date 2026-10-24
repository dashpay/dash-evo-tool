//! "Max" transfers from the Core wallet to the shielded balance.
//!
//! Max moves the whole Core balance minus fees through the upstream wallet
//! drain, so the amount is known only once the funding transaction is built
//! and no estimate can leave the transfer short of its fees. The drain is
//! identified by an operation id DET stores before starting it, which keeps
//! an interrupted transfer visible and finishable after a restart.

use std::sync::Arc;

use platform_wallet::wallet::shielded::{WalletDrainPreview, WalletDrainPreviewStatus};

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::shield_max_minimum_lock_duffs;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded_max_transfer::{
    PendingShieldedMaxTransfer, ShieldedMaxTransferState,
};
use crate::wallet_backend::{MaxTransferAttempt, MaxTransferClaim, WalletBackend};

impl AppContext {
    /// Start a Max transfer for `seed_hash`, unless one is still unsettled.
    pub(super) async fn shield_max_from_core(
        self: &Arc<Self>,
        backend: &WalletBackend,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let records = backend.shielded_max_transfer_records();
        if records.pending(self.network(), &seed_hash)?.is_some() {
            return Err(TaskError::ShieldedMaxTransferAlreadyPending);
        }
        let preview = backend.shielded_max_transfer_preview(&seed_hash).await?;
        let minimum_lock_duffs = max_transfer_minimum_lock(&preview)?;

        let record = PendingShieldedMaxTransfer {
            operation_id: rand::random(),
        };
        // Stored before the transfer starts: from here on a crash, a restart
        // or a lost reply still leaves the id to look the transfer up by.
        if let MaxTransferClaim::AlreadyPending(_) =
            records.claim(self.network(), &seed_hash, record)?
        {
            return Err(TaskError::ShieldedMaxTransferAlreadyPending);
        }
        self.run_max_transfer(backend, seed_hash, record, minimum_lock_duffs)
            .await
    }

    /// Finish the wallet's unsettled Max transfer with its stored id.
    pub(super) async fn finish_shield_max_from_core(
        self: &Arc<Self>,
        backend: &WalletBackend,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let records = backend.shielded_max_transfer_records();
        let Some(record) = records.pending(self.network(), &seed_hash)? else {
            return Ok(BackendTaskSuccessResult::ShieldedMaxTransferStatus {
                seed_hash,
                state: None,
            });
        };
        // The floor only matters if no lock was built yet; a resumed drain
        // keeps the lock it already has, so the preview's status (which no
        // longer counts the locked coins) is irrelevant and only its pool fee
        // is used. Without a fee there is no safe floor, so nothing is tried.
        let pool_fee_credits = backend
            .shielded_max_transfer_preview(&seed_hash)
            .await
            .map_err(|error| TaskError::ShieldedMaxTransferUnfinished {
                source: Box::new(error),
            })?
            .pool_fee_credits;
        let minimum_lock_duffs = shield_max_minimum_lock_duffs(pool_fee_credits);
        self.run_max_transfer(backend, seed_hash, record, minimum_lock_duffs)
            .await
    }

    /// Where the wallet's Max transfer stands, releasing the stored id once
    /// it has an outcome. `state` is `None` when there is no transfer.
    pub(super) async fn shield_max_from_core_status(
        self: &Arc<Self>,
        backend: &WalletBackend,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let records = backend.shielded_max_transfer_records();
        let Some(record) = records.pending(self.network(), &seed_hash)? else {
            return Ok(BackendTaskSuccessResult::ShieldedMaxTransferStatus {
                seed_hash,
                state: None,
            });
        };
        let state = backend
            .shielded_max_transfer_state(&seed_hash, record.operation_id)
            .await?;
        if state.is_settled() {
            self.release_max_transfer(backend, &seed_hash, record);
        }
        if state == ShieldedMaxTransferState::Finished {
            self.refresh_shielded_balance_snapshot(&seed_hash).await;
        }
        Ok(BackendTaskSuccessResult::ShieldedMaxTransferStatus {
            seed_hash,
            state: Some(state),
        })
    }

    async fn run_max_transfer(
        self: &Arc<Self>,
        backend: &WalletBackend,
        seed_hash: WalletSeedHash,
        record: PendingShieldedMaxTransfer,
        minimum_lock_duffs: u64,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let MaxTransferAttempt { state, result } = backend
            .shield_max_from_core(&seed_hash, record.operation_id, minimum_lock_duffs)
            .await;
        let state = match state {
            Some(state) => state,
            // Only a durable read can show that nothing was sent; when even
            // that fails, the transfer is treated as still pending.
            None => backend
                .shielded_max_transfer_state(&seed_hash, record.operation_id)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(?error, "Max transfer status unreadable; keeping its id");
                    ShieldedMaxTransferState::Pending
                }),
        };
        if state.is_settled() {
            self.release_max_transfer(backend, &seed_hash, record);
        }
        if state == ShieldedMaxTransferState::Finished {
            self.refresh_shielded_balance_snapshot(&seed_hash).await;
        }
        settle_max_transfer(seed_hash, state, result)
    }

    /// Drop the stored id of a settled transfer. A failed removal only leaves
    /// the id behind, and the next status read settles it again.
    fn release_max_transfer(
        &self,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
        record: PendingShieldedMaxTransfer,
    ) {
        if let Err(error) = backend.shielded_max_transfer_records().release(
            self.network(),
            seed_hash,
            record.operation_id,
        ) {
            tracing::warn!(?error, "Could not remove a settled Max transfer record");
        }
    }
}

/// The floor for the funding lock of a new Max transfer, or why the wallet
/// cannot make one.
fn max_transfer_minimum_lock(preview: &WalletDrainPreview) -> Result<u64, TaskError> {
    match preview.status {
        WalletDrainPreviewStatus::Ready => {
            Ok(shield_max_minimum_lock_duffs(preview.pool_fee_credits))
        }
        WalletDrainPreviewStatus::InsufficientFunds => Err(TaskError::ShieldedMaxInsufficientFunds),
        WalletDrainPreviewStatus::InputLimitMayBeExceeded => {
            Err(TaskError::ShieldedMaxTooManyCoins)
        }
    }
}

/// What the user learns about a Max transfer that ended in `state`.
///
/// INTENTIONAL(drain-conflict-clear): a conflicted transfer releases its id
/// like a finished one. Upstream lets a new transfer start, and the funding
/// lock can come back only through authenticated recovery of its original
/// outpoint, which the generic asset-lock recovery owns, not this flow.
fn settle_max_transfer(
    seed_hash: WalletSeedHash,
    state: ShieldedMaxTransferState,
    result: Result<(), TaskError>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    match (state, result) {
        // The amount is the builder's, after fees; the app has no figure it
        // can vouch for.
        (ShieldedMaxTransferState::Finished, _) => {
            Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
                seed_hash,
                amount: None,
            })
        }
        (ShieldedMaxTransferState::Pending, Err(error)) => {
            Err(TaskError::ShieldedMaxTransferUnfinished {
                source: Box::new(error),
            })
        }
        (ShieldedMaxTransferState::NotSent | ShieldedMaxTransferState::Conflicted, Err(error)) => {
            Err(error)
        }
        // A successful return is settled as Finished before it gets here.
        (_, Ok(())) => Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash,
            amount: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WalletSeedHash = [0x2C; 32];

    fn preview(status: WalletDrainPreviewStatus) -> WalletDrainPreview {
        WalletDrainPreview {
            status,
            candidate_input_count: 2,
            candidate_balance_duffs: 300_000,
            pool_fee_credits: 12_345_678,
            estimated_l1_fee_duffs: Some(400),
            estimated_recipient_credits: Some(1),
        }
    }

    #[test]
    fn a_ready_wallet_gets_a_lock_floor_above_the_pool_fee() {
        assert_eq!(
            max_transfer_minimum_lock(&preview(WalletDrainPreviewStatus::Ready)).unwrap(),
            12_346
        );
    }

    #[test]
    fn a_wallet_that_cannot_drain_gets_a_typed_refusal() {
        assert!(matches!(
            max_transfer_minimum_lock(&preview(WalletDrainPreviewStatus::InsufficientFunds)),
            Err(TaskError::ShieldedMaxInsufficientFunds)
        ));
        assert!(matches!(
            max_transfer_minimum_lock(&preview(WalletDrainPreviewStatus::InputLimitMayBeExceeded)),
            Err(TaskError::ShieldedMaxTooManyCoins)
        ));
    }

    #[test]
    fn a_finished_transfer_succeeds_even_when_the_resume_reports_it_consumed() {
        for result in [Ok(()), Err(TaskError::WalletStateInconsistent)] {
            assert!(matches!(
                settle_max_transfer(SEED, ShieldedMaxTransferState::Finished, result),
                Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
                    seed_hash: SEED,
                    amount: None,
                })
            ));
        }
    }

    #[test]
    fn a_pending_transfer_tells_the_user_to_finish_it() {
        let settled = settle_max_transfer(
            SEED,
            ShieldedMaxTransferState::Pending,
            Err(TaskError::ShieldedNotBound),
        );
        match settled {
            Err(TaskError::ShieldedMaxTransferUnfinished { source }) => {
                assert!(matches!(*source, TaskError::ShieldedNotBound));
            }
            other => panic!("expected ShieldedMaxTransferUnfinished, got {other:?}"),
        }
    }

    #[test]
    fn an_unsent_or_conflicted_transfer_reports_its_own_error() {
        for state in [
            ShieldedMaxTransferState::NotSent,
            ShieldedMaxTransferState::Conflicted,
        ] {
            assert!(matches!(
                settle_max_transfer(SEED, state, Err(TaskError::ShieldedNotBound)),
                Err(TaskError::ShieldedNotBound)
            ));
        }
    }
}
