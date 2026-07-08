//! DET-side avatar image cache.
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
//! should outlive wallet deletion) with two shapes per URL:
//!
//! ```text
//! det:avatar:<sha256(url)_hex>     the validated image bytes
//! det:avatar_ts:<sha256(url)_hex>  sibling index: the fetch time only
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
//! Eviction orders entries by the sibling `det:avatar_ts:` index, so it reads
//! only the small fetch timestamps rather than deserializing every cached
//! image's full bytes on each [`put`].
//!
//! [`get`]: AvatarCacheView::get
//! [`put`]: AvatarCacheView::put

use std::sync::Arc;

use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};

use crate::backend_task::error::TaskError;
use crate::model::dashpay::CachedAvatar;
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Key prefix for every cached avatar entry (the image bytes).
const KEY_PREFIX: &str = "det:avatar:";

/// Key prefix for the sibling timestamp index. Each cached avatar has a
/// matching `det:avatar_ts:<hash>` entry holding only its `fetched_at_ms`, so
/// [`AvatarCacheView::put`] can order entries for eviction by reading these
/// small i64 values instead of deserializing every image's bytes.
const TS_KEY_PREFIX: &str = "det:avatar_ts:";

/// Maximum age of a cached avatar before [`AvatarCacheView::get`] treats it as
/// stale and re-fetches. Seven days balances offline survival against picking
/// up a changed avatar at a stable URL within a reasonable window.
const AVATAR_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Maximum number of cached avatar entries kept in the Global scope. Once a
/// [`AvatarCacheView::put`] would exceed this, the oldest entries are evicted
/// so the cache stays bounded regardless of how many distinct contacts are
/// viewed over a wallet's lifetime.
const MAX_AVATAR_ENTRIES: usize = 256;

/// SHA-256 of `url`, hex-encoded — the fixed-length, metacharacter-free suffix
/// shared by an avatar's byte key and its timestamp-index key.
fn digest_hex(url: &str) -> String {
    let digest = sha256::Hash::hash(url.as_bytes());
    hex::encode(digest.to_byte_array())
}

/// Build the canonical k/v key for an avatar URL (the image bytes).
fn key_for(url: &str) -> String {
    format!("{KEY_PREFIX}{}", digest_hex(url))
}

/// Build the sibling timestamp-index key for an avatar URL.
fn ts_key_for(url: &str) -> String {
    format!("{TS_KEY_PREFIX}{}", digest_hex(url))
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
        self.kv
            .put(DetScope::Global, &key_for(url), &entry)
            .map_err(map_kv_error_to_task_error)?;
        // Sibling index: lets eviction order entries without loading image bytes.
        self.kv
            .put(DetScope::Global, &ts_key_for(url), &now_ms)
            .map_err(map_kv_error_to_task_error)?;
        self.evict_to_bound()?;
        Ok(())
    }

    /// Drop the cached avatar for `url`. Idempotent — a missing entry returns
    /// `Ok(())`. Used to invalidate a stale image (e.g. the profile's
    /// `avatarUrl` changed, leaving the old URL's bytes orphaned).
    pub fn invalidate(&self, url: &str) -> Result<(), TaskError> {
        self.kv
            .delete(DetScope::Global, &key_for(url))
            .map_err(map_kv_error_to_task_error)?;
        self.kv
            .delete(DetScope::Global, &ts_key_for(url))
            .map_err(map_kv_error_to_task_error)
    }

    /// Drop every cached avatar. Called on wallet deletion so the cache cannot
    /// outlive the wallets whose contacts populated it. Best-effort per entry —
    /// a single delete failure is logged and the sweep continues.
    pub fn clear(&self) -> Result<(), TaskError> {
        for prefix in [KEY_PREFIX, TS_KEY_PREFIX] {
            let keys = self
                .kv
                .list(DetScope::Global, Some(prefix))
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
        }
        Ok(())
    }

    /// Evict the oldest entries until the cache holds at most
    /// [`MAX_AVATAR_ENTRIES`]. A best-effort housekeeping pass run after each
    /// `put`; a read or delete failure on any single entry is non-fatal.
    ///
    /// Ordering reads only the sibling `det:avatar_ts:` index (small `i64`
    /// timestamps), never the cached image bytes.
    fn evict_to_bound(&self) -> Result<(), TaskError> {
        let ts_keys = self
            .kv
            .list(DetScope::Global, Some(TS_KEY_PREFIX))
            .map_err(map_kv_error_to_task_error)?;
        if ts_keys.len() <= MAX_AVATAR_ENTRIES {
            return Ok(());
        }

        // Order by fetch time so the oldest are dropped first. Entries with a
        // missing/unreadable timestamp sort to the front (age 0) and go first.
        let mut aged: Vec<(i64, String)> = ts_keys
            .into_iter()
            .map(|ts_key| {
                let age = self
                    .kv
                    .get::<i64>(DetScope::Global, &ts_key)
                    .ok()
                    .flatten()
                    .unwrap_or(0);
                (age, ts_key)
            })
            .collect();
        aged.sort_by_key(|(age, _)| *age);

        let evict = aged.len().saturating_sub(MAX_AVATAR_ENTRIES);
        for (_, ts_key) in aged.into_iter().take(evict) {
            // Drop both the image and its sibling timestamp so the index stays
            // in lock-step with the byte entries.
            let avatar_key = format!(
                "{KEY_PREFIX}{}",
                ts_key.strip_prefix(TS_KEY_PREFIX).unwrap_or(&ts_key)
            );
            for key in [avatar_key, ts_key] {
                if let Err(e) = self.kv.delete(DetScope::Global, &key) {
                    tracing::debug!(
                        target = "wallet_backend::avatar_cache",
                        error = ?e,
                        "Failed to evict cached avatar over bound",
                    );
                }
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

    use crate::wallet_backend::kv_test_support::InMemoryKv;

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

    /// Every `put` maintains a sibling timestamp entry, and `invalidate` drops
    /// both — so the index never drifts from the byte entries.
    #[test]
    fn put_and_invalidate_keep_timestamp_index_in_lockstep() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        view.put_at(URL_A, b"a".to_vec(), 42).expect("put");

        let ts = kv
            .get::<i64>(DetScope::Global, &ts_key_for(URL_A))
            .expect("read ts")
            .expect("ts present");
        assert_eq!(ts, 42, "the sibling index stores the fetch time");

        view.invalidate(URL_A).expect("invalidate");
        assert_eq!(
            kv.get::<i64>(DetScope::Global, &ts_key_for(URL_A))
                .expect("read ts"),
            None,
            "invalidate must drop the sibling timestamp too"
        );
    }

    /// Eviction removes the sibling timestamp alongside the byte entry, leaving
    /// no orphaned index keys behind.
    #[test]
    fn eviction_drops_sibling_timestamp_entries() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        for i in 0..MAX_AVATAR_ENTRIES {
            let url = format!("https://example.com/a{i}.png");
            view.put_at(&url, vec![i as u8], i as i64).expect("put");
        }
        view.put_at("https://example.com/overflow.png", vec![0xff], i64::MAX)
            .expect("put overflow");

        let byte_keys = kv
            .list(DetScope::Global, Some(KEY_PREFIX))
            .expect("list bytes");
        let ts_keys = kv
            .list(DetScope::Global, Some(TS_KEY_PREFIX))
            .expect("list ts");
        assert_eq!(
            byte_keys.len(),
            MAX_AVATAR_ENTRIES,
            "byte entries stay at the bound"
        );
        assert_eq!(
            ts_keys.len(),
            MAX_AVATAR_ENTRIES,
            "the sibling index tracks the byte entries exactly, no orphans"
        );
    }

    #[test]
    fn clear_drops_timestamp_index() {
        let kv = kv();
        let view = AvatarCacheView::new(&kv);
        view.put(URL_A, b"a".to_vec()).expect("put a");
        view.put(URL_B, b"b".to_vec()).expect("put b");
        view.clear().expect("clear");
        assert!(
            kv.list(DetScope::Global, Some(TS_KEY_PREFIX))
                .expect("list ts")
                .is_empty(),
            "clear must sweep the sibling timestamp index too"
        );
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
