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
//!
//! ## Invalidation and bounds
//!
//! A cached avatar carries the wall-clock fetch time (`fetched_at_ms`). The
//! read path treats an entry older than [`AVATAR_TTL_MS`] as stale: [`get`]
//! drops it and returns `None`, so a changed avatar served at the same URL is
//! re-fetched rather than pinned forever. The cache is also size-bounded —
//! [`put`] evicts the oldest entries once the entry count exceeds
//! [`MAX_AVATAR_ENTRIES`], so the cross-network Global scope cannot grow
//! without limit, and the whole cache is cleared when a wallet is forgotten.
//!
//! [`get`]: AvatarCacheView::get
//! [`put`]: AvatarCacheView::put

use std::sync::Arc;

use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};

use crate::backend_task::error::TaskError;
use crate::model::dashpay::CachedAvatar;
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Key prefix for every cached avatar entry.
const KEY_PREFIX: &str = "det:avatar:";

/// Maximum age of a cached avatar before [`AvatarCacheView::get`] treats it as
/// stale and re-fetches. Seven days balances offline survival against picking
/// up a changed avatar at a stable URL within a reasonable window.
pub const AVATAR_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Maximum number of cached avatar entries kept in the Global scope. Once a
/// [`AvatarCacheView::put`] would exceed this, the oldest entries are evicted
/// so the cache stays bounded regardless of how many distinct contacts are
/// viewed over a wallet's lifetime.
pub const MAX_AVATAR_ENTRIES: usize = 256;

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

    /// Fetch the cached avatar for `url`, honouring the [`AVATAR_TTL_MS`]
    /// freshness window. Returns `None` when the URL is not cached, its blob
    /// fails to decode (logged and treated as absent), or the entry is older
    /// than the TTL — in which case the stale entry is dropped so the next read
    /// re-fetches. The caller falls back to a network fetch on any `None`.
    pub fn get(&self, url: &str) -> Option<CachedAvatar> {
        self.get_at(url, chrono::Utc::now().timestamp_millis())
    }

    /// TTL-aware read with an injected `now_ms` clock — the testable core of
    /// [`Self::get`]. An entry whose `fetched_at_ms` precedes `now_ms` by more
    /// than [`AVATAR_TTL_MS`] is invalidated and reported absent.
    fn get_at(&self, url: &str, now_ms: i64) -> Option<CachedAvatar> {
        let key = key_for(url);
        let cached = match self.kv.get::<CachedAvatar>(DetScope::Global, &key) {
            Ok(v) => v?,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::avatar_cache",
                    error = ?e,
                    "Failed to read cached avatar; treating as absent",
                );
                return None;
            }
        };

        if now_ms.saturating_sub(cached.fetched_at_ms) > AVATAR_TTL_MS {
            // Stale: drop so a changed avatar at the same URL is re-fetched.
            if let Err(e) = self.invalidate(url) {
                tracing::debug!(
                    target = "wallet_backend::avatar_cache",
                    error = ?e,
                    "Failed to evict stale cached avatar",
                );
            }
            return None;
        }

        Some(cached)
    }

    /// Cache `bytes` for `url`, computing and storing the content hash and
    /// fetch timestamp. Overwrites any prior entry for the same URL (so a
    /// changed avatar at a stable URL is refreshed in place) and evicts the
    /// oldest entries when the cache exceeds [`MAX_AVATAR_ENTRIES`].
    pub fn put(&self, url: &str, bytes: Vec<u8>) -> Result<(), TaskError> {
        self.put_at(url, bytes, chrono::Utc::now().timestamp_millis())
    }

    /// [`Self::put`] with an injected `now_ms` clock — the testable core.
    fn put_at(&self, url: &str, bytes: Vec<u8>, now_ms: i64) -> Result<(), TaskError> {
        let sha256 = sha256::Hash::hash(&bytes).to_byte_array().to_vec();
        let entry = CachedAvatar {
            bytes,
            sha256,
            fetched_at_ms: now_ms,
        };
        let key = key_for(url);
        self.kv
            .put(DetScope::Global, &key, &entry)
            .map_err(map_kv_error_to_task_error)?;
        self.evict_to_bound()?;
        Ok(())
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

    /// Drop every cached avatar. Called on wallet deletion so the cache cannot
    /// outlive the wallets whose contacts populated it. Best-effort per entry —
    /// a single delete failure is logged and the sweep continues.
    pub fn clear(&self) -> Result<(), TaskError> {
        let keys = self
            .kv
            .list(DetScope::Global, Some(KEY_PREFIX))
            .map_err(map_kv_error_to_task_error)?;
        for key in keys {
            if let Err(e) = self.kv.delete(DetScope::Global, &key) {
                tracing::debug!(
                    target = "wallet_backend::avatar_cache",
                    error = ?e,
                    "Failed to delete cached avatar during clear",
                );
            }
        }
        Ok(())
    }

    /// Evict the oldest entries until the cache holds at most
    /// [`MAX_AVATAR_ENTRIES`]. A best-effort housekeeping pass run after each
    /// `put`; a read or delete failure on any single entry is non-fatal.
    fn evict_to_bound(&self) -> Result<(), TaskError> {
        let keys = self
            .kv
            .list(DetScope::Global, Some(KEY_PREFIX))
            .map_err(map_kv_error_to_task_error)?;
        if keys.len() <= MAX_AVATAR_ENTRIES {
            return Ok(());
        }

        // Order by fetch time so the oldest are dropped first. Unreadable
        // entries sort to the front (treated as age 0) and are evicted first.
        let mut aged: Vec<(i64, String)> = keys
            .into_iter()
            .map(|key| {
                let age = self
                    .kv
                    .get::<CachedAvatar>(DetScope::Global, &key)
                    .ok()
                    .flatten()
                    .map(|c| c.fetched_at_ms)
                    .unwrap_or(0);
                (age, key)
            })
            .collect();
        aged.sort_by_key(|(age, _)| *age);

        let evict = aged.len().saturating_sub(MAX_AVATAR_ENTRIES);
        for (_, key) in aged.into_iter().take(evict) {
            if let Err(e) = self.kv.delete(DetScope::Global, &key) {
                tracing::debug!(
                    target = "wallet_backend::avatar_cache",
                    error = ?e,
                    "Failed to evict cached avatar over bound",
                );
            }
        }
        Ok(())
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
    fn stale_entry_is_not_served_and_is_invalidated() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        // Seed at t=0.
        view.put_at(URL_A, b"old".to_vec(), 0).expect("put");

        // A read within the TTL still serves the cached bytes.
        let fresh = view
            .get_at(URL_A, AVATAR_TTL_MS)
            .expect("entry within TTL is served");
        assert_eq!(fresh.bytes, b"old".to_vec());

        // One millisecond past the TTL: the entry is stale, must not be served,
        // and must be evicted so the next read re-fetches.
        assert_eq!(
            view.get_at(URL_A, AVATAR_TTL_MS + 1),
            None,
            "an entry older than the TTL must not be served"
        );
        // A fresh read (even at t=0) sees nothing — the stale read dropped it.
        assert_eq!(
            view.get_at(URL_A, 0),
            None,
            "the stale entry must have been invalidated, not just hidden"
        );
    }

    #[test]
    fn changed_avatar_at_same_url_is_refetched_after_ttl() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        view.put_at(URL_A, b"v1".to_vec(), 0).expect("seed v1");
        // After the TTL the old bytes are not served; a re-fetch caches v2.
        assert_eq!(view.get_at(URL_A, AVATAR_TTL_MS + 1), None);
        view.put_at(URL_A, b"v2".to_vec(), AVATAR_TTL_MS + 1)
            .expect("re-cache v2");
        let now = view.get_at(URL_A, AVATAR_TTL_MS + 1).expect("v2 served");
        assert_eq!(now.bytes, b"v2".to_vec());
    }

    #[test]
    fn put_evicts_oldest_when_over_bound() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        // Fill exactly to the bound, oldest first (monotonic timestamps).
        for i in 0..MAX_AVATAR_ENTRIES {
            let url = format!("https://example.com/a{i}.png");
            view.put_at(&url, vec![i as u8], i as i64).expect("put");
        }
        // The oldest entry (i=0) is still present at the bound.
        assert!(view.get_at("https://example.com/a0.png", 0).is_some());

        // One more entry pushes past the bound; the oldest must be evicted.
        view.put_at(
            "https://example.com/overflow.png",
            vec![0xff],
            MAX_AVATAR_ENTRIES as i64,
        )
        .expect("put overflow");
        assert_eq!(
            view.get_at("https://example.com/a0.png", 0),
            None,
            "the oldest entry must be evicted once the cache exceeds the bound"
        );
        // The newest entry survives.
        assert!(
            view.get_at(
                "https://example.com/overflow.png",
                MAX_AVATAR_ENTRIES as i64
            )
            .is_some()
        );
    }

    #[test]
    fn clear_drops_every_entry() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        view.put(URL_A, b"a".to_vec()).expect("put a");
        view.put(URL_B, b"b".to_vec()).expect("put b");
        view.clear().expect("clear");
        assert_eq!(view.get(URL_A), None);
        assert_eq!(view.get(URL_B), None);
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
