//! Non-widget UI state: per-screen view-models and async fetch-state caches.
//!
//! Modules here own no egui rendering surface — they are neither `Component`
//! nor `ComponentResponse`. Screens own them and may dispatch `BackendTask`
//! through them; the module placement policy (P14) keeps these out of
//! `ui/components/`, which is reserved for renderable widget types.

pub mod tracked_asset_lock_cache;

pub use tracked_asset_lock_cache::TrackedAssetLockCache;
