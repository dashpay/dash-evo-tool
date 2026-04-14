//! Implements [`PlatformEventHandler`] to translate SPV events into
//! evo-tool's app-level concerns: [`ConnectionStatus`] updates, sync progress
//! snapshots, and reconcile signals.

use crate::context::connection_status::ConnectionStatus;
use crate::spv::types::failed_manager_name;
use crate::spv::types::{SpvStatus, SpvStatusSnapshot};
use dash_sdk::dash_spv::EventHandler;
use dash_sdk::dash_spv::network::NetworkEvent;
use dash_sdk::dash_spv::sync::{SyncEvent, SyncProgress as SpvSyncProgress, SyncState};
use platform_wallet::events::{PlatformEventHandler, WalletEvent};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio::sync::mpsc;

/// Implements [`PlatformEventHandler`] to receive SPV events by reference
/// and translate them into evo-tool's UI-facing state.
///
/// Owns the shared [`SpvStatusSnapshot`] and pushes updates to
/// [`ConnectionStatus`] and the reconcile channel.
pub struct SpvEventBridge {
    connection_status: Arc<ConnectionStatus>,
    status: Arc<RwLock<SpvStatusSnapshot>>,
    reconcile_tx: Mutex<mpsc::Sender<()>>,
}

impl SpvEventBridge {
    /// Create a new event bridge.
    ///
    /// * `connection_status` — shared status indicator updated on every
    ///   relevant event.
    /// * `reconcile_tx` — channel used to signal wallet reconciliation
    ///   (debounced downstream).
    pub fn new(connection_status: Arc<ConnectionStatus>, reconcile_tx: mpsc::Sender<()>) -> Self {
        Self {
            connection_status,
            status: Arc::new(RwLock::new(SpvStatusSnapshot::default())),
            reconcile_tx: Mutex::new(reconcile_tx),
        }
    }

    /// Replace the reconcile channel with a fresh one.
    ///
    /// Returns the new receiver. Called by `start_spv()` so each SPV
    /// session gets a clean reconcile pipeline.
    pub fn new_reconcile_channel(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(64);
        if let Ok(mut guard) = self.reconcile_tx.lock() {
            *guard = tx;
        }
        rx
    }

    /// Read the current status snapshot (used by the tooltip and UI).
    pub fn status(&self) -> SpvStatusSnapshot {
        self.status.read().map(|g| g.clone()).unwrap_or_default()
    }

    // ------------------------------------------------------------------
    // Internal handlers — logic extracted from the old SpvEventHandler
    // ------------------------------------------------------------------

    /// Handle wallet-level events (balance changes, transaction status, etc.).
    ///
    /// `BalanceUpdated` events are emitted by `WalletManager` after SPV block
    /// processing. The `WalletBalance` atomics are already updated during block
    /// processing via `PlatformWalletInfo::check_core_transaction()`, so the
    /// main purpose here is to signal reconciliation so evo-tool's UI-facing
    /// state picks up the change.
    fn handle_wallet_event(&self, event: &WalletEvent) {
        match event {
            WalletEvent::BalanceUpdated {
                wallet_id,
                confirmed,
                unconfirmed,
                ..
            } => {
                tracing::debug!(
                    wallet_id = %hex::encode(wallet_id),
                    confirmed,
                    unconfirmed,
                    "BalanceUpdated event received"
                );
                // Signal reconciliation so the UI picks up the new balance.
                if let Ok(tx) = self.reconcile_tx.lock() {
                    let _ = tx.try_send(());
                }
            }
            WalletEvent::TransactionReceived {
                wallet_id, record, ..
            } => {
                tracing::debug!(
                    wallet_id = %hex::encode(wallet_id),
                    txid = %record.txid,
                    "TransactionReceived event"
                );
                // Signal reconciliation for new transactions.
                if let Ok(tx) = self.reconcile_tx.lock() {
                    let _ = tx.try_send(());
                }
            }
            WalletEvent::TransactionStatusChanged {
                wallet_id,
                txid,
                status,
                ..
            } => {
                tracing::debug!(
                    wallet_id = %hex::encode(wallet_id),
                    %txid,
                    %status,
                    "TransactionStatusChanged event"
                );
                // Signal reconciliation for status changes.
                if let Ok(tx) = self.reconcile_tx.lock() {
                    let _ = tx.try_send(());
                }
            }
        }
    }

    /// Translate a sync progress update into status + connection status changes.
    ///
    /// Mirrors the old `EventHandler::on_progress` implementation.
    fn handle_progress(&self, progress: &SpvSyncProgress) {
        let is_synced = progress.is_synced();
        let is_error = progress.state() == SyncState::Error;

        // Update the shared snapshot (read by the UI tooltip / progress bars).
        let (new_status, error_msg) = if let Ok(mut snap) = self.status.write() {
            snap.sync_progress = Some(progress.clone());
            snap.last_updated = Some(SystemTime::now());

            // Derive SpvStatus from progress.
            let new_status = if is_synced {
                snap.status = SpvStatus::Running;
                Some(SpvStatus::Running)
            } else if is_error {
                snap.status = SpvStatus::Error;
                Some(SpvStatus::Error)
            } else if !matches!(
                snap.status,
                SpvStatus::Stopping | SpvStatus::Stopped | SpvStatus::Error
            ) {
                snap.status = SpvStatus::Syncing;
                Some(SpvStatus::Syncing)
            } else {
                None
            };

            // Record error message (only the first one).
            let error_msg = if is_error && snap.last_error.is_none() {
                let phase = failed_manager_name(progress);
                let msg = format!("Sync failed: {phase} (reported by SPV progress channel)");
                snap.last_error = Some(msg.clone());
                Some(msg)
            } else {
                None
            };

            (new_status, error_msg)
        } else {
            (None, None)
        };

        // Push to ConnectionStatus for the status indicator.
        let cs = &self.connection_status;
        if let Some(s) = new_status {
            cs.set_spv_status(s);
        }
        if let Some(msg) = error_msg {
            cs.set_spv_last_error(Some(msg));
        } else if !is_error {
            // Only clear errors when the snapshot status is not Error —
            // otherwise on_sync_event(ManagerError) sets the error but the
            // next progress update would immediately clear it.
            let snapshot_is_error = self
                .status
                .read()
                .ok()
                .is_some_and(|s| s.status == SpvStatus::Error);
            if !snapshot_is_error {
                cs.set_spv_last_error(None);
            }
        }
        cs.refresh_state();
    }

    /// Handle sync lifecycle events (SyncComplete, ManagerError,
    /// BlockProcessed, ChainLock, InstantLock).
    ///
    /// Mirrors the old `EventHandler::on_sync_event` implementation
    /// (minus the dead finality channel code).
    fn handle_sync_event(&self, event: &SyncEvent) {
        eprintln!(
            "[DEBUG handle_sync_event] t={:?} {:?}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
                % 100000,
            event
        );
        // Transition to Running on SyncComplete.
        if matches!(event, SyncEvent::SyncComplete { .. }) {
            eprintln!("[DEBUG handle_sync_event] SyncComplete! Setting Running status");
            if let Ok(mut snap) = self.status.write() {
                snap.status = SpvStatus::Running;
            }
            self.connection_status.set_spv_status(SpvStatus::Running);
            self.connection_status.refresh_state();
        }

        // Handle ManagerError — only fatal for core managers (headers, filters).
        // Masternode sync errors are non-fatal: the wallet is fully functional
        // for transactions without masternodes (testnet often has QRInfo errors).
        if let SyncEvent::ManagerError { manager, error } = event {
            let is_masternode = manager.to_string() == "Masternode";
            if is_masternode {
                tracing::warn!("SPV masternode sync error (non-fatal): {error}");
            } else {
                tracing::error!("SPV manager {manager} reported error: {error}");
                if let Ok(mut snap) = self.status.write() {
                    snap.status = SpvStatus::Error;
                    if snap.last_error.is_none() {
                        let limit = error.floor_char_boundary(100);
                        let msg = format!("Sync manager {manager} failed: {}", &error[..limit]);
                        snap.last_error = Some(msg.clone());
                        self.connection_status.set_spv_last_error(Some(msg));
                    }
                }
                self.connection_status.set_spv_status(SpvStatus::Error);
                self.connection_status.refresh_state();
            }
        }

        // Signal reconciliation for wallet-relevant events.
        let should_signal = matches!(
            event,
            SyncEvent::BlockProcessed { .. }
                | SyncEvent::ChainLockReceived { .. }
                | SyncEvent::InstantLockReceived { .. }
                | SyncEvent::SyncComplete { .. }
        );

        if should_signal {
            // Silently discard full-channel errors — reconcile is debounced
            // downstream.
            if let Ok(tx) = self.reconcile_tx.lock() {
                let _ = tx.try_send(());
            }
        }
    }

    /// Handle network events (peer count changes).
    ///
    /// Mirrors the old `EventHandler::on_network_event` implementation.
    fn handle_network_event(&self, event: &NetworkEvent) {
        if let NetworkEvent::PeersUpdated {
            connected_count, ..
        } = event
        {
            if let Ok(mut snap) = self.status.write() {
                snap.connected_peers = *connected_count;
            }
            self.connection_status
                .set_spv_connected_peers((*connected_count).min(u16::MAX as usize) as u16);
            self.connection_status.refresh_state();
        }
    }
}

impl EventHandler for SpvEventBridge {
    fn on_sync_event(&self, event: &SyncEvent) {
        self.handle_sync_event(event);
    }

    fn on_network_event(&self, event: &NetworkEvent) {
        self.handle_network_event(event);
    }

    fn on_progress(&self, progress: &SpvSyncProgress) {
        self.handle_progress(progress);
    }

    fn on_wallet_event(&self, event: &WalletEvent) {
        self.handle_wallet_event(event);
    }

    fn on_error(&self, error: &str) {
        tracing::error!("SPV error: {}", error);
    }
}

impl PlatformEventHandler for SpvEventBridge {}

impl std::fmt::Debug for SpvEventBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvEventBridge")
            .field("status", &self.status())
            .finish()
    }
}
