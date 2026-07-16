//! One-shot boot import of the legacy `settings` row into [`AppSettings`].
//!
//! The unwire moved user preferences into the app k/v store but never
//! carried the existing `data.db` row across, so an upgrading user booted
//! with `AppSettings::default()`: mainnet, System theme, onboarding
//! un-completed. Relaunching a testnet user on mainnet is a safety hazard,
//! not a cosmetic reset — this import closes that gap.
//!
//! Unlike [`finish_unwire`](super::finish_unwire) this is **not** a backend
//! task. The active network is chosen in `AppState::new_inner` from the
//! settings blob, before any `AppContext` (and therefore any async runtime)
//! exists, so the import has to run synchronously at that point or the boot
//! would already have picked the wrong network.

use dash_sdk::dpp::dashcore::Network;
use serde::{Deserialize, Serialize};

use crate::database::Database;
use crate::model::data_migration::{
    DirectMigrationVersion, MAX_DIRECT_MIGRATION_VERSION, MIN_DIRECT_MIGRATION_VERSION,
    classify_direct_migration_version,
};
use crate::model::settings::AppSettings;
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};

/// Sentinel marking the settings import as done for this install. Global —
/// [`AppSettings`] is a single cross-network blob, so unlike the
/// [`finish_unwire`](super::finish_unwire) sentinel this one is not
/// per-network. Versioned so a future format change bumps the key.
const SENTINEL_KEY: &str = "det:migration:legacy_settings:v1";

/// What the import did on this launch. Returned so the caller can log it and
/// tests can assert the branch taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsImport {
    /// The sentinel was already present — a previous launch imported (or
    /// found nothing to import). Never touches the user's current settings.
    AlreadyDone,
    /// No legacy `settings` row existed (fresh install). Sentinel written so
    /// later launches skip the probe.
    NoLegacyData,
    /// Preferences were carried across. Carries the network that was
    /// restored, which is the safety-critical field.
    Imported { network: Network },
}

/// Failure of the settings import. The caller boots with defaults on error
/// and leaves the sentinel unwritten, so the next launch retries.
#[derive(Debug, thiserror::Error)]
pub enum SettingsImportError {
    /// The legacy database predates the oldest layout this import supports.
    #[error(
        "legacy data version {found} is older than the minimum directly migratable version {minimum_supported}"
    )]
    LegacyDataTooOld { found: i64, minimum_supported: i64 },

    /// The legacy database comes from a newer, unknown layout.
    #[error(
        "legacy data version {found} is newer than the maximum directly migratable version {maximum_supported}"
    )]
    LegacyDataTooNew { found: i64, maximum_supported: i64 },

    /// The legacy `settings` row could not be read from `data.db`.
    #[error("could not read legacy settings")]
    LegacyRead {
        #[source]
        source: rusqlite::Error,
    },

    /// The imported settings, or the sentinel, could not be written to the
    /// app k/v store.
    #[error("could not write imported settings")]
    Write {
        #[source]
        source: KvAdapterError,
    },
}

/// Carry the legacy `settings` row into the app k/v store, once per install.
///
/// Idempotent: the [`SENTINEL_KEY`] short-circuits every launch after the
/// first. The import deliberately **overwrites** any settings blob already
/// present, because until this import shipped an upgrading user's first
/// launch wrote a `default()` blob (mainnet) over their real preferences —
/// skipping on "a blob exists" would make that reset permanent.
///
/// # Errors
///
/// Returns [`SettingsImportError`] when the saved data is outside the supported
/// direct-update range, `data.db` cannot be read, or the k/v store cannot be
/// written. The sentinel stays unwritten on failure.
pub fn import_legacy_settings(
    app_kv: &DetKv,
    db: &Database,
) -> Result<SettingsImport, SettingsImportError> {
    if sentinel_present(app_kv)? {
        return Ok(SettingsImport::AlreadyDone);
    }

    let version = db
        .stored_data_version()
        .map_err(|source| SettingsImportError::LegacyRead { source })?
        .unwrap_or(0);

    match classify_direct_migration_version(version) {
        DirectMigrationVersion::TooOld => {
            return Err(SettingsImportError::LegacyDataTooOld {
                found: version,
                minimum_supported: MIN_DIRECT_MIGRATION_VERSION,
            });
        }
        DirectMigrationVersion::Supported => {}
        DirectMigrationVersion::TooNew => {
            return Err(SettingsImportError::LegacyDataTooNew {
                found: version,
                maximum_supported: MAX_DIRECT_MIGRATION_VERSION,
            });
        }
    }

    let legacy = db
        .read_legacy_app_settings()
        .map_err(|source| SettingsImportError::LegacyRead { source })?;

    let Some(settings) = legacy else {
        write_sentinel(app_kv)?;
        tracing::debug!(
            target = "migration::legacy_settings",
            version,
            "Supported saved data has no legacy preferences — nothing to import",
        );
        return Ok(SettingsImport::NoLegacyData);
    };

    let network = settings.network;
    app_kv
        .put(DetScope::Global, AppSettings::KV_KEY, &settings)
        .map_err(|source| SettingsImportError::Write { source })?;
    write_sentinel(app_kv)?;

    tracing::info!(
        target = "migration::legacy_settings",
        network = ?network,
        theme = ?settings.theme_mode,
        onboarding_completed = settings.onboarding_completed,
        "Imported preferences from the previous version",
    );
    Ok(SettingsImport::Imported { network })
}

/// Marker payload for [`SENTINEL_KEY`]. A struct rather than a bare `bool` so
/// the payload can gain diagnostics fields without a key bump.
#[derive(Debug, Serialize, Deserialize)]
struct SettingsImportSentinel {
    /// Version tag of the build that ran the import.
    sha: String,
}

fn sentinel_present(app_kv: &DetKv) -> Result<bool, SettingsImportError> {
    app_kv
        .get::<SettingsImportSentinel>(DetScope::Global, SENTINEL_KEY)
        .map(|v| v.is_some())
        .map_err(|source| SettingsImportError::Write { source })
}

fn write_sentinel(app_kv: &DetKv) -> Result<(), SettingsImportError> {
    let sentinel = SettingsImportSentinel {
        sha: env!("CARGO_PKG_VERSION").to_string(),
    };
    app_kv
        .put(DetScope::Global, SENTINEL_KEY, &sentinel)
        .map_err(|source| SettingsImportError::Write { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::settings::{RootScreenType, ThemeMode};
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use std::sync::Arc;

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    /// A `data.db` shaped like a v0.10-dev install whose owner runs on
    /// testnet with a dark theme and finished onboarding.
    fn legacy_db(dir: &std::path::Path) -> Database {
        let db = Database::new(dir.join("data.db")).expect("open db");
        {
            let conn = db.locked_conn();
            conn.execute_batch(
                "CREATE TABLE settings (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    network TEXT NOT NULL,
                    start_root_screen INTEGER NOT NULL,
                    theme_preference TEXT,
                    onboarding_completed INTEGER,
                    database_version INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO settings
                 (id, network, start_root_screen, theme_preference, onboarding_completed,
                  database_version)
                 VALUES (1, 'testnet', ?1, 'Dark', 1, 40)",
                rusqlite::params![RootScreenType::RootScreenDPNSScheduledVotes.to_int()],
            )
            .unwrap();
        }
        db
    }

    fn stored(app_kv: &DetKv) -> Option<AppSettings> {
        app_kv
            .get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY)
            .expect("read settings blob")
    }

    /// The headline regression: an upgrading testnet user must not relaunch
    /// on mainnet. Also proves theme and the onboarding flag come across.
    #[test]
    fn import_restores_network_theme_and_onboarding() {
        let dir = tempfile::tempdir().unwrap();
        let db = legacy_db(dir.path());
        let app_kv = kv();

        let outcome = import_legacy_settings(&app_kv, &db).expect("import");

        assert_eq!(
            outcome,
            SettingsImport::Imported {
                network: Network::Testnet
            }
        );
        let settings = stored(&app_kv).expect("settings written");
        assert_eq!(
            settings.network,
            Network::Testnet,
            "a testnet user must not be relaunched on mainnet",
        );
        assert_eq!(settings.theme_mode, ThemeMode::Dark);
        assert!(settings.onboarding_completed);
        assert_eq!(
            settings.root_screen_type,
            RootScreenType::RootScreenDPNSScheduledVotes
        );
    }

    #[test]
    fn too_old_version_is_rejected_without_marking_the_import_done() {
        let dir = tempfile::tempdir().unwrap();
        let db = legacy_db(dir.path());
        db.execute("UPDATE settings SET database_version = 10 WHERE id = 1", [])
            .expect("set too-old version");
        let app_kv = kv();

        let error = import_legacy_settings(&app_kv, &db).expect_err("version 10 must fail");

        assert!(matches!(
            error,
            SettingsImportError::LegacyDataTooOld {
                found: 10,
                minimum_supported: 11,
            }
        ));
        assert!(stored(&app_kv).is_none(), "no preferences may be imported");

        db.execute("UPDATE settings SET database_version = 11 WHERE id = 1", [])
            .expect("set v0.9.3 version");
        assert!(matches!(
            import_legacy_settings(&app_kv, &db),
            Ok(SettingsImport::Imported { .. })
        ));
    }

    #[test]
    fn empty_settings_table_is_rejected_without_writing_the_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::new(dir.path().join("data.db")).expect("open db");
        db.execute(
            "CREATE TABLE settings (id INTEGER PRIMARY KEY, database_version INTEGER NOT NULL)",
            [],
        )
        .expect("create empty settings table");
        let app_kv = kv();

        assert!(matches!(
            import_legacy_settings(&app_kv, &db),
            Err(SettingsImportError::LegacyDataTooOld { found: 0, .. })
        ));
        assert!(!sentinel_present(&app_kv).expect("read sentinel"));
    }

    #[test]
    fn missing_settings_table_is_rejected_without_writing_the_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::new(dir.path().join("data.db")).expect("open db");
        let app_kv = kv();

        assert!(matches!(
            import_legacy_settings(&app_kv, &db),
            Err(SettingsImportError::LegacyDataTooOld { found: 0, .. })
        ));
        assert!(!sentinel_present(&app_kv).expect("read sentinel"));
    }

    /// The first launch after upgrading (before this import existed) wrote a
    /// `default()` blob over the user's preferences. The import must repair
    /// that, not treat the default blob as a user choice worth keeping.
    #[test]
    fn import_overwrites_the_default_blob_a_prior_launch_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let db = legacy_db(dir.path());
        let app_kv = kv();
        app_kv
            .put(
                DetScope::Global,
                AppSettings::KV_KEY,
                &AppSettings::default(),
            )
            .expect("seed default blob");
        assert_eq!(stored(&app_kv).unwrap().network, Network::Mainnet);

        import_legacy_settings(&app_kv, &db).expect("import");

        assert_eq!(
            stored(&app_kv).unwrap().network,
            Network::Testnet,
            "the reset-to-mainnet blob must be repaired from the legacy row",
        );
    }

    /// Once imported, later launches must never clobber settings the user has
    /// since changed — the sentinel, not the blob's contents, is the guard.
    #[test]
    fn import_is_a_no_op_once_the_sentinel_exists() {
        let dir = tempfile::tempdir().unwrap();
        let db = legacy_db(dir.path());
        let app_kv = kv();

        import_legacy_settings(&app_kv, &db).expect("first import");

        // The user switches to mainnet after the import.
        let mut chosen = stored(&app_kv).expect("settings");
        chosen.network = Network::Mainnet;
        app_kv
            .put(DetScope::Global, AppSettings::KV_KEY, &chosen)
            .expect("user changes network");

        let outcome = import_legacy_settings(&app_kv, &db).expect("second import");

        assert_eq!(outcome, SettingsImport::AlreadyDone);
        assert_eq!(
            stored(&app_kv).unwrap().network,
            Network::Mainnet,
            "a re-run must not resurrect the legacy network over the user's choice",
        );
    }

    /// A fresh install has no legacy row. The import writes the sentinel so
    /// the probe does not repeat, and leaves the settings blob alone.
    #[test]
    fn fresh_install_records_the_sentinel_without_writing_settings() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::new(dir.path().join("data.db")).expect("open db");
        db.initialize(&dir.path().join("data.db"))
            .expect("initialize fresh database");
        let app_kv = kv();

        let outcome = import_legacy_settings(&app_kv, &db).expect("import");

        assert_eq!(outcome, SettingsImport::NoLegacyData);
        assert!(
            stored(&app_kv).is_none(),
            "no legacy data must not fabricate a settings blob",
        );
        assert_eq!(
            import_legacy_settings(&app_kv, &db).expect("second import"),
            SettingsImport::AlreadyDone,
            "the sentinel must stop the probe from repeating",
        );
    }
}
