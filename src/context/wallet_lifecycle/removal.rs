//! Wallet removal: evicting a wallet and wiping its at-rest secrets.

use super::*;

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
            }

            // The upstream (watch-only, seedless) persistor row removal is the
            // sole async step; it carries no secret, so drive it off-thread.
            if let Some(wallet_id) = upstream_id {
                let backend = Arc::clone(&backend);
                let _ = self
                    .subtasks
                    .spawn_sync("wallet_upstream_removal", async move {
                        if let Err(error) = backend.remove_upstream_wallet(&wallet_id).await {
                            tracing::warn!(%error, "Upstream wallet removal failed");
                        }
                    });
            }
        }

        Ok(())
    }
}
