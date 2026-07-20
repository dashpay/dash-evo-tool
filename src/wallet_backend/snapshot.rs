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
//! contrast, are read straight off the live wallet — and always from the *same*
//! read, so the headline balance and the per-address breakdown a snapshot
//! publishes are one consistent generation (see [`SnapshotStore::recompute`]).

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use arc_swap::ArcSwap;
use dash_sdk::dpp::dashcore::{Address, OutPoint, ScriptBuf, Transaction, TxOut, Txid};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
    OutputRole, TransactionRecord,
};
use dash_sdk::dpp::key_wallet::transaction_checking::TransactionContext;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use platform_wallet::PlatformWallet;

use crate::backend_task::error::TaskError;
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
    /// Feeds the account-summary view.
    pub address_balances: BTreeMap<Address, u64>,
    /// The BIP-44 external (receive) addresses SPV actually watches, as strings.
    /// The Receive flow renders this set, so it can only ever show, copy, or QR
    /// an address inside the watched gap-limit window — never a legacy DET-side
    /// index past it. Lock-free read on the UI hot path.
    pub monitored_receive_addresses: Vec<String>,
    /// Authoritative derivation path for every address the upstream wallet has
    /// generated, across all accounts and pools. Lets the account-summary view
    /// categorize funded addresses that DET's own `watched_addresses`
    /// bookkeeping has not indexed yet, so no funded address is dropped from the
    /// per-category tab totals.
    pub address_paths: BTreeMap<Address, DerivationPath>,
    /// Whether persisted transaction history was fully restored at startup.
    pub transaction_history_status: TransactionHistoryStatus,
}

/// Startup restoration state for the display-only transaction history.
#[derive(Debug, Clone, Default)]
pub enum TransactionHistoryStatus {
    #[default]
    Complete,
    Partial {
        skipped_rows: usize,
        error: Arc<TaskError>,
    },
    Unavailable {
        error: Arc<TaskError>,
    },
}

impl PartialEq for TransactionHistoryStatus {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Complete, Self::Complete) => true,
            (
                Self::Partial {
                    skipped_rows: left, ..
                },
                Self::Partial {
                    skipped_rows: right,
                    ..
                },
            ) => left == right,
            (Self::Unavailable { .. }, Self::Unavailable { .. }) => true,
            _ => false,
        }
    }
}

impl Eq for TransactionHistoryStatus {}

impl TransactionHistoryStatus {
    pub fn error(&self) -> Option<&Arc<TaskError>> {
        match self {
            Self::Complete => None,
            Self::Partial { error, .. } | Self::Unavailable { error } => Some(error),
        }
    }
}

/// Chain-derived parts of a published snapshot — everything except the
/// event-sourced transaction history, which [`SnapshotStore::publish`]
/// assembles from the tx log. Bundling these keeps `publish` under the
/// argument limit and makes the carry-forward-on-contention path explicit.
struct SnapshotState {
    balance: DetWalletBalance,
    utxos: Vec<DetUtxo>,
    address_balances: BTreeMap<Address, u64>,
    monitored_receive_addresses: Vec<String>,
    address_paths: BTreeMap<Address, DerivationPath>,
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
        fee: transaction_fee(record),
        label: Some(record.label.clone()).filter(|s| !s.is_empty()),
        // Per-wallet history — every record involves our addresses.
        is_ours: true,
        status: status_from_context(&record.context),
    }
}

fn transaction_fee(record: &TransactionRecord) -> Option<u64> {
    if record.fee.is_some() {
        return record.fee;
    }
    if record.transaction.input.is_empty()
        || record.input_details.len() != record.transaction.input.len()
        || record
            .input_details
            .iter()
            .enumerate()
            .any(|(index, detail)| detail.index as usize != index)
    {
        return None;
    }

    let input_total = record
        .input_details
        .iter()
        .try_fold(0u64, |total, input| total.checked_add(input.value))?;
    let output_total = record
        .transaction
        .output
        .iter()
        .try_fold(0u64, |total, output| total.checked_add(output.value))?;
    input_total.checked_sub(output_total)
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

/// The wallet's BIP-44 external (receive) addresses — the SPV-watched gap-limit
/// window — read from the managed wallet info.
///
/// Reads the standard BIP-44 account's external pool the same way the upstream
/// `account_address_pools_blocking` accessor does. The caller derefs the
/// non-blocking `try_state()` guard to its `core_wallet`, so the event callback
/// never blocks.
fn external_addresses_from_info(
    info: &dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> Vec<String> {
    use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

    let standard = AccountType::Standard {
        index: 0,
        standard_account_type: StandardAccountType::BIP44Account,
    };
    info.accounts
        .all_accounts()
        .into_iter()
        .find(|a| a.managed_account_type().to_account_type() == standard)
        .map(|account| {
            account
                .managed_account_type()
                .address_pools()
                .into_iter()
                .filter(|pool| pool.is_external())
                .flat_map(|pool| pool.all_addresses())
                .map(|addr| addr.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The derivation path of every address the wallet has generated, across all
/// accounts and every pool (external and internal), including the DIP-17
/// platform-payment pool.
///
/// Read straight off each pool's [`AddressInfo`](dash_sdk::dpp::key_wallet::managed_account::address_pool::AddressInfo)
/// (`address → path`). This is the authoritative set of addresses the wallet
/// owns: the account-summary view categorizes funded addresses through it, and
/// the send-autocomplete offers only addresses that appear here.
///
/// Platform-payment accounts hold credits rather than Core UTXOs, so upstream
/// keys them separately and `all_accounts()` — which yields only Core-chain
/// accounts — omits them. They must be walked through `all_platform_accounts()`
/// or their addresses never reach the snapshot.
fn address_paths_from_info(
    info: &dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> BTreeMap<Address, DerivationPath> {
    let mut paths = BTreeMap::new();
    let mut index_pool =
        |pool: &dash_sdk::dpp::key_wallet::managed_account::address_pool::AddressPool| {
            for entry in pool.addresses.values() {
                paths.insert(entry.address.clone(), entry.path.clone());
            }
        };
    for account in info.accounts.all_accounts() {
        for pool in account.managed_account_type().address_pools() {
            index_pool(pool);
        }
    }
    for account in info.accounts.all_platform_accounts() {
        index_pool(&account.addresses);
    }
    paths
}

/// Build the carry-forward chain state used when `try_state()` is contended:
/// the **entire** prior snapshot — balance included — so the published snapshot
/// stays internally consistent (header total and per-address breakdown from one
/// generation). Refreshing balance alone here while carrying the breakdown
/// stale is exactly the split-source divergence [`SnapshotStore::recompute`]
/// avoids.
fn carried_forward_state(prior: &WalletSnapshot) -> SnapshotState {
    SnapshotState {
        balance: prior.balance,
        utxos: prior.utxos.clone(),
        address_balances: prior.address_balances.clone(),
        monitored_receive_addresses: prior.monitored_receive_addresses.clone(),
        address_paths: prior.address_paths.clone(),
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
    transaction_history_status: Mutex<HashMap<WalletId, TransactionHistoryStatus>>,
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
            transaction_history_status: Mutex::new(HashMap::new()),
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
        if let Ok(mut statuses) = self.transaction_history_status.lock() {
            statuses.remove(wallet_id);
        }
    }

    /// Resolve an upstream `WalletId` to DET's `WalletSeedHash`, if the wallet
    /// is registered. The shielded sync-completed event is keyed by `WalletId`
    /// (network-scoped, upstream-native), but DET's balance snapshot is keyed by
    /// `WalletSeedHash`, so the `EventBridge` maps through this registry.
    pub(super) fn seed_hash_for(&self, wallet_id: &WalletId) -> Option<WalletSeedHash> {
        self.registered
            .lock()
            .ok()
            .and_then(|map| map.get(wallet_id).map(|r| r.seed_hash))
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

    /// Seed persisted history without overwriting a record already observed
    /// live during registration. Later live accumulation replaces by txid.
    pub(super) fn hydrate_transactions<'a, I>(&self, wallet_id: &WalletId, records: I)
    where
        I: IntoIterator<Item = &'a TransactionRecord>,
    {
        let Ok(mut log) = self.tx_log.lock() else {
            return;
        };
        let per_wallet = log.entry(*wallet_id).or_default();
        for record in records {
            per_wallet
                .entry(record.txid)
                .or_insert_with(|| map_transaction_record(record));
        }
    }

    pub(super) fn set_transaction_history_status(
        &self,
        wallet_id: WalletId,
        status: TransactionHistoryStatus,
    ) {
        if let Ok(mut statuses) = self.transaction_history_status.lock() {
            statuses.insert(wallet_id, status);
        }
    }

    /// Upgrade a previously-accumulated record's status to
    /// `InstantSendLocked`.
    ///
    /// `WalletEvent::TransactionInstantLocked` carries only a `txid`, not a
    /// full `TransactionRecord` (upstream has already recorded the tx
    /// off-chain via `TransactionDetected`), so this mutates the tracked
    /// entry in place rather than going through
    /// [`Self::accumulate_transactions`]. A no-op if the txid isn't tracked
    /// yet, or if it already carries a stronger status — a block
    /// confirmation may race the InstantLock notification, and this must
    /// never regress `Confirmed`/`ChainLocked` back to `InstantSendLocked`.
    pub(super) fn mark_instant_locked(&self, wallet_id: &WalletId, txid: Txid) {
        let Ok(mut log) = self.tx_log.lock() else {
            return;
        };
        if let Some(tx) = log.get_mut(wallet_id).and_then(|m| m.get_mut(&txid))
            && tx.status < TransactionStatus::InstantSendLocked
        {
            tx.status = TransactionStatus::InstantSendLocked;
        }
    }

    /// Read a single tracked record's current status. Test-only seam for
    /// asserting the `EventBridge` → `SnapshotStore` upgrade path without a
    /// full registered-wallet publish.
    #[cfg(test)]
    pub(super) fn transaction_status(
        &self,
        wallet_id: &WalletId,
        txid: &Txid,
    ) -> Option<TransactionStatus> {
        self.tx_log.lock().ok().and_then(|log| {
            log.get(wallet_id)
                .and_then(|m| m.get(txid))
                .map(|tx| tx.status)
        })
    }

    #[cfg(test)]
    pub(super) fn transaction_count(&self, wallet_id: &WalletId) -> usize {
        self.tx_log
            .lock()
            .ok()
            .and_then(|log| log.get(wallet_id).map(BTreeMap::len))
            .unwrap_or_default()
    }

    /// Recompute and atomically publish the snapshot for one wallet, off the
    /// non-blocking UTXO state plus the event-sourced tx log. Called by the
    /// `EventBridge` after it has accumulated the event's records.
    ///
    /// Never blocks and never awaits: `try_state()` is a non-blocking try-lock
    /// that yields `None` under contention, in which case the *entire* prior
    /// snapshot is carried forward and a subsequent event recomputes once the
    /// lock is free.
    ///
    /// # Balance / breakdown consistency
    ///
    /// The headline balance and the UTXO-derived per-address breakdown are read
    /// from the **same** `try_state()` guard — `state.balance()` (the stored
    /// [`WalletCoreBalance`], refreshed alongside every UTXO mutation under the
    /// wallet-manager write lock) and `state.utxos()` reflect one and the same
    /// generation of wallet state. So the wallet-header total
    /// ([`WalletSnapshot::balance`]`.total`) and the per-account tab breakdown
    /// (summed from [`WalletSnapshot::address_balances`]) are always computed
    /// from the same underlying data — a fresh balance is never spliced onto a
    /// stale breakdown. This deliberately does *not* read the lock-free
    /// `wallet.balance()` atomics: those are maintained by a separate event
    /// handler that can lag the state under its own map contention, which is
    /// exactly the split-source divergence this method avoids. On contention we
    /// carry the whole prior (already-consistent) snapshot forward rather than
    /// refreshing balance alone.
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

        // Non-blocking read. Balance and UTXO breakdown come from the same
        // guard (see the consistency note above). On contention, carry the
        // entire prior snapshot forward — balance included — so every field of
        // the published snapshot reflects one consistency point.
        let state = match wallet.try_state() {
            Some(state) => {
                let core_balance = state.balance();
                let balance = DetWalletBalance {
                    confirmed: core_balance.confirmed(),
                    unconfirmed: core_balance.unconfirmed(),
                    total: core_balance.total(),
                };
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
                SnapshotState {
                    balance,
                    utxos,
                    address_balances,
                    monitored_receive_addresses: external_addresses_from_info(&state.core_wallet),
                    address_paths: address_paths_from_info(&state.core_wallet),
                }
            }
            // Accepted, self-healing edge case: if this is the very first
            // recompute for the wallet and it loses the try-lock, there is no
            // prior snapshot, so `snapshot()` returns the all-zero default and
            // `publish` still marks the wallet as having a snapshot. A wallet
            // with genuine prior funds (app restart on an existing seed) can
            // then render as "0 DASH, synced" for a single event cycle instead
            // of "syncing". The next event wins the lock and publishes the real
            // balance, correcting it.
            None => carried_forward_state(&self.snapshot(&seed_hash)),
        };

        self.publish(&seed_hash, wallet_id, state);
    }

    /// Assemble the event-sourced tx history with the freshly-read
    /// balance/UTXO state and atomically publish the snapshot. Split from
    /// [`Self::recompute`] so the publish + tx-log assembly is unit-testable
    /// without a live `PlatformWallet`.
    fn publish(&self, seed_hash: &WalletSeedHash, wallet_id: &WalletId, state: SnapshotState) {
        let transactions: Vec<WalletTransaction> = self
            .tx_log
            .lock()
            .ok()
            .and_then(|log| log.get(wallet_id).map(|m| m.values().cloned().collect()))
            .unwrap_or_default();

        let snapshot = Arc::new(WalletSnapshot {
            balance: state.balance,
            transactions,
            utxos: state.utxos,
            address_balances: state.address_balances,
            monitored_receive_addresses: state.monitored_receive_addresses,
            address_paths: state.address_paths,
            transaction_history_status: self
                .transaction_history_status
                .lock()
                .ok()
                .and_then(|statuses| statuses.get(wallet_id).cloned())
                .unwrap_or_default(),
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
        InputDetail, OutputDetail, OutputRole, TransactionDirection, TransactionRecord,
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
            SnapshotState {
                balance: DetWalletBalance::default(),
                utxos: Vec::new(),
                address_balances: BTreeMap::new(),
                monitored_receive_addresses: Vec::new(),
                address_paths: BTreeMap::new(),
            },
        );
    }

    #[test]
    fn empty_store_yields_default_snapshot() {
        let store = SnapshotStore::new();
        let snap = store.snapshot(&seed(1));
        assert_eq!(snap.balance, DetWalletBalance::default());
        assert!(snap.transactions.is_empty());
        assert!(snap.utxos.is_empty());
        // Pre-sync: no watched receive set is published yet.
        assert!(snap.monitored_receive_addresses.is_empty());
    }

    /// FUNDS-SAFETY (display list): the Receive list is sourced from the
    /// snapshot's `monitored_receive_addresses` — the SPV-watched set published
    /// off the event-bridge recompute. Publishing a watched set makes it the
    /// only set the read accessor returns, so the rendered list ⊆ watched set by
    /// construction (it shows nothing else). Pins the round-trip the UI relies
    /// on: the Receive list can never surface an unwatched index with Copy + QR.
    #[test]
    fn published_monitored_set_is_the_only_receive_list_source() {
        let store = SnapshotStore::new();
        let watched = vec!["yWatched1".to_string(), "yWatched2".to_string()];

        store.publish(
            &seed(9),
            &wid(9),
            SnapshotState {
                balance: DetWalletBalance::default(),
                utxos: Vec::new(),
                address_balances: BTreeMap::new(),
                monitored_receive_addresses: watched.clone(),
                address_paths: BTreeMap::new(),
            },
        );

        let snap = store.snapshot(&seed(9));
        assert_eq!(
            snap.monitored_receive_addresses, watched,
            "the read accessor returns exactly the published watched set"
        );
        // Every address the Receive list would render comes from this set —
        // there is no other source, so the rendered set ⊆ watched set.
        for addr in &snap.monitored_receive_addresses {
            assert!(watched.contains(addr));
        }
    }

    /// The `external_addresses_from_info` filter — the real seam the
    /// recompute uses — extracts exactly the standard BIP-44 account's external
    /// (receive) pool. Exercised against a real upstream `ManagedWalletInfo`, not
    /// a hand-injected vec: this is what publishes the watched receive set the
    /// Receive list renders.
    #[test]
    fn external_addresses_from_info_returns_the_standard_external_pool() {
        use dash_sdk::dpp::dashcore::Address;
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        let wallet =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("upstream wallet");
        let info = ManagedWalletInfo::from_wallet(&wallet, 1);

        let external = external_addresses_from_info(&info);

        // The standard BIP-44 external pool is bootstrapped to the gap limit, so
        // the filter must return a non-empty set of decodable testnet addresses.
        assert!(
            !external.is_empty(),
            "the external pool must yield the bootstrapped receive addresses"
        );
        for addr in &external {
            let parsed = addr
                .parse::<Address<_>>()
                .expect("a decodable address")
                .require_network(network);
            assert!(parsed.is_ok(), "every address is on the active network");
        }
    }

    /// `address_paths_from_info` must surface a derivation path for every
    /// generated address — the account-summary view categorizes funded
    /// addresses through this map, so a missing entry silently drops funds from
    /// the per-category totals. Every external (receive) address must appear.
    #[test]
    fn address_paths_from_info_covers_the_generated_addresses() {
        use dash_sdk::dpp::dashcore::Address;
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        let wallet =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("upstream wallet");
        let info = ManagedWalletInfo::from_wallet(&wallet, 1);

        let paths = address_paths_from_info(&info);
        assert!(
            !paths.is_empty(),
            "generated addresses must carry derivation paths"
        );

        for addr in external_addresses_from_info(&info) {
            let parsed = addr
                .parse::<Address<_>>()
                .expect("a decodable address")
                .require_network(network)
                .expect("address on the active network");
            assert!(
                paths.contains_key(&parsed),
                "external receive address {addr} must have a derivation path"
            );
        }
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
    fn outgoing_transaction_fee_is_derived_from_known_inputs() {
        use dash_sdk::dpp::dashcore::TxIn;

        let source = addr(10);
        let destination = addr(11);
        let mut tx = tx_with(10);
        tx.input.push(TxIn::default());
        tx.output.push(TxOut {
            value: 9_700,
            script_pubkey: destination.script_pubkey(),
        });
        let record = TransactionRecord::new(
            tx,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            vec![InputDetail {
                index: 0,
                value: 10_000,
                address: source,
            }],
            vec![OutputDetail {
                index: 0,
                role: OutputRole::Sent,
                address: Some(destination),
                value: 9_700,
            }],
            -10_000,
        );

        assert_eq!(map_transaction_record(&record).fee, Some(300));
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

    /// The bug this fix kills: an InstantLock notification (which carries
    /// only a `txid`, not a `TransactionRecord`) must still upgrade the
    /// already-tracked mempool record's status, not leave it `Unconfirmed`
    /// forever.
    #[test]
    fn mark_instant_locked_upgrades_a_pending_record() {
        let store = SnapshotStore::new();
        let rec = record(1, 500);
        let txid = rec.txid;
        store.accumulate_transactions(&wid(4), [&rec]);

        store.mark_instant_locked(&wid(4), txid);
        publish_tx_only(&store, seed(4), wid(4));

        let snap = store.snapshot(&seed(4));
        assert_eq!(snap.transactions.len(), 1);
        assert_eq!(
            snap.transactions[0].status,
            TransactionStatus::InstantSendLocked
        );
    }

    /// A block confirmation may race the InstantLock notification. If the
    /// stronger status already landed first, the InstantLock must not
    /// regress it back to `InstantSendLocked`.
    #[test]
    fn mark_instant_locked_does_not_downgrade_a_stronger_status() {
        let store = SnapshotStore::new();
        let mut rec = record(2, 300);
        rec.context = TransactionContext::InChainLockedBlock(BlockInfo::new(
            10,
            BlockHash::from_byte_array([0u8; 32]),
            123,
        ));
        let txid = rec.txid;
        store.accumulate_transactions(&wid(5), [&rec]);

        store.mark_instant_locked(&wid(5), txid);
        publish_tx_only(&store, seed(5), wid(5));

        let snap = store.snapshot(&seed(5));
        assert_eq!(snap.transactions[0].status, TransactionStatus::ChainLocked);
    }

    /// An InstantLock for a wallet/txid combination the log hasn't seen yet
    /// (e.g. events arriving out of order) must not panic and must not
    /// fabricate a phantom entry.
    #[test]
    fn mark_instant_locked_for_untracked_txid_is_a_noop() {
        let store = SnapshotStore::new();
        let rec = record(1, 1);
        store.accumulate_transactions(&wid(6), [&rec]);

        store.mark_instant_locked(&wid(9), Txid::all_zeros());
        publish_tx_only(&store, seed(6), wid(6));

        assert_eq!(store.snapshot(&seed(6)).transactions.len(), 1);
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
    fn unavailable_history_status_is_published_with_the_wallet_snapshot() {
        let store = SnapshotStore::new();
        store.set_transaction_history_status(
            wid(8),
            TransactionHistoryStatus::Unavailable {
                error: Arc::new(TaskError::WalletTransactionHistoryPartial {
                    source:
                        crate::backend_task::error::WalletTransactionHistoryError::RowsSkipped {
                            skipped_rows: 1,
                        },
                }),
            },
        );

        publish_tx_only(&store, seed(8), wid(8));

        assert!(matches!(
            store.snapshot(&seed(8)).transaction_history_status,
            TransactionHistoryStatus::Unavailable { .. }
        ));
    }

    /// A published snapshot whose header total equals the sum of its
    /// per-address breakdown — the consistency invariant the wallets screen
    /// relies on (`core_balance_duffs` reads `.balance.total`; the Core tab sums
    /// `.address_balances`).
    fn consistent_snapshot() -> WalletSnapshot {
        let a = addr(11);
        let b = addr(12);
        let mut address_balances = BTreeMap::new();
        address_balances.insert(a.clone(), 1_000u64);
        address_balances.insert(b.clone(), 5_000u64);
        let mut address_paths = BTreeMap::new();
        address_paths.insert(a.clone(), DerivationPath::from(Vec::new()));
        address_paths.insert(b.clone(), DerivationPath::from(Vec::new()));
        WalletSnapshot {
            balance: DetWalletBalance {
                confirmed: 6_000,
                unconfirmed: 0,
                total: 6_000,
            },
            transactions: Vec::new(),
            utxos: vec![DetUtxo {
                outpoint: OutPoint::null(),
                value: 1_000,
                script_pubkey: a.script_pubkey(),
                address: a,
            }],
            address_balances,
            monitored_receive_addresses: vec!["yWatched".to_string()],
            address_paths,
            transaction_history_status: TransactionHistoryStatus::Complete,
        }
    }

    /// QA-001: on `try_state()` contention the recompute carries the **entire**
    /// prior snapshot forward — balance included — so the header total and the
    /// per-address breakdown never split across generations. Refreshing balance
    /// alone here (the old behavior) spliced a fresh total onto a stale
    /// breakdown; this pins the whole-snapshot carry so that regression cannot
    /// return. (The live lock race itself is only reachable through the
    /// network-backed backend-e2e harness; this exercises the exact state the
    /// contention branch publishes.)
    #[test]
    fn contention_carries_whole_prior_snapshot_keeping_total_and_breakdown_consistent() {
        let prior = consistent_snapshot();
        let breakdown_sum: u64 = prior.address_balances.values().sum();
        assert_eq!(
            prior.balance.total, breakdown_sum,
            "precondition: the prior snapshot is itself consistent"
        );

        let carried = carried_forward_state(&prior);

        // Balance is carried, never freshly re-read: it still matches the
        // breakdown it was published with.
        assert_eq!(
            carried.balance, prior.balance,
            "the header total is carried from the prior generation, not refreshed alone"
        );
        let carried_sum: u64 = carried.address_balances.values().sum();
        assert_eq!(
            carried.balance.total, carried_sum,
            "header total and per-address breakdown stay from one generation under contention"
        );

        // Every other field is carried verbatim too.
        assert_eq!(carried.address_balances, prior.address_balances);
        assert_eq!(carried.utxos, prior.utxos);
        assert_eq!(
            carried.monitored_receive_addresses,
            prior.monitored_receive_addresses
        );
        assert_eq!(carried.address_paths, prior.address_paths);
    }

    /// The same invariant proven through the store's publish/read round-trip:
    /// publishing the contention carry-forward of a consistent snapshot leaves
    /// the readable snapshot consistent (total == breakdown sum) and unchanged.
    #[test]
    fn published_contention_carry_forward_stays_consistent() {
        let store = SnapshotStore::new();
        store.publish(
            &seed(11),
            &wid(11),
            carried_forward_state(&consistent_snapshot()),
        );

        // A contention recompute republishes the carry-forward of what's stored.
        let prior = store.snapshot(&seed(11));
        store.publish(&seed(11), &wid(11), carried_forward_state(&prior));

        let snap = store.snapshot(&seed(11));
        let breakdown_sum: u64 = snap.address_balances.values().sum();
        assert_eq!(
            snap.balance.total, breakdown_sum,
            "header total still equals the per-address breakdown after carry-forward"
        );
        assert_eq!(snap.balance, prior.balance);
        assert_eq!(snap.address_balances, prior.address_balances);
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

    /// A BIP-44 external path `m/44'/1'/acct'/0/0` on testnet.
    fn bip44_account_path(acct: u32) -> DerivationPath {
        use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
        DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: acct },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ])
    }

    /// QA-003: the wallets-screen header total and the Core-tab breakdown must
    /// reconcile through the **real** read accessors — not hand-fed literals
    /// compared to themselves.
    ///
    /// Publishes a realistic snapshot through the same `publish` seam
    /// `recompute` uses. Its `balance.total` is the exact field
    /// `WalletBackend::wallet_balance().total` returns and the header renders;
    /// its `address_balances` / `address_paths` are the exact accessors the Core
    /// tabs feed to `collect_account_summaries`. Crucially the snapshot includes
    /// a funded address the generated-path set has NOT indexed, and `total`
    /// counts it — so the assertion "Σ per-account summaries == header total"
    /// only holds if `collect_account_summaries` surfaces that stray balance
    /// (pass 2) rather than dropping it. That is the header-vs-tab agreement the
    /// original tautological test claimed but never checked.
    #[test]
    fn header_total_reconciles_with_core_tab_breakdown_through_real_accessors() {
        use crate::model::wallet::DerivationPathReference;
        use crate::ui::state::account_summary::{AccountCategory, collect_account_summaries};

        // account #0: two receive addresses (3.5 DASH); account #1: one (0.5).
        let a0 = addr(20);
        let a0b = addr(21);
        let a1 = addr(22);
        // A funded address outside the generated-path window (0.07 DASH).
        let stray = addr(23);

        let mut address_balances = BTreeMap::new();
        address_balances.insert(a0.clone(), 250_000_000u64);
        address_balances.insert(a0b.clone(), 100_000_000u64);
        address_balances.insert(a1.clone(), 50_000_000u64);
        address_balances.insert(stray, 7_000_000u64);
        // `address_balances` is summed from `state.utxos()` (all UTXOs), so this
        // total mirrors what `state.balance().total()` reports at the same
        // generation — the QA-001 consistency the header relies on.
        let total: u64 = address_balances.values().sum();

        // Only the generated addresses carry paths (the stray one does not).
        let mut address_paths = BTreeMap::new();
        address_paths.insert(a0, bip44_account_path(0));
        address_paths.insert(a0b, bip44_account_path(0));
        address_paths.insert(a1, bip44_account_path(1));

        let store = SnapshotStore::new();
        store.publish(
            &seed(20),
            &wid(20),
            SnapshotState {
                balance: DetWalletBalance {
                    confirmed: total,
                    unconfirmed: 0,
                    total,
                },
                utxos: Vec::new(),
                address_balances: address_balances.clone(),
                monitored_receive_addresses: Vec::new(),
                address_paths: address_paths.clone(),
            },
        );

        let snap = store.snapshot(&seed(20));
        // The exact value `WalletBackend::wallet_balance().total` returns.
        let header_total = snap.balance.total;
        assert_eq!(header_total, 407_000_000);

        // The exact accessors the Core tabs read.
        let summaries = collect_account_summaries(
            Network::Testnet,
            &snap.address_balances,
            &snap.address_paths,
        );
        let core_tab_sum: u64 = summaries.iter().map(|s| s.confirmed_balance).sum();
        assert_eq!(
            core_tab_sum, header_total,
            "every funded address is reconciled into the header total — none dropped"
        );

        // Per-account distinctness (the original test's intent, now proven
        // against the real header total rather than a hand-fed literal).
        let acct0 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44 && s.index == Some(0))
            .expect("account #0 summary");
        assert_eq!(acct0.confirmed_balance, 350_000_000);
        let acct1 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44 && s.index == Some(1))
            .expect("account #1 summary");
        assert_eq!(acct1.confirmed_balance, 50_000_000);
        // The stray funded address is surfaced (pass 2), never dropped.
        let other = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Other(DerivationPathReference::Unknown))
            .expect("stray funded address surfaced as Other");
        assert_eq!(other.confirmed_balance, 7_000_000);
    }

    /// The DIP-17 platform-payment pool must reach the snapshot. Upstream keys
    /// it separately from `all_accounts()` (which yields only Core-chain
    /// accounts), so `address_paths_from_info` has to walk
    /// `all_platform_accounts()` as well. Without it the send-autocomplete's
    /// Platform branch has no source and silently offers nothing.
    ///
    /// `WalletAccountCreationOptions::Default` — the option DET registers every
    /// wallet with — creates platform-payment account `(0, 0)`, so a default
    /// wallet always has this pool.
    #[test]
    fn address_paths_include_the_platform_payment_pool() {
        use crate::model::wallet::DerivationPathHelpers;
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        let wallet =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("upstream wallet");
        let info = ManagedWalletInfo::from_wallet(&wallet, 1);

        let paths = address_paths_from_info(&info);

        assert!(
            paths.values().any(|p| p.is_platform_payment(network)),
            "the DIP-17 platform-payment pool must carry paths into the snapshot"
        );
    }

    /// The platform-payment pool joining the generated-path set makes a
    /// `PlatformPayment` summary appear — the state `plan_account_tabs`'s dedup
    /// guard exists for. Pins the categorization here; the tab-dedup assertion
    /// lives with the planner in `wallets_screen`.
    #[test]
    fn platform_payment_pool_categorizes_as_platform_payment() {
        use crate::ui::state::account_summary::{AccountCategory, collect_account_summaries};
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        let wallet =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("upstream wallet");
        let info = ManagedWalletInfo::from_wallet(&wallet, 1);

        let summaries =
            collect_account_summaries(network, &BTreeMap::new(), &address_paths_from_info(&info));

        assert!(
            summaries
                .iter()
                .any(|s| s.category == AccountCategory::PlatformPayment),
            "the platform-payment pool must categorize as PlatformPayment"
        );
    }
}
