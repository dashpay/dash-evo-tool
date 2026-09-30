use super::{TransactionHistoryStatus, WalletBackend};
use crate::backend_task::error::TaskError;
use crate::model::pending_transfers::{
    PendingPlatformTransfer, TransferAssessment, TransferConflict, TransferStage,
};
use crate::model::wallet::{TransactionStatus, WalletSeedHash, WalletTransaction};
use dash_sdk::dpp::dashcore::OutPoint;
use platform_wallet::AssetLockFundingType;
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
use std::collections::BTreeMap;

impl WalletBackend {
    /// Inspect persisted funding locks and hydrated history without resuming or releasing funds.
    pub async fn assess_platform_transfers(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<TransferAssessment, TaskError> {
        let locks = self.list_tracked_asset_locks(seed_hash).await?;
        let snapshot = self.inner.snapshots.snapshot(seed_hash);
        Ok(TransferAssessment {
            transfers: assess_locks(&locks, &snapshot.transactions),
            history_complete: matches!(
                snapshot.transaction_history_status,
                TransactionHistoryStatus::Complete
            ),
            checked_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }
}

pub(super) fn assess_locks(
    locks: &[TrackedAssetLock],
    history: &[WalletTransaction],
) -> Vec<PendingPlatformTransfer> {
    let records: BTreeMap<_, _> = history.iter().map(|tx| (tx.txid, tx)).collect();
    let mut spends: BTreeMap<OutPoint, Vec<TransferConflict>> = BTreeMap::new();
    for record in history.iter().filter(|tx| tx.is_confirmed()) {
        let Some(height) = record.height else {
            continue;
        };
        for input in &record.transaction.input {
            spends
                .entry(input.previous_output)
                .or_default()
                .push(TransferConflict {
                    input: input.previous_output,
                    competing_txid: record.txid,
                    height,
                });
        }
    }
    let mut transfers: Vec<_> = locks
        .iter()
        .filter(|lock| {
            lock.funding_type == AssetLockFundingType::AssetLockAddressTopUp
                && lock.status != AssetLockStatus::Consumed
        })
        .map(|lock| {
            let record = records.get(&lock.out_point.txid);
            let conflicts: Vec<_> = lock
                .transaction
                .input
                .iter()
                .flat_map(|input| spends.get(&input.previous_output).into_iter().flatten())
                .filter(|conflict| conflict.competing_txid != lock.out_point.txid)
                .cloned()
                .collect();
            let confirmed = matches!(
                lock.status,
                AssetLockStatus::InstantSendLocked
                    | AssetLockStatus::ChainLocked
                    | AssetLockStatus::RecoveredFromChain
            ) || record
                .is_some_and(|tx| tx.status != TransactionStatus::Unconfirmed);
            PendingPlatformTransfer {
                out_point: lock.out_point,
                funding_amount: lock.amount,
                core_fee: record.and_then(|tx| tx.fee),
                block_time: record.and_then(|tx| (tx.timestamp != 0).then_some(tx.timestamp)),
                stage: if !conflicts.is_empty() {
                    TransferStage::ConflictObserved
                } else if lock.status == AssetLockStatus::RecoveredFromChain {
                    TransferStage::Recovered
                } else if confirmed {
                    TransferStage::DeliveryUnknown
                } else {
                    TransferStage::AwaitingConfirmation
                },
                conflicts,
            }
        })
        .collect();
    transfers.sort_by(|a, b| {
        b.block_time
            .cmp(&a.block_time)
            .then_with(|| a.out_point.cmp(&b.out_point))
    });
    transfers
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::{Transaction, TxIn, Txid, hashes::Hash};

    fn lock() -> TrackedAssetLock {
        TrackedAssetLock {
            out_point: OutPoint::new(Txid::from_byte_array([2; 32]), 0),
            transaction: Transaction {
                version: 3,
                lock_time: 0,
                input: vec![TxIn {
                    previous_output: OutPoint::new(Txid::from_byte_array([1; 32]), 1),
                    ..Default::default()
                }],
                output: vec![],
                special_transaction_payload: None,
            },
            account_index: 0,
            funding_type: AssetLockFundingType::AssetLockAddressTopUp,
            identity_index: 0,
            amount: 100,
            status: AssetLockStatus::Built,
            proof: None,
        }
    }

    fn competing_record(lock: &TrackedAssetLock, status: TransactionStatus) -> WalletTransaction {
        WalletTransaction {
            txid: Txid::from_byte_array([3; 32]),
            transaction: lock.transaction.clone(),
            timestamp: 123,
            height: Some(17),
            block_hash: None,
            net_amount: -150,
            fee: None,
            label: None,
            is_ours: true,
            status,
        }
    }

    #[test]
    fn pending_transfers_find_conflict_in_restored_history_without_claiming_finality() {
        let lock = lock();
        let history = vec![competing_record(&lock, TransactionStatus::ChainLocked)];
        let assessed = assess_locks(std::slice::from_ref(&lock), &history);
        assert_eq!(assessed[0].stage, TransferStage::ConflictObserved);
        assert_eq!(assessed[0].conflicts[0].competing_txid, history[0].txid);
        assert_eq!(
            assessed[0].conflicts[0].input,
            lock.transaction.input[0].previous_output
        );
        assert_eq!(lock.status, AssetLockStatus::Built);
    }

    #[test]
    fn pending_transfers_missing_or_unconfirmed_competitor_proves_nothing() {
        let lock = lock();
        let history = [competing_record(&lock, TransactionStatus::Unconfirmed)];
        for history in [&history[..], &[][..]] {
            let assessed = assess_locks(std::slice::from_ref(&lock), history);
            assert_eq!(assessed[0].stage, TransferStage::AwaitingConfirmation);
            assert!(assessed[0].conflicts.is_empty());
        }
    }

    #[test]
    fn pending_transfers_do_not_treat_own_confirmation_as_a_conflict_or_delivery() {
        let lock = lock();
        let mut record = competing_record(&lock, TransactionStatus::ChainLocked);
        record.txid = lock.out_point.txid;
        let assessed = assess_locks(&[lock], &[record]);
        assert_eq!(assessed[0].stage, TransferStage::DeliveryUnknown);
        assert!(assessed[0].conflicts.is_empty());
    }

    #[test]
    fn pending_transfers_exclude_consumed_and_other_funding_operations() {
        let mut consumed = lock();
        consumed.status = AssetLockStatus::Consumed;
        let mut identity = lock();
        identity.funding_type = AssetLockFundingType::IdentityRegistration;
        assert!(assess_locks(&[consumed, identity], &[]).is_empty());
    }

    #[test]
    fn pending_transfers_recovered_lock_does_not_claim_success_or_known_recipient() {
        let mut recovered = lock();
        recovered.status = AssetLockStatus::RecoveredFromChain;
        let assessed = assess_locks(&[recovered], &[]);
        assert_eq!(assessed[0].stage, TransferStage::Recovered);
        assert_eq!(assessed[0].funding_amount, 100);
        assert_eq!(assessed[0].core_fee, None);
        assert_eq!(assessed[0].block_time, None);
    }
}
