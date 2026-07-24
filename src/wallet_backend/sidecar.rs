//! Generic network-scoped, base58-keyed sidecar view over [`DetKv`].
//!
//! The wallet-metadata, identity-metadata, and auth-pubkey-cache sidecars share
//! one shape: a k/v store keyed by `<network><infix><base58(32-byte id)>`, read
//! best-effort (a corrupt blob degrades to absent rather than blocking the UI),
//! and written through a domain-specific [`TaskError`] envelope.
//! [`SidecarView`] captures that shape once; each domain is a thin typed wrapper
//! choosing the value type, key infix, k/v scope, and error mapper.
//!
//! Domains whose blobs need a legacy-format fallback (e.g. `WalletMeta`)
//! override [`SidecarValue::read`]; the common case uses the plain typed read.

use std::marker::PhantomData;
use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::base58;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::backend_task::error::TaskError;
use crate::wallet_backend::kv::{KvAdapterError, network_prefix};
use crate::wallet_backend::{DetKv, DetScope};

/// A 32-byte sidecar identifier — a wallet seed hash or an identity id. Both are
/// `[u8; 32]` and are keyed identically as base58.
pub(crate) type SidecarId = [u8; 32];

/// Build the canonical sidecar key `<network><infix><base58(id)>`. The single
/// definition of the sidecar key shape.
pub(crate) fn sidecar_key(network: Network, infix: &str, id: &SidecarId) -> String {
    format!(
        "{}{infix}{}",
        network_prefix(network),
        base58::encode_slice(id)
    )
}

/// Extract the 32-byte id from a key starting with `prefix`. `None` when the
/// suffix is not 32 bytes of base58 — catches both prefix mismatches and
/// corrupt keys.
fn parse_id(key: &str, prefix: &str) -> Option<SidecarId> {
    let rest = key.strip_prefix(prefix)?;
    base58::decode(rest).ok()?.try_into().ok()
}

/// Which k/v partition a sidecar's entries live in.
#[derive(Clone, Copy)]
pub(crate) enum SidecarScope {
    /// Cross-network global partition; entries survive wallet deletion and are
    /// enumerable via [`SidecarView::list`].
    Global,
    /// Scoped to the wallet whose seed hash is the entry id, so entries cascade
    /// when that wallet is removed. Not cross-wallet enumerable.
    WalletById,
}

impl SidecarScope {
    /// Resolve the concrete [`DetScope`] for `id` under this partition.
    fn det_scope(self, id: &SidecarId) -> DetScope<'_> {
        match self {
            SidecarScope::Global => DetScope::Global,
            SidecarScope::WalletById => DetScope::Wallet(id),
        }
    }
}

/// A value stored in a [`SidecarView`]. The read strategy is part of the value
/// type so a domain needing a legacy-format fallback can override it while the
/// common case uses a plain typed read.
pub(crate) trait SidecarValue: Serialize + DeserializeOwned + Sized {
    /// Read one blob by fully-qualified `key`. Default: a plain typed `get`.
    /// Override to add a legacy-format fallback and one-shot migration re-store.
    fn read(kv: &DetKv, scope: DetScope<'_>, key: &str) -> Result<Option<Self>, KvAdapterError> {
        kv.get::<Self>(scope, key)
    }
}

/// A network-scoped, base58-keyed sidecar over [`DetKv`]. Cheap to construct
/// (borrow only), so wrappers build one per operation.
pub(crate) struct SidecarView<'a, V> {
    kv: &'a Arc<DetKv>,
    /// Colon-wrapped namespace, e.g. `:wallet_meta:`. The full key is
    /// `<network><infix><base58(id)>`.
    infix: &'static str,
    scope: SidecarScope,
    /// Maps a store failure to the domain's user-facing [`TaskError`] envelope.
    map_err: fn(KvAdapterError) -> TaskError,
    _marker: PhantomData<fn() -> V>,
}

impl<'a, V: SidecarValue> SidecarView<'a, V> {
    /// Build a view over `kv` for the `infix` namespace, `scope` partition, and
    /// `map_err` envelope.
    pub(crate) fn new(
        kv: &'a Arc<DetKv>,
        infix: &'static str,
        scope: SidecarScope,
        map_err: fn(KvAdapterError) -> TaskError,
    ) -> Self {
        Self {
            kv,
            infix,
            scope,
            map_err,
            _marker: PhantomData,
        }
    }

    /// Fetch the value for `id`, degrading to `None` on a missing or corrupt
    /// blob (logged). The sidecar read never fails the caller.
    pub(crate) fn get(&self, network: Network, id: &SidecarId) -> Option<V> {
        let key = sidecar_key(network, self.infix, id);
        match V::read(self.kv, self.scope.det_scope(id), &key) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "wallet_backend::sidecar",
                    sidecar = self.infix,
                    key = %key,
                    error = ?e,
                    "Failed to read sidecar entry; treating as absent",
                );
                None
            }
        }
    }

    /// Fetch the value for `id`, distinguishing "genuinely absent" (`Ok(None)`)
    /// from "the blob is present but could not be read" (`Err`). Unlike
    /// [`Self::get`], a read/decode/schema failure is surfaced through the
    /// view's error envelope instead of degrading to `None`. Callers that must
    /// not blindly overwrite existing fields on a failed read use this so a
    /// storage fault aborts the write rather than clobbering the stored blob.
    pub(crate) fn try_get(&self, network: Network, id: &SidecarId) -> Result<Option<V>, TaskError> {
        let key = sidecar_key(network, self.infix, id);
        V::read(self.kv, self.scope.det_scope(id), &key).map_err(self.map_err)
    }

    /// Upsert the value for `id`. Re-writing the same value is an idempotent
    /// overwrite (DetKv upserts by key).
    pub(crate) fn set(&self, network: Network, id: &SidecarId, value: &V) -> Result<(), TaskError> {
        let key = sidecar_key(network, self.infix, id);
        self.kv
            .put(self.scope.det_scope(id), &key, value)
            .map_err(self.map_err)
    }

    /// Delete the value for `id`. Idempotent — a missing key returns `Ok(())`.
    pub(crate) fn delete(&self, network: Network, id: &SidecarId) -> Result<(), TaskError> {
        let key = sidecar_key(network, self.infix, id);
        self.kv
            .delete(self.scope.det_scope(id), &key)
            .map_err(self.map_err)
    }

    /// All `(id, value)` pairs persisted for `network`.
    ///
    /// A key with a non-base58 id suffix, or a blob that fails to read, is
    /// logged and skipped so one corrupt row cannot poison the listing. Only
    /// meaningful for [`SidecarScope::Global`]; a `WalletById` sidecar has no
    /// cross-wallet listing and returns an empty vec.
    pub(crate) fn list(&self, network: Network) -> Vec<(SidecarId, V)> {
        if !matches!(self.scope, SidecarScope::Global) {
            return Vec::new();
        }
        let prefix = format!("{}{}", network_prefix(network), self.infix);
        let keys = match self.kv.list(DetScope::Global, Some(&prefix)) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    target: "wallet_backend::sidecar",
                    sidecar = self.infix,
                    network = ?network,
                    error = ?e,
                    "Failed to list sidecar keys; returning empty list",
                );
                return Vec::new();
            }
        };
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(id) = parse_id(&key, &prefix) else {
                tracing::warn!(
                    target: "wallet_backend::sidecar",
                    sidecar = self.infix,
                    key = %key,
                    "Skipping sidecar key with non-base58 id suffix",
                );
                continue;
            };
            match V::read(self.kv, DetScope::Global, &key) {
                Ok(Some(value)) => out.push((id, value)),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        target: "wallet_backend::sidecar",
                        sidecar = self.infix,
                        key = %key,
                        error = ?e,
                        "Skipping unreadable sidecar blob",
                    );
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::kv_test_support::InMemoryKv;

    const INFIX: &str = ":blob:";

    #[derive(Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Blob(u32);
    impl SidecarValue for Blob {}

    fn mapper(e: KvAdapterError) -> TaskError {
        TaskError::AvatarCacheStorage {
            source: Box::new(e),
        }
    }

    fn kv() -> Arc<DetKv> {
        Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    /// SIDECAR-001 — a `Global` view round-trips set→get, enumerates via `list`,
    /// partitions by network, and deletes idempotently.
    #[test]
    fn global_view_round_trips_and_lists() {
        let kv = kv();
        let view = SidecarView::<Blob>::new(&kv, INFIX, SidecarScope::Global, mapper);
        let a = [0x11; 32];
        let b = [0x22; 32];
        view.set(Network::Testnet, &a, &Blob(1)).unwrap();
        view.set(Network::Testnet, &b, &Blob(2)).unwrap();
        view.set(Network::Mainnet, &a, &Blob(9)).unwrap();

        assert_eq!(view.get(Network::Testnet, &a), Some(Blob(1)));
        let mut listed = view.list(Network::Testnet);
        listed.sort_by_key(|(id, _)| *id);
        assert_eq!(listed, vec![(a, Blob(1)), (b, Blob(2))]);
        // Other network is partitioned off.
        assert_eq!(view.list(Network::Mainnet), vec![(a, Blob(9))]);

        view.delete(Network::Testnet, &a).unwrap();
        view.delete(Network::Testnet, &a).unwrap();
        assert_eq!(view.get(Network::Testnet, &a), None);
    }

    /// SIDECAR-002 — a `WalletById` view stores under the wallet scope and is
    /// retrievable by id, but has no cross-wallet listing (`list` is empty).
    #[test]
    fn wallet_scoped_view_has_no_cross_wallet_list() {
        let kv = kv();
        let view = SidecarView::<Blob>::new(&kv, INFIX, SidecarScope::WalletById, mapper);
        let id = [0x33; 32];
        view.set(Network::Testnet, &id, &Blob(7)).unwrap();
        assert_eq!(view.get(Network::Testnet, &id), Some(Blob(7)));
        assert!(view.list(Network::Testnet).is_empty());
    }

    /// SIDECAR-003 — a key whose id suffix is not base58 is skipped, so one
    /// corrupt row cannot poison the listing.
    #[test]
    fn list_skips_non_base58_suffix() {
        let kv = kv();
        let view = SidecarView::<Blob>::new(&kv, INFIX, SidecarScope::Global, mapper);
        let id = [0x44; 32];
        view.set(Network::Testnet, &id, &Blob(5)).unwrap();
        kv.put(
            DetScope::Global,
            &format!("testnet{INFIX}!!!not-base58!!!"),
            &Blob(0),
        )
        .unwrap();
        assert_eq!(view.list(Network::Testnet), vec![(id, Blob(5))]);
    }
}
