use super::{AppContext, SettingsCacheGuard};
use crate::backend_task::error::TaskError;
use crate::model::secret::Secret;
use crate::model::settings::Settings;
use crate::model::wallet::encryption::{DASH_SECRET_MESSAGE, derive_password_key};
use crate::ui::RootScreenType;
use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rusqlite::Result;

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
        let mut guard = self.cached_settings.write().unwrap();
        *guard = None;
        guard
    }

    /// Whether the app has a main password configured.
    ///
    /// The main password is set once via the wallet creation/import UI and
    /// protects wallet seeds at rest. Its presence is derived from whether
    /// a `password_check` blob and its Argon2 salt / AES-GCM nonce have been
    /// stored in the settings row.
    pub fn has_main_password(&self) -> bool {
        self.password_info.is_some()
    }

    /// Verify that the supplied password matches the app's main password.
    ///
    /// Fails with `MainPasswordMismatch` if no main password is set or the
    /// supplied password does not decrypt the `password_check` blob back to
    /// `DASH_SECRET_MESSAGE`. Argon2 + AES-GCM make this operation slow on
    /// purpose — callers should not put it inside tight loops.
    pub fn verify_main_password(&self, candidate: &Secret) -> std::result::Result<(), TaskError> {
        let info = self
            .password_info
            .as_ref()
            .ok_or(TaskError::MainPasswordMismatch)?;

        let key = derive_password_key(candidate.expose_secret(), &info.salt)
            .map_err(|_| TaskError::MainPasswordMismatch)?;

        let cipher =
            Aes256Gcm::new_from_slice(&key).map_err(|_| TaskError::MainPasswordMismatch)?;
        let nonce = Nonce::from_slice(&info.nonce);
        let plaintext = cipher
            .decrypt(nonce, info.password_checker.as_slice())
            .map_err(|_| TaskError::MainPasswordMismatch)?;

        if plaintext.as_slice() == DASH_SECRET_MESSAGE.as_slice() {
            Ok(())
        } else {
            Err(TaskError::MainPasswordMismatch)
        }
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
            let cache = self.cached_settings.read().unwrap();
            if let Some(ref settings) = *cache {
                return Ok(Some(settings.clone()));
            }
        }

        // Cache miss, read from database
        let settings = self.db.get_settings()?.map(Settings::from);

        // Update cache with the fresh data
        {
            let mut cache = self.cached_settings.write().unwrap();
            *cache = settings.clone();
        }

        Ok(settings)
    }
}
