//! Per-wallet Platform address-info + sync-cursor persistence.
//!
//! Thin `AppContext` façade over the [`PlatformAddressView`] seam
//! (`src/wallet_backend/platform_address.rs`). The view owns the storage
//! strategy — today the active impl caches `(balance, nonce)` and the
//! `(timestamp, height)` cursor in the per-network wallet k/v store; once
//! upstream exposes a public balance+nonce reader the cache is swapped out
//! behind the same view with no caller change.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::PlatformAddressView;
use dash_sdk::dpp::dashcore::address::Address;

impl AppContext {
    /// Upsert the persisted Platform balance + nonce for one address
    /// owned by `seed_hash`.
    pub fn set_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
        balance: u64,
        nonce: u32,
    ) -> Result<(), TaskError> {
        self.wallet_backend()?
            .platform_addresses()
            .set_address_info(seed_hash, address, self.network, balance, nonce)
            .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Read the stored `(balance, nonce)` for a single Platform address,
    /// or `Ok(None)` if no record exists.
    pub fn get_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
    ) -> Result<Option<(u64, u32)>, TaskError> {
        self.wallet_backend()?
            .platform_addresses()
            .get_address_info(seed_hash, address, self.network)
            .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Return every stored `(address, balance, nonce)` triple for the
    /// wallet. Entries whose address fails to re-parse against the active
    /// network are skipped (logged at warn) so a single corrupt key does
    /// not block the rest of the rehydration.
    pub fn get_all_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Vec<(Address, u64, u32)>, TaskError> {
        self.wallet_backend()?
            .platform_addresses()
            .all_address_info(seed_hash, self.network)
            .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Delete every stored Platform address-info entry for `seed_hash`.
    /// Called when a wallet is removed. Idempotent.
    pub fn delete_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        self.wallet_backend()?
            .platform_addresses()
            .delete_address_info(seed_hash)
            .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Read the persisted `(last_sync_timestamp, sync_height)` cursor for
    /// `seed_hash`. Returns `(0, 0)` when no cursor has been recorded
    /// yet — callers treat that as "sync from scratch".
    pub fn get_platform_sync_info(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(u64, u64), TaskError> {
        self.wallet_backend()?
            .platform_addresses()
            .get_sync_info(seed_hash)
            .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Upsert the `(last_sync_timestamp, sync_height)` cursor for
    /// `seed_hash`.
    pub fn set_platform_sync_info(
        &self,
        seed_hash: &WalletSeedHash,
        last_sync_timestamp: u64,
        sync_height: u64,
    ) -> Result<(), TaskError> {
        self.wallet_backend()?
            .platform_addresses()
            .set_sync_info(seed_hash, last_sync_timestamp, sync_height)
            .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Populate the in-memory `Wallet.platform_address_info` maps from
    /// the per-wallet k/v store. Invoked once the wallet backend exists.
    /// Per-wallet failures are logged and skipped: the affected wallet
    /// starts with an empty cache and rehydrates on the next Platform
    /// sync.
    pub(crate) fn restore_platform_address_info_from_kv(&self) {
        let wallets = match self.wallets.read() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = ?e, "wallet lock poisoned during platform-address rehydrate");
                return;
            }
        };
        for (seed_hash, wallet_arc) in wallets.iter() {
            let entries = match self.get_all_platform_address_info(seed_hash) {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::warn!(
                        wallet = %hex::encode(seed_hash),
                        error = ?e,
                        "failed to rehydrate platform address info from k/v"
                    );
                    continue;
                }
            };
            if entries.is_empty() {
                continue;
            }
            let Ok(mut wallet) = wallet_arc.write() else {
                continue;
            };
            for (address, balance, nonce) in entries {
                wallet.platform_address_info.insert(
                    address,
                    crate::model::wallet::PlatformAddressInfo { balance, nonce },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::AppContext;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::wallet::{Wallet, WalletSeedHash};
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::address::Address;
    use dash_sdk::dpp::dashcore::{Network, PublicKey};
    use std::sync::Arc;

    /// Build an offline testnet `AppContext` with the wallet backend wired so
    /// the platform-address façade resolves a real k/v store. No network I/O:
    /// construction reads bundled `.env` addresses and connects lazily. The
    /// `TempDir` must outlive the context — its drop removes the data dir.
    async fn wired_context() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);

        let db = Arc::new(
            create_database_at_path(&data_dir.join("data.db")).expect("create test database"),
        );
        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();
        let app_kv = AppContext::open_app_kv(&data_dir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("open secret store");

        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
            secret_store,
        )
        .expect("offline testnet AppContext::new");

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        (ctx, temp_dir)
    }

    fn sample_address() -> Address {
        let pubkey = PublicKey::from_slice(&[0x02; 33]).unwrap();
        Wallet::canonical_address(&Address::p2pkh(&pubkey, Network::Testnet), Network::Testnet)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn address_info_round_trips_through_facade() {
        let (ctx, _tmp) = wired_context().await;
        let seed: WalletSeedHash = [1u8; 32];
        let addr = sample_address();
        ctx.set_platform_address_info(&seed, &addr, 1_000, 7)
            .unwrap();
        assert_eq!(
            ctx.get_platform_address_info(&seed, &addr).unwrap(),
            Some((1_000, 7))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missing_address_info_reads_as_none() {
        let (ctx, _tmp) = wired_context().await;
        let seed: WalletSeedHash = [2u8; 32];
        assert_eq!(
            ctx.get_platform_address_info(&seed, &sample_address())
                .unwrap(),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn all_address_info_lists_only_the_wallets_own_entries() {
        let (ctx, _tmp) = wired_context().await;
        let seed: WalletSeedHash = [3u8; 32];
        let other: WalletSeedHash = [4u8; 32];
        let addr = sample_address();
        ctx.set_platform_address_info(&seed, &addr, 42, 1).unwrap();
        ctx.set_platform_address_info(&other, &addr, 99, 2).unwrap();

        let entries = ctx.get_all_platform_address_info(&seed).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].1, entries[0].2), (42, 1));
        // A wallet with no stored entries lists empty.
        assert!(
            ctx.get_all_platform_address_info(&[9u8; 32])
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_clears_address_entries_but_leaves_sync_cursor() {
        let (ctx, _tmp) = wired_context().await;
        let seed: WalletSeedHash = [5u8; 32];
        let addr = sample_address();
        ctx.set_platform_address_info(&seed, &addr, 5, 0).unwrap();
        ctx.set_platform_sync_info(&seed, 1_700, 900).unwrap();

        ctx.delete_platform_address_info(&seed).unwrap();
        assert!(ctx.get_all_platform_address_info(&seed).unwrap().is_empty());
        // The sync cursor lives in its own slot, untouched by the delete.
        assert_eq!(ctx.get_platform_sync_info(&seed).unwrap(), (1_700, 900));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_cursor_round_trips_and_defaults_to_zero() {
        let (ctx, _tmp) = wired_context().await;
        let seed: WalletSeedHash = [6u8; 32];
        // Unset cursor reads as (0, 0) — callers treat that as "from scratch".
        assert_eq!(ctx.get_platform_sync_info(&seed).unwrap(), (0, 0));
        ctx.set_platform_sync_info(&seed, 2_500, 1_234).unwrap();
        assert_eq!(ctx.get_platform_sync_info(&seed).unwrap(), (2_500, 1_234));
    }
}
