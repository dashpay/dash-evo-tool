//! Platform-address funds/sync cache seam (T5).
//!
//! [`PlatformAddressView`] is the only doorway DET code uses to read or
//! write per-address Platform funds (`balance` + `nonce`) and the
//! per-wallet sync cursor (`last_sync_timestamp` + `sync_height`).
//!
//! ## Why a seam, and why the cache is still active
//!
//! Upstream owns the per-address balance: `platform_addresses` rows in
//! the wallet persister, surfaced by the **public** reader
//! `PlatformAddressWallet::addresses_with_balances() ->
//! Vec<(PlatformAddress, Credits)>` (reachable via
//! `manager.get_wallet(id).platform()`). DET cannot drop its cache onto
//! that reader yet for two reasons:
//!
//! 1. **No nonce.** `addresses_with_balances` returns balance only. DET
//!    maintains a per-address Platform nonce that it bumps optimistically
//!    after each shielded send and re-aligns on a nonce-mismatch retry
//!    (`AppContext::bump_platform_address_nonce` /
//!    `set_platform_address_nonce`). That counter is fund-adjacent and has
//!    no public upstream reader (`AddressFunds.nonce` exists upstream but
//!    is behind `pub(crate)` accessors). Deleting the cache would lose it.
//! 2. **No public sync cursor in DET's shape.** The watermark is exposed
//!    (`PlatformAddressWallet::sync_watermark`), but the
//!    `(last_sync_timestamp, sync_height)` pair DET persists for the
//!    "syncing vs. zero" UX has no single public reader.
//!
//! So the cache stays the ACTIVE impl ([`KvCachedPlatformAddresses`]) —
//! zero behaviour change. [`UpstreamPlatformAddresses`] is the reserved
//! swap target whose read body is stubbed pending a public
//! balance+**nonce** per-address reader (platform todo `e817b66a`, now
//! narrowed: a balance-only reader already exists; the gap is nonce + the
//! DET sync-cursor shape). When that lands, implement it for real, make
//! it ACTIVE, and delete `det:platform_addr` / `det:platform_sync`.
//!
//! No upstream `ObjectId` / `WalletId` / `PlatformWalletManager` type
//! crosses this seam — the view speaks DET types only ([`WalletSeedHash`],
//! core [`Address`]).

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::address::{Address, NetworkUnchecked};
use std::str::FromStr;

use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};

const PLATFORM_ADDR_KEY_PREFIX: &str = "det:platform_addr:";
const PLATFORM_SYNC_KEY: &str = "det:platform_sync:v1";

fn platform_addr_key(address: &Address) -> String {
    format!("{PLATFORM_ADDR_KEY_PREFIX}{address}")
}

fn parse_canonical_address(s: &str, network: Network) -> Option<Address> {
    let unchecked = Address::<NetworkUnchecked>::from_str(s).ok()?;
    let checked = unchecked.require_network(network).ok()?;
    Some(Wallet::canonical_address(&checked, network))
}

/// Persisted per-address funds: Platform credit balance and the DET-local
/// optimistic nonce. Bincode-encoded behind the [`DetKv`] schema-version
/// envelope.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct StoredPlatformAddressInfo {
    balance: u64,
    nonce: u32,
}

/// Persisted per-wallet sync cursor.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct StoredPlatformSyncInfo {
    last_sync_timestamp: u64,
    sync_height: u64,
}

/// Read/write access to per-address Platform funds and the per-wallet sync
/// cursor. DET-typed throughout; implementors decide where the data lives
/// (DET cache today, upstream reader once one exposes balance + nonce).
pub trait PlatformAddressView: Send + Sync {
    /// Upsert `(balance, nonce)` for one address owned by `seed_hash`. The
    /// address is canonicalised against `network` before keying.
    fn set_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
        network: Network,
        balance: u64,
        nonce: u32,
    ) -> Result<(), KvAdapterError>;

    /// Read `(balance, nonce)` for one address, or `Ok(None)` if absent.
    /// The address is canonicalised against `network` before keying.
    fn get_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
        network: Network,
    ) -> Result<Option<(u64, u32)>, KvAdapterError>;

    /// Every stored `(address, balance, nonce)` triple for the wallet.
    /// Entries whose address fails to re-parse for `network` are skipped
    /// (logged at warn) so one corrupt key cannot block rehydration.
    fn all_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        network: Network,
    ) -> Result<Vec<(Address, u64, u32)>, KvAdapterError>;

    /// Delete every stored address-info entry for `seed_hash`. Idempotent.
    fn delete_address_info(&self, seed_hash: &WalletSeedHash) -> Result<(), KvAdapterError>;

    /// Read `(last_sync_timestamp, sync_height)`; `(0, 0)` when unset.
    fn get_sync_info(&self, seed_hash: &WalletSeedHash) -> Result<(u64, u64), KvAdapterError>;

    /// Upsert `(last_sync_timestamp, sync_height)` for `seed_hash`.
    fn set_sync_info(
        &self,
        seed_hash: &WalletSeedHash,
        last_sync_timestamp: u64,
        sync_height: u64,
    ) -> Result<(), KvAdapterError>;
}

/// ACTIVE impl: balance + nonce + sync cursor live in the per-network
/// wallet k/v store under `det:platform_addr:*` / `det:platform_sync:v1`,
/// scoped per-wallet so entries cascade on wallet removal.
pub struct KvCachedPlatformAddresses {
    kv: DetKv,
}

impl KvCachedPlatformAddresses {
    pub fn new(kv: DetKv) -> Self {
        Self { kv }
    }
}

impl PlatformAddressView for KvCachedPlatformAddresses {
    fn set_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
        network: Network,
        balance: u64,
        nonce: u32,
    ) -> Result<(), KvAdapterError> {
        let canonical = Wallet::canonical_address(address, network);
        self.kv.put(
            DetScope::Wallet(seed_hash),
            &platform_addr_key(&canonical),
            &StoredPlatformAddressInfo { balance, nonce },
        )
    }

    fn get_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        address: &Address,
        network: Network,
    ) -> Result<Option<(u64, u32)>, KvAdapterError> {
        let canonical = Wallet::canonical_address(address, network);
        let stored: Option<StoredPlatformAddressInfo> = self
            .kv
            .get(DetScope::Wallet(seed_hash), &platform_addr_key(&canonical))?;
        Ok(stored.map(|s| (s.balance, s.nonce)))
    }

    fn all_address_info(
        &self,
        seed_hash: &WalletSeedHash,
        network: Network,
    ) -> Result<Vec<(Address, u64, u32)>, KvAdapterError> {
        let keys = self
            .kv
            .list(DetScope::Wallet(seed_hash), Some(PLATFORM_ADDR_KEY_PREFIX))?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(addr_str) = key.strip_prefix(PLATFORM_ADDR_KEY_PREFIX) else {
                continue;
            };
            let Some(stored) = self
                .kv
                .get::<StoredPlatformAddressInfo>(DetScope::Wallet(seed_hash), &key)?
            else {
                continue;
            };
            let address = match parse_canonical_address(addr_str, network) {
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

    fn delete_address_info(&self, seed_hash: &WalletSeedHash) -> Result<(), KvAdapterError> {
        let keys = self
            .kv
            .list(DetScope::Wallet(seed_hash), Some(PLATFORM_ADDR_KEY_PREFIX))?;
        for key in keys {
            self.kv.delete(DetScope::Wallet(seed_hash), &key)?;
        }
        Ok(())
    }

    fn get_sync_info(&self, seed_hash: &WalletSeedHash) -> Result<(u64, u64), KvAdapterError> {
        let stored: Option<StoredPlatformSyncInfo> = self
            .kv
            .get(DetScope::Wallet(seed_hash), PLATFORM_SYNC_KEY)?;
        Ok(stored
            .map(|s| (s.last_sync_timestamp, s.sync_height))
            .unwrap_or((0, 0)))
    }

    fn set_sync_info(
        &self,
        seed_hash: &WalletSeedHash,
        last_sync_timestamp: u64,
        sync_height: u64,
    ) -> Result<(), KvAdapterError> {
        self.kv.put(
            DetScope::Wallet(seed_hash),
            PLATFORM_SYNC_KEY,
            &StoredPlatformSyncInfo {
                last_sync_timestamp,
                sync_height,
            },
        )
    }
}

/// Reserved swap target: read per-address funds from upstream instead of
/// DET's cache. Not selected — its read methods are stubbed until upstream
/// exposes a public per-address balance **and nonce** reader plus the
/// sync-cursor shape DET needs.
///
/// The balance half already exists at 08b0ed9
/// (`PlatformAddressWallet::addresses_with_balances`); the missing pieces
/// are the per-address nonce and the `(timestamp, height)` cursor.
pub struct UpstreamPlatformAddresses;

impl PlatformAddressView for UpstreamPlatformAddresses {
    fn set_address_info(
        &self,
        _seed_hash: &WalletSeedHash,
        _address: &Address,
        _network: Network,
        _balance: u64,
        _nonce: u32,
    ) -> Result<(), KvAdapterError> {
        // Writes become no-ops once balance is read straight from upstream;
        // the nonce path migrates with the public reader.
        // TODO(e817b66a): drop with the cache once upstream owns nonce.
        Ok(())
    }

    fn get_address_info(
        &self,
        _seed_hash: &WalletSeedHash,
        _address: &Address,
        _network: Network,
    ) -> Result<Option<(u64, u32)>, KvAdapterError> {
        // TODO(e817b66a): public per-address balance+nonce reader.
        unimplemented!(
            "pending platform todo e817b66a — public per-address platform-address funds (balance+nonce) reader"
        )
    }

    fn all_address_info(
        &self,
        _seed_hash: &WalletSeedHash,
        _network: Network,
    ) -> Result<Vec<(Address, u64, u32)>, KvAdapterError> {
        // TODO(e817b66a): public per-address balance+nonce reader.
        unimplemented!(
            "pending platform todo e817b66a — public per-address platform-address funds (balance+nonce) reader"
        )
    }

    fn delete_address_info(&self, _seed_hash: &WalletSeedHash) -> Result<(), KvAdapterError> {
        // TODO(e817b66a): native cascade replaces explicit deletion.
        Ok(())
    }

    fn get_sync_info(&self, _seed_hash: &WalletSeedHash) -> Result<(u64, u64), KvAdapterError> {
        // TODO(e817b66a): public sync-cursor reader.
        unimplemented!(
            "pending platform todo e817b66a — public per-wallet platform-address sync-cursor reader"
        )
    }

    fn set_sync_info(
        &self,
        _seed_hash: &WalletSeedHash,
        _last_sync_timestamp: u64,
        _sync_height: u64,
    ) -> Result<(), KvAdapterError> {
        // TODO(e817b66a): cursor becomes upstream-owned.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::DetKv;
    use dash_sdk::dpp::dashcore::PublicKey;
    use platform_wallet_storage::{KvError, KvStore, ObjectId};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct InMemoryKv {
        slots: Mutex<Vec<(ObjectId, String, Vec<u8>)>>,
    }

    impl KvStore for InMemoryKv {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            Ok(self
                .slots
                .lock()
                .unwrap()
                .iter()
                .find(|(s, k, _)| s == scope && k == key)
                .map(|(_, _, v)| v.clone()))
        }
        fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
            let mut slots = self.slots.lock().unwrap();
            if let Some(slot) = slots.iter_mut().find(|(s, k, _)| s == scope && k == key) {
                slot.2 = value.to_vec();
            } else {
                slots.push((scope.clone(), key.to_string(), value.to_vec()));
            }
            Ok(())
        }
        fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
            self.slots
                .lock()
                .unwrap()
                .retain(|(s, k, _)| !(s == scope && k == key));
            Ok(())
        }
        fn list_keys(
            &self,
            scope: &ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            let pred = |k: &str| -> bool { prefix.is_none_or(|p| k.starts_with(p)) };
            Ok(self
                .slots
                .lock()
                .unwrap()
                .iter()
                .filter(|(s, k, _)| s == scope && pred(k))
                .map(|(_, k, _)| k.clone())
                .collect())
        }
    }

    fn view() -> KvCachedPlatformAddresses {
        KvCachedPlatformAddresses::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    fn sample_address() -> Address {
        let pubkey = PublicKey::from_slice(&[0x02; 33]).unwrap();
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// PA1: balance + nonce round-trip through the active cache.
    #[test]
    fn address_info_round_trips() {
        let v = view();
        let seed: WalletSeedHash = [1u8; 32];
        let addr = sample_address();
        v.set_address_info(&seed, &addr, Network::Testnet, 1_000, 7)
            .unwrap();
        assert_eq!(
            v.get_address_info(&seed, &addr, Network::Testnet).unwrap(),
            Some((1_000, 7))
        );
    }

    /// PA2: per-wallet listing returns only the wallet's own entries with
    /// balance and nonce preserved.
    #[test]
    fn all_address_info_lists_wallet_entries() {
        let v = view();
        let seed: WalletSeedHash = [2u8; 32];
        let other: WalletSeedHash = [3u8; 32];
        let addr = sample_address();
        v.set_address_info(&seed, &addr, Network::Testnet, 42, 1)
            .unwrap();
        v.set_address_info(&other, &addr, Network::Testnet, 99, 2)
            .unwrap();

        let entries = v.all_address_info(&seed, Network::Testnet).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, 42);
        assert_eq!(entries[0].2, 1);
    }

    /// PA3: delete drops the wallet's address entries; sync cursor lives in
    /// its own slot and is unaffected.
    #[test]
    fn delete_clears_addresses_but_not_sync() {
        let v = view();
        let seed: WalletSeedHash = [4u8; 32];
        let addr = sample_address();
        v.set_address_info(&seed, &addr, Network::Testnet, 5, 0)
            .unwrap();
        v.set_sync_info(&seed, 1_700, 900).unwrap();

        v.delete_address_info(&seed).unwrap();
        assert!(
            v.all_address_info(&seed, Network::Testnet)
                .unwrap()
                .is_empty()
        );
        assert_eq!(v.get_sync_info(&seed).unwrap(), (1_700, 900));
    }

    /// PA4: an unset sync cursor reads as `(0, 0)` — callers treat that as
    /// "sync from scratch" (preserves the syncing-vs-zero UX).
    #[test]
    fn sync_info_defaults_to_zero() {
        let v = view();
        let seed: WalletSeedHash = [5u8; 32];
        assert_eq!(v.get_sync_info(&seed).unwrap(), (0, 0));
    }
}
