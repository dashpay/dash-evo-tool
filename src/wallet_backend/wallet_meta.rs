//! DET-side wallet-metadata view (T-W-00).
//!
//! [`WalletMetaView`] is the only doorway DET code uses to read or
//! write [`WalletMeta`] (alias / `is_main` / `core_wallet_name`) for
//! HD wallets. The view borrows a shared [`DetKv`] handle pointing at
//! `det-app.sqlite` and serialises every entry under a colon-prefixed,
//! network-scoped key:
//!
//! ```text
//! <network>:wallet_meta:<seed_hash_base58>
//! ```
//!
//! Network-prefixed keys + the global (`None`) wallet scope mirror the
//! C3 `det:settings:v1` pattern: the cross-network `det-app.sqlite`
//! file is the right store (one file, one schema, easy backup), and
//! per-wallet scope (`Some(&WalletId)`) cannot be used because the
//! upstream `WalletId` does not exist until a wallet is registered
//! with `PlatformWalletManager`. The seed hash is the stable
//! DET-level identifier and fits the key naturally.
//!
//! All accessors are infallible at the read path: a missing key
//! returns `None`, a corrupted blob (schema mismatch / truncated /
//! decode failure) is logged and treated as absent so the wallet
//! picker degrades gracefully rather than blocking the UI.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::base58;

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::meta::{WalletMeta, WalletMetaV1};
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Colon-separated namespace shared across networks. The full key is
/// `<network>:wallet_meta:<seed_hash_base58>` — the prefix below is
/// the cross-network shape used by [`list`](WalletMetaView::list).
pub(crate) const KEY_INFIX: &str = ":wallet_meta:";

/// Build the canonical k/v key for a wallet's metadata blob.
pub(crate) fn key_for(network: Network, seed_hash: &WalletSeedHash) -> String {
    let net = network_prefix(network);
    let hash = base58::encode_slice(seed_hash);
    format!("{net}{KEY_INFIX}{hash}")
}

/// Cross-network prefix `<network>:` used by every entry key. Matches
/// the network display convention already in
/// `src/wallet_backend/mod.rs::resolve_spv_storage_dir` so the same
/// vocabulary appears in both the on-disk path and the k/v keys.
fn network_prefix(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

/// Build the `<network>:wallet_meta:` prefix used to enumerate every
/// wallet meta entry for a single network.
fn prefix_for(network: Network) -> String {
    format!("{}{KEY_INFIX}", network_prefix(network))
}

/// View borrowing a shared [`DetKv`] handle. Cheap to construct, so
/// callers can build one per operation rather than threading it.
pub struct WalletMetaView<'a> {
    kv: &'a Arc<DetKv>,
}

impl<'a> WalletMetaView<'a> {
    /// Borrow a [`DetKv`] handle as a typed wallet-metadata view. Kept
    /// `pub` so benches and downstream tooling can build the view
    /// without going through [`WalletBackend::wallet_meta`].
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self { kv }
    }

    /// All `(seed_hash, meta)` pairs persisted for `network`.
    ///
    /// Decode errors on individual entries are logged and skipped so a
    /// single corrupt row cannot poison the picker; the wallet listing
    /// degrades to "name unknown" rather than refusing to open the
    /// app. The same key parser is used by both the cross-network
    /// listing and the one-shot migration writer (T-W-00) so a key
    /// shape change forces a review here.
    pub fn list(&self, network: Network) -> Vec<(WalletSeedHash, WalletMeta)> {
        let prefix = prefix_for(network);
        let keys = match self.kv.list(DetScope::Global, Some(&prefix)) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::wallet_meta",
                    network = ?network,
                    error = ?e,
                    "Failed to list wallet-meta keys; returning empty list",
                );
                return Vec::new();
            }
        };
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(hash) = parse_seed_hash(&key, &prefix) else {
                tracing::warn!(
                    target = "wallet_backend::wallet_meta",
                    key = %key,
                    "Skipping wallet-meta key with non-base58 seed-hash suffix",
                );
                continue;
            };
            match self.read_meta(&key) {
                Ok(Some(meta)) => out.push((hash, meta)),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        target = "wallet_backend::wallet_meta",
                        key = %key,
                        error = ?e,
                        "Skipping unreadable wallet-meta blob",
                    );
                }
            }
        }
        out
    }

    /// Fetch the metadata for a single wallet. `None` when the key is
    /// absent or the blob fails to decode (logged).
    pub fn get(&self, network: Network, seed_hash: &WalletSeedHash) -> Option<WalletMeta> {
        let key = key_for(network, seed_hash);
        match self.read_meta(&key) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::wallet_meta",
                    key = %key,
                    error = ?e,
                    "Failed to read wallet meta; treating as absent",
                );
                None
            }
        }
    }

    /// Upsert the metadata for a single wallet. Re-writing the same
    /// value is a no-op-effective write (DetKv upserts by key). Written in the
    /// current `WalletMeta` shape directly through the `DetKv` schema envelope.
    pub fn set(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        meta: &WalletMeta,
    ) -> Result<(), TaskError> {
        let key = key_for(network, seed_hash);
        self.kv
            .put(DetScope::Global, &key, meta)
            .map_err(map_kv_error_to_task_error)
    }

    /// Read a single wallet-meta blob with a dual-format fallback. Tries the
    /// current 6-field [`WalletMeta`] shape first; on a decode failure (an old
    /// 4-field blob runs out of bytes for the appended fields) falls back to the
    /// legacy [`WalletMetaV1`] shape and RE-STORES it in the current shape
    /// (one-shot migration). `Ok(None)` when the key is absent.
    fn read_meta(&self, key: &str) -> Result<Option<WalletMeta>, KvAdapterError> {
        // New shape first. The DetKv schema-version mismatch is a hard error
        // (propagate); only a bincode *decode* failure means "try legacy".
        match self.kv.get::<WalletMeta>(DetScope::Global, key) {
            Ok(opt) => return Ok(opt),
            Err(KvAdapterError::Decode(_)) => {}
            Err(e) => return Err(e),
        }
        // Legacy 4-field shape. A success here is an old blob: migrate it.
        let Some(v1) = self.kv.get::<WalletMetaV1>(DetScope::Global, key)? else {
            return Ok(None);
        };
        let migrated: WalletMeta = v1.into();
        if let Err(e) = self.kv.put(DetScope::Global, key, &migrated) {
            // Re-store is best-effort: the in-memory value is correct this
            // session; the next read retries the migration.
            tracing::warn!(
                target = "wallet_backend::wallet_meta",
                key = %key,
                error = ?e,
                "Could not re-store migrated wallet meta; will retry next read",
            );
        }
        Ok(Some(migrated))
    }

    /// Delete the metadata for a single wallet. Idempotent — a
    /// missing key returns `Ok(())`.
    pub fn delete(&self, network: Network, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        let key = key_for(network, seed_hash);
        self.kv
            .delete(DetScope::Global, &key)
            .map_err(map_kv_error_to_task_error)
    }
}

/// Wallet-meta adapter errors all funnel into the dedicated
/// [`TaskError::KvSidecarStorage`] envelope so the banner copy
/// matches the surface ("wallet details") rather than the more
/// generic upstream wallet-storage one.
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    TaskError::KvSidecarStorage {
        sidecar: "wallet_meta",
        source: Box::new(e),
    }
}

/// Extract the base58 seed-hash suffix from a key starting with
/// `prefix`. Returns `None` when the suffix is not 32 bytes of
/// base58, which catches both prefix mismatches and corrupt keys.
fn parse_seed_hash(key: &str, prefix: &str) -> Option<WalletSeedHash> {
    let rest = key.strip_prefix(prefix)?;
    let bytes = base58::decode(rest).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use platform_wallet_storage::{KvError, KvStore, ObjectId};

    /// Minimal in-memory `KvStore` — mirrors `kv.rs`'s test fixture so
    /// the view tests can exercise list/get/set/delete without touching
    /// the file system or building a `WalletBackend`. Models every
    /// `ObjectId` scope FK-free via a flat `Vec` (upstream `ObjectId` is
    /// not `Ord`, so it cannot key a map).
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

    fn kv() -> Arc<DetKv> {
        Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    fn meta(alias: &str, is_main: bool, core: Option<&str>) -> WalletMeta {
        WalletMeta {
            alias: alias.into(),
            is_main,
            core_wallet_name: core.map(str::to_string),
            xpub_encoded: Vec::new(),
            uses_password: false,
            password_hint: None,
        }
    }

    /// W-META-VIEW-001 (TC-W-001 storage half) — a written meta
    /// round-trips through `get` and shows up in `list` for the same
    /// network.
    #[test]
    fn set_then_get_round_trips() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let seed: WalletSeedHash = [0x11; 32];
        let m = meta("paycheque", true, Some("local-dashd"));
        view.set(Network::Testnet, &seed, &m).expect("set");
        assert_eq!(view.get(Network::Testnet, &seed), Some(m.clone()));
        let listed = view.list(Network::Testnet);
        assert_eq!(listed, vec![(seed, m)]);
    }

    /// W-META-VIEW-002 (TC-W-008) — set overwrites; renaming via the
    /// view is a single upsert and the new alias surfaces on the next
    /// read.
    #[test]
    fn set_overwrites_existing_entry() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let seed: WalletSeedHash = [0x22; 32];
        view.set(Network::Mainnet, &seed, &meta("old", false, None))
            .expect("first set");
        view.set(Network::Mainnet, &seed, &meta("new", true, None))
            .expect("second set");
        assert_eq!(
            view.get(Network::Mainnet, &seed),
            Some(meta("new", true, None))
        );
    }

    /// W-META-VIEW-003 — `list` does not leak entries from other
    /// networks (the `<network>:` prefix is the partition). Mirrors
    /// the per-network isolation contract from `kv.rs::list`.
    #[test]
    fn list_partitions_by_network() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let a: WalletSeedHash = [0x33; 32];
        let b: WalletSeedHash = [0x44; 32];
        view.set(Network::Testnet, &a, &meta("on testnet", false, None))
            .unwrap();
        view.set(Network::Mainnet, &b, &meta("on mainnet", true, None))
            .unwrap();
        let testnet = view.list(Network::Testnet);
        let mainnet = view.list(Network::Mainnet);
        assert_eq!(testnet, vec![(a, meta("on testnet", false, None))]);
        assert_eq!(mainnet, vec![(b, meta("on mainnet", true, None))]);
    }

    /// W-META-VIEW-004 — `delete` is idempotent (matches the
    /// underlying `DetKv::delete` contract).
    #[test]
    fn delete_is_idempotent() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let seed: WalletSeedHash = [0x55; 32];
        view.delete(Network::Testnet, &seed).expect("delete absent");
        view.set(Network::Testnet, &seed, &meta("x", false, None))
            .unwrap();
        view.delete(Network::Testnet, &seed).expect("first delete");
        view.delete(Network::Testnet, &seed).expect("second delete");
        assert_eq!(view.get(Network::Testnet, &seed), None);
    }

    /// W-META-VIEW-005 — `get` on a missing key returns `None` rather
    /// than erroring; this is the listing path's graceful-degradation
    /// contract.
    #[test]
    fn get_missing_returns_none() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let seed: WalletSeedHash = [0x66; 32];
        assert_eq!(view.get(Network::Devnet, &seed), None);
    }

    /// W-META-VIEW-006 — a corrupt key in the store (non-base58
    /// suffix) is skipped silently rather than blocking the listing.
    #[test]
    fn list_skips_unparseable_keys() {
        let kv = kv();
        let store = kv.clone();
        let view = WalletMetaView::new(&kv);
        // Plant a valid entry plus a garbage entry that shares the
        // network prefix but has a non-base58 suffix.
        let seed: WalletSeedHash = [0x77; 32];
        view.set(Network::Testnet, &seed, &meta("ok", false, None))
            .unwrap();
        store
            .put(
                DetScope::Global,
                &format!("testnet{KEY_INFIX}!!!not-base58!!!"),
                &meta("garbage", false, None),
            )
            .unwrap();
        let listed = view.list(Network::Testnet);
        assert_eq!(listed, vec![(seed, meta("ok", false, None))]);
    }

    /// W-META-VIEW-007 — the canonical key shape uses base58 encoding
    /// for the 32-byte seed hash. Locks the shape so a future change
    /// (hex, etc.) needs an explicit migration.
    #[test]
    fn key_for_uses_base58_seed_hash() {
        let seed: WalletSeedHash = [0xAB; 32];
        let key = key_for(Network::Mainnet, &seed);
        assert!(key.starts_with("mainnet:wallet_meta:"));
        let suffix = key.trim_start_matches("mainnet:wallet_meta:");
        let decoded = base58::decode(suffix).expect("base58 decodes");
        assert_eq!(decoded.as_slice(), seed.as_slice());
    }

    /// Dual-format legacy-blob upgrade — an OLD 4-field blob, written exactly
    /// as the base branch did (`kv.put::<WalletMetaV1>`), is read back through
    /// the view: its `alias`/`is_main`/`core_wallet_name`/`xpub` are preserved
    /// (NOT silently lost to a `Vec<u8>` type-confusion), the new fields default,
    /// and the entry is RE-STORED in the new 6-field shape (a subsequent
    /// `get::<WalletMeta>` succeeds directly). Covers a 1-char alias (the
    /// leading-byte-collision case a version-tag dispatch would mis-route).
    /// Makes the `WalletMetaV1` legacy path live + tested end-to-end.
    #[test]
    fn old_wallet_meta_blob_decodes_preserves_fields_and_restores() {
        for alias in ["paycheque", "a", "ab"] {
            let kv = kv();
            let view = WalletMetaView::new(&kv);
            let seed: WalletSeedHash = [0x5A; 32];
            let key = key_for(Network::Testnet, &seed);

            // Write the OLD shape directly, the way the base branch did.
            let v1 = WalletMetaV1 {
                alias: alias.into(),
                is_main: true,
                core_wallet_name: Some("local-dashd".into()),
                xpub_encoded: vec![0x22; 78],
            };
            kv.put(DetScope::Global, &key, &v1).expect("write old blob");

            // The view reads it (dual-format fallback), preserving every field.
            let got = view.get(Network::Testnet, &seed).expect("old blob decodes");
            assert_eq!(got.alias, alias, "alias preserved");
            assert!(got.is_main, "is_main preserved");
            assert_eq!(got.core_wallet_name.as_deref(), Some("local-dashd"));
            assert_eq!(got.xpub_encoded, vec![0x22; 78]);
            assert!(!got.uses_password, "new field defaults false");
            assert!(got.password_hint.is_none());

            // It was re-stored in the new shape: a direct new-shape decode now
            // succeeds (no more legacy fallback needed).
            let direct: Option<WalletMeta> = kv
                .get(DetScope::Global, &key)
                .expect("direct new-shape read");
            assert_eq!(direct.expect("present").alias, alias);
        }
    }
}
