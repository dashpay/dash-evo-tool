//! Typed key/value adapter on top of the upstream `KvStore`.
//!
//! [`DetKv`] is the DET-side façade for the upstream
//! [`platform_wallet_storage::KvStore`]: it serializes values with
//! `bincode` behind a one-byte schema version prefix, validates the
//! schema byte on read, and exposes a small `get / put / delete / list`
//! surface keyed by `Option<&WalletId>` (`None` = global slot, `Some`
//! = per-wallet, cascades on wallet delete).
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

use platform_wallet::wallet::platform_wallet::WalletId;
use platform_wallet_storage::{KvError, KvStore, SqlitePersister};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Schema version prefix prepended to every encoded value. Mirrors the
/// upstream `entry_blob` convention so future readers can detect
/// format-breaking changes deterministically. Bump only when the
/// encoding scheme itself changes (not when payload structs evolve —
/// bincode already tolerates compatible struct evolution).
pub const SCHEMA_VERSION: u8 = 1;

/// Errors returned by the [`DetKv`] adapter.
#[derive(Debug, thiserror::Error)]
pub enum KvAdapterError {
    /// The underlying key/value store rejected an operation.
    #[error("kv store error")]
    Store(#[from] KvError),

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

    /// Read and decode the value bound to `(wallet_id, key)`. Returns
    /// `Ok(None)` when the key is absent.
    pub fn get<T: DeserializeOwned>(
        &self,
        wallet_id: Option<&WalletId>,
        key: &str,
    ) -> Result<Option<T>, KvAdapterError> {
        let raw = self.store.get(wallet_id, key)?;
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

    /// Encode and upsert the value bound to `(wallet_id, key)`.
    pub fn put<T: Serialize>(
        &self,
        wallet_id: Option<&WalletId>,
        key: &str,
        value: &T,
    ) -> Result<(), KvAdapterError> {
        let mut buf = Vec::with_capacity(64);
        buf.push(SCHEMA_VERSION);
        let body = bincode::serde::encode_to_vec(value, bincode::config::standard())?;
        buf.extend_from_slice(&body);
        self.store.put(wallet_id, key, &buf)?;
        Ok(())
    }

    /// Idempotent delete — a missing key returns `Ok(())`.
    pub fn delete(&self, wallet_id: Option<&WalletId>, key: &str) -> Result<(), KvAdapterError> {
        self.store.delete(wallet_id, key)?;
        Ok(())
    }

    /// List keys in the given scope. `prefix = None` returns every
    /// key in the scope. The upstream store escapes pattern
    /// metacharacters in `prefix`, so any colon-separated namespace
    /// is treated literally.
    pub fn list(
        &self,
        wallet_id: Option<&WalletId>,
        prefix: Option<&str>,
    ) -> Result<Vec<String>, KvAdapterError> {
        Ok(self.store.list_keys(wallet_id, prefix)?)
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
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// In-memory KvStore implementation for the adapter tests.
    ///
    /// Mirrors the upstream `SqlitePersister` semantics for the
    /// surface this adapter exercises:
    /// - global (`None`) and per-wallet (`Some`) slots are independent;
    /// - `put` is upsert;
    /// - `delete` is idempotent;
    /// - `list_keys` supports an optional prefix and returns sorted keys.
    ///
    /// LIKE-pattern escaping is irrelevant for the adapter — colon
    /// separators are not pattern metacharacters — so prefix matching
    /// here is plain `str::starts_with`.
    #[derive(Default)]
    struct InMemoryKv {
        global: Mutex<BTreeMap<String, Vec<u8>>>,
        per_wallet: Mutex<BTreeMap<(WalletId, String), Vec<u8>>>,
    }

    impl KvStore for InMemoryKv {
        fn get(&self, wallet_id: Option<&WalletId>, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            match wallet_id {
                None => Ok(self.global.lock().unwrap().get(key).cloned()),
                Some(id) => Ok(self
                    .per_wallet
                    .lock()
                    .unwrap()
                    .get(&(*id, key.to_string()))
                    .cloned()),
            }
        }

        fn put(
            &self,
            wallet_id: Option<&WalletId>,
            key: &str,
            value: &[u8],
        ) -> Result<(), KvError> {
            match wallet_id {
                None => {
                    self.global
                        .lock()
                        .unwrap()
                        .insert(key.to_string(), value.to_vec());
                }
                Some(id) => {
                    self.per_wallet
                        .lock()
                        .unwrap()
                        .insert((*id, key.to_string()), value.to_vec());
                }
            }
            Ok(())
        }

        fn delete(&self, wallet_id: Option<&WalletId>, key: &str) -> Result<(), KvError> {
            match wallet_id {
                None => {
                    self.global.lock().unwrap().remove(key);
                }
                Some(id) => {
                    self.per_wallet
                        .lock()
                        .unwrap()
                        .remove(&(*id, key.to_string()));
                }
            }
            Ok(())
        }

        fn list_keys(
            &self,
            wallet_id: Option<&WalletId>,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            let pred = |k: &String| -> bool { prefix.is_none_or(|p| k.starts_with(p)) };
            match wallet_id {
                None => Ok(self
                    .global
                    .lock()
                    .unwrap()
                    .keys()
                    .filter(|k| pred(k))
                    .cloned()
                    .collect()),
                Some(id) => Ok(self
                    .per_wallet
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|((wid, _), _)| wid == id)
                    .map(|((_, k), _)| k.clone())
                    .filter(|k| pred(k))
                    .collect()),
            }
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
        kv.put(None, "mainnet:settings:v1", &v).unwrap();
        let got: Option<Sample> = kv.get(None, "mainnet:settings:v1").unwrap();
        assert_eq!(got, Some(v));
    }

    /// K2: a missing key is signalled with `Ok(None)`, never an error
    /// — callers branch on the option, not on a sentinel error.
    #[test]
    fn get_missing_returns_none() {
        let kv = fixture();
        let got: Option<Sample> = kv.get(None, "mainnet:settings:v1").unwrap();
        assert!(got.is_none());
    }

    /// K3: a global put and a per-wallet put under the same key do not
    /// alias — they live in independent scopes (mirrors the upstream
    /// partitioned-index contract).
    #[test]
    fn global_and_wallet_scopes_are_independent() {
        let kv = fixture();
        let wallet: WalletId = [7u8; 32];
        let g = Sample {
            name: "global".to_string(),
            value: 1,
        };
        let w = Sample {
            name: "wallet".to_string(),
            value: 2,
        };
        kv.put(None, "shared", &g).unwrap();
        kv.put(Some(&wallet), "shared", &w).unwrap();
        let got_g: Option<Sample> = kv.get(None, "shared").unwrap();
        let got_w: Option<Sample> = kv.get(Some(&wallet), "shared").unwrap();
        assert_eq!(got_g, Some(g));
        assert_eq!(got_w, Some(w));
    }

    /// K4: `put` upserts — a second write of the same key replaces the
    /// previous value rather than failing on conflict.
    #[test]
    fn put_upserts() {
        let kv = fixture();
        kv.put(
            None,
            "k",
            &Sample {
                name: "first".to_string(),
                value: 1,
            },
        )
        .unwrap();
        kv.put(
            None,
            "k",
            &Sample {
                name: "second".to_string(),
                value: 2,
            },
        )
        .unwrap();
        let got: Sample = kv.get(None, "k").unwrap().unwrap();
        assert_eq!(got.name, "second");
        assert_eq!(got.value, 2);
    }

    /// K5: `delete` is idempotent — deleting a missing key is `Ok(())`,
    /// matching the upstream KvStore contract.
    #[test]
    fn delete_is_idempotent() {
        let kv = fixture();
        kv.delete(None, "absent").unwrap();
        kv.put(
            None,
            "k",
            &Sample {
                name: "v".to_string(),
                value: 0,
            },
        )
        .unwrap();
        kv.delete(None, "k").unwrap();
        kv.delete(None, "k").unwrap();
        let got: Option<Sample> = kv.get(None, "k").unwrap();
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
        store.put(None, "k", &raw).unwrap();
        let kv = DetKv::from_store(store);
        match kv.get::<Sample>(None, "k") {
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
        store.put(None, "k", &[]).unwrap();
        let kv = DetKv::from_store(store);
        match kv.get::<Sample>(None, "k") {
            Err(KvAdapterError::Truncated) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    /// K8: `list` honours the prefix filter and returns keys in the
    /// scope only — global and per-wallet listings stay partitioned.
    #[test]
    fn list_respects_scope_and_prefix() {
        let kv = fixture();
        let wallet: WalletId = [3u8; 32];
        let v = Sample {
            name: "v".to_string(),
            value: 0,
        };
        kv.put(None, "mainnet:settings:v1", &v).unwrap();
        kv.put(None, "mainnet:scheduled_votes:1", &v).unwrap();
        kv.put(None, "testnet:settings:v1", &v).unwrap();
        kv.put(Some(&wallet), "dashpay:contact:abc", &v).unwrap();

        let mut globals = kv.list(None, Some("mainnet:")).unwrap();
        globals.sort();
        assert_eq!(
            globals,
            vec![
                "mainnet:scheduled_votes:1".to_string(),
                "mainnet:settings:v1".to_string(),
            ]
        );

        let wallet_keys = kv.list(Some(&wallet), None).unwrap();
        assert_eq!(wallet_keys, vec!["dashpay:contact:abc".to_string()]);

        let no_match = kv.list(None, Some("regtest:")).unwrap();
        assert!(no_match.is_empty());
    }
}
