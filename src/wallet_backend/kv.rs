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
//! [`DetScope::Global`] and [`DetScope::Wallet`] are the only scopes used
//! today; [`DetScope::Identity`] and [`DetScope::Token`] are defined now
//! and wired through the mapping, reserved for the Wave 2 scope
//! promotions (they need an upstream FK relaxation before they can be
//! written to safely).
//!
//! All keys carried by this adapter follow a colon-separated namespace
//! convention, with a mandatory `<network>:` prefix for global slots so
//! mainnet / testnet / devnet entries cannot collide inside the same
//! upstream database file. See the documentation on the consumer
//! callers (e.g. settings storage) for the canonical key schema.
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

use platform_wallet_storage::kv::ObjectKind;
use platform_wallet_storage::{KvError, KvStore, ObjectId, SqlitePersister};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model::wallet::WalletSeedHash;

/// Schema version prefix prepended to every encoded value. Mirrors the
/// upstream `entry_blob` convention so future readers can detect
/// format-breaking changes deterministically. Bump only when the
/// encoding scheme itself changes (not when payload structs evolve —
/// bincode already tolerates compatible struct evolution).
pub const SCHEMA_VERSION: u8 = 1;

/// DET-side metadata scope. Maps onto the upstream object-scoped key/value
/// store without leaking the upstream `ObjectId` / `WalletId` types past
/// the wallet-backend seam.
///
/// `Global` survives wallet deletion; every other variant anchors its
/// metadata to a parent object that cascades on removal. `Wallet` borrows
/// a [`WalletSeedHash`] (transparently the same `[u8; 32]` the upstream
/// store uses as its `WalletId`). `Identity` and `Token` are reserved for
/// the Wave 2 scope promotions — defined and mapped now, not yet written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetScope<'a> {
    /// Global app metadata; no parent, survives wallet deletion.
    Global,
    /// Per-wallet metadata; cascades when the wallet is removed.
    Wallet(&'a WalletSeedHash),
    /// Per-identity metadata. Reserved for Wave 2.
    Identity(&'a [u8; 32]),
    /// Per-token-balance metadata. Reserved for Wave 2.
    Token {
        identity_id: &'a [u8; 32],
        token_id: &'a [u8; 32],
    },
}

/// Map a DET-side [`DetScope`] onto the upstream [`ObjectId`]. The single
/// chokepoint where the upstream scope type is constructed — keeps
/// `ObjectId` / `WalletId` confined to this module.
fn to_object_id(scope: DetScope<'_>) -> ObjectId {
    match scope {
        DetScope::Global => ObjectId::Global,
        DetScope::Wallet(seed_hash) => ObjectId::Wallet(*seed_hash),
        DetScope::Identity(identity_id) => ObjectId::Identity(*identity_id),
        DetScope::Token {
            identity_id,
            token_id,
        } => ObjectId::Token {
            identity_id: *identity_id,
            token_id: *token_id,
        },
    }
}

/// Object kind a scoped write referenced — DET mirror of the upstream
/// `ObjectKind`, kept so [`KvAdapterError`] carries no upstream type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKindLite {
    /// A wallet.
    Wallet,
    /// An identity.
    Identity,
    /// A token balance.
    Token,
    /// An established contact.
    Contact,
    /// A platform address.
    PlatformAddress,
}

impl From<ObjectKind> for ObjectKindLite {
    fn from(kind: ObjectKind) -> Self {
        match kind {
            ObjectKind::Wallet => ObjectKindLite::Wallet,
            ObjectKind::Identity => ObjectKindLite::Identity,
            ObjectKind::Token => ObjectKindLite::Token,
            ObjectKind::Contact => ObjectKindLite::Contact,
            ObjectKind::PlatformAddress => ObjectKindLite::PlatformAddress,
        }
    }
}

/// Errors returned by the [`DetKv`] adapter.
#[derive(Debug, thiserror::Error)]
pub enum KvAdapterError {
    /// The underlying key/value store rejected an operation.
    #[error("kv store error")]
    Store(#[source] KvError),

    /// A scoped write referenced a parent object that does not exist in
    /// the store. Carries the DET-side [`ObjectKindLite`] so callers can
    /// branch on the missing parent's kind without an upstream type.
    #[error("kv parent object not found: {kind:?}")]
    ObjectNotFound { kind: ObjectKindLite },

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

/// Convert an upstream [`KvError`] into a [`KvAdapterError`], promoting the
/// FK-violation variant to the typed [`KvAdapterError::ObjectNotFound`]
/// instead of letting it ride the generic [`KvAdapterError::Store`] arm.
fn map_kv_error(err: KvError) -> KvAdapterError {
    match err {
        KvError::ObjectNotFound { kind } => KvAdapterError::ObjectNotFound { kind: kind.into() },
        other => KvAdapterError::Store(other),
    }
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
            .map_err(map_kv_error)?;
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
            .map_err(map_kv_error)?;
        Ok(())
    }

    /// Idempotent delete — a missing key returns `Ok(())`.
    pub fn delete(&self, scope: DetScope<'_>, key: &str) -> Result<(), KvAdapterError> {
        self.store
            .delete(&to_object_id(scope), key)
            .map_err(map_kv_error)?;
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
            .map_err(map_kv_error)
    }
}

impl std::fmt::Debug for DetKv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetKv").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory KvStore implementation for the adapter tests.
    ///
    /// Models every [`ObjectId`] scope FK-free (no parent-existence
    /// checks) so the adapter can be exercised without a real
    /// `SqlitePersister`:
    /// - each scope is an independent slot;
    /// - `put` is upsert;
    /// - `delete` is idempotent;
    /// - `list_keys` supports an optional prefix and returns sorted keys.
    ///
    /// Upstream `ObjectId` is not `Ord`, so the backing store is a flat
    /// `Vec` scanned by `PartialEq` rather than a map. LIKE-pattern
    /// escaping is irrelevant for the adapter — colon separators are not
    /// pattern metacharacters — so prefix matching here is plain
    /// `str::starts_with`.
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

    /// K9: the upstream FK-violation variant is promoted to the typed
    /// `ObjectNotFound` with the kind mapped to the DET mirror — it does
    /// NOT ride the generic `Store` arm.
    #[test]
    fn object_not_found_is_promoted() {
        struct FkRejectingKv;
        impl KvStore for FkRejectingKv {
            fn get(&self, _scope: &ObjectId, _key: &str) -> Result<Option<Vec<u8>>, KvError> {
                Ok(None)
            }
            fn put(&self, _scope: &ObjectId, _key: &str, _value: &[u8]) -> Result<(), KvError> {
                Err(KvError::ObjectNotFound {
                    kind: ObjectKind::Wallet,
                })
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
        let kv = DetKv::from_store(Arc::new(FkRejectingKv));
        let wallet: WalletSeedHash = [9u8; 32];
        match kv.put(
            DetScope::Wallet(&wallet),
            "k",
            &Sample {
                name: "x".to_string(),
                value: 0,
            },
        ) {
            Err(KvAdapterError::ObjectNotFound {
                kind: ObjectKindLite::Wallet,
            }) => {}
            other => panic!("expected ObjectNotFound(Wallet), got {other:?}"),
        }
    }
}
