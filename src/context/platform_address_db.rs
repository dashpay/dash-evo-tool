//! Per-wallet Platform address-info + sync-cursor persistence.
//!
//! Two slots in the per-network wallet k/v store, both scoped to
//! `Some(&WalletId)` so entries cascade when a wallet is removed:
//!
//! - `det:platform_addr:<canonical_address>` — `StoredPlatformAddressInfo`
//!   (balance + nonce), one per Platform address held by the wallet.
//! - `det:platform_sync:v1` — `StoredPlatformSyncInfo`
//!   (last sync timestamp + sync-height cursor), one per wallet.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::address::{Address, NetworkUnchecked};
use platform_wallet::wallet::platform_wallet::WalletId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

const PLATFORM_ADDR_KEY_PREFIX: &str = "det:platform_addr:";
const PLATFORM_SYNC_KEY: &str = "det:platform_sync:v1";

fn platform_addr_key(address: &Address) -> String {
    format!("{PLATFORM_ADDR_KEY_PREFIX}{address}")
}

/// `WalletSeedHash` and the upstream `WalletId` are both `[u8; 32]`. The
/// k/v adapter wants the upstream type — borrow through.
fn wallet_id(seed_hash: &WalletSeedHash) -> &WalletId {
    seed_hash
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct StoredPlatformAddressInfo {
    balance: u64,
    nonce: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct StoredPlatformSyncInfo {
    last_sync_timestamp: u64,
    sync_height: u64,
}

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
        let canonical = Wallet::canonical_address(address, self.network);
        let kv = self.wallet_backend()?.kv();
        kv.put(
            Some(wallet_id(seed_hash)),
            &platform_addr_key(&canonical),
            &StoredPlatformAddressInfo { balance, nonce },
        )
        .map_err(|source| TaskError::PlatformAddressStorage { source })
    }

    /// Read the stored `(balance, nonce)` for a single Platform address,
    /// or `Ok(None)` if no record exists.
    pub fn get_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
    ) -> Result<Option<(u64, u32)>, TaskError> {
        let canonical = Wallet::canonical_address(address, self.network);
        let kv = self.wallet_backend()?.kv();
        let stored: Option<StoredPlatformAddressInfo> = kv
            .get(Some(wallet_id(seed_hash)), &platform_addr_key(&canonical))
            .map_err(|source| TaskError::PlatformAddressStorage { source })?;
        Ok(stored.map(|s| (s.balance, s.nonce)))
    }

    /// Return every stored `(address, balance, nonce)` triple for the
    /// wallet. Entries whose address fails to re-parse against the active
    /// network are skipped (logged at warn) so a single corrupt key does
    /// not block the rest of the rehydration.
    pub fn get_all_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Vec<(Address, u64, u32)>, TaskError> {
        let kv = self.wallet_backend()?.kv();
        let keys = kv
            .list(Some(wallet_id(seed_hash)), Some(PLATFORM_ADDR_KEY_PREFIX))
            .map_err(|source| TaskError::PlatformAddressStorage { source })?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(addr_str) = key.strip_prefix(PLATFORM_ADDR_KEY_PREFIX) else {
                continue;
            };
            let Some(stored) = kv
                .get::<StoredPlatformAddressInfo>(Some(wallet_id(seed_hash)), &key)
                .map_err(|source| TaskError::PlatformAddressStorage { source })?
            else {
                continue;
            };
            let address = match parse_canonical_address(addr_str, self.network) {
                Some(a) => a,
                None => {
                    tracing::warn!(
                        address = addr_str,
                        "skipping unparseable platform address entry"
                    );
                    continue;
                }
            };
            out.push((address, stored.balance, stored.nonce));
        }
        Ok(out)
    }

    /// Delete every stored Platform address-info entry for `seed_hash`.
    /// Called when a wallet is removed. Idempotent.
    pub fn delete_platform_address_info(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        let kv = self.wallet_backend()?.kv();
        let keys = kv
            .list(Some(wallet_id(seed_hash)), Some(PLATFORM_ADDR_KEY_PREFIX))
            .map_err(|source| TaskError::PlatformAddressStorage { source })?;
        for key in keys {
            kv.delete(Some(wallet_id(seed_hash)), &key)
                .map_err(|source| TaskError::PlatformAddressStorage { source })?;
        }
        Ok(())
    }

    /// Read the persisted `(last_sync_timestamp, sync_height)` cursor for
    /// `seed_hash`. Returns `(0, 0)` when no cursor has been recorded
    /// yet — callers treat that as "sync from scratch".
    pub fn get_platform_sync_info(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(u64, u64), TaskError> {
        let kv = self.wallet_backend()?.kv();
        let stored: Option<StoredPlatformSyncInfo> = kv
            .get(Some(wallet_id(seed_hash)), PLATFORM_SYNC_KEY)
            .map_err(|source| TaskError::PlatformAddressStorage { source })?;
        Ok(stored
            .map(|s| (s.last_sync_timestamp, s.sync_height))
            .unwrap_or((0, 0)))
    }

    /// Upsert the `(last_sync_timestamp, sync_height)` cursor for
    /// `seed_hash`.
    pub fn set_platform_sync_info(
        &self,
        seed_hash: &WalletSeedHash,
        last_sync_timestamp: u64,
        sync_height: u64,
    ) -> Result<(), TaskError> {
        let kv = self.wallet_backend()?.kv();
        kv.put(
            Some(wallet_id(seed_hash)),
            PLATFORM_SYNC_KEY,
            &StoredPlatformSyncInfo {
                last_sync_timestamp,
                sync_height,
            },
        )
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

fn parse_canonical_address(s: &str, network: Network) -> Option<Address> {
    let unchecked = Address::<NetworkUnchecked>::from_str(s).ok()?;
    let checked = unchecked.require_network(network).ok()?;
    Some(Wallet::canonical_address(&checked, network))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encoding sanity: the stored payload round-trips through bincode
    /// plus the k/v schema-version envelope without drift. Guards against
    /// silent struct evolution that would corrupt persisted balances or
    /// nonces.
    #[test]
    fn stored_platform_address_info_round_trips() {
        let original = StoredPlatformAddressInfo {
            balance: 1_234_567,
            nonce: 42,
        };
        let bytes = bincode::serde::encode_to_vec(original, bincode::config::standard()).unwrap();
        let (decoded, _): (StoredPlatformAddressInfo, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(decoded, original);
    }

    /// Sync-cursor payload round-trips.
    #[test]
    fn stored_platform_sync_info_round_trips() {
        let original = StoredPlatformSyncInfo {
            last_sync_timestamp: 1_700_000_000,
            sync_height: 987_654,
        };
        let bytes = bincode::serde::encode_to_vec(original, bincode::config::standard()).unwrap();
        let (decoded, _): (StoredPlatformSyncInfo, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(decoded, original);
    }

    /// Key format is the contract everything else relies on. Pin the
    /// prefix so changes are deliberate, not accidental.
    #[test]
    fn platform_addr_key_uses_canonical_prefix() {
        let pubkey_bytes = [0x02; 33];
        let pubkey = dash_sdk::dpp::dashcore::PublicKey::from_slice(&pubkey_bytes).unwrap();
        let address = Address::p2pkh(&pubkey, Network::Testnet);
        let key = platform_addr_key(&address);
        assert!(key.starts_with(PLATFORM_ADDR_KEY_PREFIX));
        assert!(key.ends_with(&address.to_string()));
    }

    /// Sync-cursor key is a versioned single-slot constant — protect it
    /// from drift.
    #[test]
    fn platform_sync_key_is_versioned() {
        assert_eq!(PLATFORM_SYNC_KEY, "det:platform_sync:v1");
    }
}
