//! Typed key/value adapter on top of the upstream `KvStore`.
//!
//! [`DetKv`] is the DET-side façade for the upstream
//! [`platform_wallet_storage::KvStore`]: it serializes values with
//! `bincode` behind a one-byte schema version prefix, validates the
//! schema byte on read, and exposes a small `get / put / delete / list`
//! surface keyed by [`DetScope`].
//!
//! ## Scope seam
//!
//! Callers address entries with [`DetScope`] — a DET-owned enum that
//! never exposes the upstream `ObjectId` / `WalletId` types. The mapping
//! to upstream [`platform_wallet_storage::ObjectId`] happens in exactly
//! one place ([`to_object_id`]) so the wallet-backend seam stays clean.
//! [`DetScope::Global`] and [`DetScope::Wallet`] are used for cross-network
//! settings and per-wallet data respectively. [`DetScope::Identity`] is
//! active: identities, top-up history, scheduled votes, and the DashPay
//! `private` / `address_index` overlays are all identity-scoped.
//!
//! All keys carried by this adapter follow a colon-separated namespace
//! convention. Whether a key needs a `<network>:` prefix depends on the
//! store behind it, not on the scope: entries in the cross-network
//! `det-app.sqlite` (wallet-meta and single-key sidecars, migration
//! sentinels) share one file across every network and must carry the
//! prefix so mainnet / testnet / devnet cannot collide. Entries in the
//! per-network `spv/<net>/platform-wallet.sqlite` get one file per
//! network already, so they omit it — see `det:identity_index:v1` and
//! its siblings. `docs/kv-keys.md` catalogues every key and its store.
//!
//! ## Encoding
//!
//! Each value is `[ SCHEMA_VERSION (1B) | bincode(payload) ]`, where the
//! payload is encoded with [`bincode::serde::encode_to_vec`] using
//! `bincode::config::standard()`. Reads validate the leading byte and
//! return [`KvAdapterError::SchemaVersion`] if it does not match the
//! adapter's `SCHEMA_VERSION` constant. Bumping the version is a
//! deliberate breaking change — readers will refuse mismatched blobs
//! rather than guessing.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use platform_wallet_storage::{KvError, KvStore, ObjectId, SqlitePersister};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;

/// Stable lowercase network token (`mainnet` / `testnet` / `devnet` /
/// `regtest`) used across the wallet-backend sidecars, SPV storage paths, and
/// migration sentinels.
///
/// Hardcoded rather than derived from [`Network`]'s `Display` so an upstream
/// change to that impl cannot silently shift already-persisted on-disk keys.
pub(crate) fn network_prefix(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

/// Schema version prefix prepended to every encoded value, the envelope
/// that makes stored blobs migratable.
///
/// `bincode::config::standard()` is positional and non-self-describing:
/// fields are written in declaration order with no names or tags, so
/// adding, removing, or reordering a payload struct's fields is a
/// format-breaking change for already-stored blobs (a `#[serde(default)]`
/// attribute does not rescue an old blob — there are no field names to
/// match). This version byte is the only forward-compatibility mechanism:
/// bump it whenever a payload struct's wire shape changes, then teach
/// readers to migrate or reject the old version explicitly.
pub const SCHEMA_VERSION: u8 = 1;

/// DET-side metadata scope. Maps onto the upstream object-scoped key/value
/// store without leaking the upstream `ObjectId` / `WalletId` types past
/// the wallet-backend seam.
///
/// `Global` survives wallet deletion; every other variant anchors its
/// metadata to a parent object that cascades on removal. `Wallet` borrows
/// a [`WalletSeedHash`] (transparently the same `[u8; 32]` the upstream
/// store uses as its `WalletId`). `Identity` is active — identities,
/// top-up history, scheduled votes, and the DashPay `private` /
/// `address_index` overlays are all identity-scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetScope<'a> {
    /// Global app metadata; no parent, survives wallet deletion.
    Global,
    /// Per-wallet metadata; cascades when the wallet is removed.
    Wallet(&'a WalletSeedHash),
    /// Per-identity metadata; cascades when the identity is removed. Active:
    /// identities, top-up history, scheduled votes, and the DashPay
    /// `private` / `address_index` overlays are all identity-scoped.
    Identity(&'a [u8; 32]),
}

/// Map a DET-side [`DetScope`] onto the upstream [`ObjectId`]. The single
/// chokepoint where the upstream scope type is constructed — keeps
/// `ObjectId` / `WalletId` confined to this module.
fn to_object_id(scope: DetScope<'_>) -> ObjectId {
    match scope {
        DetScope::Global => ObjectId::Global,
        DetScope::Wallet(seed_hash) => ObjectId::Wallet(*seed_hash),
        DetScope::Identity(identity_id) => ObjectId::Identity(*identity_id),
    }
}

/// Errors returned by the [`DetKv`] adapter.
#[derive(Debug, thiserror::Error)]
pub enum KvAdapterError {
    /// The underlying key/value store rejected an operation. The store accepts
    /// scoped writes whose parent object does not exist yet (an `AFTER DELETE`
    /// trigger reaps the metadata if the object is later removed), so there is
    /// no FK-violation variant to promote — every store error maps here.
    #[error("kv store error")]
    Store(#[source] KvError),

    /// A stored value's schema version byte did not match
    /// [`SCHEMA_VERSION`]. Treated as a hard error rather than a silent
    /// fallback so corrupted or future-format blobs are loud.
    #[error("kv value has unexpected schema version {found} (expected {expected})")]
    SchemaVersion { expected: u8, found: u8 },

    /// A stored value was empty — the leading schema byte is mandatory.
    #[error("kv value is empty (missing schema version byte)")]
    Truncated,

    /// `bincode` failed to encode a value for storage.
    #[error("kv value encode failed")]
    Encode(#[from] bincode::error::EncodeError),

    /// `bincode` failed to decode a stored value.
    #[error("kv value decode failed")]
    Decode(#[from] bincode::error::DecodeError),
}

/// Typed key/value adapter. Cheap to clone (`Arc<dyn KvStore>` inside).
#[derive(Clone)]
pub struct DetKv {
    store: Arc<dyn KvStore + Send + Sync>,
}

impl DetKv {
    /// Wrap an upstream [`SqlitePersister`] as the backing store.
    pub fn new(persister: Arc<SqlitePersister>) -> Self {
        Self { store: persister }
    }

    /// Construct from any [`KvStore`] implementor. Used by tests that
    /// want to inject an in-memory backend.
    pub fn from_store(store: Arc<dyn KvStore + Send + Sync>) -> Self {
        Self { store }
    }

    /// Read and decode the value bound to `(scope, key)`. Returns
    /// `Ok(None)` when the key is absent.
    pub fn get<T: DeserializeOwned>(
        &self,
        scope: DetScope<'_>,
        key: &str,
    ) -> Result<Option<T>, KvAdapterError> {
        let raw = self
            .store
            .get(&to_object_id(scope), key)
            .map_err(KvAdapterError::Store)?;
        let Some(bytes) = raw else {
            return Ok(None);
        };
        let (&first, rest) = bytes.split_first().ok_or(KvAdapterError::Truncated)?;
        if first != SCHEMA_VERSION {
            return Err(KvAdapterError::SchemaVersion {
                expected: SCHEMA_VERSION,
                found: first,
            });
        }
        let (value, _) =
            bincode::serde::decode_from_slice::<T, _>(rest, bincode::config::standard())?;
        Ok(Some(value))
    }

    /// Encode and upsert the value bound to `(scope, key)`.
    pub fn put<T: Serialize>(
        &self,
        scope: DetScope<'_>,
        key: &str,
        value: &T,
    ) -> Result<(), KvAdapterError> {
        let mut buf = Vec::with_capacity(64);
        buf.push(SCHEMA_VERSION);
        let body = bincode::serde::encode_to_vec(value, bincode::config::standard())?;
        buf.extend_from_slice(&body);
        self.store
            .put(&to_object_id(scope), key, &buf)
            .map_err(KvAdapterError::Store)?;
        Ok(())
    }

    /// Idempotent delete — a missing key returns `Ok(())`.
    pub fn delete(&self, scope: DetScope<'_>, key: &str) -> Result<(), KvAdapterError> {
        self.store
            .delete(&to_object_id(scope), key)
            .map_err(KvAdapterError::Store)?;
        Ok(())
    }

    /// List keys in the given scope. `prefix = None` returns every
    /// key in the scope. The upstream store escapes pattern
    /// metacharacters in `prefix`, so any colon-separated namespace
    /// is treated literally.
    pub fn list(
        &self,
        scope: DetScope<'_>,
        prefix: Option<&str>,
    ) -> Result<Vec<String>, KvAdapterError> {
        self.store
            .list_keys(&to_object_id(scope), prefix)
            .map_err(KvAdapterError::Store)
    }
}

impl std::fmt::Debug for DetKv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetKv").finish_non_exhaustive()
    }
}

/// Read `(scope, key)` as `T`, treating a decode/store failure as absence.
///
/// Absence (`Ok(None)`) returns `None` silently — the steady state for an
/// unwritten optional slot. Only an error is logged (at `warn`), tagged with
/// `context` so the failing sidecar is identifiable. The single home for the
/// "read a best-effort k/v value, degrade to absent on error" pattern.
pub(crate) fn kv_get_logged<T: DeserializeOwned>(
    kv: &DetKv,
    scope: DetScope<'_>,
    key: &str,
    context: &str,
) -> Option<T> {
    match kv.get::<T>(scope, key) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "wallet_backend::kv",
                context,
                key = %key,
                error = ?e,
                "Failed to read k/v value; treating as absent",
            );
            None
        }
    }
}

/// [`kv_get_logged`] with a [`Default`] fallback for both absence and error.
/// Callers that always want a concrete value (never an `Option`) use this.
pub(crate) fn kv_get_or_default<T: DeserializeOwned + Default>(
    kv: &DetKv,
    scope: DetScope<'_>,
    key: &str,
    context: &str,
) -> T {
    kv_get_logged(kv, scope, key, context).unwrap_or_default()
}

/// Wrap a [`KvAdapterError`] into the caller's chosen [`TaskError`] variant.
///
/// The single boxing/funnel point shared by every offline-cache and sidecar
/// mapper: `into_variant` names which surface failed (avatar cache, contact
/// profile, wallet/identity metadata) so the banner copy matches while the
/// error chain is preserved through the boxed `#[source]`.
pub(crate) fn map_kv_storage_error(
    e: KvAdapterError,
    into_variant: impl FnOnce(Box<KvAdapterError>) -> TaskError,
) -> TaskError {
    into_variant(Box::new(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::kv_test_support::InMemoryKv;

    fn fixture() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    #[derive(Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
    struct Sample {
        name: String,
        value: u64,
    }

    /// K1: a value written with `put` round-trips through `get` with the
    /// schema byte transparently stripped.
    #[test]
    fn put_then_get_round_trips() {
        let kv = fixture();
        let v = Sample {
            name: "alpha".to_string(),
            value: 42,
        };
        kv.put(DetScope::Global, "mainnet:settings:v1", &v).unwrap();
        let got: Option<Sample> = kv.get(DetScope::Global, "mainnet:settings:v1").unwrap();
        assert_eq!(got, Some(v));
    }

    /// K2: a missing key is signalled with `Ok(None)`, never an error
    /// — callers branch on the option, not on a sentinel error.
    #[test]
    fn get_missing_returns_none() {
        let kv = fixture();
        let got: Option<Sample> = kv.get(DetScope::Global, "mainnet:settings:v1").unwrap();
        assert!(got.is_none());
    }

    /// K3: a global put and a per-wallet put under the same key do not
    /// alias — they live in independent scopes (mirrors the upstream
    /// partitioned-table contract).
    #[test]
    fn global_and_wallet_scopes_are_independent() {
        let kv = fixture();
        let wallet: WalletSeedHash = [7u8; 32];
        let g = Sample {
            name: "global".to_string(),
            value: 1,
        };
        let w = Sample {
            name: "wallet".to_string(),
            value: 2,
        };
        kv.put(DetScope::Global, "shared", &g).unwrap();
        kv.put(DetScope::Wallet(&wallet), "shared", &w).unwrap();
        let got_g: Option<Sample> = kv.get(DetScope::Global, "shared").unwrap();
        let got_w: Option<Sample> = kv.get(DetScope::Wallet(&wallet), "shared").unwrap();
        assert_eq!(got_g, Some(g));
        assert_eq!(got_w, Some(w));
    }

    /// K4: `put` upserts — a second write of the same key replaces the
    /// previous value rather than failing on conflict.
    #[test]
    fn put_upserts() {
        let kv = fixture();
        kv.put(
            DetScope::Global,
            "k",
            &Sample {
                name: "first".to_string(),
                value: 1,
            },
        )
        .unwrap();
        kv.put(
            DetScope::Global,
            "k",
            &Sample {
                name: "second".to_string(),
                value: 2,
            },
        )
        .unwrap();
        let got: Sample = kv.get(DetScope::Global, "k").unwrap().unwrap();
        assert_eq!(got.name, "second");
        assert_eq!(got.value, 2);
    }

    /// K5: `delete` is idempotent — deleting a missing key is `Ok(())`,
    /// matching the upstream KvStore contract.
    #[test]
    fn delete_is_idempotent() {
        let kv = fixture();
        kv.delete(DetScope::Global, "absent").unwrap();
        kv.put(
            DetScope::Global,
            "k",
            &Sample {
                name: "v".to_string(),
                value: 0,
            },
        )
        .unwrap();
        kv.delete(DetScope::Global, "k").unwrap();
        kv.delete(DetScope::Global, "k").unwrap();
        let got: Option<Sample> = kv.get(DetScope::Global, "k").unwrap();
        assert!(got.is_none());
    }

    /// K6: a corrupted leading byte surfaces as `SchemaVersion` — the
    /// adapter never silently misinterprets a foreign blob as a valid
    /// value.
    #[test]
    fn schema_mismatch_is_loud() {
        let store = Arc::new(InMemoryKv::default());
        // Bypass the adapter to plant a value with the wrong leading byte.
        let mut raw = vec![SCHEMA_VERSION.wrapping_add(1)];
        raw.extend(
            bincode::serde::encode_to_vec(
                &Sample {
                    name: "x".to_string(),
                    value: 0,
                },
                bincode::config::standard(),
            )
            .unwrap(),
        );
        store.put(&ObjectId::Global, "k", &raw).unwrap();
        let kv = DetKv::from_store(store);
        match kv.get::<Sample>(DetScope::Global, "k") {
            Err(KvAdapterError::SchemaVersion { expected, found }) => {
                assert_eq!(expected, SCHEMA_VERSION);
                assert_eq!(found, SCHEMA_VERSION.wrapping_add(1));
            }
            other => panic!("expected SchemaVersion error, got {other:?}"),
        }
    }

    /// K7: a zero-byte stored value is reported as `Truncated` rather
    /// than panicking or returning an empty success.
    #[test]
    fn empty_blob_is_truncated() {
        let store = Arc::new(InMemoryKv::default());
        store.put(&ObjectId::Global, "k", &[]).unwrap();
        let kv = DetKv::from_store(store);
        match kv.get::<Sample>(DetScope::Global, "k") {
            Err(KvAdapterError::Truncated) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    /// K8: `list` honours the prefix filter and returns keys in the
    /// scope only — global and per-wallet listings stay partitioned.
    #[test]
    fn list_respects_scope_and_prefix() {
        let kv = fixture();
        let wallet: WalletSeedHash = [3u8; 32];
        let v = Sample {
            name: "v".to_string(),
            value: 0,
        };
        kv.put(DetScope::Global, "mainnet:settings:v1", &v).unwrap();
        kv.put(DetScope::Global, "mainnet:scheduled_votes:1", &v)
            .unwrap();
        kv.put(DetScope::Global, "testnet:settings:v1", &v).unwrap();
        kv.put(DetScope::Wallet(&wallet), "dashpay:contact:abc", &v)
            .unwrap();

        let mut globals = kv.list(DetScope::Global, Some("mainnet:")).unwrap();
        globals.sort();
        assert_eq!(
            globals,
            vec![
                "mainnet:scheduled_votes:1".to_string(),
                "mainnet:settings:v1".to_string(),
            ]
        );

        let wallet_keys = kv.list(DetScope::Wallet(&wallet), None).unwrap();
        assert_eq!(wallet_keys, vec!["dashpay:contact:abc".to_string()]);

        let no_match = kv.list(DetScope::Global, Some("regtest:")).unwrap();
        assert!(no_match.is_empty());
    }

    /// K10: `network_prefix` is a stable lowercase token per network — the
    /// on-disk key shape must not shift.
    #[test]
    fn network_prefix_is_stable() {
        assert_eq!(network_prefix(Network::Mainnet), "mainnet");
        assert_eq!(network_prefix(Network::Testnet), "testnet");
        assert_eq!(network_prefix(Network::Devnet), "devnet");
        assert_eq!(network_prefix(Network::Regtest), "regtest");
    }

    /// K11: `kv_get_or_default` returns the stored value when present and the
    /// `Default` for both a missing key and a corrupt (wrong-schema) blob.
    #[test]
    fn kv_get_or_default_present_missing_and_corrupt() {
        let store = Arc::new(InMemoryKv::default());
        let kv = DetKv::from_store(store.clone());

        kv.put(DetScope::Global, "present", &7u64).unwrap();
        let got: u64 = kv_get_or_default(&kv, DetScope::Global, "present", "k11");
        assert_eq!(got, 7);

        let missing: u64 = kv_get_or_default(&kv, DetScope::Global, "absent", "k11");
        assert_eq!(missing, 0);

        // Plant a blob with a bad leading schema byte: read degrades to default.
        store
            .put(
                &ObjectId::Global,
                "corrupt",
                &[SCHEMA_VERSION.wrapping_add(1), 0x00],
            )
            .unwrap();
        let corrupt: u64 = kv_get_or_default(&kv, DetScope::Global, "corrupt", "k11");
        assert_eq!(corrupt, 0);
    }

    /// K12: `kv_get_logged` reports absence and read failure as `None`, and the
    /// stored value as `Some`.
    #[test]
    fn kv_get_logged_reports_none_on_absence_and_error() {
        let kv = fixture();
        assert_eq!(
            kv_get_logged::<u64>(&kv, DetScope::Global, "absent", "k12"),
            None
        );
        kv.put(DetScope::Global, "present", &42u64).unwrap();
        assert_eq!(
            kv_get_logged::<u64>(&kv, DetScope::Global, "present", "k12"),
            Some(42)
        );
    }

    /// K9: an upstream store failure surfaces on the generic `Store` arm.
    /// The store now accepts writes whose parent object is absent, so there
    /// is no FK-violation variant to special-case.
    #[test]
    fn store_error_rides_store_arm() {
        struct FailingKv;
        impl KvStore for FailingKv {
            fn get(&self, _scope: &ObjectId, _key: &str) -> Result<Option<Vec<u8>>, KvError> {
                Ok(None)
            }
            fn put(&self, _scope: &ObjectId, _key: &str, _value: &[u8]) -> Result<(), KvError> {
                Err(KvError::LockPoisoned)
            }
            fn delete(&self, _scope: &ObjectId, _key: &str) -> Result<(), KvError> {
                Ok(())
            }
            fn list_keys(
                &self,
                _scope: &ObjectId,
                _prefix: Option<&str>,
            ) -> Result<Vec<String>, KvError> {
                Ok(Vec::new())
            }
        }
        let kv = DetKv::from_store(Arc::new(FailingKv));
        let wallet: WalletSeedHash = [9u8; 32];
        match kv.put(
            DetScope::Wallet(&wallet),
            "k",
            &Sample {
                name: "x".to_string(),
                value: 0,
            },
        ) {
            Err(KvAdapterError::Store(KvError::LockPoisoned)) => {}
            other => panic!("expected Store(LockPoisoned), got {other:?}"),
        }
    }
}
