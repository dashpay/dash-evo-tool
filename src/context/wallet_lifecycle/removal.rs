//! Wallet removal: evicting a wallet and wiping its at-rest secrets.

use super::*;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;

const WALLET_DATA_REMOVAL_WARNING: &str = "Some wallet data could not be deleted and may remain on this device. Open Network settings and clear this network's database to remove it.";

fn show_wallet_data_removal_warning(ctx: &egui::Context, error: TaskError) {
    let handle = MessageBanner::set_global(ctx, WALLET_DATA_REMOVAL_WARNING, MessageType::Warning);
    handle.with_details(error);
    handle.disable_auto_dismiss();
    ctx.request_repaint();
}

impl AppContext {
    pub fn remove_wallet(self: &Arc<Self>, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        // Acquire write lock first to ensure atomicity — if the lock fails,
        // no changes have been made to the database.
        let mut wallets = self.wallets.write()?;
        if !wallets.contains_key(seed_hash) {
            return Err(TaskError::WalletNotFound);
        }

        wallets.remove(seed_hash);
        let has_wallet = !wallets.is_empty();
        drop(wallets);

        self.has_wallet.store(has_wallet, Ordering::Relaxed);

        // Evict the wallet's shielded balance snapshot. The seed hash is
        // deterministic from the seed, so re-importing the same recovery phrase
        // re-binds this exact key — without eviction the freshly-imported wallet
        // would surface the removed wallet's stale shielded balance until the
        // next completed sync overwrites it.
        if let Ok(mut balances) = self.shielded_balances.lock() {
            balances.remove(seed_hash);
        }

        // Evict the receive-address snapshot for the same reason, and with a
        // sharper edge: a stale address left behind here is one a user could
        // copy and be paid at, so it must not outlive the wallet that owns it.
        if let Ok(mut addresses) = self.shielded_addresses.lock() {
            addresses.remove(seed_hash);
        }

        // Wipe the wallet's current secret-bearing state: the encrypted
        // seed-envelope vault, session cache, wallet-meta sidecar, and shielded
        // rows. The pre-update database remains a read-only recovery artifact
        // and is deliberately not changed. Best-effort when the backend is not
        // wired yet — a pre-wire context has none of the current state.
        if let Ok(backend) = self.wallet_backend() {
            let upstream_id = backend.registered_wallet_id(seed_hash);
            if let Err(e) = backend.forget_wallet_local_state(seed_hash, upstream_id) {
                tracing::warn!(
                    wallet = %hex::encode(seed_hash),
                    error = ?e,
                    "Failed to wipe local wallet secret state on removal"
                );
                show_wallet_data_removal_warning(self.egui_ctx(), e);
            }

            // The upstream (watch-only, seedless) persistor row removal is the
            // sole async step; it carries no secret, so drive it off-thread.
            if let Some(wallet_id) = upstream_id {
                let backend = Arc::clone(&backend);
                let egui_ctx = self.egui_ctx().clone();
                let removal = async move {
                    if let Err(error) = backend.remove_upstream_wallet(&wallet_id).await {
                        show_wallet_data_removal_warning(&egui_ctx, error);
                    }
                };
                if self
                    .subtasks
                    .spawn_sync("wallet_upstream_removal", removal)
                    .is_err()
                {
                    show_wallet_data_removal_warning(
                        self.egui_ctx(),
                        TaskError::TaskManagerShuttingDown,
                    );
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn wallet_data_removal_warning_requests_repaint() {
        let ctx = egui::Context::default();
        let repaint_requests = Arc::new(AtomicUsize::new(0));
        let callback_requests = Arc::clone(&repaint_requests);
        ctx.set_request_repaint_callback(move |_| {
            callback_requests.fetch_add(1, Ordering::Relaxed);
        });

        show_wallet_data_removal_warning(&ctx, TaskError::TaskManagerShuttingDown);

        assert!(
            repaint_requests.load(Ordering::Relaxed) > 0,
            "showing an asynchronous warning must wake an idle UI"
        );
    }
}
