use crate::lock_helper::RwLockExt;
use crate::model::settings::Settings;
use crate::ui::RootScreenType;
use rusqlite::Result;
use std::sync::RwLockWriteGuard;

use super::AppContext;

/// A guard that ensures settings cache invalidation happens atomically
///
/// This guard holds a write lock on the cached settings, preventing reads
/// until the database update is complete and the cache is properly invalidated.
pub(crate) type SettingsCacheGuard<'a> = RwLockWriteGuard<'a, Option<Settings>>;

impl AppContext {
    /// Updates the `start_root_screen` in the settings table
    pub fn update_settings(&self, root_screen_type: RootScreenType) -> Result<()> {
        let _guard = self.invalidate_settings_cache();

        self.db
            .insert_or_update_settings(self.network, root_screen_type)
    }

    /// Updates the main password settings
    pub fn update_main_password(
        &self,
        salt: &[u8],
        nonce: &[u8],
        password_check: &[u8],
    ) -> Result<()> {
        let _guard = self.invalidate_settings_cache();

        self.db.update_main_password(salt, nonce, password_check)
    }

    /// Updates the Dash Core execution settings
    pub fn update_dash_core_execution_settings(
        &self,
        custom_dash_qt_path: Option<std::path::PathBuf>,
        overwrite_dash_conf: bool,
    ) -> Result<()> {
        let _guard = self.invalidate_settings_cache();

        self.db
            .update_dash_core_execution_settings(custom_dash_qt_path, overwrite_dash_conf)
    }

    /// Updates the disable_zmq flag in settings
    pub fn update_disable_zmq(&self, disable: bool) -> Result<()> {
        let _guard = self.invalidate_settings_cache();
        self.db.update_disable_zmq(disable)
    }

    /// Invalidates the settings cache and returns a guard
    ///
    /// The cache is invalidated immediately and the guard prevents concurrent access
    /// until the database operation is complete. This ensures atomicity and prevents
    /// race conditions regardless of whether the database operation succeeds or fails.
    pub fn invalidate_settings_cache(&'_ self) -> SettingsCacheGuard<'_> {
        let mut guard = self.cached_settings.write_or_recover();
        *guard = None;
        guard
    }

    /// Retrieves the current settings
    ///
    /// ## Cached
    ///
    /// This function uses a cache to avoid expensive database operations.
    /// The cache is invalidated when settings are updated.
    ///
    /// Use [`AppContext::invalidate_settings_cache`] to invalidate the cache.
    pub fn get_settings(&self) -> Result<Option<Settings>> {
        // First, try to read from cache
        {
            let cache = self.cached_settings.read_or_recover();
            if let Some(ref settings) = *cache {
                return Ok(Some(settings.clone()));
            }
        }

        // Cache miss, read from database
        let settings = self.db.get_settings()?.map(Settings::from);

        // Update cache with the fresh data
        {
            let mut cache = self.cached_settings.write_or_recover();
            *cache = settings.clone();
        }

        Ok(settings)
    }
}
