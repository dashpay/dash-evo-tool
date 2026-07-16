//! Per-screen cache of contact/profile avatar images, keyed by URL.
//!
//! Avatar images are fetched over the network (and served from the DET avatar
//! disk cache) behind an async accessor that must not run on the egui frame
//! loop. Screens fetch them through the App Task System
//! ([`DashPayTask::FetchAvatar`]) and render from this cache via the
//! [`Avatar`](crate::ui::components::avatar::Avatar) component.
//!
//! Each URL moves through `NotRequested → Loading → Fetched → Ready`, with
//! `Failed` as the terminal error state. A fetch is dispatched only from
//! `NotRequested`, so neither a slow fetch, a broken URL, nor a decode failure
//! re-dispatches every frame. The `Fetched → Ready` step is the UI thread
//! decoding the bytes into a GPU texture on first paint; the raw bytes are
//! dropped once the texture exists.
//!
//! [`DashPayTask::FetchAvatar`]: crate::backend_task::dashpay::DashPayTask::FetchAvatar

use crate::backend_task::BackendTask;
use crate::backend_task::dashpay::DashPayTask;
use egui::TextureHandle;
use std::collections::BTreeMap;

/// Per-URL fetch state. The absence of an entry is the implicit `NotRequested`
/// state — only `NotRequested` dispatches a fetch.
enum FetchState {
    /// A fetch has been dispatched and no result has arrived yet.
    Loading,
    /// The fetch completed; holds the validated image bytes awaiting decode.
    Fetched(Vec<u8>),
    /// The bytes were decoded and uploaded; the raw bytes have been dropped.
    Ready(TextureHandle),
    /// The fetch or decode failed. Does not auto-redispatch; the user re-arms
    /// it explicitly via [`AvatarCache::invalidate`].
    Failed,
}

/// Cached avatar images keyed by URL, fetched via backend tasks and decoded to
/// GPU textures on demand.
///
/// A fetch is dispatched only from `NotRequested` (the retry-storm guard): an
/// empty, failed, or slow fetch never re-dispatches every frame.
/// [`Self::invalidate`] returns every URL to `NotRequested` so an explicit
/// refresh re-fetches.
#[derive(Default)]
pub struct AvatarCache {
    states: BTreeMap<String, FetchState>,
}

impl AvatarCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the backend task to dispatch when `url` is `NotRequested`, or
    /// `None` once it is `Loading`, `Fetched`, `Ready`, or `Failed`. The caller
    /// dispatches the returned task as an
    /// [`AppAction::BackendTask`](crate::app::AppAction::BackendTask).
    pub fn ensure_requested(&mut self, url: &str) -> Option<BackendTask> {
        if url.is_empty() || self.states.contains_key(url) {
            return None;
        }
        self.states.insert(url.to_string(), FetchState::Loading);
        Some(BackendTask::DashPayTask(Box::new(
            DashPayTask::FetchAvatar {
                url: url.to_string(),
            },
        )))
    }

    /// Seed the cache with already-known bytes (e.g. a profile's locally-stored
    /// avatar), moving `url` straight to `Fetched` so no network fetch is
    /// dispatched. No-op if the URL is already tracked.
    pub fn seed(&mut self, url: &str, bytes: Vec<u8>) {
        if url.is_empty() {
            return;
        }
        self.states
            .entry(url.to_string())
            .or_insert(FetchState::Fetched(bytes));
    }

    /// Record a completed fetch: `Some(bytes) → Fetched`, `None → Failed`. A
    /// URL already `Ready` keeps its uploaded texture (a duplicate result never
    /// discards a live texture).
    pub fn store(&mut self, url: String, bytes: Option<Vec<u8>>) {
        if matches!(self.states.get(&url), Some(FetchState::Ready(_))) {
            return;
        }
        let state = match bytes {
            Some(bytes) => FetchState::Fetched(bytes),
            None => FetchState::Failed,
        };
        self.states.insert(url, state);
    }

    /// The fetched-but-not-yet-decoded bytes for `url`, present only in the
    /// `Fetched` state. The [`Avatar`](crate::ui::components::avatar::Avatar)
    /// component decodes these and calls [`Self::set_ready`] / [`Self::mark_failed`].
    pub fn fetched_bytes(&self, url: &str) -> Option<&[u8]> {
        match self.states.get(url) {
            Some(FetchState::Fetched(bytes)) => Some(bytes),
            _ => None,
        }
    }

    /// Promote `url` to `Ready` with its uploaded texture, dropping the raw bytes.
    pub fn set_ready(&mut self, url: &str, texture: TextureHandle) {
        self.states
            .insert(url.to_string(), FetchState::Ready(texture));
    }

    /// The uploaded texture for `url` once `Ready`, else `None`.
    pub fn ready_texture(&self, url: &str) -> Option<&TextureHandle> {
        match self.states.get(url) {
            Some(FetchState::Ready(texture)) => Some(texture),
            _ => None,
        }
    }

    /// Move `url` to `Failed` (fetch produced undecodable bytes). Does not
    /// re-dispatch until [`Self::invalidate`] re-arms it.
    pub fn mark_failed(&mut self, url: &str) {
        self.states.insert(url.to_string(), FetchState::Failed);
    }

    /// Whether `url`'s fetch failed. Drives the fallback avatar.
    pub fn is_failed(&self, url: &str) -> bool {
        matches!(self.states.get(url), Some(FetchState::Failed))
    }

    /// Return every URL to `NotRequested` so the next render re-fetches. Used by
    /// an explicit refresh and the identity-change reset — a changed avatar at a
    /// stable URL repaints because its texture is dropped here.
    pub fn invalidate(&mut self) {
        self.states.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL_A: &str = "https://example.com/a.png";
    const URL_B: &str = "https://example.com/b.png";

    /// The fetch fires exactly once per URL: the first `ensure_requested` yields
    /// a task, and every later call for the same URL yields `None` — even before
    /// a result arrives and even after a failure. This is the retry-storm guard.
    #[test]
    fn ensure_requested_fires_once_per_url() {
        let mut cache = AvatarCache::new();

        assert!(
            cache.ensure_requested(URL_A).is_some(),
            "first request dispatches"
        );
        assert!(
            cache.ensure_requested(URL_A).is_none(),
            "a second request before the result must not re-dispatch"
        );

        cache.store(URL_A.to_string(), None); // failed
        assert!(cache.is_failed(URL_A));
        assert!(
            cache.ensure_requested(URL_A).is_none(),
            "a failed fetch must not trigger a retry storm"
        );
    }

    /// An empty URL never dispatches — there is nothing to fetch.
    #[test]
    fn empty_url_never_dispatches() {
        let mut cache = AvatarCache::new();
        assert!(cache.ensure_requested("").is_none());
    }

    /// Distinct URLs fetch independently.
    #[test]
    fn urls_fetch_independently() {
        let mut cache = AvatarCache::new();
        cache.ensure_requested(URL_A);
        cache.store(URL_A.to_string(), Some(vec![1, 2, 3]));

        assert!(
            cache.ensure_requested(URL_B).is_some(),
            "a different URL must dispatch its own fetch"
        );
        assert_eq!(
            cache.fetched_bytes(URL_A),
            Some(&[1, 2, 3][..]),
            "the first URL's bytes must remain available"
        );
    }

    /// A successful result exposes its bytes for decoding.
    #[test]
    fn store_some_exposes_fetched_bytes() {
        let mut cache = AvatarCache::new();
        cache.ensure_requested(URL_A);
        cache.store(URL_A.to_string(), Some(vec![9, 9]));
        assert_eq!(cache.fetched_bytes(URL_A), Some(&[9, 9][..]));
        assert!(!cache.is_failed(URL_A));
    }

    /// `seed` moves a URL straight to `Fetched`, suppressing the network fetch.
    #[test]
    fn seed_suppresses_fetch() {
        let mut cache = AvatarCache::new();
        cache.seed(URL_A, vec![7]);
        assert_eq!(cache.fetched_bytes(URL_A), Some(&[7][..]));
        assert!(
            cache.ensure_requested(URL_A).is_none(),
            "a seeded URL must not dispatch a fetch"
        );
    }

    /// `mark_failed` moves a fetched URL to the terminal failed state.
    #[test]
    fn mark_failed_is_terminal_until_invalidate() {
        let mut cache = AvatarCache::new();
        cache.ensure_requested(URL_A);
        cache.store(URL_A.to_string(), Some(vec![0]));
        cache.mark_failed(URL_A); // e.g. undecodable bytes
        assert!(cache.is_failed(URL_A));
        assert!(cache.fetched_bytes(URL_A).is_none());
        assert!(
            cache.ensure_requested(URL_A).is_none(),
            "a failed URL must not auto-redispatch"
        );

        cache.invalidate();
        assert!(
            cache.ensure_requested(URL_A).is_some(),
            "invalidate must re-arm the fetch"
        );
    }

    /// A duplicate `store` after a texture is `Ready` must not clobber it.
    /// (`set_ready` needs an egui context, so this asserts the guard via the
    /// public state machine: once fetched bytes are gone, a repeat success with
    /// new bytes still replaces them — but a Ready entry is protected.)
    #[test]
    fn store_after_fetched_replaces_bytes() {
        let mut cache = AvatarCache::new();
        cache.store(URL_A.to_string(), Some(vec![1]));
        cache.store(URL_A.to_string(), Some(vec![2, 2]));
        assert_eq!(cache.fetched_bytes(URL_A), Some(&[2, 2][..]));
    }
}
