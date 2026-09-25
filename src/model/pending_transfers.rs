//! Read-only presentation of unresolved Core funding transactions.

use super::wallet::WalletTransaction;
use dash_sdk::dpp::dashcore::{OutPoint, Txid};

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
    /// Historical funding recovered from Core; no evidence of a pending operation.
    Recovered,
}

impl TransferStage {
    /// Historical recovery alone is not evidence of an unfinished user transfer.
    pub fn needs_attention(self) -> bool {
        self != Self::Recovered
    }
}

/// A single Core history row, optionally enriched with funding observations.
#[derive(Debug)]
pub struct CoreHistoryEntry<'a> {
    /// Core transaction identifier, shared by every funding output in this row.
    pub txid: Txid,
    /// Original history record, when it was available during hydration.
    pub transaction: Option<&'a WalletTransaction>,
    /// Funding outputs associated with this transaction.
    pub funding: Vec<&'a PendingPlatformTransfer>,
}

impl CoreHistoryEntry<'_> {
    /// Recorded block time, or zero when unavailable.
    pub fn timestamp(&self) -> u64 {
        self.transaction
            .map(|tx| tx.timestamp)
            .or_else(|| self.funding.iter().find_map(|f| f.block_time))
            .unwrap_or_default()
    }

    /// Confirmation is independent of whether Platform delivery is known.
    pub fn unconfirmed(&self) -> bool {
        self.transaction
            .map(|tx| tx.status == super::wallet::TransactionStatus::Unconfirmed)
            .unwrap_or_else(|| {
                self.funding.iter().any(|f| {
                    matches!(
                        f.stage,
                        TransferStage::AwaitingConfirmation | TransferStage::ConflictObserved
                    )
                })
            })
    }
}

/// Merge funding observations into Core history without losing funding-only records.
pub fn core_history_entries<'a>(
    transactions: &'a [WalletTransaction],
    funding: &'a [PendingPlatformTransfer],
) -> Vec<CoreHistoryEntry<'a>> {
    let mut entries = std::collections::BTreeMap::new();
    for transaction in transactions.iter().filter(|tx| tx.is_ours) {
        entries.insert(
            transaction.txid,
            CoreHistoryEntry {
                txid: transaction.txid,
                transaction: Some(transaction),
                funding: vec![],
            },
        );
    }
    for transfer in funding {
        entries
            .entry(transfer.out_point.txid)
            .or_insert_with(|| CoreHistoryEntry {
                txid: transfer.out_point.txid,
                transaction: None,
                funding: vec![],
            })
            .funding
            .push(transfer);
    }
    let mut entries: Vec<_> = entries.into_values().collect();
    entries.sort_by(|a, b| {
        b.unconfirmed()
            .cmp(&a.unconfirmed())
            .then_with(|| b.timestamp().cmp(&a.timestamp()))
            .then_with(|| b.txid.cmp(&a.txid))
    });
    entries
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
        let entries = core_history_entries(&history, &[]);
        assert_eq!(entries[0].txid, pending.txid);
        assert_eq!(entries[1].timestamp(), 1_700_000_199);
    }

    #[test]
    fn pending_transfers_have_stable_order_and_instant_locked_records_are_not_pending() {
        let history = [
            record(1, 0, TransactionStatus::Unconfirmed),
            record(2, 0, TransactionStatus::InstantSendLocked),
            record(3, 0, TransactionStatus::Unconfirmed),
        ];
        let entries = core_history_entries(&history, &[]);
        assert_eq!(
            entries.iter().map(|tx| tx.txid).collect::<Vec<_>>(),
            vec![
                Txid::from_byte_array([3; 32]),
                Txid::from_byte_array([1; 32]),
                Txid::from_byte_array([2; 32]),
            ]
        );
    }
    #[test]
    fn pending_transfers_merge_history_and_funding_without_duplicates_or_false_pending() {
        let history = [
            record(1, 500, TransactionStatus::Confirmed),
            record(2, 100, TransactionStatus::Confirmed),
        ];
        let funding = [
            PendingPlatformTransfer {
                out_point: OutPoint::new(history[1].txid, 0),
                funding_amount: 1_000_000,
                core_fee: None,
                block_time: Some(100),
                stage: TransferStage::Recovered,
                conflicts: vec![],
            },
            PendingPlatformTransfer {
                out_point: OutPoint::new(Txid::from_byte_array([3; 32]), 0),
                funding_amount: 100,
                core_fee: None,
                block_time: None,
                stage: TransferStage::AwaitingConfirmation,
                conflicts: vec![],
            },
        ];
        let entries = core_history_entries(&history, &funding);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].txid, funding[1].out_point.txid);
        assert!(entries[0].unconfirmed());
        assert!(entries[0].transaction.is_none());
        assert_eq!(entries[1].txid, history[0].txid);
        assert_eq!(entries[2].txid, history[1].txid);
        assert!(!entries[2].unconfirmed());
        assert_eq!(entries[2].funding.len(), 1);
        assert!(!entries[2].funding[0].stage.needs_attention());
    }
}
