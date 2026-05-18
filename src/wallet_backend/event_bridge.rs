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
}

impl EventBridge {
    pub fn new(
        connection_status: Arc<ConnectionStatus>,
        task_result_sender: SenderAsync<TaskResult>,
    ) -> Self {
        Self {
            connection_status,
            task_result_sender,
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

    fn on_wallet_event(&self, _event: &WalletEvent) {
        // Wallet balances/transactions mutated upstream — re-read next frame.
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
