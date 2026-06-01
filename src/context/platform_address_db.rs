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
