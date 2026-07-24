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

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::meta::{WalletMeta, WalletMetaV1};
use crate::wallet_backend::DetKv;
use crate::wallet_backend::kv::{KvAdapterError, map_kv_storage_error};
#[cfg(test)]
use crate::wallet_backend::sidecar::sidecar_key;
use crate::wallet_backend::sidecar::{SidecarScope, SidecarValue, SidecarView};

/// Colon-separated namespace shared across networks. The full key is
/// `<network>:wallet_meta:<seed_hash_base58>`.
pub(crate) const KEY_INFIX: &str = ":wallet_meta:";

/// Build the canonical k/v key for a wallet's metadata blob. The generic view
/// builds keys itself; this mirror exists for key-shape tests.
#[cfg(test)]
pub(crate) fn key_for(network: Network, seed_hash: &WalletSeedHash) -> String {
    sidecar_key(network, KEY_INFIX, seed_hash)
}

impl SidecarValue for WalletMeta {
    /// Read a single wallet-meta blob with a dual-format fallback. Tries the
    /// current 6-field [`WalletMeta`] shape first; on a decode failure (an old
    /// 4-field blob runs out of bytes for the appended fields) falls back to the
    /// legacy [`WalletMetaV1`] shape and RE-STORES it in the current shape
    /// (one-shot migration). `Ok(None)` when the key is absent.
    fn read(
        kv: &DetKv,
        scope: crate::wallet_backend::DetScope<'_>,
        key: &str,
    ) -> Result<Option<Self>, KvAdapterError> {
        // New shape first. The DetKv schema-version mismatch is a hard error
        // (propagate); only a bincode *decode* failure means "try legacy".
        match kv.get::<WalletMeta>(scope, key) {
            Ok(opt) => return Ok(opt),
            Err(KvAdapterError::Decode(_)) => {}
            Err(e) => return Err(e),
        }
        // Legacy 4-field shape. A success here is an old blob: migrate it.
        let Some(v1) = kv.get::<WalletMetaV1>(scope, key)? else {
            return Ok(None);
        };
        let migrated: WalletMeta = v1.into();
        if let Err(e) = kv.put(scope, key, &migrated) {
            // Re-store is best-effort: the in-memory value is correct this
            // session; the next read retries the migration.
            tracing::warn!(
                target: "wallet_backend::wallet_meta",
                key = %key,
                error = ?e,
                "Could not re-store migrated wallet meta; will retry next read",
            );
        }
        Ok(Some(migrated))
    }
}

/// Typed wallet-metadata sidecar (T-W-00). A thin wrapper over the generic
/// [`SidecarView`]: metadata (`alias` / `is_main` / `core_wallet_name` / xpub /
/// password fields) is `Global`-scoped because the picker renders it before any
/// wallet is registered with `PlatformWalletManager` — so per-wallet scope is
/// unavailable and the seed hash is the stable DET-level key. Reads degrade to
/// `None`/skip on a corrupt blob (with a legacy-format fallback, see
/// [`SidecarValue::read`]) so the picker never blocks.
pub struct WalletMetaView<'a>(SidecarView<'a, WalletMeta>);

impl<'a> WalletMetaView<'a> {
    /// Borrow a [`DetKv`] handle as a typed wallet-metadata view. Kept
    /// `pub` so benches and downstream tooling can build the view
    /// without going through [`WalletBackend::wallet_meta`].
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self(SidecarView::new(
            kv,
            KEY_INFIX,
            SidecarScope::Global,
            map_kv_error_to_task_error,
        ))
    }

    /// All `(seed_hash, meta)` pairs persisted for `network`. A single corrupt
    /// row is logged and skipped so the wallet listing degrades to "name
    /// unknown" rather than refusing to open the app.
    pub fn list(&self, network: Network) -> Vec<(WalletSeedHash, WalletMeta)> {
        self.0.list(network)
    }

    /// Fetch the metadata for a single wallet. `None` when the key is
    /// absent or the blob fails to decode (logged).
    pub fn get(&self, network: Network, seed_hash: &WalletSeedHash) -> Option<WalletMeta> {
        self.0.get(network, seed_hash)
    }

    /// Fetch the metadata for a single wallet, surfacing a read failure instead
    /// of degrading it to absence. `Ok(None)` means the row was never written;
    /// `Err(`[`TaskError::KvSidecarStorage`]`)` means the blob is present but
    /// unreadable (storage fault, schema mismatch, corrupt bytes). A rename that
    /// only edits the alias must not overwrite the row on a failed read — a
    /// blind `set` after a swallowed error would drop every other stored field —
    /// so it reads through this fallible path.
    pub fn try_get(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
    ) -> Result<Option<WalletMeta>, TaskError> {
        self.0.try_get(network, seed_hash)
    }

    /// Upsert the metadata for a single wallet. Re-writing the same value is an
    /// idempotent overwrite (DetKv upserts by key).
    pub fn set(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        meta: &WalletMeta,
    ) -> Result<(), TaskError> {
        crate::model::wallet::validate_wallet_alias(&meta.alias)
            .map_err(|source| TaskError::InvalidWalletAliasLength { source })?;
        self.0.set(network, seed_hash, meta)
    }

    /// Preserve metadata imported from legacy storage, including aliases that
    /// predate the current length limit. New writes must use [`Self::set`].
    pub(crate) fn set_migrated(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        meta: &WalletMeta,
    ) -> Result<(), TaskError> {
        if let Err(error) = crate::model::wallet::validate_wallet_alias(&meta.alias) {
            tracing::warn!(
                alias_chars = error.actual,
                max_alias_chars = error.max,
                "Preserving an overlong legacy wallet alias during migration"
            );
        }
        self.0.set(network, seed_hash, meta)
    }

    /// Delete the metadata for a single wallet. Idempotent — a
    /// missing key returns `Ok(())`.
    pub fn delete(&self, network: Network, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        self.0.delete(network, seed_hash)
    }
}

/// Wallet-meta adapter errors all funnel into the dedicated
/// [`TaskError::KvSidecarStorage`] envelope so the banner copy
/// matches the surface ("wallet details") rather than the more
/// generic upstream wallet-storage one.
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    map_kv_storage_error(e, |source| TaskError::KvSidecarStorage {
        sidecar: "wallet_meta",
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use dash_sdk::dpp::dashcore::base58;

    use crate::wallet_backend::DetScope;
    use crate::wallet_backend::kv_test_support::InMemoryKv;

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

    #[test]
    fn set_rejects_overlong_alias() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let seed: WalletSeedHash = [0x23; 32];
        let error = view
            .set(Network::Mainnet, &seed, &meta(&"w".repeat(65), false, None))
            .expect_err("overlong alias must fail");
        assert!(matches!(error, TaskError::InvalidWalletAliasLength { .. }));
        assert_eq!(view.get(Network::Mainnet, &seed), None);
    }

    #[test]
    fn migration_preserves_legacy_overlong_alias() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);
        let seed: WalletSeedHash = [0x24; 32];
        let legacy_meta = meta(&"w".repeat(65), false, None);
        view.set_migrated(Network::Mainnet, &seed, &legacy_meta)
            .expect("legacy alias must be preserved");
        assert_eq!(view.get(Network::Mainnet, &seed), Some(legacy_meta));
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

    /// W-META-VIEW-008 — `try_get` distinguishes the three outcomes that
    /// `get` collapses to `None`: a genuinely absent row (`Ok(None)`), a
    /// present readable row (`Ok(Some)`), and a present-but-unreadable blob
    /// (`Err(KvSidecarStorage)`). A rename that only edits the alias relies on
    /// this to abort rather than overwrite the other stored fields on a failed
    /// read.
    #[test]
    fn try_get_distinguishes_absent_present_and_read_failure() {
        let kv = kv();
        let view = WalletMetaView::new(&kv);

        // Absent: never written.
        let absent: WalletSeedHash = [0x01; 32];
        assert_eq!(
            view.try_get(Network::Testnet, &absent)
                .expect("absent read"),
            None
        );

        // Present: round-trips as the stored value.
        let present: WalletSeedHash = [0x02; 32];
        let m = meta("paycheque", true, Some("local-dashd"));
        view.set(Network::Testnet, &present, &m).expect("set");
        assert_eq!(
            view.try_get(Network::Testnet, &present)
                .expect("present read"),
            Some(m)
        );

        // Present-but-unreadable: plant a blob that decodes as neither the
        // current nor the legacy shape (a bare `u8` whose string-length varint
        // runs past the end). `get` swallows this to `None`; `try_get` surfaces
        // it as the sidecar-storage error.
        let corrupt: WalletSeedHash = [0x03; 32];
        let key = key_for(Network::Testnet, &corrupt);
        kv.put(DetScope::Global, &key, &2u8)
            .expect("plant corrupt blob");
        let err = view
            .try_get(Network::Testnet, &corrupt)
            .expect_err("a present-but-unreadable blob must surface as an error");
        assert!(
            matches!(
                err,
                TaskError::KvSidecarStorage {
                    sidecar: "wallet_meta",
                    ..
                }
            ),
            "got {err:?}"
        );
        // The same blob still degrades to `None` through `get` — the contrast
        // the rename fix depends on.
        assert_eq!(view.get(Network::Testnet, &corrupt), None);
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
