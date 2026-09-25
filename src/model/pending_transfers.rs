//! Read-only presentation of unresolved Core funding transactions.

use super::wallet::WalletTransaction;
use dash_sdk::dpp::dashcore::{OutPoint, Txid};
use std::cmp::Ordering;

/// An observed competing spend, not proof that reserved funds can be released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferConflict {
    /// Funding input also present in the competing transaction.
    pub input: OutPoint,
    /// Transaction observed spending the same input.
    pub competing_txid: Txid,
    /// Recorded block height, without a finalized-ancestry guarantee.
    pub height: u32,
}

/// The strongest statement the available funding records support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStage {
    /// No recorded confirmation; this does not imply network rejection.
    AwaitingConfirmation,
    /// Another recorded payment uses a shared input; finality remains unverified.
    ConflictObserved,
    /// Core confirmation is recorded, but Platform consumption is not verified.
    DeliveryUnknown,
}

/// One persisted Platform-address funding transaction with an unresolved outcome.
#[derive(Debug, Clone)]
pub struct PendingPlatformTransfer {
    /// Persisted credit output identifying the funding operation.
    pub out_point: OutPoint,
    /// Credit-output value in duffs, potentially including a fee allowance.
    pub funding_amount: u64,
    /// Known fee of the Core funding transaction, in duffs.
    pub core_fee: Option<u64>,
    /// Recorded block time, never an inferred creation/first-seen time.
    pub block_time: Option<u64>,
    /// Provisional presentation state, not permission to retry or release funds.
    pub stage: TransferStage,
    /// Observed input conflicts retained for inspection.
    pub conflicts: Vec<TransferConflict>,
}

/// A local assessment of the latest wallet observations; never an accounting authority.
#[derive(Debug, Clone)]
pub struct TransferAssessment {
    /// Address-funding locks whose Platform consumption is not recorded.
    pub transfers: Vec<PendingPlatformTransfer>,
    /// Whether startup successfully hydrated every persisted history record.
    pub history_complete: bool,
    /// Local assessment time in Unix seconds, not a network verification time.
    pub checked_at: u64,
}

/// Pending transactions precede dated history, with a deterministic tie-breaker.
pub fn compare_history(a: &WalletTransaction, b: &WalletTransaction) -> Ordering {
    use super::wallet::TransactionStatus;
    let pending_a = a.status == TransactionStatus::Unconfirmed;
    let pending_b = b.status == TransactionStatus::Unconfirmed;
    pending_b
        .cmp(&pending_a)
        .then_with(|| b.timestamp.cmp(&a.timestamp))
        .then_with(|| b.txid.cmp(&a.txid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::TransactionStatus;
    use dash_sdk::dpp::dashcore::{Transaction, hashes::Hash};

    fn record(id: u8, timestamp: u64, status: TransactionStatus) -> WalletTransaction {
        WalletTransaction {
            txid: Txid::from_byte_array([id; 32]),
            transaction: Transaction {
                version: 3,
                lock_time: 0,
                input: vec![],
                output: vec![],
                special_transaction_payload: None,
            },
            timestamp,
            height: (status == TransactionStatus::Confirmed).then_some(100),
            block_hash: None,
            net_amount: -100,
            fee: None,
            label: None,
            is_ours: true,
            status,
        }
    }

    #[test]
    fn pending_transfers_precede_hundreds_of_confirmed_records_without_dates() {
        let pending = record(250, 0, TransactionStatus::Unconfirmed);
        let mut history: Vec<_> = (0..200)
            .map(|id| {
                record(
                    id,
                    1_700_000_000 + u64::from(id),
                    TransactionStatus::Confirmed,
                )
            })
            .collect();
        history.push(pending.clone());
        history.sort_by(compare_history);
        assert_eq!(history[0].txid, pending.txid);
        assert_eq!(history[1].timestamp, 1_700_000_199);
    }

    #[test]
    fn pending_transfers_have_stable_order_and_instant_locked_records_are_not_pending() {
        let mut history = [
            record(1, 0, TransactionStatus::Unconfirmed),
            record(2, 0, TransactionStatus::InstantSendLocked),
            record(3, 0, TransactionStatus::Unconfirmed),
        ];
        history.sort_by(compare_history);
        assert_eq!(
            history.iter().map(|tx| tx.txid).collect::<Vec<_>>(),
            vec![
                Txid::from_byte_array([3; 32]),
                Txid::from_byte_array([1; 32]),
                Txid::from_byte_array([2; 32]),
            ]
        );
    }
}
