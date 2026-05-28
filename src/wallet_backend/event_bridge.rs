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

use super::snapshot::SnapshotStore;
use crate::app::TaskResult;
use crate::context::connection_status::ConnectionStatus;
use crate::model::spv_status::SpvStatus;
use crate::utils::egui_mpsc::SenderAsync;

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
                tracing::error!(%manager, error, "SPV manager error");
                let limit = error.floor_char_boundary(100);
                self.connection_status.set_spv_last_error(Some(format!(
                    "Sync manager {manager}: {}",
                    &error[..limit]
                )));
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

    // `on_shielded_sync_completed` is left at its upstream no-op default:
    // `platform-wallet`'s `shielded` feature is not enabled for DET (only
    // `serde`), so that callback never fires. DET's shielded path is the
    // separate retained grovestark flow, unrelated to this event.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::egui_mpsc::EguiMpscAsync;
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
}
