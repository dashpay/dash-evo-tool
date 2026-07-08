//! DET-side identity-metadata view.
//!
//! [`IdentityMetaView`] is the only doorway DET code uses to read or write
//! [`IdentityMeta`] (the password hint shown next to the sign-time prompt) for
//! an identity whose keys are password-protected. It is a thin
//! [`SidecarView`](crate::wallet_backend::sidecar::SidecarView) wrapper over a
//! shared [`DetKv`] handle pointing at `det-app.sqlite`, serialising every entry
//! under a colon-prefixed, network-scoped key:
//!
//! ```text
//! <network>:identity_meta:<identity_id_base58>
//! ```
//!
//! Network-prefixed keys + the global (`DetScope::Global`) scope mean the
//! cross-network `det-app.sqlite` file is the right store (one file, one schema,
//! easy backup), and the 32-byte identity id is the stable DET-level identifier.
//!
//! This sidecar is **display-only** — it never gates whether a prompt fires
//! (the at-rest vault scheme does). Every read path is therefore infallible at
//! the value level: a missing key returns `None`, a corrupt blob is logged and
//! treated as absent so the prompt degrades to "no hint" rather than failing.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;

use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::identity_meta::IdentityMeta;
use crate::wallet_backend::DetKv;
use crate::wallet_backend::kv::{KvAdapterError, map_kv_storage_error};
#[cfg(test)]
use crate::wallet_backend::sidecar::{SidecarId, sidecar_key};
use crate::wallet_backend::sidecar::{SidecarScope, SidecarValue, SidecarView};

/// Colon-separated namespace shared across networks. The full key is
/// `<network>:identity_meta:<identity_id_base58>`.
pub(crate) const KEY_INFIX: &str = ":identity_meta:";

/// Build the canonical k/v key for an identity's metadata blob. The generic
/// view builds keys itself; this mirror exists for key-shape tests.
#[cfg(test)]
pub(crate) fn key_for(network: Network, identity_id: &SidecarId) -> String {
    sidecar_key(network, KEY_INFIX, identity_id)
}

impl SidecarValue for IdentityMeta {}

/// Typed identity-metadata sidecar. A thin, display-only wrapper over the
/// generic [`SidecarView`]: identity metadata (the password hint shown next to
/// the sign-time prompt) is `Global`-scoped and never gates whether a prompt
/// fires, so every read degrades to `None` on error.
pub struct IdentityMetaView<'a>(SidecarView<'a, IdentityMeta>);

impl<'a> IdentityMetaView<'a> {
    /// Borrow a [`DetKv`] handle as a typed identity-metadata view.
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self(SidecarView::new(
            kv,
            KEY_INFIX,
            SidecarScope::Global,
            map_kv_error_to_task_error,
        ))
    }

    /// All `(identity_id, meta)` pairs persisted for `network`. A single
    /// corrupt row is logged and skipped rather than poisoning the listing.
    pub fn list(&self, network: Network) -> Vec<([u8; 32], IdentityMeta)> {
        self.0.list(network)
    }

    /// Fetch the metadata for a single identity. `None` when the key is absent
    /// or the blob fails to decode (logged) — the sidecar is cosmetic, so a
    /// read never fails the caller.
    pub fn get(&self, network: Network, identity_id: &[u8; 32]) -> Option<IdentityMeta> {
        self.0.get(network, identity_id)
    }

    /// Upsert the metadata for a single identity. Re-writing the same value is
    /// an idempotent overwrite (DetKv upserts by key).
    pub fn set(
        &self,
        network: Network,
        identity_id: &[u8; 32],
        meta: &IdentityMeta,
    ) -> Result<(), TaskError> {
        self.0.set(network, identity_id, meta)
    }

    /// Delete the metadata for a single identity. Idempotent — a missing key
    /// returns `Ok(())`.
    pub fn delete(&self, network: Network, identity_id: &[u8; 32]) -> Result<(), TaskError> {
        self.0.delete(network, identity_id)
    }
}

/// Identity-meta adapter errors funnel into [`TaskError::IdentityMetaStorage`]
/// so the banner copy matches the surface ("identity details").
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    map_kv_storage_error(e, |source| TaskError::IdentityMetaStorage { source })
}

#[cfg(test)]
mod tests {
    use super::*;

    use dash_sdk::dpp::dashcore::base58;

    use crate::wallet_backend::kv_test_support::InMemoryKv;

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
