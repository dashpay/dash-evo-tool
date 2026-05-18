//! Display-only wallet snapshot model.
//!
//! `WalletSnapshot` drives the wallets screen (balance, transaction list,
//! UTXO list) without ever blocking the egui frame thread. It is fed by the
//! [`EventBridge`](super::EventBridge) off upstream `platform-wallet` events
//! and read synchronously, lock-free, infallibly by the UI via
//! [`AppContext::wallet_backend()`](crate::context::AppContext::wallet_backend).
//!
//! # FUND-SAFETY MANDATE — DISPLAY-ONLY
//!
//! The snapshot exists ONLY to render the UI. Coin selection and transaction
//! construction MUST go through
//! [`WalletBackend::send_payment`](super::WalletBackend::send_payment) /
//! [`WalletBackend::create_asset_lock_proof`](super::WalletBackend::create_asset_lock_proof),
//! which use the upstream-authoritative live UTXO set at send time. No code
//! path may select spendable inputs from a `WalletSnapshot`
//! (`backend-architecture.md` §A04 reviewer gate).
//!
//! # Why transactions are accumulated, not recomputed
//!
//! DET does not enable upstream's `keep-finalized-transactions` Cargo feature,
//! so once a transaction is chain-locked upstream drops its record from the
//! in-memory wallet. The full history is therefore *event-sourced*:
//! `WalletEvent::{TransactionDetected, BlockProcessed}` carry the records and
//! the upstream persister stores them durably. The snapshot store accumulates
//! these records (the surviving piece of the deleted `reconcile_spv_wallets`
//! `TransactionRecord` → `WalletTransaction` mapping). Balance and UTXOs, by
//! contrast, are read straight off the live wallet — they are always current
//! there.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use arc_swap::ArcSwap;
use dash_sdk::dpp::dashcore::{Address, OutPoint, ScriptBuf, Txid};
use dash_sdk::dpp::key_wallet::managed_account::transaction_record::TransactionRecord;
use dash_sdk::dpp::key_wallet::transaction_checking::TransactionContext;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use platform_wallet::PlatformWallet;

use crate::model::wallet::{TransactionStatus, WalletSeedHash, WalletTransaction};

/// Upstream `WalletId` (`SHA256(root_xpub || root_chain_code)`), distinct from
/// DET's `WalletSeedHash`. Mirrors the alias in [`super`].
type WalletId = [u8; 32];

/// Confirmed / unconfirmed / total balance in duffs. DET-shaped — no upstream
/// `WalletBalance` / `WalletCoreBalance` crosses the seam
/// (rust-best-practices M-DONT-LEAK-TYPES).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetWalletBalance {
    pub confirmed: u64,
    pub unconfirmed: u64,
    pub total: u64,
}

/// One unspent output. DET-shaped — no upstream `Utxo` crosses the seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetUtxo {
    pub outpoint: OutPoint,
    pub value: u64,
    pub script_pubkey: ScriptBuf,
    pub address: Address,
}

/// Per-wallet display snapshot. Cheap to clone-share via the enclosing `Arc`.
#[derive(Debug, Clone, Default)]
pub struct WalletSnapshot {
    pub balance: DetWalletBalance,
    pub transactions: Vec<WalletTransaction>,
    pub utxos: Vec<DetUtxo>,
    /// UTXO-derived per-address balances, summed across this wallet's UTXOs.
    /// Feeds the account-summary view that used to read
    /// `Wallet.address_balances`.
    pub address_balances: BTreeMap<Address, u64>,
}

/// Map a finalized-or-pending upstream `TransactionContext` to DET's richer
/// `TransactionStatus`. Upstream now distinguishes InstantSend and chain-lock,
/// so this supersedes the old height-only `from_height` heuristic.
fn status_from_context(context: &TransactionContext) -> TransactionStatus {
    match context {
        TransactionContext::Mempool => TransactionStatus::Unconfirmed,
        TransactionContext::InstantSend(_) => TransactionStatus::InstantSendLocked,
        TransactionContext::InBlock(_) => TransactionStatus::Confirmed,
        TransactionContext::InChainLockedBlock(_) => TransactionStatus::ChainLocked,
    }
}

/// The single `TransactionRecord` → `WalletTransaction` mapping. Relocated
/// verbatim-in-spirit from the deleted `reconcile_spv_wallets`; this is the
/// only place that conversion lives now (`backend-architecture.md`).
pub(super) fn map_transaction_record(record: &TransactionRecord) -> WalletTransaction {
    let block_info = record.block_info();
    WalletTransaction {
        txid: record.txid,
        transaction: record.transaction.clone(),
        timestamp: block_info.map(|bi| bi.timestamp() as u64).unwrap_or(0),
        height: record.height(),
        block_hash: block_info.map(|bi| bi.block_hash()),
        net_amount: record.net_amount,
        fee: record.fee,
        label: Some(record.label.clone()).filter(|s| !s.is_empty()),
        // Per-wallet history — every record involves our addresses.
        is_ours: true,
        status: status_from_context(&record.context),
    }
}

/// Shared store of per-wallet display snapshots plus the event-sourced
/// transaction accumulator. Held by both [`WalletBackend`](super::WalletBackend)
/// (for the read accessors) and the [`EventBridge`](super::EventBridge) (for
/// the recompute-on-event push).
///
/// Wallet handles are wired in at registration (after construction — the
/// bridge that this store backs is built first), so a pre-registration read
/// yields an empty snapshot, which the UI renders as the existing "syncing"
/// state, not a zero-balance bug.
pub(super) struct SnapshotStore {
    /// DET-keyed published snapshots. Lock-free read on the UI hot path.
    snapshots: ArcSwap<HashMap<WalletSeedHash, Arc<WalletSnapshot>>>,
    /// Event-sourced transaction history, keyed by upstream `WalletId` then
    /// `Txid`. A `BTreeMap` per wallet so re-seen records (mempool → block →
    /// chainlock) upsert in place and iteration is deterministic.
    tx_log: Mutex<HashMap<WalletId, BTreeMap<Txid, WalletTransaction>>>,
    /// Per-wallet registration: upstream `WalletId` → (DET `WalletSeedHash`,
    /// cheap shared `PlatformWallet` handle). The handle gives lock-free
    /// balance (`balance()`) and non-blocking UTXO (`try_state()`) reads, so
    /// the event callback never blocks and never touches an async lock.
    registered: Mutex<HashMap<WalletId, RegisteredWallet>>,
}

struct RegisteredWallet {
    seed_hash: WalletSeedHash,
    wallet: Arc<PlatformWallet>,
}

impl std::fmt::Debug for SnapshotStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotStore").finish_non_exhaustive()
    }
}

impl SnapshotStore {
    pub(super) fn new() -> Self {
        Self {
            snapshots: ArcSwap::from_pointee(HashMap::new()),
            tx_log: Mutex::new(HashMap::new()),
            registered: Mutex::new(HashMap::new()),
        }
    }

    /// Record a wallet's `WalletId` ⇄ (`WalletSeedHash`, handle) association
    /// at registration so events (keyed by `WalletId`) can recompute the
    /// DET-keyed snapshot off the lock-free balance + non-blocking UTXO read.
    pub(super) fn register_wallet(
        &self,
        seed_hash: WalletSeedHash,
        wallet_id: WalletId,
        wallet: Arc<PlatformWallet>,
    ) {
        if let Ok(mut map) = self.registered.lock() {
            map.insert(wallet_id, RegisteredWallet { seed_hash, wallet });
        }
    }

    /// Read a wallet's published snapshot. Lock-free, infallible. An absent
    /// entry (pre-first-sync) yields the default empty snapshot, which the UI
    /// renders as "syncing".
    pub(super) fn snapshot(&self, seed_hash: &WalletSeedHash) -> Arc<WalletSnapshot> {
        self.snapshots
            .load()
            .get(seed_hash)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether a snapshot has been published for the wallet yet. `false`
    /// before the first `EventBridge` recompute ⇒ the UI shows "syncing".
    pub(super) fn has_snapshot(&self, seed_hash: &WalletSeedHash) -> bool {
        self.snapshots.load().contains_key(seed_hash)
    }

    /// Merge freshly-seen transaction records into the per-wallet log.
    /// Upsert-keyed by `Txid` so a record re-seen at a higher confirmation
    /// tier replaces the lower one.
    pub(super) fn accumulate_transactions<'a, I>(&self, wallet_id: &WalletId, records: I)
    where
        I: IntoIterator<Item = &'a TransactionRecord>,
    {
        let Ok(mut log) = self.tx_log.lock() else {
            return;
        };
        let per_wallet = log.entry(*wallet_id).or_default();
        for record in records {
            per_wallet.insert(record.txid, map_transaction_record(record));
        }
    }

    /// Recompute and atomically publish the snapshot for one wallet, off the
    /// lock-free upstream balance + non-blocking UTXO state plus the
    /// event-sourced tx log. Called by the `EventBridge` after it has
    /// accumulated the event's records.
    ///
    /// Never blocks and never awaits: `balance()` is lock-free atomics,
    /// `try_state()` is a non-blocking try-lock that yields `None` under
    /// contention (in which case the UTXO list and UTXO-derived per-address
    /// balances from the previously published snapshot are carried forward —
    /// a subsequent event recomputes them once the lock is free). Balance and
    /// transactions are always refreshed.
    pub(super) fn recompute(&self, wallet_id: &WalletId) {
        let (seed_hash, wallet) = {
            let Ok(map) = self.registered.lock() else {
                return;
            };
            match map.get(wallet_id) {
                Some(r) => (r.seed_hash, Arc::clone(&r.wallet)),
                // Event for a wallet not registered DET-side — its snapshot
                // is built once registration records the handle and a later
                // event fires.
                None => return,
            }
        };

        // Lock-free balance read (atomics).
        let bal = wallet.balance();
        let balance = DetWalletBalance {
            confirmed: bal.confirmed(),
            unconfirmed: bal.unconfirmed(),
            total: bal.total(),
        };

        // Non-blocking UTXO read. On contention, carry the prior snapshot's
        // UTXO view forward rather than blocking the event callback.
        let prior = self.snapshot(&seed_hash);
        let (utxos, address_balances) = match wallet.try_state() {
            Some(state) => {
                let mut utxos = Vec::new();
                let mut address_balances: BTreeMap<Address, u64> = BTreeMap::new();
                for u in state.utxos() {
                    *address_balances.entry(u.address.clone()).or_insert(0) += u.txout.value;
                    utxos.push(DetUtxo {
                        outpoint: u.outpoint,
                        value: u.txout.value,
                        script_pubkey: u.txout.script_pubkey.clone(),
                        address: u.address.clone(),
                    });
                }
                (utxos, address_balances)
            }
            None => (prior.utxos.clone(), prior.address_balances.clone()),
        };

        self.publish(&seed_hash, wallet_id, balance, utxos, address_balances);
    }

    /// Assemble the event-sourced tx history with the freshly-read
    /// balance/UTXO state and atomically publish the snapshot. Split from
    /// [`Self::recompute`] so the publish + tx-log assembly is unit-testable
    /// without a live `PlatformWallet`.
    fn publish(
        &self,
        seed_hash: &WalletSeedHash,
        wallet_id: &WalletId,
        balance: DetWalletBalance,
        utxos: Vec<DetUtxo>,
        address_balances: BTreeMap<Address, u64>,
    ) {
        let transactions: Vec<WalletTransaction> = self
            .tx_log
            .lock()
            .ok()
            .and_then(|log| log.get(wallet_id).map(|m| m.values().cloned().collect()))
            .unwrap_or_default();

        let snapshot = Arc::new(WalletSnapshot {
            balance,
            transactions,
            utxos,
            address_balances,
        });

        self.snapshots.rcu(|current| {
            let mut next = (**current).clone();
            next.insert(*seed_hash, snapshot.clone());
            next
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{BlockHash, Transaction};
    use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
    use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
        TransactionDirection, TransactionRecord,
    };
    use dash_sdk::dpp::key_wallet::transaction_checking::BlockInfo;
    use dash_sdk::dpp::key_wallet::transaction_checking::transaction_router::TransactionType;

    fn seed(n: u8) -> WalletSeedHash {
        [n; 32]
    }
    fn wid(n: u8) -> WalletId {
        [n; 32]
    }

    /// A transaction whose txid is distinct per `n` (the lock_time perturbs
    /// the hash) so re-`new`'d records key apart in the log.
    fn tx_with(n: u8) -> Transaction {
        Transaction {
            version: 1,
            lock_time: n as u32,
            input: vec![],
            output: vec![],
            special_transaction_payload: None,
        }
    }

    fn record(n: u8, net: i64) -> TransactionRecord {
        TransactionRecord::new(
            tx_with(n),
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Incoming,
            Vec::new(),
            Vec::new(),
            net,
        )
    }

    /// Publish a snapshot directly off the tx-log via the same seam
    /// `recompute` uses for its final step — exercises tx accumulation +
    /// publish without needing a live `PlatformWallet`.
    fn publish_tx_only(store: &SnapshotStore, seed: WalletSeedHash, wid: WalletId) {
        store.publish(
            &seed,
            &wid,
            DetWalletBalance::default(),
            Vec::new(),
            BTreeMap::new(),
        );
    }

    #[test]
    fn empty_store_yields_default_snapshot() {
        let store = SnapshotStore::new();
        let snap = store.snapshot(&seed(1));
        assert_eq!(snap.balance, DetWalletBalance::default());
        assert!(snap.transactions.is_empty());
        assert!(snap.utxos.is_empty());
    }

    #[test]
    fn accumulate_then_publish_surfaces_tx_history() {
        let store = SnapshotStore::new();
        store.accumulate_transactions(&wid(7), [&record(1, 500), &record(2, -200)]);
        publish_tx_only(&store, seed(7), wid(7));
        let snap = store.snapshot(&seed(7));
        assert_eq!(snap.transactions.len(), 2);
        assert_eq!(snap.balance, DetWalletBalance::default());
    }

    #[test]
    fn reseen_txid_upserts_in_place() {
        let store = SnapshotStore::new();
        store.accumulate_transactions(&wid(3), [&record(1, 100)]);
        let mut confirmed = record(1, 100);
        confirmed.context = TransactionContext::InChainLockedBlock(BlockInfo::new(
            10,
            BlockHash::from_byte_array([0u8; 32]),
            123,
        ));
        store.accumulate_transactions(&wid(3), [&confirmed]);
        publish_tx_only(&store, seed(3), wid(3));
        let snap = store.snapshot(&seed(3));
        assert_eq!(snap.transactions.len(), 1);
        assert_eq!(snap.transactions[0].status, TransactionStatus::ChainLocked);
    }

    #[test]
    fn recompute_for_unregistered_wallet_is_a_noop() {
        let store = SnapshotStore::new();
        store.accumulate_transactions(&wid(9), [&record(1, 1)]);
        // No registered handle → recompute returns early, nothing published.
        store.recompute(&wid(9));
        assert!(store.snapshot(&seed(9)).transactions.is_empty());
    }

    #[test]
    fn status_mapping_covers_every_context() {
        assert_eq!(
            status_from_context(&TransactionContext::Mempool),
            TransactionStatus::Unconfirmed
        );
        let bi = BlockInfo::new(1, BlockHash::from_byte_array([0u8; 32]), 1);
        assert_eq!(
            status_from_context(&TransactionContext::InBlock(bi)),
            TransactionStatus::Confirmed
        );
        assert_eq!(
            status_from_context(&TransactionContext::InChainLockedBlock(bi)),
            TransactionStatus::ChainLocked
        );
    }
}
