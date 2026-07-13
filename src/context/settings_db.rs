//! `AppSettings` accessors layered on the shared app k/v store.
//!
//! Reads use a small in-memory cache so the egui frame loop can probe
//! settings every frame without paying for a bincode decode each time.
//! Writes go straight to the k/v store; the cache is invalidated under
//! a `SettingsCacheGuard` so concurrent readers cannot observe a
//! stale value.

use super::{AppContext, SettingsCacheGuard};
use crate::model::edition::Edition;
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
    ///
    /// # Errors
    ///
    /// Returns the k/v error when the current settings cannot be read, or when
    /// the mutated value cannot be written. A failed *read* aborts before the
    /// write: committing `mutate` on top of a defaults blob would reset every
    /// field it does not touch.
    pub fn update_app_settings(
        &self,
        mutate: impl FnOnce(&mut AppSettings),
    ) -> Result<(), KvAdapterError> {
        // Holding this guard across the full cycle serialises updaters and
        // blocks readers from observing a half-applied value.
        let mut guard = self.invalidate_settings_cache();
        let mut settings = self.load_app_settings_uncached()?;
        mutate(&mut settings);
        // On a put failure the cache stays cleared, so the next read reloads
        // from the store instead of serving a value that never persisted.
        self.app_kv
            .put(DetScope::Global, AppSettings::KV_KEY, &settings)?;
        // The blob is persisted verbatim, the cache holds the resolved view — the
        // value a reload would produce. A `mutate` that clears the role must not
        // leave the cache serving one no reader expects (see `with_default_user_role`).
        *guard = Some(with_default_user_role(settings));
        Ok(())
    }

    /// Set the app-global user role and persist it as the canonical string in
    /// [`AppSettings`] — the single source of truth the runtime role is seeded
    /// from at the next boot. Both role-setting UI surfaces (Settings selector,
    /// onboarding row) go through here so the runtime role and the persisted
    /// value never diverge.
    ///
    /// The role is persisted **before** it is published to the runtime role cell:
    /// a role that only ever reached the cell would look accepted, then silently
    /// revert at the next boot, when the seed reads the canonical value that was
    /// never written.
    ///
    /// # Errors
    ///
    /// Returns the k/v error when the role could not be persisted. The runtime
    /// role is then left untouched, and the caller must surface the failure —
    /// the user's selection did not take effect.
    pub fn set_and_persist_user_role(&self, role: UserRole) -> Result<(), KvAdapterError> {
        self.update_app_settings(|s| s.user_role = Some(role))?;
        self.set_user_role(role);
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
    /// Returns defaults when the blob is absent (first run), and — for this read
    /// only — when it cannot be read or decoded (e.g. a future schema): the frame
    /// loop needs a value every frame, so an unreadable blob degrades to defaults
    /// in memory. The stored blob is left untouched and the fallback is never
    /// cached, so a transient failure costs the user nothing once the store
    /// recovers. Successful reads are cached in-memory between updates.
    ///
    /// Callers that make a *durable* decision from the result — one nothing later
    /// re-derives, such as seeding the runtime role — must use
    /// [`try_get_app_settings`](Self::try_get_app_settings) instead: they need to
    /// tell "the user stored nothing" apart from "we could not read what the user
    /// stored", and this method deliberately erases that difference.
    pub fn get_app_settings(&self) -> AppSettings {
        self.try_get_app_settings().unwrap_or_else(|e| {
            tracing::warn!(
                error = ?e,
                "Could not read the stored app settings; serving defaults for this read only"
            );
            with_default_user_role(AppSettings::default())
        })
    }

    /// Read the persisted [`AppSettings`], reporting a failed read instead of
    /// degrading to defaults.
    ///
    /// The `user_role` of a returned value is always `Some`, on the cached path as
    /// well as on a fresh load — an absent role is resolved to
    /// [`UserRole::WHEN_UNSET`] in memory (see [`with_default_user_role`]), and every
    /// writer caches that resolved view. Callers may rely on the invariant.
    ///
    /// # Errors
    ///
    /// Returns the k/v error when the stored blob cannot be read or decoded.
    /// The failure is never cached: only a write invalidates the cache, and a
    /// read-only session performs none, so caching it would pin the process to
    /// defaults long after the store recovered.
    pub fn try_get_app_settings(&self) -> Result<AppSettings, KvAdapterError> {
        // Fast path: cache hit under a read lock.
        if let Some(cached) = self.cached_settings.read_recover().clone() {
            return Ok(cached);
        }

        // Cache miss: hold the write lock across the load+populate so a
        // concurrent `set_app_settings` (which also holds this write lock for
        // its whole duration) cannot slip a fresh value in between our read and
        // write and then be clobbered by our stale load.
        let mut guard = self.cached_settings.write_recover();
        // Double-check: a racer may have populated the cache while we waited.
        if let Some(cached) = guard.clone() {
            return Ok(cached);
        }

        let loaded = self.load_app_settings_uncached()?;
        *guard = Some(loaded.clone());
        Ok(loaded)
    }

    /// Seed the runtime role cell from the persisted [`AppSettings`] — the single
    /// source of truth — publishing it to every context that shares the cell.
    ///
    /// Runs once per process, at boot. A successful read is authoritative,
    /// *including* a blob that never recorded a role: that resolves to
    /// [`UserRole::WHEN_UNSET`], which is how a fresh install and a pre-role
    /// account keep the power surface they have always had.
    ///
    /// A **failed** read is not authoritative and must not borrow that resolution.
    /// Nothing re-seeds the cell from the later reads that succeed, so treating an
    /// unreadable store as "no role recorded" would grant `WHEN_UNSET`'s elevated
    /// surface for the whole session — on one transient hiccup, to a user whose
    /// stored role may be Everyday. It seeds [`UserRole::LEAST_PRIVILEGED`]
    /// instead. The real role returns at the next start; until then the user sees
    /// the least-privileged mode in the interface-mode selector and can re-pick
    /// their own, which persists and republishes it.
    pub fn seed_user_role_from_settings(&self) {
        let role = match self.try_get_app_settings() {
            Ok(settings) => settings
                .user_role
                .expect("invariant: try_get_app_settings resolves an absent role to WHEN_UNSET"),
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    role = UserRole::LEAST_PRIVILEGED.as_str(),
                    "Could not read the stored interface mode at startup; starting in the \
                     least-privileged mode. Restart the app, or pick the mode again in \
                     Settings, to restore it."
                );
                UserRole::LEAST_PRIVILEGED
            }
        };
        self.set_user_role(role);
    }

    /// Land a masternode-owner edition build on [`UserRole::Power`] the first
    /// time it runs, so its only meaningful surface — the Power-gated
    /// Masternodes screen — is reachable out of the box.
    ///
    /// A no-op in every other edition. In the masternode-owner edition it fires
    /// **only on first run**: when no role has ever been persisted (a fresh or
    /// pre-role install). An explicit prior choice is never overridden — a user
    /// who picked Developer (the edition's escape hatch) or any other role keeps
    /// it. Once this records `Power`, the disk holds an explicit role and the
    /// force never fires again ("persists normally after that").
    ///
    /// Deliberately keyed on a role that was **never recorded**, distinct from
    /// one that could not be **read**: a transient k/v read failure must not be
    /// mistaken for "first run" and silently rewrite the user's real role. On a
    /// read error this does nothing and leaves the boot seed
    /// ([`seed_user_role_from_settings`](Self::seed_user_role_from_settings)) in
    /// charge.
    ///
    /// Runs once per process, at boot, after the role has been seeded.
    pub fn apply_edition_first_run_role(&self) {
        if Edition::CURRENT != Edition::MasternodeOwner {
            return;
        }
        // Read the raw persisted role (pre-resolution): `Some(None)` is "a blob
        // exists but recorded no role", `None` is "no blob yet" — both are first
        // run; `Some(Some(_))` is an explicit prior choice to respect.
        match self
            .app_kv
            .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
        {
            Ok(Some(settings)) if settings.user_role.is_some() => {} // explicit choice — keep it
            Ok(_) => {
                if let Err(e) = self.set_and_persist_user_role(UserRole::Power) {
                    tracing::warn!(
                        error = ?e,
                        "Could not record the initial interface mode for this edition; \
                         the Masternodes screen may be hidden until a role is picked in Settings."
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "Could not read the stored interface mode at startup; leaving the \
                     seeded role in place for this edition."
                );
            }
        }
    }

    /// Load and decode [`AppSettings`] straight from the k/v store, applying the
    /// dash-qt autodetect and default-role fallbacks. Bypasses the cache — the
    /// caller must hold the `cached_settings` write lock.
    ///
    /// An absent blob (first run) reads as defaults. Read-only: both fallbacks
    /// resolve in memory, so a load never writes back and can never trade a
    /// decoded blob for a defaults one.
    ///
    /// # Errors
    ///
    /// Returns the k/v error when the stored blob cannot be read or decoded. An
    /// unreadable blob is emphatically *not* "no settings stored": the value is
    /// still there, just unavailable right now (poisoned lock, SQLite hiccup,
    /// future schema). Reporting that as an error is what stops the
    /// read-modify-write in [`Self::update_app_settings`] from overwriting the
    /// user's real settings with a defaults blob on one transient failure.
    fn load_app_settings_uncached(&self) -> Result<AppSettings, KvAdapterError> {
        let stored = self
            .app_kv
            .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)?;
        let settings = stored.map(with_dash_qt_path_fallback).unwrap_or_default();
        Ok(with_default_user_role(settings))
    }

    /// Write the [`AppSettings`] blob to the shared app k/v store.
    ///
    /// The blob is stored verbatim — a `user_role` of `None` stays `None` on disk
    /// until the user picks one. The cache takes the *resolved* view (see
    /// [`with_default_user_role`]), so a cached read cannot hand out a role-less
    /// value that a reload of the same blob would have resolved.
    pub fn set_app_settings(&self, settings: &AppSettings) -> Result<(), KvAdapterError> {
        let mut guard = self.invalidate_settings_cache();
        self.app_kv
            .put(DetScope::Global, AppSettings::KV_KEY, settings)?;
        *guard = Some(with_default_user_role(settings.clone()));
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

/// Resolves the role of an account that never recorded one — a fresh install, or
/// a blob written before the role field existed — to [`UserRole::WHEN_UNSET`].
/// In-memory only: the stored `None` stays until the user picks a role, so this
/// fallback never has a write to fail.
fn with_default_user_role(mut settings: AppSettings) -> AppSettings {
    settings.user_role.get_or_insert(UserRole::WHEN_UNSET);
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{test_app_context, test_app_context_with_kv};
    use crate::wallet_backend::DetKv;
    use crate::wallet_backend::kv_test_support::{FailingKv, InMemoryKv};
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

    /// An account with no stored blob at all (fresh install) resolves to Power,
    /// not to the enum's `Default` (Everyday).
    #[test]
    fn absent_blob_resolves_to_power() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        assert_eq!(
            ctx.get_app_settings().user_role,
            Some(UserRole::Power),
            "an account with no recorded role starts at Power"
        );
    }

    /// An explicit persisted role is authoritative: the default only fills a
    /// role-less (`None`) blob, so it must never override a recorded choice —
    /// including a deliberate downgrade to Everyday.
    #[test]
    fn explicit_persisted_role_overrides_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        ctx.update_app_settings(|s| s.user_role = Some(UserRole::Everyday))
            .unwrap();
        drop(ctx.invalidate_settings_cache());

        assert_eq!(
            ctx.get_app_settings().user_role,
            Some(UserRole::Everyday),
            "an explicit role must survive; the default only fills a role-less blob"
        );
    }

    /// Regression: a failed *read* must never provoke a write-back.
    ///
    /// A read error is not "no settings stored" — the blob is still there, just
    /// unreadable right now (poisoned lock, SQLite hiccup, future schema). Taking
    /// `AppSettings::default()` as the loaded value and persisting anything on top
    /// of it would overwrite the user's real settings — network, onboarding, SPV
    /// prefs, theme — with defaults, on one transient hiccup.
    #[test]
    fn read_error_does_not_overwrite_persisted_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let ctx = test_app_context_with_kv(tmp.path(), Arc::new(DetKv::from_store(store.clone())));

        // The user's real, fully-populated settings.
        let stored = AppSettings {
            network: Network::Testnet,
            theme_mode: ThemeMode::Dark,
            user_role: Some(UserRole::Developer),
            auto_start_spv: true,
            onboarding_completed: true,
            ..AppSettings::default()
        };
        ctx.set_app_settings(&stored).unwrap();
        let puts_before = store.put_count();

        // Every read now fails. Bypass the cache so the load actually hits it.
        store.fail_reads(true);
        drop(ctx.invalidate_settings_cache());
        let got = ctx.get_app_settings();

        assert_eq!(
            store.put_count(),
            puts_before,
            "a failed read must not write anything back to the store"
        );
        assert!(
            got.user_role.is_some(),
            "the caller still gets a usable role for this read"
        );

        // The store recovers: the stored settings must be exactly as they were.
        store.fail_reads(false);
        drop(ctx.invalidate_settings_cache());
        let recovered = ctx.get_app_settings();
        assert_eq!(
            recovered.user_role,
            Some(UserRole::Developer),
            "the persisted role survived the failed read"
        );
        assert_eq!(recovered.theme_mode, ThemeMode::Dark);
        assert!(recovered.onboarding_completed);
        assert!(recovered.auto_start_spv);
    }

    /// Regression: a transient read failure must not poison the in-memory cache.
    ///
    /// The error fallback is a defaults blob served for *that one read*. Caching it
    /// pins the whole process to defaults — network, theme, onboarding flag and role
    /// all reset in memory — until the next explicit write invalidates the cache.
    /// A read-only session (the MCP stdio boot path, a UI session that changes no
    /// setting) never performs that write, so the next read after the store recovers
    /// must go back to the store rather than serve the stale fallback.
    #[test]
    fn read_error_does_not_poison_the_settings_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let ctx = test_app_context_with_kv(tmp.path(), Arc::new(DetKv::from_store(store.clone())));

        let stored = AppSettings {
            network: Network::Testnet,
            theme_mode: ThemeMode::Dark,
            user_role: Some(UserRole::Developer),
            onboarding_completed: true,
            ..AppSettings::default()
        };
        ctx.set_app_settings(&stored).unwrap();

        // One failing read, with the cache cold so the load actually hits the store.
        store.fail_reads(true);
        drop(ctx.invalidate_settings_cache());
        assert_eq!(
            ctx.get_app_settings().network,
            AppSettings::default().network,
            "an unreadable store degrades to defaults for that read"
        );

        // The store recovers on its own; nothing invalidates the cache in between.
        store.fail_reads(false);
        let recovered = ctx.get_app_settings();

        assert_eq!(
            recovered.network,
            Network::Testnet,
            "the read after recovery must retry the store, not serve the cached fallback"
        );
        assert_eq!(recovered.theme_mode, ThemeMode::Dark);
        assert_eq!(recovered.user_role, Some(UserRole::Developer));
        assert!(recovered.onboarding_completed);
    }

    /// A role-less blob resolves to the default role in memory only: the decoded
    /// settings keep every field the user did store, and nothing is written back.
    ///
    /// The write-back is what used to make this path dangerous — a failing `put`
    /// discarded the successfully decoded blob and dropped the caller all the way
    /// to `AppSettings::default()`. With the role defaulted purely in memory there
    /// is no write to fail.
    #[test]
    fn role_less_blob_keeps_real_settings_and_is_not_written_back() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let ctx = test_app_context_with_kv(tmp.path(), Arc::new(DetKv::from_store(store.clone())));

        // A pre-migration blob: real settings, no role ever recorded.
        ctx.set_app_settings(&AppSettings {
            network: Network::Testnet,
            theme_mode: ThemeMode::Dark,
            onboarding_completed: true,
            user_role: None,
            ..AppSettings::default()
        })
        .unwrap();
        let puts_before = store.put_count();

        drop(ctx.invalidate_settings_cache());
        let got = ctx.get_app_settings();

        assert_eq!(
            got.user_role,
            Some(UserRole::Power),
            "a role-less account defaults to Power"
        );
        assert_eq!(got.network, Network::Testnet, "real settings survive");
        assert_eq!(got.theme_mode, ThemeMode::Dark);
        assert!(got.onboarding_completed);
        assert_eq!(
            store.put_count(),
            puts_before,
            "resolving the default role must not write to the store"
        );
    }

    /// A write that cannot read the current blob must fail instead of committing
    /// its mutation on top of defaults — the read-modify-write would otherwise
    /// silently reset every field it does not touch.
    #[test]
    fn update_on_unreadable_store_fails_without_clobbering() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let ctx = test_app_context_with_kv(tmp.path(), Arc::new(DetKv::from_store(store.clone())));

        // `auto_start_spv` defaults to true, so pin the baseline the update flips.
        ctx.set_app_settings(&AppSettings {
            user_role: Some(UserRole::Developer),
            onboarding_completed: true,
            auto_start_spv: false,
            ..AppSettings::default()
        })
        .unwrap();
        let puts_before = store.put_count();

        store.fail_reads(true);
        drop(ctx.invalidate_settings_cache());
        let result = ctx.update_app_settings(|s| s.auto_start_spv = true);

        assert!(result.is_err(), "an unreadable store must fail the update");
        assert_eq!(
            store.put_count(),
            puts_before,
            "a failed read-modify-write must not commit anything"
        );

        store.fail_reads(false);
        drop(ctx.invalidate_settings_cache());
        let recovered = ctx.get_app_settings();
        assert_eq!(recovered.user_role, Some(UserRole::Developer));
        assert!(recovered.onboarding_completed);
        assert!(
            !recovered.auto_start_spv,
            "the failed update left the stored blob untouched"
        );
    }

    /// The role reaches the runtime role cell only once it is safely persisted:
    /// publishing first would show the user a mode that silently reverts on the
    /// next boot, because the boot seed reads the (unwritten) canonical value.
    #[test]
    fn user_role_is_not_published_when_it_cannot_be_persisted() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let ctx = test_app_context_with_kv(tmp.path(), Arc::new(DetKv::from_store(store.clone())));

        ctx.set_and_persist_user_role(UserRole::Everyday)
            .expect("a healthy store persists the role");
        assert_eq!(ctx.user_role(), UserRole::Everyday);

        store.fail_reads(true);
        drop(ctx.invalidate_settings_cache());
        let result = ctx.set_and_persist_user_role(UserRole::Developer);

        assert!(
            result.is_err(),
            "a failed persist must be reported to the caller, not swallowed"
        );
        assert_eq!(
            ctx.user_role(),
            UserRole::Everyday,
            "the runtime role must not advertise a mode that never reached the store"
        );
    }

    /// The "a returned `user_role` is always `Some`" invariant holds on the *cached*
    /// path too, not only on a fresh load.
    ///
    /// Both writers populate the cache from a caller-supplied blob whose role may be
    /// `None`, so the resolution has to happen there as well —
    /// `seed_user_role_from_settings` reads the invariant with an `expect`, and a
    /// cached role-less blob would otherwise panic it.
    #[test]
    fn a_cached_role_less_blob_still_reads_as_a_resolved_role() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        // Nothing invalidates the cache between each write and its read, so the
        // value under test is the cached one, not a reload.
        ctx.set_app_settings(&AppSettings {
            user_role: None,
            ..AppSettings::default()
        })
        .unwrap();
        assert_eq!(
            ctx.get_app_settings().user_role,
            Some(UserRole::WHEN_UNSET),
            "set_app_settings must cache the resolved role, not the raw None"
        );

        ctx.update_app_settings(|s| s.user_role = None).unwrap();
        assert_eq!(
            ctx.try_get_app_settings().unwrap().user_role,
            Some(UserRole::WHEN_UNSET),
            "update_app_settings must cache the resolved role, not the raw None"
        );

        // The seed `expect`s that invariant: a role-less cached blob must not panic it.
        ctx.seed_user_role_from_settings();
        assert_eq!(ctx.user_role(), UserRole::WHEN_UNSET);
    }

    /// Regression: a settings read that *fails* at boot must not elevate the
    /// session's role.
    ///
    /// The seed runs once per process and nothing re-seeds the cell from the later
    /// reads that succeed, so resolving an unreadable store the way a role-less blob
    /// is resolved (to `WHEN_UNSET`, i.e. Power) would hand the whole session the
    /// power surface — on one poisoned lock, for a user whose stored role is
    /// Everyday. Unknown must resolve *down*.
    #[test]
    fn failed_boot_read_seeds_the_least_privileged_role_never_power() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let ctx = test_app_context_with_kv(tmp.path(), Arc::new(DetKv::from_store(store.clone())));

        ctx.set_and_persist_user_role(UserRole::Everyday)
            .expect("a healthy store persists the role");

        // Park the cell above the expected outcome, so a seed that silently did
        // nothing would be indistinguishable from neither fallback firing.
        ctx.set_user_role(UserRole::Developer);

        store.fail_reads(true);
        drop(ctx.invalidate_settings_cache());
        ctx.seed_user_role_from_settings();

        assert_eq!(
            ctx.user_role(),
            UserRole::LEAST_PRIVILEGED,
            "an unreadable store must seed the least-privileged role"
        );
        assert_ne!(
            ctx.user_role(),
            UserRole::WHEN_UNSET,
            "a failed read is not 'no role recorded': it must never grant Power"
        );

        // The store recovers. The user's real role is still Everyday — a restart
        // (or re-picking the mode) seeds it, and the failed read left nothing behind.
        store.fail_reads(false);
        drop(ctx.invalidate_settings_cache());
        ctx.seed_user_role_from_settings();

        assert_eq!(
            ctx.user_role(),
            UserRole::Everyday,
            "the persisted role is authoritative once the store can be read"
        );
    }

    /// The other half of the contract: a *successful* read of a blob that never
    /// recorded a role still resolves to `WHEN_UNSET` (Power). This is intentional
    /// product behaviour for fresh installs and pre-role blobs, and hardening the
    /// failure path must not have quietly demoted it.
    #[test]
    fn successful_boot_read_of_a_role_less_blob_seeds_when_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        ctx.update_app_settings(|s| s.user_role = None).unwrap();
        ctx.set_user_role(UserRole::Everyday);
        drop(ctx.invalidate_settings_cache());

        ctx.seed_user_role_from_settings();

        assert_eq!(
            ctx.user_role(),
            UserRole::WHEN_UNSET,
            "an account that never chose a role keeps the power surface it always had"
        );
    }

    /// A persisted role is seeded verbatim — the boot seed reads the canonical
    /// value, so a deliberate downgrade survives a restart.
    #[test]
    fn boot_seed_publishes_the_persisted_role() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            ctx.update_app_settings(|s| s.user_role = Some(role))
                .unwrap();
            ctx.set_user_role(UserRole::LEAST_PRIVILEGED);

            ctx.seed_user_role_from_settings();

            assert_eq!(ctx.user_role(), role, "the boot seed must publish {role:?}");
        }
    }

    /// The happy path still publishes and persists together.
    #[test]
    fn user_role_is_published_and_persisted_together() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());

        ctx.set_and_persist_user_role(UserRole::Power).unwrap();

        assert_eq!(ctx.user_role(), UserRole::Power);
        assert_eq!(ctx.get_app_settings().user_role, Some(UserRole::Power));
    }

    /// In a non-edition (Full) build the first-run force is a no-op: it must not
    /// touch the role, so a Full build never silently promotes anyone.
    #[cfg(not(feature = "masternode-owner-edition"))]
    #[test]
    fn first_run_force_is_a_no_op_outside_the_edition() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_user_role(UserRole::Everyday);

        ctx.apply_edition_first_run_role();

        assert_eq!(ctx.user_role(), UserRole::Everyday);
        // Nothing was persisted either.
        assert_eq!(
            ctx.app_kv()
                .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
                .unwrap()
                .and_then(|s| s.user_role),
            None,
        );
    }
}

/// First-run role-forcing behaviour of the masternode-owner edition. These only
/// have teeth when compiled as that edition (`Edition::CURRENT` is a no-op
/// otherwise), so the whole module is feature-gated.
#[cfg(all(test, feature = "masternode-owner-edition"))]
mod edition_first_run_tests {
    use super::*;
    use crate::context::test_support::test_app_context;

    /// The raw persisted role, before the `WHEN_UNSET` resolution — `None` means
    /// no explicit choice was ever recorded.
    fn persisted_role(ctx: &AppContext) -> Option<UserRole> {
        ctx.app_kv()
            .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
            .unwrap()
            .and_then(|s| s.user_role)
    }

    /// First run (no role ever recorded) lands on Power and records it, so the
    /// Power-gated Masternodes surface is reachable out of the box and the force
    /// never fires again.
    #[test]
    fn first_run_lands_on_power_and_persists_it() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        assert_eq!(persisted_role(&ctx), None, "no role recorded yet");

        ctx.apply_edition_first_run_role();

        assert_eq!(ctx.user_role(), UserRole::Power);
        assert_eq!(
            persisted_role(&ctx),
            Some(UserRole::Power),
            "the force must record an explicit role so it fires only once"
        );
    }

    /// An explicit prior choice is never overridden — the force is first-run
    /// only. A user who dropped to Everyday keeps it (recoverable via Settings).
    #[test]
    fn explicit_everyday_choice_is_not_overridden() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_and_persist_user_role(UserRole::Everyday).unwrap();

        ctx.apply_edition_first_run_role();

        assert_eq!(ctx.user_role(), UserRole::Everyday);
        assert_eq!(persisted_role(&ctx), Some(UserRole::Everyday));
    }

    /// The Developer escape hatch survives the force untouched.
    #[test]
    fn developer_escape_hatch_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_and_persist_user_role(UserRole::Developer).unwrap();

        ctx.apply_edition_first_run_role();

        assert_eq!(ctx.user_role(), UserRole::Developer);
    }

    /// Idempotent: a second boot (role now recorded as Power) leaves an
    /// Everyday runtime role alone — proving the force keys on the persisted
    /// value, not the live one, and cannot re-fire.
    #[test]
    fn force_does_not_refire_once_a_role_is_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.apply_edition_first_run_role(); // records Power
        assert_eq!(persisted_role(&ctx), Some(UserRole::Power));

        // Simulate a later, lower runtime role (as if the user picked Everyday
        // this session but it is already recorded as Power on disk).
        ctx.set_user_role(UserRole::Everyday);
        ctx.apply_edition_first_run_role();

        assert_eq!(
            ctx.user_role(),
            UserRole::Everyday,
            "an already-recorded role must not be re-forced"
        );
    }
}
