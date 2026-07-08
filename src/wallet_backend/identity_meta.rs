//! DET-side identity-metadata view.
//!
//! [`IdentityMetaView`] is the only doorway DET code uses to read or write
//! [`IdentityMeta`] (the password hint shown next to the sign-time prompt) for
//! an identity whose keys are password-protected. A verbatim twin of
//! [`WalletMetaView`](crate::wallet_backend::WalletMetaView): it borrows a
//! shared [`DetKv`] handle pointing at `det-app.sqlite` and serialises every
//! entry under a colon-prefixed, network-scoped key:
//!
//! ```text
//! <network>:identity_meta:<identity_id_base58>
//! ```
//!
//! Network-prefixed keys + the global (`DetScope::Global`) scope mirror the
//! `wallet_meta` pattern: the cross-network `det-app.sqlite` file is the right
//! store (one file, one schema, easy backup), and the 32-byte identity id is
//! the stable DET-level identifier.
//!
//! This sidecar is **display-only** — it never gates whether a prompt fires
//! (the at-rest vault scheme does). Every read path is therefore infallible at
//! the value level: a missing key returns `None`, a corrupt blob is logged and
//! treated as absent so the prompt degrades to "no hint" rather than failing.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::base58;

use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::identity_meta::IdentityMeta;
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Colon-separated namespace shared across networks. The full key is
/// `<network>:identity_meta:<identity_id_base58>`.
pub(crate) const KEY_INFIX: &str = ":identity_meta:";

/// Build the canonical k/v key for an identity's metadata blob.
pub(crate) fn key_for(network: Network, identity_id: &[u8; 32]) -> String {
    let net = network_prefix(network);
    let id = base58::encode_slice(identity_id);
    format!("{net}{KEY_INFIX}{id}")
}

/// Cross-network prefix `<network>:` used by every entry key. Matches the
/// `wallet_meta` convention so the same vocabulary appears across sidecars.
fn network_prefix(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

/// Build the `<network>:identity_meta:` prefix used to enumerate every identity
/// meta entry for a single network.
fn prefix_for(network: Network) -> String {
    format!("{}{KEY_INFIX}", network_prefix(network))
}

/// View borrowing a shared [`DetKv`] handle. Cheap to construct, so callers
/// build one per operation rather than threading it.
pub struct IdentityMetaView<'a> {
    kv: &'a Arc<DetKv>,
}

impl<'a> IdentityMetaView<'a> {
    /// Borrow a [`DetKv`] handle as a typed identity-metadata view.
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self { kv }
    }

    /// All `(identity_id, meta)` pairs persisted for `network`. A single
    /// corrupt row is logged and skipped rather than poisoning the listing.
    pub fn list(&self, network: Network) -> Vec<([u8; 32], IdentityMeta)> {
        let prefix = prefix_for(network);
        let keys = match self.kv.list(DetScope::Global, Some(&prefix)) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::identity_meta",
                    network = ?network,
                    error = ?e,
                    "Failed to list identity-meta keys; returning empty list",
                );
                return Vec::new();
            }
        };
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(id) = parse_identity_id(&key, &prefix) else {
                tracing::warn!(
                    target = "wallet_backend::identity_meta",
                    key = %key,
                    "Skipping identity-meta key with non-base58 id suffix",
                );
                continue;
            };
            match self.kv.get::<IdentityMeta>(DetScope::Global, &key) {
                Ok(Some(meta)) => out.push((id, meta)),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        target = "wallet_backend::identity_meta",
                        key = %key,
                        error = ?e,
                        "Skipping unreadable identity-meta blob",
                    );
                }
            }
        }
        out
    }

    /// Fetch the metadata for a single identity. `None` when the key is absent
    /// or the blob fails to decode (logged) — the sidecar is cosmetic, so a
    /// read never fails the caller.
    pub fn get(&self, network: Network, identity_id: &[u8; 32]) -> Option<IdentityMeta> {
        let key = key_for(network, identity_id);
        match self.kv.get::<IdentityMeta>(DetScope::Global, &key) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::identity_meta",
                    key = %key,
                    error = ?e,
                    "Failed to read identity meta; treating as absent",
                );
                None
            }
        }
    }

    /// Upsert the metadata for a single identity. Re-writing the same value is
    /// an idempotent overwrite (DetKv upserts by key).
    pub fn set(
        &self,
        network: Network,
        identity_id: &[u8; 32],
        meta: &IdentityMeta,
    ) -> Result<(), TaskError> {
        let key = key_for(network, identity_id);
        self.kv
            .put(DetScope::Global, &key, meta)
            .map_err(map_kv_error_to_task_error)
    }

    /// Delete the metadata for a single identity. Idempotent — a missing key
    /// returns `Ok(())`.
    pub fn delete(&self, network: Network, identity_id: &[u8; 32]) -> Result<(), TaskError> {
        let key = key_for(network, identity_id);
        self.kv
            .delete(DetScope::Global, &key)
            .map_err(map_kv_error_to_task_error)
    }
}

/// Identity-meta adapter errors funnel into [`TaskError::IdentityMetaStorage`]
/// so the banner copy matches the surface ("identity details").
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    TaskError::IdentityMetaStorage {
        source: Box::new(e),
    }
}

/// Extract the base58 identity-id suffix from a key starting with `prefix`.
/// Returns `None` when the suffix is not 32 bytes of base58.
fn parse_identity_id(key: &str, prefix: &str) -> Option<[u8; 32]> {
    let rest = key.strip_prefix(prefix)?;
    let bytes = base58::decode(rest).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use platform_wallet_storage::{KvError, KvStore, ObjectId};

    /// Minimal in-memory `KvStore` — mirrors the `wallet_meta` test fixture.
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

    fn meta(hint: Option<&str>) -> IdentityMeta {
        IdentityMeta {
            password_hint: hint.map(str::to_string),
        }
    }

    /// ID-META-VIEW-001 — a written meta round-trips through `get` and shows up
    /// in `list` for the same network.
    #[test]
    fn set_then_get_round_trips() {
        let kv = kv();
        let view = IdentityMetaView::new(&kv);
        let id = [0x11; 32];
        let m = meta(Some("granny's birthday"));
        view.set(Network::Testnet, &id, &m).expect("set");
        assert_eq!(view.get(Network::Testnet, &id), Some(m.clone()));
        assert_eq!(view.list(Network::Testnet), vec![(id, m)]);
    }

    /// ID-META-VIEW-002 — `set` overwrites; updating the hint is one upsert.
    #[test]
    fn set_overwrites_existing_entry() {
        let kv = kv();
        let view = IdentityMetaView::new(&kv);
        let id = [0x22; 32];
        view.set(Network::Mainnet, &id, &meta(Some("old")))
            .expect("first set");
        view.set(Network::Mainnet, &id, &meta(Some("new")))
            .expect("second set");
        assert_eq!(view.get(Network::Mainnet, &id), Some(meta(Some("new"))));
    }

    /// ID-META-VIEW-003 — `list` does not leak entries from other networks (the
    /// `<network>:` prefix is the partition).
    #[test]
    fn list_partitions_by_network() {
        let kv = kv();
        let view = IdentityMetaView::new(&kv);
        let a = [0x33; 32];
        let b = [0x44; 32];
        view.set(Network::Testnet, &a, &meta(Some("on testnet")))
            .unwrap();
        view.set(Network::Mainnet, &b, &meta(Some("on mainnet")))
            .unwrap();
        assert_eq!(
            view.list(Network::Testnet),
            vec![(a, meta(Some("on testnet")))]
        );
        assert_eq!(
            view.list(Network::Mainnet),
            vec![(b, meta(Some("on mainnet")))]
        );
    }

    /// ID-META-VIEW-004 — `delete` is idempotent.
    #[test]
    fn delete_is_idempotent() {
        let kv = kv();
        let view = IdentityMetaView::new(&kv);
        let id = [0x55; 32];
        view.delete(Network::Testnet, &id).expect("delete absent");
        view.set(Network::Testnet, &id, &meta(Some("x"))).unwrap();
        view.delete(Network::Testnet, &id).expect("first delete");
        view.delete(Network::Testnet, &id).expect("second delete");
        assert_eq!(view.get(Network::Testnet, &id), None);
    }

    /// ID-META-VIEW-005 — `get` on a missing key returns `None` rather than
    /// erroring (graceful-degradation contract).
    #[test]
    fn get_missing_returns_none() {
        let kv = kv();
        let view = IdentityMetaView::new(&kv);
        assert_eq!(view.get(Network::Devnet, &[0x66; 32]), None);
    }

    /// ID-META-VIEW-006 — the canonical key shape uses base58 for the 32-byte
    /// identity id; locks the shape so a future change needs a migration.
    #[test]
    fn key_for_uses_base58_identity_id() {
        let id = [0xAB; 32];
        let key = key_for(Network::Mainnet, &id);
        assert!(key.starts_with("mainnet:identity_meta:"));
        let suffix = key.trim_start_matches("mainnet:identity_meta:");
        assert_eq!(base58::decode(suffix).expect("base58").as_slice(), &id[..]);
    }
}
