//! Bridges upstream `platform-wallet` events into DET's frame loop.
//!
//! `EventBridge` implements `platform_wallet::PlatformEventHandler` (and its
//! `dash_spv::EventHandler` supertrait). Upstream owns chain sync; this is the
//! only path by which sync/wallet state changes reach DET. Each callback is
//! sync and must not block — it updates [`ConnectionStatus`] atomics and
//! nudges the frame loop with a non-blocking `TaskResult::Refresh`. The
//! visible-screen `display_task_result` / `refresh` then re-reads state
//! through `WalletBackend` accessors, exactly as the old reconcile path did.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dash_sdk::dash_spv::network::NetworkEvent;
use dash_sdk::dash_spv::sync::{SyncEvent, SyncProgress, SyncState};
use platform_wallet::events::{EventHandler, PlatformEventHandler, WalletEvent};
use platform_wallet::manager::platform_address_sync::{
    PlatformAddressSyncSummary, WalletSyncOutcome,
};
use platform_wallet::manager::shielded_sync::{ShieldedSyncPassSummary, WalletShieldedOutcome};

use super::coordinator_gate::CoordinatorGate;
use super::snapshot::{SnapshotStore, incoming_payment_candidates, received_outputs_for_record};
use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::CoreItem;
use crate::context::connection_status::{
    ConnectionStatus, ShieldedSyncProgress, ShieldedTreeProgress,
};
use crate::model::spv_status::SpvStatus;
use crate::model::wallet::{PlatformAddressEntry, PlatformAddressUpdates, WalletSeedHash};
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::dpp::key_wallet::managed_account::transaction_record::TransactionRecord;

/// DET-authored handler registered with `PlatformWalletManager` at
/// construction. Holds only cheap shared handles so it stays `Send + Sync`
/// and clone-free on the event hot path.
pub struct EventBridge {
    connection_status: Arc<ConnectionStatus>,
    task_result_sender: SenderAsync<TaskResult>,
    snapshots: Arc<SnapshotStore>,
    /// Quorum-readiness gate, shared with `WalletBackend`. Fired here when the
    /// masternode list reaches `Synced` so the Platform/identity coordinators
    /// start only once quorums are resolvable.
    coordinator_gate: Arc<CoordinatorGate>,
    /// Phase-E push writer: AppContext's frame-safe shielded balance snapshot
    /// (credits, keyed by `WalletSeedHash`). Written by
    /// `on_shielded_sync_completed`; read synchronously in the frame loop via
    /// `AppContext::shielded_balance_credits`.
    shielded_balances: Arc<Mutex<HashMap<WalletSeedHash, u64>>>,
    /// Platform-address push writer: AppContext's frame-safe platform balance snapshot
    /// (duffs, keyed by `WalletSeedHash`). Written by
    /// `on_platform_address_sync_completed`; read synchronously in the frame loop via
    /// `AppContext::platform_balance_duffs`. Only OWNED addresses (those whose
    /// coordinator wallet-id matches the registered wallet) contribute to the sum —
    /// no orphan inflation.
    platform_balances: Arc<Mutex<HashMap<WalletSeedHash, u64>>>,
}

impl EventBridge {
    pub(super) fn new(
        connection_status: Arc<ConnectionStatus>,
        task_result_sender: SenderAsync<TaskResult>,
        snapshots: Arc<SnapshotStore>,
        coordinator_gate: Arc<CoordinatorGate>,
        shielded_balances: Arc<Mutex<HashMap<WalletSeedHash, u64>>>,
        platform_balances: Arc<Mutex<HashMap<WalletSeedHash, u64>>>,
    ) -> Self {
        Self {
            connection_status,
            task_result_sender,
            snapshots,
            coordinator_gate,
            shielded_balances,
            platform_balances,
        }
    }

    /// Mark masternodes ready and release the Platform-sync gate when the
    /// masternode-list phase has reached `Synced`. Idempotent — the gate fires
    /// the coordinators at most once and the flag is a cheap atomic store, so
    /// re-checking on every progress event is safe.
    fn check_masternodes_ready(&self, progress: &SyncProgress) {
        // `masternodes()` is `Err` until the manager has started reporting; a
        // not-yet-started or still-syncing phase simply leaves the gate closed.
        if progress
            .masternodes()
            .is_ok_and(|mn| mn.state() == SyncState::Synced)
        {
            self.connection_status.set_masternodes_ready(true);
            self.coordinator_gate.on_masternodes_ready();
        }
    }

    /// Nudge the frame loop. Non-blocking; a full channel is harmless because
    /// `Refresh` is idempotent and the next event coalesces.
    fn nudge_refresh(&self) {
        let _ = self.task_result_sender.try_send(TaskResult::Refresh);
    }

    /// Emit a `ReceivedAvailableUTXOTransaction` for any freshly-seen records
    /// that pay into one of our wallet addresses.
    ///
    /// Best-effort nudge for the Create-Asset-Lock and identity-funding screens:
    /// it fires only for records with a wallet-owned output, so for an asset-lock
    /// tx only when that tx carries a wallet change output. Terminal
    /// `RegisteredIdentity` drives final success regardless. Non-blocking;
    /// records with no wallet-owned outputs are skipped.
    fn emit_received_utxos<'a, I>(&self, records: I)
    where
        I: IntoIterator<Item = &'a TransactionRecord>,
    {
        for record in records {
            if let Some((tx, outpoints_with_addresses)) = received_outputs_for_record(record) {
                let result = BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, outpoints_with_addresses),
                );
                let _ = self
                    .task_result_sender
                    .try_send(TaskResult::Success(Box::new(result)));
            }
        }
    }

    /// Emit a `DashPayIncomingDetected` for the received outputs of freshly-
    /// seen records, so the app can run the detect-match-record path for any
    /// that pay into a DashPay contact-receiving address.
    ///
    /// The matching (and recording) is async and KV-backed, so it cannot run
    /// on this sync event callback — the candidates are forwarded as a typed
    /// `TaskResult` and the backend task does the owner-scoped match. A record
    /// with no received outputs is skipped, so non-incoming transactions never
    /// produce an event.
    fn emit_incoming_payment_candidates<'a, I>(&self, records: I)
    where
        I: IntoIterator<Item = &'a TransactionRecord>,
    {
        let mut candidates = Vec::new();
        for record in records {
            candidates.extend(incoming_payment_candidates(record));
        }
        if candidates.is_empty() {
            return;
        }
        let result = BackendTaskSuccessResult::DashPayIncomingDetected(candidates);
        let _ = self
            .task_result_sender
            .try_send(TaskResult::Success(Box::new(result)));
    }

    fn apply_status(&self, status: SpvStatus) {
        self.connection_status.set_spv_status(status);
        self.connection_status.refresh_state();
    }
}

impl std::fmt::Debug for EventBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBridge").finish_non_exhaustive()
    }
}

impl EventHandler for EventBridge {
    fn on_progress(&self, progress: &SyncProgress) {
        let status = if progress.is_synced() {
            SpvStatus::Running
        } else if progress.state() == SyncState::Error {
            SpvStatus::Error
        } else {
            SpvStatus::Syncing
        };
        // Publish the per-phase heights/targets so the UI can render a
        // determinate progress bar, not just a coarse status label.
        self.connection_status
            .set_spv_sync_progress(Some(progress.clone()));
        // Release the Platform-sync gate once the masternode list is synced —
        // before the (slow) filter/block scan that `is_synced()` waits on.
        self.check_masternodes_ready(progress);
        self.apply_status(status);
        self.nudge_refresh();
    }

    fn on_sync_event(&self, event: &SyncEvent) {
        match event {
            SyncEvent::SyncComplete { .. } => {
                self.apply_status(SpvStatus::Running);
                self.nudge_refresh();
            }
            SyncEvent::ManagerError { manager, error } => {
                // The manager id is a useful log dimension but internal jargon —
                // keep it out of the stored error text the UI surfaces. The raw
                // upstream message is stored verbatim (truncated) and shown only
                // as a tooltip behind a fixed user-facing label.
                tracing::error!(%manager, error, "SPV manager error");
                let limit = error.floor_char_boundary(100);
                self.connection_status
                    .set_spv_last_error(Some(error[..limit].to_string()));
                self.apply_status(SpvStatus::Error);
                self.nudge_refresh();
            }
            SyncEvent::BlockProcessed { .. }
            | SyncEvent::ChainLockReceived { .. }
            | SyncEvent::InstantLockReceived { .. } => {
                // Wallet-relevant chain progress — re-read state next frame.
                self.nudge_refresh();
            }
            _ => {}
        }
    }

    fn on_network_event(&self, event: &NetworkEvent) {
        if let NetworkEvent::PeersUpdated {
            connected_count, ..
        } = event
        {
            self.connection_status
                .set_spv_connected_peers((*connected_count).min(u16::MAX as usize) as u16);
            self.connection_status.refresh_state();
            self.nudge_refresh();
        }
    }

    fn on_wallet_event(&self, event: &WalletEvent) {
        // Accumulate the event's transaction records (upstream drops
        // finalized records from memory — `keep-finalized-transactions` is
        // off — so the snapshot's history is event-sourced), then recompute
        // and publish the affected wallet's display snapshot off the
        // lock-free balance + non-blocking UTXO read. UI re-reads next frame.
        let wallet_id = match event {
            WalletEvent::TransactionDetected {
                wallet_id, record, ..
            } => {
                self.snapshots
                    .accumulate_transactions(wallet_id, std::iter::once(record.as_ref()));
                // A wallet-relevant transaction just appeared off-chain (mempool
                // or direct InstantSend) — surface its received UTXOs so a
                // waiting funding screen advances.
                self.emit_received_utxos(std::iter::once(record.as_ref()));
                // Same first-seen record drives incoming DashPay contact-
                // payment detection.
                self.emit_incoming_payment_candidates(std::iter::once(record.as_ref()));
                *wallet_id
            }
            WalletEvent::BlockProcessed {
                wallet_id,
                inserted,
                updated,
                matured,
                ..
            } => {
                self.snapshots.accumulate_transactions(
                    wallet_id,
                    inserted.iter().chain(updated.iter()).chain(matured.iter()),
                );
                // `inserted` records are first-seen-in-block — a funding tx DET
                // missed during the mempool window. Surface those too so the
                // funding screen still advances on a confirmed-first transaction.
                self.emit_received_utxos(inserted.iter());
                // Detect contact payments first observed at block time (DET
                // was offline during the mempool window).
                self.emit_incoming_payment_candidates(inserted.iter());
                *wallet_id
            }
            WalletEvent::TransactionInstantLocked { wallet_id, .. }
            | WalletEvent::SyncHeightAdvanced { wallet_id, .. } => *wallet_id,
            WalletEvent::ChainLockProcessed { wallet_id, .. } => {
                // Upstream chain-lock notification: no transaction deltas to
                // accumulate, but balances may shift from unconfirmed to
                // confirmed — recompute and nudge the frame loop.
                *wallet_id
            }
        };
        self.snapshots.recompute(&wallet_id);
        self.nudge_refresh();
    }

    fn on_error(&self, error: &str) {
        let limit = error.floor_char_boundary(200);
        self.connection_status
            .set_spv_last_error(Some(error[..limit].to_string()));
        self.apply_status(SpvStatus::Error);
        self.nudge_refresh();
    }
}

impl PlatformEventHandler for EventBridge {
    fn on_platform_address_sync_completed(&self, summary: &PlatformAddressSyncSummary) {
        // PUSH PRODUCER — two outputs per wallet that synced successfully:
        //
        // 1. Frame-safe total-balance snapshot (`platform_balances` Mutex): the
        //    selector reads this synchronously each frame without blocking.
        //
        // 2. `PlatformAddressSyncPushed` task result: `AppState` routes this to
        //    `AppContext::apply_platform_address_push` which populates per-address
        //    `wallet.platform_address_info`, keeping the per-address tab current
        //    without a manual Refresh (fixes the cold-start mismatch).
        //
        // Only OWNED addresses appear in the coordinator result (the provider
        // tracks exactly the wallets registered with the manager), so no orphan
        // inflation is possible. Errored wallets leave both snapshots untouched.
        use crate::app::TaskResult;
        use crate::backend_task::BackendTaskSuccessResult;
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;

        // Derive the per-wallet per-address data once from the pure helper.
        let per_wallet = summary_ok_platform_entries(summary);

        // (1) Update the frame-safe total-balance snapshot.
        if let Ok(mut balances) = self.platform_balances.lock() {
            for (wallet_id, entries) in &per_wallet {
                if let Some(seed_hash) = self.snapshots.seed_hash_for(wallet_id) {
                    // QA-B-003 (accepted): integer division truncates sub-duff amounts.
                    // Loss ≤ (CREDITS_PER_DUFF − 1) per address, i.e. < 0.001 DASH max
                    // per address. This is consistent with the direct-query path and is
                    // intentional — displaying sub-duff precision has no practical value.
                    let total_duffs: u64 = entries
                        .iter()
                        .map(|(_, balance_credits, _)| balance_credits / CREDITS_PER_DUFF)
                        .sum();
                    balances.insert(seed_hash, total_duffs);
                }
            }
        }

        // (2) Emit the per-address update. Build the seed_hash-keyed vec only for
        // wallets that are registered (seed_hash resolved); unregistered wallet_ids
        // (no DET mapping) are silently skipped.
        let push_updates: PlatformAddressUpdates = per_wallet
            .into_iter()
            .filter_map(|(wallet_id, entries)| {
                self.snapshots
                    .seed_hash_for(&wallet_id)
                    .map(|seed_hash| (seed_hash, entries))
            })
            .filter(|(_, entries)| !entries.is_empty())
            .collect();

        if !push_updates.is_empty() {
            let result = BackendTaskSuccessResult::PlatformAddressSyncPushed {
                updates: push_updates,
            };
            // Non-blocking send; a full channel means the coordinator retries in
            // ~15 s. The frame-safe total-balance snapshot (step 1) is already
            // written — only the per-address update is deferred.
            let _ = self
                .task_result_sender
                .try_send(TaskResult::Success(Box::new(result)));
        }

        self.nudge_refresh();
    }

    fn on_platform_event(&self, event: &platform_wallet::events::PlatformEvent) {
        match event {
            platform_wallet::events::PlatformEvent::WalletSkippedOnLoad { wallet_id, reason } => {
                // Public wallet id + structural reason only; never a secret.
                // TODO(PROJ-010-T6): surface a calm MessageBanner ("One saved
                // wallet couldn't be opened. Re-add it from its recovery
                // phrase to restore it.") once the construction path can reach
                // an egui context. The skip is logged here in the meantime and
                // also reported via `LoadedWallets.skipped`.
                tracing::warn!(
                    wallet_id = %hex::encode(wallet_id),
                    %reason,
                    "A saved wallet was skipped on load because its stored data is corrupt"
                );
            }
        }
    }

    fn on_shielded_sync_progress(&self, cumulative_scanned: u64, block_height: u64) {
        // Downloaded-notes progress for an in-flight pass — drives the
        // shielded tab's "scanning" indicator. Network-scoped (one pass covers
        // every viewing key), so it lands on ConnectionStatus, not per-wallet.
        self.connection_status
            .set_shielded_sync_progress(Some(ShieldedSyncProgress {
                cumulative_scanned,
                block_height,
            }));
        self.nudge_refresh();
    }

    fn on_shielded_tree_progress(&self, leaves_committed: u64, total_target: u64) {
        // Committed-to-tree ("checked") progress for an in-flight pass.
        self.connection_status
            .set_shielded_tree_progress(Some(ShieldedTreeProgress {
                leaves_committed,
                total_target,
            }));
        self.nudge_refresh();
    }

    fn on_shielded_sync_completed(&self, summary: &ShieldedSyncPassSummary) {
        // PUSH PRODUCER (Phase E): write each successfully-synced wallet's
        // shielded balance into AppContext's frame-safe snapshot so the frame
        // loop reads the current balance with no blocking coordinator call.
        // The summary is keyed by upstream `WalletId`; map it to DET's
        // `WalletSeedHash` through the snapshot registry. Skipped/errored
        // wallets leave their prior balance untouched.
        if let Ok(mut balances) = self.shielded_balances.lock() {
            for (wallet_id, balance) in summary_ok_balances(summary) {
                if let Some(seed_hash) = self.snapshots.seed_hash_for(&wallet_id) {
                    balances.insert(seed_hash, balance);
                }
            }
        }
        // The pass finished — clear the in-flight progress so the UI stops
        // rendering the "scanning / checking" indicators.
        self.connection_status.set_shielded_sync_progress(None);
        self.connection_status.set_shielded_tree_progress(None);
        self.nudge_refresh();
    }
}

/// Collect per-address `(wallet_id, entries)` for every wallet whose sync
/// completed successfully in `summary`. Each entry carries the raw 20-byte
/// P2PKH hash, credits balance, and nonce for one found address.
///
/// `Err` outcomes are skipped so their cached per-address data is left
/// untouched. Pure — no I/O — so it is unit-testable without a coordinator.
fn summary_ok_platform_entries(
    summary: &PlatformAddressSyncSummary,
) -> Vec<([u8; 32], Vec<PlatformAddressEntry>)> {
    summary
        .wallet_results
        .iter()
        .filter_map(|(wallet_id, outcome)| match outcome {
            WalletSyncOutcome::Ok(result) => {
                let entries: Vec<PlatformAddressEntry> = result
                    .found
                    .iter()
                    .map(|((_, p2pkh), funds)| (p2pkh.to_bytes(), funds.balance, funds.nonce))
                    .collect();
                Some((*wallet_id, entries))
            }
            WalletSyncOutcome::Err(_) => None,
        })
        .collect()
}

/// Collect `(wallet_id, balance_credits)` for every wallet that synced
/// successfully in `summary`. Skipped (no bound shielded sub-wallet) and
/// errored wallets are excluded so their snapshot balance is left untouched.
/// Pure — no I/O — so it is unit-testable without a coordinator or a
/// registered wallet.
fn summary_ok_balances(summary: &ShieldedSyncPassSummary) -> Vec<([u8; 32], u64)> {
    summary
        .wallet_results
        .iter()
        .filter_map(|(wallet_id, outcome)| match outcome {
            WalletShieldedOutcome::Ok(sync) => Some((*wallet_id, sync.balance_total())),
            WalletShieldedOutcome::Skipped | WalletShieldedOutcome::Err(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand for the shielded-balance snapshot handle the helpers wire.
    type ShieldedBalancesHandle = Arc<Mutex<HashMap<WalletSeedHash, u64>>>;
    use crate::utils::egui_mpsc::EguiMpscAsync;
    use dash_sdk::dpp::dashcore::{Address, Network, PublicKey, Transaction, TxOut};
    use dash_sdk::dpp::key_wallet::WalletCoreBalance;
    use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
    use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
        OutputDetail, OutputRole, TransactionDirection,
    };
    use dash_sdk::dpp::key_wallet::transaction_checking::TransactionContext;
    use dash_sdk::dpp::key_wallet::transaction_checking::transaction_router::TransactionType;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn make_bridge() -> (
        EventBridge,
        Arc<ConnectionStatus>,
        tokio::sync::mpsc::Receiver<TaskResult>,
    ) {
        let (bridge, cs, rx, _gate) = make_bridge_with_gate();
        (bridge, cs, rx)
    }

    fn make_bridge_with_gate() -> (
        EventBridge,
        Arc<ConnectionStatus>,
        tokio::sync::mpsc::Receiver<TaskResult>,
        Arc<CoordinatorGate>,
    ) {
        let cs = Arc::new(ConnectionStatus::new());
        let (tx, rx) =
            tokio::sync::mpsc::channel::<TaskResult>(8).with_egui_ctx(egui::Context::default());
        let gate = Arc::new(CoordinatorGate::default());
        let bridge = EventBridge::new(
            Arc::clone(&cs),
            tx,
            Arc::new(SnapshotStore::new()),
            Arc::clone(&gate),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
        );
        (bridge, cs, rx, gate)
    }

    /// Like [`make_bridge`] but also returns the shielded-balances snapshot Arc
    /// so shielded-event tests can assert the push writer's effect.
    fn make_bridge_with_balances() -> (
        EventBridge,
        Arc<ConnectionStatus>,
        tokio::sync::mpsc::Receiver<TaskResult>,
        ShieldedBalancesHandle,
    ) {
        let cs = Arc::new(ConnectionStatus::new());
        let (tx, rx) =
            tokio::sync::mpsc::channel::<TaskResult>(8).with_egui_ctx(egui::Context::default());
        let gate = Arc::new(CoordinatorGate::default());
        let balances = Arc::new(Mutex::new(HashMap::new()));
        let bridge = EventBridge::new(
            Arc::clone(&cs),
            tx,
            Arc::new(SnapshotStore::new()),
            gate,
            Arc::clone(&balances),
            Arc::new(Mutex::new(HashMap::new())),
        );
        (bridge, cs, rx, balances)
    }

    /// Like [`make_bridge`] but also returns the platform-balances snapshot Arc
    /// so platform-address-sync-event tests can assert the push writer's effect.
    fn make_bridge_with_platform_balances() -> (
        EventBridge,
        Arc<ConnectionStatus>,
        tokio::sync::mpsc::Receiver<TaskResult>,
        ShieldedBalancesHandle, // same type: Arc<Mutex<HashMap<WalletSeedHash, u64>>>
    ) {
        let cs = Arc::new(ConnectionStatus::new());
        let (tx, rx) =
            tokio::sync::mpsc::channel::<TaskResult>(8).with_egui_ctx(egui::Context::default());
        let gate = Arc::new(CoordinatorGate::default());
        let platform_balances = Arc::new(Mutex::new(HashMap::new()));
        let bridge = EventBridge::new(
            Arc::clone(&cs),
            tx,
            Arc::new(SnapshotStore::new()),
            gate,
            Arc::new(Mutex::new(HashMap::new())),
            Arc::clone(&platform_balances),
        );
        (bridge, cs, rx, platform_balances)
    }

    fn drained_refresh(rx: &mut tokio::sync::mpsc::Receiver<TaskResult>) -> bool {
        let mut saw_refresh = false;
        while let Ok(r) = rx.try_recv() {
            if matches!(r, TaskResult::Refresh) {
                saw_refresh = true;
            }
        }
        saw_refresh
    }

    /// A funding address paying into our wallet.
    fn funding_address() -> Address {
        let pubkey = PublicKey::from_slice(&[0x02; 33]).unwrap();
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// A `TransactionDetected` record whose single output pays `value` into
    /// `address` (role `Received`) — the funding-payment shape SPV reports.
    fn received_record(address: &Address, value: u64) -> TransactionRecord {
        let tx = Transaction {
            version: 1,
            lock_time: 0,
            input: vec![],
            output: vec![TxOut {
                value,
                script_pubkey: address.script_pubkey(),
            }],
            special_transaction_payload: None,
        };
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
            vec![OutputDetail {
                index: 0,
                role: OutputRole::Received,
                address: Some(address.clone()),
                value,
            }],
            value as i64,
        )
    }

    fn transaction_detected(record: TransactionRecord) -> WalletEvent {
        WalletEvent::TransactionDetected {
            wallet_id: [9u8; 32],
            record: Box::new(record),
            balance: WalletCoreBalance::default(),
            account_balances: BTreeMap::new(),
            addresses_derived: Vec::new(),
        }
    }

    /// Drain the channel and return the addresses of the first
    /// `ReceivedAvailableUTXOTransaction` produced, if any.
    fn drained_received_utxo_addresses(
        rx: &mut tokio::sync::mpsc::Receiver<TaskResult>,
    ) -> Option<Vec<Address>> {
        while let Ok(r) = rx.try_recv() {
            if let TaskResult::Success(result) = r
                && let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(_, outpoints_with_addresses),
                ) = *result
            {
                return Some(
                    outpoints_with_addresses
                        .into_iter()
                        .map(|(_, _, address)| address)
                        .collect(),
                );
            }
        }
        None
    }

    #[test]
    fn sync_complete_sets_running_and_nudges() {
        let (bridge, cs, mut rx) = make_bridge();
        bridge.on_sync_event(&SyncEvent::SyncComplete {
            header_tip: 100,
            cycle: 0,
        });
        assert_eq!(cs.spv_status(), SpvStatus::Running);
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn manager_error_sets_error_status_and_records_message() {
        let (bridge, cs, mut rx) = make_bridge();
        bridge.on_sync_event(&SyncEvent::ManagerError {
            manager: dash_sdk::dash_spv::sync::ManagerIdentifier::BlockHeader,
            error: "boom".to_string(),
        });
        assert_eq!(cs.spv_status(), SpvStatus::Error);
        assert!(cs.spv_last_error().is_some_and(|e| e.contains("boom")));
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn peers_updated_nudges_refresh() {
        let (bridge, _cs, mut rx) = make_bridge();
        bridge.on_network_event(&NetworkEvent::PeersUpdated {
            connected_count: 3,
            addresses: Vec::new(),
            best_height: None,
        });
        // ConnectionStatus has no public peer-count getter; its own tests
        // cover the internal mutation. Here we assert the frame-loop nudge.
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn progress_default_maps_to_syncing() {
        let (bridge, cs, mut rx) = make_bridge();
        bridge.on_progress(&SyncProgress::default());
        // A default (no-manager) progress is neither synced nor errored.
        assert_eq!(cs.spv_status(), SpvStatus::Syncing);
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn progress_publishes_per_phase_heights() {
        use dash_sdk::dash_spv::sync::{BlockHeadersProgress, ProgressPercentage};

        let (bridge, cs, mut rx) = make_bridge();
        let mut headers = BlockHeadersProgress::default();
        headers.set_state(SyncState::Syncing);
        headers.update_target_height(10_000);
        headers.update_tip_height(4_200);
        let mut progress = SyncProgress::default();
        progress.update_headers(headers);

        bridge.on_progress(&progress);

        assert_eq!(cs.spv_status(), SpvStatus::Syncing);
        let stored = cs.spv_sync_progress().expect("progress published");
        let stored_headers = stored.headers().expect("headers phase present");
        assert_eq!(stored_headers.target_height(), 10_000);
        assert_eq!(stored_headers.current_height(), 4_200);
        assert!(drained_refresh(&mut rx));
    }

    /// A `SyncProgress` whose masternode phase is in `state`, headers still
    /// mid-sync (so the overall pipeline is not yet "synced").
    fn progress_with_masternode_state(state: SyncState) -> SyncProgress {
        use dash_sdk::dash_spv::sync::{BlockHeadersProgress, MasternodesProgress};

        let mut headers = BlockHeadersProgress::default();
        headers.set_state(SyncState::Syncing);
        headers.update_target_height(10_000);
        headers.update_tip_height(4_200);

        let mut mn = MasternodesProgress::default();
        mn.set_state(state);

        let mut progress = SyncProgress::default();
        progress.update_headers(headers);
        progress.update_masternodes(mn);
        progress
    }

    #[test]
    fn masternodes_synced_progress_opens_gate_and_sets_flag() {
        let (bridge, cs, mut rx, gate) = make_bridge_with_gate();
        // Arm the gate so it has a coordinator action to run (mirrors
        // `WalletBackend::start`). A bare counter stands in for the real
        // coordinator starts.
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_action = Arc::clone(&calls);
        gate.arm(Box::new(move || {
            calls_for_action.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "coordinators must not start before masternodes are synced"
        );

        bridge.on_progress(&progress_with_masternode_state(SyncState::Synced));

        assert!(
            cs.masternodes_ready(),
            "a synced masternode phase must set the readiness flag"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a synced masternode phase must open the gate and start coordinators once"
        );
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn masternodes_still_syncing_keeps_gate_closed() {
        let (bridge, cs, _rx, gate) = make_bridge_with_gate();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_action = Arc::clone(&calls);
        gate.arm(Box::new(move || {
            calls_for_action.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));

        bridge.on_progress(&progress_with_masternode_state(SyncState::Syncing));

        assert!(
            !cs.masternodes_ready(),
            "a still-syncing masternode phase must not set the readiness flag"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "coordinators must stay closed while masternodes are still syncing"
        );
    }

    #[test]
    fn on_error_sets_error_and_records_message() {
        let (bridge, cs, mut rx) = make_bridge();
        bridge.on_error("network down");
        assert_eq!(cs.spv_status(), SpvStatus::Error);
        assert!(
            cs.spv_last_error()
                .is_some_and(|e| e.contains("network down"))
        );
        assert!(drained_refresh(&mut rx));
    }

    /// Drain the channel and return the first batch of incoming-payment
    /// candidates the bridge produced, if any.
    fn drained_incoming_candidates(
        rx: &mut tokio::sync::mpsc::Receiver<TaskResult>,
    ) -> Option<Vec<crate::model::dashpay::DetectedIncomingOutput>> {
        while let Ok(r) = rx.try_recv() {
            if let TaskResult::Success(result) = r
                && let BackendTaskSuccessResult::DashPayIncomingDetected(candidates) = *result
            {
                return Some(candidates);
            }
        }
        None
    }

    #[test]
    fn transaction_detected_emits_incoming_candidate() {
        let (bridge, _cs, mut rx) = make_bridge();
        let funding = funding_address();

        bridge.on_wallet_event(&transaction_detected(received_record(&funding, 77_000)));

        let candidates =
            drained_incoming_candidates(&mut rx).expect("a received output yields a candidate");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].address, funding.to_string());
        assert_eq!(candidates[0].amount_duffs, 77_000);
    }

    #[test]
    fn block_processed_inserted_emits_incoming_candidate() {
        let (bridge, _cs, mut rx) = make_bridge();
        let funding = funding_address();

        bridge.on_wallet_event(&WalletEvent::BlockProcessed {
            wallet_id: [9u8; 32],
            height: 2_000,
            chain_lock: None,
            inserted: vec![received_record(&funding, 33_000)],
            updated: Vec::new(),
            matured: Vec::new(),
            balance: WalletCoreBalance::default(),
            account_balances: BTreeMap::new(),
            addresses_derived: Vec::new(),
        });

        let candidates = drained_incoming_candidates(&mut rx)
            .expect("a confirmed-first received output yields a candidate");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].amount_duffs, 33_000);
    }

    #[test]
    fn transaction_detected_emits_received_utxo_for_funding_address() {
        let (bridge, _cs, mut rx) = make_bridge();
        let funding = funding_address();

        bridge.on_wallet_event(&transaction_detected(received_record(&funding, 100_000)));

        let addresses =
            drained_received_utxo_addresses(&mut rx).expect("a received UTXO event is produced");
        assert!(
            addresses.contains(&funding),
            "the funding address must surface so the waiting screen advances"
        );
    }

    #[test]
    fn block_processed_inserted_emits_received_utxo() {
        let (bridge, _cs, mut rx) = make_bridge();
        let funding = funding_address();

        bridge.on_wallet_event(&WalletEvent::BlockProcessed {
            wallet_id: [9u8; 32],
            height: 1_000,
            chain_lock: None,
            inserted: vec![received_record(&funding, 50_000)],
            updated: Vec::new(),
            matured: Vec::new(),
            balance: WalletCoreBalance::default(),
            account_balances: BTreeMap::new(),
            addresses_derived: Vec::new(),
        });

        let addresses = drained_received_utxo_addresses(&mut rx)
            .expect("a confirmed-first funding tx still produces the event");
        assert!(addresses.contains(&funding));
    }

    #[test]
    fn shielded_sync_progress_event_sets_connection_status() {
        let (bridge, cs, mut rx) = make_bridge();
        bridge.on_shielded_sync_progress(4_096, 123_456);
        let got = cs
            .shielded_sync_progress()
            .expect("downloaded-notes progress published");
        assert_eq!(got.cumulative_scanned, 4_096);
        assert_eq!(got.block_height, 123_456);
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn shielded_tree_progress_event_sets_connection_status() {
        let (bridge, cs, mut rx) = make_bridge();
        bridge.on_shielded_tree_progress(2_048, 10_000);
        let got = cs
            .shielded_tree_progress()
            .expect("committed-to-tree progress published");
        assert_eq!(got.leaves_committed, 2_048);
        assert_eq!(got.total_target, 10_000);
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn shielded_sync_completed_clears_progress_and_nudges() {
        let (bridge, cs, mut rx, balances) = make_bridge_with_balances();
        // Simulate a pass mid-flight, then complete it.
        bridge.on_shielded_sync_progress(10, 20);
        bridge.on_shielded_tree_progress(30, 40);
        assert!(cs.shielded_sync_progress().is_some());
        assert!(cs.shielded_tree_progress().is_some());

        bridge.on_shielded_sync_completed(&ShieldedSyncPassSummary::default());

        assert!(
            cs.shielded_sync_progress().is_none(),
            "completion must clear the downloaded-notes progress"
        );
        assert!(
            cs.shielded_tree_progress().is_none(),
            "completion must clear the committed-to-tree progress"
        );
        // No wallet registered in the snapshot store, so nothing is written.
        assert!(
            balances.lock().unwrap().is_empty(),
            "an unresolved wallet id must not write a balance"
        );
        assert!(drained_refresh(&mut rx));
    }

    #[test]
    fn summary_ok_balances_extracts_only_successful_wallets() {
        use platform_wallet::wallet::shielded::ShieldedSyncSummary;

        let mut summary = ShieldedSyncPassSummary::default();
        let ok = ShieldedSyncSummary {
            balances: BTreeMap::from([(0u32, 1_000u64), (1u32, 234u64)]),
            ..Default::default()
        };
        summary
            .wallet_results
            .insert([1u8; 32], WalletShieldedOutcome::Ok(ok));
        summary
            .wallet_results
            .insert([2u8; 32], WalletShieldedOutcome::Skipped);
        summary
            .wallet_results
            .insert([3u8; 32], WalletShieldedOutcome::Err("boom".to_string()));

        let got = summary_ok_balances(&summary);
        assert_eq!(
            got,
            vec![([1u8; 32], 1_234)],
            "only the Ok wallet contributes its summed balance_total"
        );
    }

    /// An empty `PlatformAddressSyncSummary` must nudge the refresh loop and not
    /// write any balance (nothing to resolve in the snapshot registry).
    #[test]
    fn platform_address_sync_completed_empty_summary_nudges_only() {
        use platform_wallet::manager::platform_address_sync::PlatformAddressSyncSummary;

        let (bridge, _cs, mut rx, platform_balances) = make_bridge_with_platform_balances();
        bridge.on_platform_address_sync_completed(&PlatformAddressSyncSummary::default());

        assert!(
            platform_balances.lock().unwrap().is_empty(),
            "an empty summary must not write any balance"
        );
        assert!(
            drained_refresh(&mut rx),
            "the frame loop must be nudged even on an empty summary"
        );
    }

    /// A `WalletSyncOutcome::Err` must not write a balance for the failed wallet,
    /// but the frame loop must still be nudged.
    #[test]
    fn platform_address_sync_completed_error_outcome_skipped() {
        use platform_wallet::manager::platform_address_sync::{
            PlatformAddressSyncSummary, WalletSyncOutcome,
        };

        let (bridge, _cs, mut rx, platform_balances) = make_bridge_with_platform_balances();
        let mut summary = PlatformAddressSyncSummary::default();
        summary.wallet_results.insert(
            [7u8; 32],
            WalletSyncOutcome::Err("network error".to_string()),
        );
        bridge.on_platform_address_sync_completed(&summary);

        assert!(
            platform_balances.lock().unwrap().is_empty(),
            "an errored wallet must not write a balance"
        );
        assert!(drained_refresh(&mut rx));
    }

    /// HAPPY PATH (QA-B-002): `summary_ok_platform_entries` extracts the correct
    /// per-address data for `Ok` outcomes and skips `Err` outcomes entirely.
    #[test]
    fn summary_ok_platform_entries_extracts_only_successful_wallets() {
        use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;
        use dash_sdk::platform::address_sync::{AddressFunds, AddressSyncResult};
        use platform_wallet::manager::platform_address_sync::{
            PlatformAddressSyncSummary, WalletSyncOutcome,
        };
        use platform_wallet::wallet::PlatformAddressTag;

        let wallet_id_ok: [u8; 32] = [1u8; 32];
        let wallet_id_err: [u8; 32] = [2u8; 32];

        let p2pkh_a = PlatformP2PKHAddress::new([0xAAu8; 20]);
        let p2pkh_b = PlatformP2PKHAddress::new([0xBBu8; 20]);

        let mut result_ok: AddressSyncResult<PlatformAddressTag, PlatformP2PKHAddress> =
            AddressSyncResult::default();
        result_ok.found.insert(
            ((wallet_id_ok, 0u32, 0u32), p2pkh_a),
            AddressFunds {
                nonce: 1,
                balance: 500_000,
            },
        );
        result_ok.found.insert(
            ((wallet_id_ok, 0u32, 1u32), p2pkh_b),
            AddressFunds {
                nonce: 2,
                balance: 300_000,
            },
        );

        let mut summary = PlatformAddressSyncSummary::default();
        summary
            .wallet_results
            .insert(wallet_id_ok, WalletSyncOutcome::Ok(result_ok));
        summary.wallet_results.insert(
            wallet_id_err,
            WalletSyncOutcome::Err("network timeout".to_string()),
        );

        let got = summary_ok_platform_entries(&summary);

        // Only the Ok wallet should produce entries.
        assert_eq!(got.len(), 1, "only one wallet had an Ok outcome");
        let (got_wallet_id, entries) = &got[0];
        assert_eq!(got_wallet_id, &wallet_id_ok);
        assert_eq!(entries.len(), 2);

        // Check both addresses are present (order not guaranteed for BTreeMap).
        let mut hash_set: Vec<[u8; 20]> = entries.iter().map(|(h, _, _)| *h).collect();
        hash_set.sort();
        assert!(hash_set.contains(&[0xAAu8; 20]));
        assert!(hash_set.contains(&[0xBBu8; 20]));

        // Verify funds are passed through correctly.
        let entry_a = entries
            .iter()
            .find(|(h, _, _)| h == &[0xAAu8; 20])
            .expect("entry for p2pkh_a");
        assert_eq!(entry_a.1, 500_000, "balance_credits for p2pkh_a");
        assert_eq!(entry_a.2, 1, "nonce for p2pkh_a");

        let entry_b = entries
            .iter()
            .find(|(h, _, _)| h == &[0xBBu8; 20])
            .expect("entry for p2pkh_b");
        assert_eq!(entry_b.1, 300_000, "balance_credits for p2pkh_b");
        assert_eq!(entry_b.2, 2, "nonce for p2pkh_b");
    }

    /// An `Ok` outcome for an UNREGISTERED wallet must not write a balance
    /// (no entry in the snapshot registry to map `WalletId` → `WalletSeedHash`).
    #[test]
    fn platform_address_sync_completed_unregistered_wallet_not_written() {
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;
        use dash_sdk::platform::address_sync::{AddressFunds, AddressSyncResult};
        use platform_wallet::manager::platform_address_sync::{
            PlatformAddressSyncSummary, WalletSyncOutcome,
        };
        use platform_wallet::wallet::PlatformAddressTag;

        let (bridge, _cs, mut rx, platform_balances) = make_bridge_with_platform_balances();

        // Build a result with two found addresses (100 + 200 duffs in credits).
        let wallet_id = [9u8; 32];
        let mut result: AddressSyncResult<PlatformAddressTag, PlatformP2PKHAddress> =
            AddressSyncResult::default();
        result.found.insert(
            (
                (wallet_id, 0u32, 0u32),
                PlatformP2PKHAddress::new([0x11; 20]),
            ),
            AddressFunds {
                nonce: 0,
                balance: 100 * CREDITS_PER_DUFF,
            },
        );
        result.found.insert(
            (
                (wallet_id, 0u32, 1u32),
                PlatformP2PKHAddress::new([0x22; 20]),
            ),
            AddressFunds {
                nonce: 1,
                balance: 200 * CREDITS_PER_DUFF,
            },
        );

        let mut summary = PlatformAddressSyncSummary::default();
        summary
            .wallet_results
            .insert(wallet_id, WalletSyncOutcome::Ok(result));

        bridge.on_platform_address_sync_completed(&summary);

        // The wallet_id is NOT registered in the snapshot store, so it cannot be
        // resolved to a WalletSeedHash — nothing should be written.
        assert!(
            platform_balances.lock().unwrap().is_empty(),
            "an unregistered wallet must not write a balance (no seed_hash mapping)"
        );
        assert!(drained_refresh(&mut rx));
    }
}
