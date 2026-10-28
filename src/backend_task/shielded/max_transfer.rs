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
use crate::model::fee_estimation::shield_max_fee_credits;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded_max_transfer::{
    PendingShieldedMaxTransfer, ShieldedMaxTransferQuote, ShieldedMaxTransferState,
};
use crate::wallet_backend::{
    MaxTransferAttempt, MaxTransferClaim, MaxTransferOperation, WalletBackend,
};

impl AppContext {
    /// What a Max transfer would move right now, for the user to confirm.
    /// Reads the wallet only; reserves, signs and broadcasts nothing.
    pub(super) async fn quote_shield_max_from_core(
        &self,
        backend: &WalletBackend,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let preview = backend.shielded_max_transfer_preview(&seed_hash).await?;
        Ok(BackendTaskSuccessResult::ShieldedMaxTransferQuote {
            seed_hash,
            quote: max_transfer_quote(&preview)?,
        })
    }

    /// Start a Max transfer for `seed_hash`, unless one is still unsettled.
    pub(super) async fn shield_max_from_core(
        self: &Arc<Self>,
        backend: &WalletBackend,
        seed_hash: WalletSeedHash,
        confirmed_balance_duffs: u64,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let records = backend.shielded_max_transfer_records();
        if records.pending(self.network(), &seed_hash)?.is_some() {
            return Err(TaskError::ShieldedMaxTransferAlreadyPending);
        }
        // Refuses a wallet that cannot pay the fees, or whose balance moved
        // since the user confirmed, before anything is stored.
        let preview = backend.shielded_max_transfer_preview(&seed_hash).await?;
        check_confirmed_balance(&preview, confirmed_balance_duffs)?;
        max_transfer_quote(&preview)?;
        let (record, transfer) = self.begin_max_transfer(backend, &seed_hash).await?;
        self.run_max_transfer(backend, seed_hash, record, transfer)
            .await
    }

    /// Register a new drain with the wallet, then store its id.
    ///
    /// The order matters: from the moment the id is stored, a status read
    /// must see the drain as running, or it would report the transfer as
    /// never started and drop the id of a drain about to move the balance.
    /// The registration is held until the drain returns, then dropped before
    /// its outcome is read, so a drain that never built a lock reads as not
    /// sent.
    async fn begin_max_transfer(
        &self,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
    ) -> Result<(PendingShieldedMaxTransfer, MaxTransferOperation), TaskError> {
        let record = PendingShieldedMaxTransfer {
            operation_id: rand::random(),
        };
        self.begin_max_transfer_as(backend, seed_hash, record).await
    }

    /// [`begin_max_transfer`](Self::begin_max_transfer) for the given id.
    async fn begin_max_transfer_as(
        &self,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
        record: PendingShieldedMaxTransfer,
    ) -> Result<(PendingShieldedMaxTransfer, MaxTransferOperation), TaskError> {
        let transfer = backend
            .register_max_transfer(seed_hash, record.operation_id)
            .await?;
        // Stored before the transfer starts: from here on a crash, a restart
        // or a lost reply still leaves the id to look the transfer up by.
        match backend
            .shielded_max_transfer_records()
            .claim(self.network(), seed_hash, record)?
        {
            MaxTransferClaim::Claimed => Ok((record, transfer)),
            MaxTransferClaim::AlreadyPending(_) => {
                Err(TaskError::ShieldedMaxTransferAlreadyPending)
            }
        }
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
        let transfer = backend
            .register_max_transfer(&seed_hash, record.operation_id)
            .await?;
        self.run_max_transfer(backend, seed_hash, record, transfer)
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
        transfer: MaxTransferOperation,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let MaxTransferAttempt { state, result } =
            backend.shield_max_from_core(&seed_hash, transfer).await;
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

/// Refuse a transfer whose spendable balance is no longer the one the user
/// confirmed: the drain spends whatever is there when it is built, so the
/// figures the user saw would be wrong.
fn check_confirmed_balance(
    preview: &WalletDrainPreview,
    confirmed_balance_duffs: u64,
) -> Result<(), TaskError> {
    if preview.candidate_balance_duffs == confirmed_balance_duffs {
        Ok(())
    } else {
        Err(TaskError::ShieldedMaxBalanceChanged)
    }
}

/// What a Max transfer would move, or why the wallet cannot make one. The
/// lock floor itself is upstream's: it drops a drain too small to pay the
/// shielding fee before broadcasting anything.
fn max_transfer_quote(preview: &WalletDrainPreview) -> Result<ShieldedMaxTransferQuote, TaskError> {
    match preview.status {
        WalletDrainPreviewStatus::Ready => {
            // Upstream reports Ready only with both estimates present.
            let (Some(network_fee_duffs), Some(recipient_credits)) = (
                preview.estimated_l1_fee_duffs,
                preview.estimated_recipient_credits,
            ) else {
                return Err(TaskError::WalletStateInconsistent);
            };
            Ok(ShieldedMaxTransferQuote {
                balance_duffs: preview.candidate_balance_duffs,
                fee_credits: shield_max_fee_credits(network_fee_duffs, preview.pool_fee_credits),
                recipient_credits,
            })
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

    /// A registered wallet on a fresh offline context.
    async fn registered_wallet(
        tmp: &std::path::Path,
    ) -> (Arc<AppContext>, Arc<WalletBackend>, WalletSeedHash) {
        use crate::{app::TaskResult, utils::egui_mpsc::SenderAsync};
        let ctx = crate::context::test_support::test_app_context(tmp);
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        ctx.ensure_wallet_backend(SenderAsync::new(sender, ctx.egui_ctx().clone()))
            .await
            .unwrap();
        let backend = ctx.wallet_backend().unwrap();
        let seed = [0x2D; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            dash_sdk::dpp::dashcore::Network::Testnet,
            None,
            None,
        )
        .unwrap();
        let seed_hash = wallet.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .unwrap();
        (ctx, backend, seed_hash)
    }

    fn stored(
        ctx: &AppContext,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
    ) -> Option<PendingShieldedMaxTransfer> {
        backend
            .shielded_max_transfer_records()
            .pending(ctx.network(), seed_hash)
            .unwrap()
    }

    /// The Shielded tab reads the status on arrival. A read between storing
    /// the id and starting the drain must not take the transfer for one that
    /// never started: that would drop the id of a drain about to run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_status_read_before_the_drain_starts_keeps_the_transfer() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, backend, seed_hash) = registered_wallet(tmp.path()).await;
        let (record, _transfer) = ctx.begin_max_transfer(&backend, &seed_hash).await.unwrap();

        let status = ctx
            .shield_max_from_core_status(&backend, seed_hash)
            .await
            .unwrap();

        assert!(
            matches!(
                status,
                BackendTaskSuccessResult::ShieldedMaxTransferStatus {
                    state: Some(ShieldedMaxTransferState::Pending),
                    ..
                }
            ),
            "unexpected status: {status:?}"
        );
        assert_eq!(stored(&ctx, &backend, &seed_hash), Some(record));
    }

    /// A transfer whose drain ended without building a lock settles as not
    /// sent, and its id goes, so the wallet can start a new one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_transfer_that_never_ran_settles_as_not_sent() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, backend, seed_hash) = registered_wallet(tmp.path()).await;
        let (_record, transfer) = ctx.begin_max_transfer(&backend, &seed_hash).await.unwrap();
        drop(transfer);

        let status = ctx
            .shield_max_from_core_status(&backend, seed_hash)
            .await
            .unwrap();

        assert!(
            matches!(
                status,
                BackendTaskSuccessResult::ShieldedMaxTransferStatus {
                    state: Some(ShieldedMaxTransferState::NotSent),
                    ..
                }
            ),
            "unexpected status: {status:?}"
        );
        assert_eq!(stored(&ctx, &backend, &seed_hash), None);
    }

    /// A stored transfer refuses a second one before the wallet is read, and
    /// the refused one leaves no registration behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_transfer_waits_for_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, backend, seed_hash) = registered_wallet(tmp.path()).await;
        let (record, _transfer) = ctx.begin_max_transfer(&backend, &seed_hash).await.unwrap();

        assert!(matches!(
            ctx.shield_max_from_core(&backend, seed_hash, 0).await,
            Err(TaskError::ShieldedMaxTransferAlreadyPending)
        ));
        let loser = PendingShieldedMaxTransfer {
            operation_id: [0x5A; 16],
        };
        assert!(matches!(
            ctx.begin_max_transfer_as(&backend, &seed_hash, loser).await,
            Err(TaskError::ShieldedMaxTransferAlreadyPending)
        ));
        assert_eq!(stored(&ctx, &backend, &seed_hash), Some(record));
        // A registration still held would read as Pending.
        assert_eq!(
            backend
                .shielded_max_transfer_state(&seed_hash, loser.operation_id)
                .await
                .unwrap(),
            ShieldedMaxTransferState::NotSent
        );
        assert_eq!(
            backend
                .shielded_max_transfer_state(&seed_hash, record.operation_id)
                .await
                .unwrap(),
            ShieldedMaxTransferState::Pending
        );
    }

    /// A drain that fails before building its lock (here: no shielded
    /// support on the offline context) settles as not sent: the error is
    /// reported and the id goes, so the wallet is not left blocked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_drain_that_fails_before_its_lock_settles_as_not_sent() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, backend, seed_hash) = registered_wallet(tmp.path()).await;
        let (record, transfer) = ctx.begin_max_transfer(&backend, &seed_hash).await.unwrap();

        let result = ctx
            .run_max_transfer(&backend, seed_hash, record, transfer)
            .await;

        assert!(
            result.is_err()
                && !matches!(result, Err(TaskError::ShieldedMaxTransferUnfinished { .. })),
            "unexpected result: {result:?}"
        );
        assert_eq!(stored(&ctx, &backend, &seed_hash), None);
        assert_eq!(
            backend
                .shielded_max_transfer_state(&seed_hash, record.operation_id)
                .await
                .unwrap(),
            ShieldedMaxTransferState::NotSent
        );
    }

    #[test]
    fn a_changed_balance_refuses_the_confirmed_transfer() {
        let ready = preview(WalletDrainPreviewStatus::Ready);
        assert!(check_confirmed_balance(&ready, 300_000).is_ok());
        for confirmed in [299_999, 300_001, 0] {
            assert!(
                matches!(
                    check_confirmed_balance(&ready, confirmed),
                    Err(TaskError::ShieldedMaxBalanceChanged)
                ),
                "confirmed {confirmed} against a 300000-duff balance"
            );
        }
    }

    /// Without a stored transfer, neither a status read nor Finish touches
    /// the wallet: both report that there is no transfer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_a_stored_transfer_there_is_nothing_to_finish() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, backend, seed_hash) = registered_wallet(tmp.path()).await;

        for result in [
            ctx.finish_shield_max_from_core(&backend, seed_hash).await,
            ctx.shield_max_from_core_status(&backend, seed_hash).await,
        ] {
            assert!(
                matches!(
                    result,
                    Ok(BackendTaskSuccessResult::ShieldedMaxTransferStatus { state: None, .. })
                ),
                "unexpected result: {result:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_a_stored_or_unreadable_transfer_counts_as_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, backend, seed_hash) = registered_wallet(tmp.path()).await;
        assert!(!ctx.has_pending_shielded_max_transfer(&seed_hash));

        let record = PendingShieldedMaxTransfer {
            operation_id: [7; 16],
        };
        let records = backend.shielded_max_transfer_records();
        assert_eq!(
            records.claim(ctx.network(), &seed_hash, record).unwrap(),
            MaxTransferClaim::Claimed
        );
        assert!(ctx.has_pending_shielded_max_transfer(&seed_hash));

        records
            .release(ctx.network(), &seed_hash, record.operation_id)
            .unwrap();
        assert!(!ctx.has_pending_shielded_max_transfer(&seed_hash));

        // Unreadable counts as stored: the status read that follows reports
        // the failure instead of hiding a transfer.
        crate::wallet_backend::plant_unreadable_max_transfer_record(
            &ctx.app_kv(),
            ctx.network(),
            &seed_hash,
        );
        assert!(ctx.has_pending_shielded_max_transfer(&seed_hash));
    }
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
    fn a_ready_wallet_is_quoted_its_balance_fees_and_shielded_amount() {
        assert_eq!(
            max_transfer_quote(&preview(WalletDrainPreviewStatus::Ready)).unwrap(),
            ShieldedMaxTransferQuote {
                balance_duffs: 300_000,
                fee_credits: 12_745_678,
                recipient_credits: 1,
            }
        );
    }

    #[test]
    fn a_ready_preview_without_estimates_is_not_quoted() {
        let mut incomplete = preview(WalletDrainPreviewStatus::Ready);
        incomplete.estimated_recipient_credits = None;
        assert!(matches!(
            max_transfer_quote(&incomplete),
            Err(TaskError::WalletStateInconsistent)
        ));
    }

    #[test]
    fn a_wallet_that_cannot_drain_gets_a_typed_refusal() {
        assert!(matches!(
            max_transfer_quote(&preview(WalletDrainPreviewStatus::InsufficientFunds)),
            Err(TaskError::ShieldedMaxInsufficientFunds)
        ));
        assert!(matches!(
            max_transfer_quote(&preview(WalletDrainPreviewStatus::InputLimitMayBeExceeded)),
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
