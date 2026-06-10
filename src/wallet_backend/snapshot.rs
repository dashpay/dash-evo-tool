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
use dash_sdk::dpp::dashcore::{Address, OutPoint, ScriptBuf, Transaction, TxOut, Txid};
use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
    OutputRole, TransactionRecord,
};
use dash_sdk::dpp::key_wallet::transaction_checking::TransactionContext;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use platform_wallet::PlatformWallet;

use crate::model::dashpay::DetectedIncomingOutput;
use crate::model::wallet::{TransactionStatus, WalletSeedHash, WalletTransaction};

/// Upstream `WalletId` (`SHA256(root_xpub || root_chain_code)`), distinct from
/// DET's `WalletSeedHash`. Mirrors the alias in [`super`].
type WalletId = [u8; 32];

/// Confirmed / unconfirmed / total balance in duffs. DET-shaped — no upstream
/// `WalletBalance` / `WalletCoreBalance` crosses the seam
/// (rust-best-practices M-DONT-LEAK-TYPES).
///
/// `total` is the headline figure and counts immature coinbase and locked
/// (CoinJoin) funds that coin selection cannot touch. `spendable()` is the
/// subset the upstream `CoinSelector` actually draws from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetWalletBalance {
    pub confirmed: u64,
    pub unconfirmed: u64,
    pub total: u64,
}

impl DetWalletBalance {
    /// Funds coin selection can spend right now: confirmed plus unconfirmed.
    /// Excludes the immature and locked duffs that `total` counts but the
    /// upstream `CoinSelector` rejects. Reserve a "Max" send against this, not
    /// `total`, or the send over-shoots the selectable set and fails with
    /// insufficient funds.
    pub fn spendable(&self) -> u64 {
        self.confirmed.saturating_add(self.unconfirmed)
    }
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

/// The `(transaction, [(outpoint, txout, address)])` payload the asset-lock and
/// identity-funding screens wait on, matching the
/// `CoreItem::ReceivedAvailableUTXOTransaction` contract.
pub(super) type ReceivedUtxoTransaction = (Transaction, Vec<(OutPoint, TxOut, Address)>);

/// Extract the wallet-owned outputs of a freshly-seen transaction as the
/// payload the asset-lock and identity-funding screens wait on
/// (`CoreItem::ReceivedAvailableUTXOTransaction`).
///
/// Only outputs that pay into this wallet (`OutputRole::Received` or
/// `OutputRole::Change`) with a decodable address are included — these are the
/// funding UTXOs a waiting screen matches against its QR funding address.
/// `Sent`/`Unspendable` outputs and address-less scripts are skipped.
///
/// Returns `None` when the transaction has no wallet-owned outputs (e.g. a
/// pure outgoing payment), so the bridge emits the event only when there is a
/// received UTXO for a screen to advance on.
pub(super) fn received_outputs_for_record(
    record: &TransactionRecord,
) -> Option<ReceivedUtxoTransaction> {
    let tx = &record.transaction;
    let mut owned = Vec::new();
    for out in &record.output_details {
        if !matches!(out.role, OutputRole::Received | OutputRole::Change) {
            continue;
        }
        let Some(address) = out.address.clone() else {
            continue;
        };
        let Some(txout) = tx.output.get(out.index as usize) else {
            continue;
        };
        let outpoint = OutPoint {
            txid: record.txid,
            vout: out.index,
        };
        owned.push((outpoint, txout.clone(), address));
    }

    if owned.is_empty() {
        None
    } else {
        Some((tx.clone(), owned))
    }
}

/// Extract every received output of a freshly-seen transaction as a
/// [`DetectedIncomingOutput`] candidate for incoming DashPay
/// contact-payment detection.
///
/// Unlike [`received_outputs_for_record`], which exists to advance funding
/// screens, this carries the `(txid, address, value)` the detector needs to
/// resolve `address → (contact, index)` and record the payment. Only
/// `OutputRole::Received` outputs with a decodable address are candidates —
/// `Change` is excluded because contact payments always land on a freshly
/// derived receiving address, never on our own change. The detector applies
/// the authoritative DashPay-address match downstream; this is the cheap
/// pre-filter that keeps the event hot path free of any owner/KV lookup.
pub(super) fn incoming_payment_candidates(
    record: &TransactionRecord,
) -> Vec<DetectedIncomingOutput> {
    let txid = record.txid.to_string();
    record
        .output_details
        .iter()
        .filter(|out| matches!(out.role, OutputRole::Received))
        .filter_map(|out| {
            out.address.as_ref().map(|address| DetectedIncomingOutput {
                txid: txid.clone(),
                vout: out.index,
                address: address.to_string(),
                amount_duffs: out.value,
            })
        })
        .collect()
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

    /// Drop every trace of a forgotten wallet: its published snapshot, its
    /// `WalletId`-keyed registration, and its event-sourced transaction log.
    /// Without this a removed wallet's balance and history keep being read and
    /// re-published on the next `EventBridge` recompute.
    pub(super) fn forget_wallet(&self, seed_hash: &WalletSeedHash, wallet_id: &WalletId) {
        self.snapshots.rcu(|current| {
            let mut next = HashMap::clone(current);
            next.remove(seed_hash);
            next
        });
        if let Ok(mut map) = self.registered.lock() {
            map.remove(wallet_id);
        }
        if let Ok(mut log) = self.tx_log.lock() {
            log.remove(wallet_id);
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
    use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};
    use dash_sdk::dpp::dashcore::{BlockHash, Network, PublicKey, Transaction, TxOut};
    use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
    use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
        OutputDetail, OutputRole, TransactionDirection, TransactionRecord,
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
    fn spendable_excludes_immature_and_locked_held_in_total() {
        // `total` carries immature coinbase + locked CoinJoin funds (here the
        // 700 gap above confirmed+unconfirmed) that the CoinSelector rejects.
        // `spendable()` must report only confirmed + unconfirmed.
        let balance = DetWalletBalance {
            confirmed: 500,
            unconfirmed: 300,
            total: 1_500,
        };
        assert_eq!(balance.spendable(), 800);
        assert!(balance.spendable() < balance.total);
    }

    /// Crosses the `send_screen` "Max" seam: the Max a Core send reserves must
    /// come from the *spendable* set, never `total`. When the wallet holds
    /// immature/locked funds (total > spendable), feeding `total` to the Max
    /// math over-shoots what coin selection can spend, so the broadcast fails
    /// with insufficient funds.
    #[test]
    fn core_max_reserves_against_spendable_not_total() {
        use crate::model::fee_estimation::core_max_send_amount_duffs;

        // 800 spendable, 700 immature/locked riding in `total`.
        let balance = DetWalletBalance {
            confirmed: 500,
            unconfirmed: 300,
            total: 1_500,
        };

        let max = core_max_send_amount_duffs(balance.spendable(), 1, 1)
            .expect("spendable covers the fee");

        // Max may never exceed what coin selection can actually spend.
        assert!(
            max <= balance.spendable(),
            "Max {max} over-reserves against spendable {}",
            balance.spendable()
        );

        // Reserving against `total` would let Max exceed the spendable set —
        // the exact over-shoot this fix kills.
        let buggy_max = core_max_send_amount_duffs(balance.total, 1, 1)
            .expect("total trivially covers the fee");
        assert!(
            buggy_max > balance.spendable(),
            "the total-based Max should over-reserve, proving the seam matters"
        );
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

    /// A distinct testnet p2pkh address keyed off `n` (derived from a valid
    /// secret key so the pubkey is a real curve point).
    fn addr(n: u8) -> Address {
        let mut sk_bytes = [1u8; 32];
        sk_bytes[31] = n.max(1);
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        let pubkey = PublicKey::new(sk.public_key(&secp));
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// Build a record whose transaction carries `outputs` (value, owning
    /// address) and matching `OutputDetail`s with the given roles. Each
    /// output's `script_pubkey` is the address's own script so the converter's
    /// outpoint→address mapping is faithful.
    fn record_with_outputs(n: u8, outputs: &[(u64, Address, OutputRole)]) -> TransactionRecord {
        let mut tx = tx_with(n);
        let mut details = Vec::new();
        for (index, (value, address, role)) in outputs.iter().enumerate() {
            tx.output.push(TxOut {
                value: *value,
                script_pubkey: address.script_pubkey(),
            });
            details.push(OutputDetail {
                index: index as u32,
                role: *role,
                address: Some(address.clone()),
                value: *value,
            });
        }
        TransactionRecord::new(
            tx,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Incoming,
            Vec::new(),
            details,
            0,
        )
    }

    #[test]
    fn received_output_surfaces_as_funding_utxo() {
        let funding = addr(1);
        let rec = record_with_outputs(1, &[(100_000, funding.clone(), OutputRole::Received)]);

        let (tx, owned) =
            received_outputs_for_record(&rec).expect("a received output yields a payload");

        assert_eq!(tx.txid(), rec.txid);
        assert_eq!(owned.len(), 1);
        let (outpoint, txout, address) = &owned[0];
        assert_eq!(*address, funding);
        assert_eq!(outpoint.txid, rec.txid);
        assert_eq!(outpoint.vout, 0);
        assert_eq!(txout.value, 100_000);
        assert_eq!(txout.script_pubkey, funding.script_pubkey());
    }

    #[test]
    fn change_outputs_are_included_sent_outputs_are_not() {
        let change = addr(2);
        let counterparty = addr(3);
        let rec = record_with_outputs(
            2,
            &[
                (5_000, change.clone(), OutputRole::Change),
                (9_000, counterparty, OutputRole::Sent),
            ],
        );

        let (_, owned) =
            received_outputs_for_record(&rec).expect("a change output yields a payload");

        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].2, change);
    }

    #[test]
    fn pure_outgoing_transaction_yields_no_payload() {
        let counterparty = addr(4);
        let rec = record_with_outputs(3, &[(7_000, counterparty, OutputRole::Sent)]);
        assert!(received_outputs_for_record(&rec).is_none());
    }

    #[test]
    fn incoming_candidates_surface_received_outputs() {
        let recv = addr(5);
        let rec = record_with_outputs(5, &[(42_000, recv.clone(), OutputRole::Received)]);

        let candidates = incoming_payment_candidates(&rec);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].address, recv.to_string());
        assert_eq!(candidates[0].amount_duffs, 42_000);
        assert_eq!(candidates[0].txid, rec.txid.to_string());
        assert_eq!(candidates[0].vout, 0);
    }

    #[test]
    fn two_received_outputs_carry_distinct_vouts() {
        // One transaction paying two different contact addresses must yield two
        // candidates whose vouts differ — otherwise downstream keying by
        // (txid, vout) collapses them and one payment is lost.
        let recv_a = addr(5);
        let recv_b = addr(6);
        let rec = record_with_outputs(
            7,
            &[
                (11_000, recv_a.clone(), OutputRole::Received),
                (22_000, recv_b.clone(), OutputRole::Received),
            ],
        );

        let candidates = incoming_payment_candidates(&rec);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].vout, 0);
        assert_eq!(candidates[1].vout, 1);
        assert_ne!(candidates[0].vout, candidates[1].vout);
        assert_eq!(candidates[0].amount_duffs, 11_000);
        assert_eq!(candidates[1].amount_duffs, 22_000);
    }

    #[test]
    fn incoming_candidates_exclude_change_and_sent() {
        let recv = addr(6);
        let change = addr(7);
        let counterparty = addr(8);
        let rec = record_with_outputs(
            6,
            &[
                (10_000, recv.clone(), OutputRole::Received),
                (3_000, change, OutputRole::Change),
                (9_000, counterparty, OutputRole::Sent),
            ],
        );

        let candidates = incoming_payment_candidates(&rec);

        // Only the Received output is a contact-payment candidate; change
        // lands on our own address and sent leaves the wallet.
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].address, recv.to_string());
    }

    #[test]
    fn pure_outgoing_transaction_yields_no_incoming_candidates() {
        let counterparty = addr(9);
        let rec = record_with_outputs(9, &[(7_000, counterparty, OutputRole::Sent)]);
        assert!(incoming_payment_candidates(&rec).is_empty());
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
