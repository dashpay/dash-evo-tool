//! `AppSettings` accessors layered on the shared app k/v store.
//!
//! Reads use a small in-memory cache so the egui frame loop can probe
//! settings every frame without paying for a bincode decode each time.
//! Writes go straight to the k/v store; the cache is invalidated under
//! a `SettingsCacheGuard` so concurrent readers cannot observe a
//! stale value.

use super::{AppContext, SettingsCacheGuard};
use crate::model::settings::AppSettings;
use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use crate::wallet_backend::KvAdapterError;

impl AppContext {
    /// Persist the chosen root screen and pin the active-network field
    /// to this context's network.
    pub fn update_settings(&self, root_screen_type: RootScreenType) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.root_screen_type = root_screen_type;
        settings.network = self.network;
        self.set_app_settings(&settings)
    }

    /// Persist the Dash-Qt execution toggles.
    pub fn update_dash_core_execution_settings(
        &self,
        custom_dash_qt_path: Option<std::path::PathBuf>,
        overwrite_dash_conf: bool,
    ) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.dash_qt_path = custom_dash_qt_path;
        settings.overwrite_dash_conf = overwrite_dash_conf;
        self.set_app_settings(&settings)
    }

    /// Persist the ZMQ-disabled flag.
    pub fn update_disable_zmq(&self, disable: bool) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.disable_zmq = disable;
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

    /// Persist the `close_dash_qt_on_exit` flag.
    pub fn update_close_dash_qt_on_exit(&self, close_on_exit: bool) -> Result<(), KvAdapterError> {
        let mut settings = self.get_app_settings();
        settings.close_dash_qt_on_exit = close_on_exit;
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
        if let Some(cached) = self.cached_settings.read().unwrap().clone() {
            return cached;
        }

        let loaded = match self.app_kv.get::<AppSettings>(None, AppSettings::KV_KEY) {
            Ok(Some(s)) => s,
            Ok(None) => AppSettings::default(),
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "Failed to load AppSettings from app k/v; using defaults"
                );
                AppSettings::default()
            }
        };

        *self.cached_settings.write().unwrap() = Some(loaded.clone());
        loaded
    }

    /// Write the [`AppSettings`] blob to the shared app k/v store.
    pub fn set_app_settings(&self, settings: &AppSettings) -> Result<(), KvAdapterError> {
        let mut guard = self.invalidate_settings_cache();
        self.app_kv.put(None, AppSettings::KV_KEY, settings)?;
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
