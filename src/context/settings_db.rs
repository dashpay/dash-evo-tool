//! `AppSettings` accessors layered on the shared app k/v store.
//!
//! Reads use a small in-memory cache so the egui frame loop can probe
//! settings every frame without paying for a bincode decode each time.
//! Writes go straight to the k/v store; the cache is invalidated under
//! a `SettingsCacheGuard` so concurrent readers cannot observe a
//! stale value.

use super::{AppContext, SettingsCacheGuard};
use crate::model::settings::{AppSettings, detect_dash_qt_path};
use crate::model::user_role::UserRole;
use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use crate::wallet_backend::poison::RwLockRecover;
use crate::wallet_backend::{DetScope, KvAdapterError};

impl AppContext {
    /// Persist the chosen root screen and pin the active-network field
    /// to this context's network.
    pub fn update_settings(&self, root_screen_type: RootScreenType) -> Result<(), KvAdapterError> {
        self.update_app_settings(|settings| {
            settings.root_screen_type = root_screen_type;
            settings.network = self.network;
        })
    }

    /// Persist the theme preference.
    pub fn update_theme_preference(&self, theme_mode: ThemeMode) -> Result<(), KvAdapterError> {
        self.update_app_settings(|settings| settings.theme_mode = theme_mode)
    }

    /// Persist the `auto_start_spv` flag.
    pub fn update_auto_start_spv(&self, auto_start: bool) -> Result<(), KvAdapterError> {
        self.update_app_settings(|settings| settings.auto_start_spv = auto_start)
    }

    /// Persist the `onboarding_completed` flag.
    pub fn update_onboarding_completed(&self, completed: bool) -> Result<(), KvAdapterError> {
        self.update_app_settings(|settings| settings.onboarding_completed = completed)
    }

    /// Atomically read-modify-write the persisted [`AppSettings`].
    ///
    /// `mutate` receives the current settings by mutable reference; the new
    /// value is then persisted and mirrored into the cache. The whole
    /// read → mutate → persist cycle runs under one held `cached_settings`
    /// write lock (the same guard [`Self::set_app_settings`] uses), so
    /// concurrent updaters cannot lose each other's writes — every call
    /// observes the prior committed state.
    pub fn update_app_settings(
        &self,
        mutate: impl FnOnce(&mut AppSettings),
    ) -> Result<(), KvAdapterError> {
        // Holding this guard across the full cycle serialises updaters and
        // blocks readers from observing a half-applied value.
        let mut guard = self.invalidate_settings_cache();
        let mut settings = self.load_app_settings_uncached();
        mutate(&mut settings);
        // On a put failure the cache stays cleared, so the next read reloads
        // from the store instead of serving a value that never persisted.
        self.app_kv
            .put(DetScope::Global, AppSettings::KV_KEY, &settings)?;
        *guard = Some(settings);
        Ok(())
    }

    /// Invalidates the settings cache and returns a guard.
    ///
    /// The cache is invalidated immediately and the guard prevents concurrent access
    /// until the underlying k/v operation completes. This ensures atomicity and
    /// prevents race conditions regardless of whether the write succeeds.
    pub fn invalidate_settings_cache(&'_ self) -> SettingsCacheGuard<'_> {
        let mut guard = self.cached_settings.write_recover();
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
        if let Some(cached) = self.cached_settings.read_recover().clone() {
            return cached;
        }

        // Cache miss: hold the write lock across the load+populate so a
        // concurrent `set_app_settings` (which also holds this write lock for
        // its whole duration) cannot slip a fresh value in between our read and
        // write and then be clobbered by our stale load.
        let mut guard = self.cached_settings.write_recover();
        // Double-check: a racer may have populated the cache while we waited.
        if let Some(cached) = guard.clone() {
            return cached;
        }

        let loaded = self.load_app_settings_uncached();
        *guard = Some(loaded.clone());
        loaded
    }

    /// Load and decode [`AppSettings`] straight from the k/v store, applying the
    /// dash-qt autodetect fallback. Bypasses the cache — the caller must hold
    /// the `cached_settings` write lock. A missing or undecodable blob falls
    /// back to defaults.
    fn load_app_settings_uncached(&self) -> AppSettings {
        let settings = match self
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
        self.seed_user_role_from_env(settings)
    }

    /// Seeds `user_role` from the legacy `.env DEVELOPER_MODE` flag when the
    /// persisted blob recorded no explicit role (`None`) — every pre-migration
    /// user and every fresh install. `DEVELOPER_MODE=true` maps to Power
    /// (today's "developer mode" is power-user mode); anything else maps to
    /// Everyday. Mirrors the impure dash-qt autodetect fallback: decoding stays
    /// pure, the one-time `.env` read happens here at the load site.
    fn seed_user_role_from_env(&self, mut settings: AppSettings) -> AppSettings {
        if settings.user_role.is_none() {
            let developer = crate::config::Config::load_from(&self.data_dir)
                .ok()
                .and_then(|c| c.developer_mode)
                .unwrap_or(false);
            settings.user_role = Some(if developer {
                UserRole::Power
            } else {
                UserRole::Everyday
            });
        }
        settings
    }

    /// Write the [`AppSettings`] blob to the shared app k/v store.
    pub fn set_app_settings(&self, settings: &AppSettings) -> Result<(), KvAdapterError> {
        let mut guard = self.invalidate_settings_cache();
        self.app_kv
            .put(DetScope::Global, AppSettings::KV_KEY, settings)?;
        *guard = Some(settings.clone());
        Ok(())
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
        let settings = AppSettings {
            network: Network::Testnet,
            theme_mode: ThemeMode::Dark,
            user_role: Some(crate::model::user_role::UserRole::Power),
            auto_start_spv: true,
            disable_zmq: true,
            onboarding_completed: true,
            ..AppSettings::default()
        };

        kv.put(DetScope::Global, AppSettings::KV_KEY, &settings)
            .unwrap();
        let got: AppSettings = kv
            .get(DetScope::Global, AppSettings::KV_KEY)
            .unwrap()
            .expect("settings blob present after put");

        assert_eq!(got.network, Network::Testnet);
        assert_eq!(got.theme_mode, ThemeMode::Dark);
        assert_eq!(
            got.user_role,
            Some(crate::model::user_role::UserRole::Power)
        );
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

    /// Build a network-free [`AppContext`] backed by throwaway temp storage —
    /// enough to exercise the settings read-modify-write path.
    fn test_app_context(dir: &std::path::Path) -> Arc<AppContext> {
        crate::app_dir::ensure_env_file(dir);
        let db_file = dir.join("data.db");
        let db = Arc::new(crate::database::Database::new(&db_file).expect("db"));
        db.create_tables(true).expect("create tables");
        db.set_default_version().expect("set version");
        let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
        AppContext::new(
            dir.to_path_buf(),
            Network::Testnet,
            db,
            Default::default(),
            Default::default(),
            egui::Context::default(),
            app_kv,
            secret_store,
            std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
        )
        .expect("AppContext")
    }

    /// Each `update_app_settings` call observes the state committed by the
    /// previous call. A closure that reads a field written by an earlier update
    /// must see it, and independent updates must all accumulate rather than the
    /// last writer overwriting a stale full-blob snapshot.
    #[test]
    fn update_app_settings_reads_prior_committed_state() {
        use crate::model::user_role::UserRole;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        ctx.update_app_settings(|s| s.user_role = Some(UserRole::Power))
            .unwrap();
        ctx.update_app_settings(|s| {
            // The RMW must hand us the value the previous update committed.
            assert_eq!(
                s.user_role,
                Some(UserRole::Power),
                "read-modify-write must observe the prior committed write"
            );
            s.onboarding_completed = true;
        })
        .unwrap();

        let got = ctx.get_app_settings();
        assert_eq!(
            got.user_role,
            Some(UserRole::Power),
            "first update survived"
        );
        assert!(got.onboarding_completed, "second update landed");
    }

    /// Concurrent updates to distinct fields must not lose writes.
    /// Each thread flips its own boolean; under a non-atomic read-modify-write
    /// a thread's stale snapshot would clobber a sibling field written by
    /// another thread. Holding the cache lock across the whole cycle guarantees
    /// every flip survives.
    #[test]
    fn concurrent_field_updates_do_not_lose_writes() {
        use std::sync::Barrier;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        // Baseline: every boolean under test starts false.
        let base = AppSettings {
            overwrite_dash_conf: false,
            disable_zmq: false,
            onboarding_completed: false,
            show_evonode_tools: false,
            close_dash_qt_on_exit: false,
            auto_start_spv: false,
            ..AppSettings::default()
        };
        ctx.set_app_settings(&base).unwrap();

        type Flip = fn(&mut AppSettings);
        let flips: [Flip; 6] = [
            |s| s.overwrite_dash_conf = true,
            |s| s.disable_zmq = true,
            |s| s.onboarding_completed = true,
            |s| s.show_evonode_tools = true,
            |s| s.close_dash_qt_on_exit = true,
            |s| s.auto_start_spv = true,
        ];
        let barrier = Arc::new(Barrier::new(flips.len()));
        let handles: Vec<_> = flips
            .into_iter()
            .map(|flip| {
                let ctx = Arc::clone(&ctx);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..64 {
                        ctx.update_app_settings(flip).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let got = ctx.get_app_settings();
        assert!(got.overwrite_dash_conf);
        assert!(got.disable_zmq);
        assert!(got.onboarding_completed);
        assert!(got.show_evonode_tools);
        assert!(got.close_dash_qt_on_exit);
        assert!(got.auto_start_spv, "no concurrent field update may be lost");
    }

    /// A role-less blob (fresh install, or a pre-migration blob whose legacy
    /// `"Advanced"` sentinel decoded to `None`) is seeded from the legacy `.env`
    /// flag: `DEVELOPER_MODE=true` → Power. This is the one-time reconciliation
    /// that keeps expert users on their level after migration.
    #[test]
    fn role_less_blob_seeds_power_when_env_developer_mode_true() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        let env_path = tmp.path().join(".env");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&env_path)
            .unwrap();
        writeln!(f, "DEVELOPER_MODE=true").unwrap();
        // Drop the guard immediately: it clears the cache, and holding it would
        // deadlock the `get_app_settings` read below.
        drop(ctx.invalidate_settings_cache());

        let got = ctx.get_app_settings();
        assert_eq!(
            got.user_role,
            Some(UserRole::Power),
            "DEVELOPER_MODE=true must seed the Power role for a role-less blob"
        );
    }

    /// An explicit persisted role is authoritative: the `.env` seed only fills a
    /// role-less (`None`) blob, so it must never override a choice already
    /// recorded — even when `DEVELOPER_MODE=true` would otherwise seed Power.
    #[test]
    fn explicit_persisted_role_is_not_reseeded_from_env() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        ctx.update_app_settings(|s| s.user_role = Some(UserRole::Everyday))
            .unwrap();

        let env_path = tmp.path().join(".env");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&env_path)
            .unwrap();
        writeln!(f, "DEVELOPER_MODE=true").unwrap();
        // Drop the guard immediately: it clears the cache, and holding it would
        // deadlock the `get_app_settings` read below.
        drop(ctx.invalidate_settings_cache());

        let got = ctx.get_app_settings();
        assert_eq!(
            got.user_role,
            Some(UserRole::Everyday),
            "an explicit role must survive; the .env seed only fills a role-less blob"
        );
    }
}
