//! Bridges upstream `platform-wallet` events into DET's frame loop.
//!
//! `EventBridge` implements `platform_wallet::PlatformEventHandler` (and its
//! `dash_spv::EventHandler` supertrait). Upstream owns chain sync; this is the
//! only path by which sync/wallet state changes reach DET. Each callback is
//! sync and must not block — it updates [`ConnectionStatus`] atomics and
//! nudges the frame loop with a non-blocking `TaskResult::Refresh`. The
//! visible-screen `display_task_result` / `refresh` then re-reads state
//! through `WalletBackend` accessors, exactly as the old reconcile path did.

use std::sync::Arc;

use dash_sdk::dash_spv::network::NetworkEvent;
use dash_sdk::dash_spv::sync::{SyncEvent, SyncProgress, SyncState};
use platform_wallet::events::{EventHandler, PlatformEventHandler, WalletEvent};
use platform_wallet::manager::platform_address_sync::PlatformAddressSyncSummary;

use super::snapshot::{SnapshotStore, received_outputs_for_record};
use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::CoreItem;
use crate::context::connection_status::ConnectionStatus;
use crate::model::spv_status::SpvStatus;
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::dpp::key_wallet::managed_account::transaction_record::TransactionRecord;

/// DET-authored handler registered with `PlatformWalletManager` at
/// construction. Holds only cheap shared handles so it stays `Send + Sync`
/// and clone-free on the event hot path.
pub struct EventBridge {
    connection_status: Arc<ConnectionStatus>,
    task_result_sender: SenderAsync<TaskResult>,
    snapshots: Arc<SnapshotStore>,
}

impl EventBridge {
    pub(super) fn new(
        connection_status: Arc<ConnectionStatus>,
        task_result_sender: SenderAsync<TaskResult>,
        snapshots: Arc<SnapshotStore>,
    ) -> Self {
        Self {
            connection_status,
            task_result_sender,
            snapshots,
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
    fn on_platform_address_sync_completed(&self, _summary: &PlatformAddressSyncSummary) {
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

    // `on_shielded_sync_completed` is left at its upstream no-op default:
    // `platform-wallet`'s `shielded` feature is not enabled for DET (only
    // `serde`), so that callback never fires. DET's shielded path is the
    // separate retained grovestark flow, unrelated to this event.
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let cs = Arc::new(ConnectionStatus::new());
        let (tx, rx) =
            tokio::sync::mpsc::channel::<TaskResult>(8).with_egui_ctx(egui::Context::default());
        let bridge = EventBridge::new(Arc::clone(&cs), tx, Arc::new(SnapshotStore::new()));
        (bridge, cs, rx)
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
}
