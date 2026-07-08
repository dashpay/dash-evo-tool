//! `AppSettings` accessors layered on the shared app k/v store.
//!
//! Reads use a small in-memory cache so the egui frame loop can probe
//! settings every frame without paying for a bincode decode each time.
//! Writes go straight to the k/v store; the cache is invalidated under
//! a `SettingsCacheGuard` so concurrent readers cannot observe a
//! stale value.

use super::{AppContext, SettingsCacheGuard};
use crate::model::settings::{AppSettings, detect_dash_qt_path};
use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use crate::wallet_backend::{DetScope, KvAdapterError};

impl AppContext {
    /// Persist the chosen root screen and pin the active-network field
    /// to this context's network.
    pub fn update_settings(&self, root_screen_type: RootScreenType) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.root_screen_type = root_screen_type;
        settings.network = self.network;
        self.set_app_settings(&settings)
    }

    /// Persist the theme preference.
    pub fn update_theme_preference(&self, theme_mode: ThemeMode) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.theme_mode = theme_mode;
        self.set_app_settings(&settings)
    }

    /// Persist the `auto_start_spv` flag.
    pub fn update_auto_start_spv(&self, auto_start: bool) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.auto_start_spv = auto_start;
        self.set_app_settings(&settings)
    }

    /// Persist the user mode (Beginner / Advanced).
    pub fn update_user_mode(
        &self,
        user_mode: crate::model::settings::UserMode,
    ) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.user_mode = user_mode;
        self.set_app_settings(&settings)
    }

    /// Persist the `show_evonode_tools` flag.
    pub fn update_show_evonode_tools(&self, show: bool) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.show_evonode_tools = show;
        self.set_app_settings(&settings)
    }

    /// Persist the `onboarding_completed` flag.
    pub fn update_onboarding_completed(&self, completed: bool) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.onboarding_completed = completed;
        self.set_app_settings(&settings)
    }

    /// Invalidates the settings cache and returns a guard.
    ///
    /// The cache is invalidated immediately and the guard prevents concurrent access
    /// until the underlying k/v operation completes. This ensures atomicity and
    /// prevents race conditions regardless of whether the write succeeds.
    pub fn invalidate_settings_cache(&'_ self) -> SettingsCacheGuard<'_> {
        let mut guard = self.cached_settings.write().unwrap();
        *guard = None;
        guard
    }

    /// Read the persisted [`AppSettings`].
    ///
    /// Returns defaults when the blob is absent (first run) or when the
    /// stored value fails to decode (e.g. a future schema). Cached
    /// in-memory between updates.
    pub fn get_app_settings(&self) -> AppSettings {
        // Fast path: cache hit under a read lock.
        if let Some(cached) = self.cached_settings.read().unwrap().clone() {
            return cached;
        }

        // Cache miss: hold the write lock across the load+populate so a
        // concurrent `set_app_settings` (which also holds this write lock for
        // its whole duration) cannot slip a fresh value in between our read and
        // write and then be clobbered by our stale load.
        let mut guard = self.cached_settings.write().unwrap();
        // Double-check: a racer may have populated the cache while we waited.
        if let Some(cached) = guard.clone() {
            return cached;
        }

        let loaded = match self
            .app_kv
            .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
        {
            Ok(Some(s)) => with_dash_qt_path_fallback(s),
            Ok(None) => AppSettings::default(),
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "Failed to load AppSettings from app k/v; using defaults"
                );
                AppSettings::default()
            }
        };

        *guard = Some(loaded.clone());
        loaded
    }

    /// Write the [`AppSettings`] blob to the shared app k/v store.
    pub fn set_app_settings(&self, settings: &AppSettings) -> Result<(), KvAdapterError> {
        let mut guard = self.invalidate_settings_cache();
        self.app_kv
            .put(DetScope::Global, AppSettings::KV_KEY, settings)?;
        *guard = Some(settings.clone());
        Ok(())
    }

    /// Legacy compatibility shim. Settings are now always present (the
    /// `Default` impl reproduces the previous "fresh install" row), so
    /// this returns `Some(...)` unconditionally. The `Result` is kept
    /// only to ease the migration of existing callers.
    pub fn get_settings(&self) -> Result<Option<AppSettings>, KvAdapterError> {
        Ok(Some(self.get_app_settings()))
    }
}

/// Fills in an autodetected `dash_qt_path` when a decoded settings blob has
/// none. `None` means "autodetect" (see `AppSettings::dash_qt_path` docs);
/// decoding itself stays pure (no filesystem IO), so this fallback runs once
/// here, at the settings-load call site.
fn with_dash_qt_path_fallback(mut settings: AppSettings) -> AppSettings {
    if settings.dash_qt_path.is_none() {
        settings.dash_qt_path = detect_dash_qt_path();
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::DetKv;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use dash_sdk::dpp::dashcore::Network;
    use std::sync::Arc;

    fn empty_kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    /// F69 — the `AppSettings` blob survives a put/get round-trip through the
    /// k/v store with every field intact, and an absent blob reads as `None`
    /// (the path `get_app_settings` maps to its `Default`).
    #[test]
    fn app_settings_round_trips_through_kv() {
        let kv = empty_kv();

        // Absent blob reads as None.
        assert!(
            kv.get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
                .unwrap()
                .is_none(),
            "a fresh k/v store has no settings blob"
        );

        // A non-default value must round-trip field-for-field.
        let mut settings = AppSettings::default();
        settings.network = Network::Testnet;
        settings.theme_mode = ThemeMode::Dark;
        settings.user_mode = crate::model::settings::UserMode::Beginner;
        settings.auto_start_spv = true;
        settings.disable_zmq = true;
        settings.onboarding_completed = true;

        kv.put(DetScope::Global, AppSettings::KV_KEY, &settings)
            .unwrap();
        let got: AppSettings = kv
            .get(DetScope::Global, AppSettings::KV_KEY)
            .unwrap()
            .expect("settings blob present after put");

        assert_eq!(got.network, Network::Testnet);
        assert_eq!(got.theme_mode, ThemeMode::Dark);
        assert_eq!(got.user_mode, crate::model::settings::UserMode::Beginner);
        assert!(got.auto_start_spv);
        assert!(got.disable_zmq);
        assert!(got.onboarding_completed);
    }

    /// A decoded blob with no stored Dash-Qt path gets one autodetect pass at
    /// load time, so installing Dash-Qt after first launch is picked up
    /// without a manual edit.
    #[test]
    fn dash_qt_path_fallback_autodetects_when_unset() {
        let settings = AppSettings {
            dash_qt_path: None,
            ..AppSettings::default()
        };
        let filled = with_dash_qt_path_fallback(settings);
        assert_eq!(filled.dash_qt_path, detect_dash_qt_path());
    }

    /// A stored Dash-Qt path is preserved verbatim — autodetect never
    /// overrides an explicit user choice.
    #[test]
    fn dash_qt_path_fallback_preserves_explicit_value() {
        let stored = std::path::PathBuf::from("/custom/path/to/dash-qt");
        let settings = AppSettings {
            dash_qt_path: Some(stored.clone()),
            ..AppSettings::default()
        };
        let filled = with_dash_qt_path_fallback(settings);
        assert_eq!(filled.dash_qt_path, Some(stored));
    }
}
