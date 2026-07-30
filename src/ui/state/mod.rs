//! Non-widget UI state: per-screen view-models and async fetch-state caches.
//!
//! Modules here own no egui rendering surface — they are neither `Component`
//! nor `ComponentResponse`. Screens own them and may dispatch `BackendTask`
//! through them; the module placement policy (P14) keeps these out of
//! `ui/components/`, which is reserved for renderable widget types.

pub mod account_summary;
pub mod avatar_cache;
pub mod contacts_view;
pub mod global_nav;
pub mod hub_selection;
pub mod legacy_recovery;
pub mod masternodes_view;
pub mod tracked_asset_lock_cache;

pub use avatar_cache::AvatarCache;
pub use tracked_asset_lock_cache::TrackedAssetLockCache;
