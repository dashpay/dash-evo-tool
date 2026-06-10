//! DET-side avatar image cache (PROJ-040).
//!
//! Avatars are binary images referenced by a profile's `avatarUrl`. Upstream
//! `platform-wallet` persists only the avatar hash and perceptual fingerprint
//! — never the image bytes — so without a DET-side cache every contact view
//! re-fetches every avatar from the network. [`AvatarCacheView`] stores the
//! validated image bytes keyed by URL in the cross-network app-level k/v
//! store, so a cached avatar survives offline and is fetched at most once per
//! content change.
//!
//! Keys live under [`DetScope::Global`] (avatars are not network-specific and
//! should outlive wallet deletion) with the shape:
//!
//! ```text
//! det:avatar:<sha256(url)_hex>
//! ```
//!
//! Hashing the URL keeps the key bounded and free of the colon / pattern
//! metacharacters a raw URL would carry into the k/v `list` matcher. The read
//! path is infallible-by-degradation: a missing or corrupt entry returns
//! `None` (logged) so the UI falls back to a network fetch rather than
//! blocking.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};

use crate::backend_task::error::TaskError;
use crate::model::dashpay::CachedAvatar;
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Key prefix for every cached avatar entry.
const KEY_PREFIX: &str = "det:avatar:";

/// Build the canonical k/v key for an avatar URL. The URL is SHA-256 hashed
/// so the key is fixed-length and carries no URL metacharacters.
fn key_for(url: &str) -> String {
    let digest = sha256::Hash::hash(url.as_bytes());
    format!("{KEY_PREFIX}{}", hex::encode(digest.to_byte_array()))
}

/// View borrowing a shared [`DetKv`] handle. Cheap to construct, so callers
/// build one per operation rather than threading it.
pub struct AvatarCacheView<'a> {
    kv: &'a Arc<DetKv>,
}

impl<'a> AvatarCacheView<'a> {
    /// Borrow a [`DetKv`] handle as a typed avatar-cache view.
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self { kv }
    }

    /// Fetch the cached avatar for `url`. Returns `None` when the URL is not
    /// cached or its blob fails to decode (logged and treated as absent so
    /// the caller falls back to a network fetch).
    pub fn get(&self, url: &str) -> Option<CachedAvatar> {
        let key = key_for(url);
        match self.kv.get::<CachedAvatar>(DetScope::Global, &key) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::avatar_cache",
                    error = ?e,
                    "Failed to read cached avatar; treating as absent",
                );
                None
            }
        }
    }

    /// Cache `bytes` for `url`, computing and storing the content hash and
    /// fetch timestamp. Overwrites any prior entry for the same URL (so a
    /// changed avatar at a stable URL is refreshed in place).
    pub fn put(&self, url: &str, bytes: Vec<u8>) -> Result<(), TaskError> {
        let sha256 = sha256::Hash::hash(&bytes).to_byte_array().to_vec();
        let entry = CachedAvatar {
            bytes,
            sha256,
            fetched_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        let key = key_for(url);
        self.kv
            .put(DetScope::Global, &key, &entry)
            .map_err(map_kv_error_to_task_error)
    }

    /// Drop the cached avatar for `url`. Idempotent — a missing entry returns
    /// `Ok(())`. Used to invalidate a stale image (e.g. the profile's
    /// `avatarUrl` changed, leaving the old URL's bytes orphaned).
    pub fn invalidate(&self, url: &str) -> Result<(), TaskError> {
        let key = key_for(url);
        self.kv
            .delete(DetScope::Global, &key)
            .map_err(map_kv_error_to_task_error)
    }
}

/// Avatar-cache adapter errors funnel into the dedicated
/// [`TaskError::AvatarCacheStorage`] envelope.
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    TaskError::AvatarCacheStorage {
        source: Box::new(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use platform_wallet_storage::{KvError, KvStore, ObjectId};

    /// Minimal in-memory `KvStore` mirroring the `wallet_meta` test fixture so
    /// the view tests exercise get/put/invalidate without a file backend.
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

    const URL_A: &str = "https://example.com/avatar-a.png";
    const URL_B: &str = "https://example.com/avatar-b.png";

    #[test]
    fn put_then_get_round_trips_and_hashes_bytes() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        let bytes = b"image-bytes-a".to_vec();
        view.put(URL_A, bytes.clone()).expect("put");

        let cached = view.get(URL_A).expect("cache hit");
        assert_eq!(cached.bytes, bytes);
        assert_eq!(
            cached.sha256,
            sha256::Hash::hash(&bytes).to_byte_array().to_vec(),
            "the stored hash must match the bytes"
        );
    }

    #[test]
    fn get_missing_returns_none() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        assert_eq!(view.get(URL_A), None);
    }

    #[test]
    fn put_overwrites_in_place_for_changed_avatar() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        view.put(URL_A, b"old".to_vec()).expect("first put");
        view.put(URL_A, b"new".to_vec()).expect("second put");
        let cached = view.get(URL_A).expect("cache hit");
        assert_eq!(cached.bytes, b"new".to_vec());
        assert_eq!(
            cached.sha256,
            sha256::Hash::hash(b"new").to_byte_array().to_vec()
        );
    }

    #[test]
    fn invalidate_drops_entry_and_is_idempotent() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        // Idempotent on an absent key.
        view.invalidate(URL_A).expect("invalidate absent");
        view.put(URL_A, b"bytes".to_vec()).expect("put");
        view.invalidate(URL_A).expect("first invalidate");
        view.invalidate(URL_A).expect("second invalidate");
        assert_eq!(view.get(URL_A), None);
    }

    #[test]
    fn distinct_urls_do_not_collide() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        view.put(URL_A, b"a".to_vec()).expect("put a");
        view.put(URL_B, b"b".to_vec()).expect("put b");
        assert_eq!(view.get(URL_A).unwrap().bytes, b"a".to_vec());
        assert_eq!(view.get(URL_B).unwrap().bytes, b"b".to_vec());
        // Invalidating one leaves the other intact.
        view.invalidate(URL_A).expect("invalidate a");
        assert_eq!(view.get(URL_A), None);
        assert_eq!(view.get(URL_B).unwrap().bytes, b"b".to_vec());
    }

    #[test]
    fn key_is_url_hash_prefixed() {
        let key = key_for(URL_A);
        assert!(key.starts_with(KEY_PREFIX));
        let suffix = key.trim_start_matches(KEY_PREFIX);
        // 32-byte sha256 hex-encoded.
        assert_eq!(suffix.len(), 64);
        assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
