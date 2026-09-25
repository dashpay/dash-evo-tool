//! Preserve databases created by the pinned Platform PR when opening the development release.

mod engine;
pub use engine::UpgradeError;
pub(crate) use engine::remove_backups;

use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig, WalletStorageError};

use crate::backend_task::error::TaskError;

pub(crate) fn open(config: SqlitePersisterConfig) -> Result<SqlitePersister, TaskError> {
    open_with_retention(config, engine::retain_one_backup_locked)
}

fn open_with_retention(
    config: SqlitePersisterConfig,
    mut retain: impl FnMut(&engine::BackupGuard, Option<&std::path::Path>) -> std::io::Result<()>,
) -> Result<SqlitePersister, TaskError> {
    let guard = engine::backup_lock(&config.path).map_err(lock_error)?;
    let auto_dir = config.auto_backup_dir.as_deref();
    // Check retention before another attempt can create a snapshot, including after a restart.
    // Retention stays strict only when a snapshot could follow: a database that is already
    // current still opens, since failed housekeeping does not endanger it.
    if let Err(retention) = retain(&guard, auto_dir) {
        return open_current_only(&config).map_err(|probe| {
            tracing::debug!(
                error = ?probe,
                "Wallet database is not openable without a snapshot; backup retention failure stands"
            );
            lock_error(retention)
        });
    }
    let result = open_inner(&config, &guard);
    // Post-open retention is best-effort housekeeping: it must neither discard a successful
    // open nor mask the open's own error. The next open retries it before any new snapshot.
    if let Err(error) = retain(&guard, auto_dir) {
        tracing::warn!(
            ?error,
            "Upgrade backup retention failed after opening the wallet database"
        );
    }
    result
}

/// Open only when no snapshot can be taken: with automatic backups disabled, upstream
/// refuses any pending migration before touching the database, and a brand-new database
/// has nothing to back up. The pinned-profile upgrade is never attempted here.
///
/// On success the probe is dropped and the database reopened with the caller's config —
/// the database is current, so that open creates no snapshot, while the persister keeps
/// its backup directory for later destructive operations (e.g. wallet deletion).
/// The caller holds the lifecycle guard, so no other DET open can migrate in between.
fn open_current_only(config: &SqlitePersisterConfig) -> Result<SqlitePersister, TaskError> {
    let probe = SqlitePersister::open(config.clone().with_auto_backup_dir(None))
        .map_err(TaskError::from_wallet_storage_open_error)?;
    drop(probe);
    let persister =
        SqlitePersister::open(config.clone()).map_err(TaskError::from_wallet_storage_open_error)?;
    tracing::warn!(
        "Upgrade backup retention failed; opened the current wallet database and will retry retention on the next open"
    );
    Ok(persister)
}

fn open_inner(
    config: &SqlitePersisterConfig,
    guard: &engine::BackupGuard,
) -> Result<SqlitePersister, TaskError> {
    let original = match SqlitePersister::open(config.clone()) {
        Ok(persister) => return Ok(persister),
        Err(error @ WalletStorageError::Migration(_)) => error,
        Err(error) => return Err(TaskError::from_wallet_storage_open_error(error)),
    };
    let result =
        upgrade(config, guard).map_err(|source| TaskError::PlatformDatabaseUpgrade { source })?;
    if !result {
        return Err(TaskError::from_wallet_storage_open_error(original));
    }
    SqlitePersister::open(config.clone()).map_err(TaskError::from_wallet_storage_open_error)
}

fn upgrade(
    config: &SqlitePersisterConfig,
    guard: &engine::BackupGuard,
) -> Result<bool, UpgradeError> {
    let parent = config.path.parent().ok_or(UpgradeError::Unrecognized)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix("platform-upgrade-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let stage_dir = builder.tempdir_in(parent)?;
    let stage_path = stage_dir.path().join("wallet.sqlite");
    drop(SqlitePersister::open(SqlitePersisterConfig::new(&stage_path)).map_err(validation_error)?);
    let backup = engine::upgrade_locked(guard, &stage_path, |path| {
        let staged =
            SqlitePersister::open(SqlitePersisterConfig::new(path)).map_err(validation_error)?;
        staged.load().map_err(validation_error)?;
        staged.load_unowned_identities().map_err(validation_error)?;
        Ok(())
    })?;
    if let Some(backup) = backup {
        tracing::info!(backup = %backup.display(), "Upgraded pinned Platform database and retained original backup");
        Ok(true)
    } else {
        Ok(false)
    }
}

fn lock_error(source: std::io::Error) -> TaskError {
    if source.kind() == std::io::ErrorKind::WouldBlock {
        TaskError::WalletStorageNotReady
    } else {
        TaskError::FileSystem { source }
    }
}

fn validation_error(error: impl std::error::Error + Send + Sync + 'static) -> UpgradeError {
    use platform_wallet::changeset::PersistenceError;
    use rusqlite::ErrorCode;
    use std::io::ErrorKind;

    type WrapError = fn(Box<dyn std::error::Error + Send + Sync>) -> UpgradeError;

    let mut cause: &dyn std::error::Error = &error;
    let mut transient = false;
    loop {
        let category: Option<WrapError> =
            if let Some(sqlite) = cause.downcast_ref::<rusqlite::Error>() {
                match sqlite.sqlite_error_code() {
                    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
                        Some(UpgradeError::InUse)
                    }
                    Some(ErrorCode::DiskFull) => Some(UpgradeError::StorageFull),
                    Some(ErrorCode::SystemIoFailure) => Some(UpgradeError::StorageUnavailable),
                    Some(ErrorCode::OutOfMemory) => Some(UpgradeError::OutOfMemory),
                    Some(ErrorCode::PermissionDenied | ErrorCode::ReadOnly) => {
                        Some(UpgradeError::AccessDenied)
                    }
                    _ => None,
                }
            } else if let Some(io) = cause.downcast_ref::<std::io::Error>() {
                match io.kind() {
                    ErrorKind::StorageFull => Some(UpgradeError::StorageFull),
                    ErrorKind::OutOfMemory => Some(UpgradeError::OutOfMemory),
                    ErrorKind::Interrupted
                    | ErrorKind::WouldBlock
                    | ErrorKind::TimedOut
                    | ErrorKind::ResourceBusy => Some(UpgradeError::StorageUnavailable),
                    ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
                        Some(UpgradeError::AccessDenied)
                    }
                    _ => None,
                }
            } else {
                None
            };
        if let Some(wrap) = category {
            return wrap(Box::new(error));
        }
        if let Some(persistence) = cause.downcast_ref::<PersistenceError>() {
            transient |= persistence.is_transient();
        }
        let Some(source) = cause.source() else { break };
        cause = source;
    }
    if transient {
        UpgradeError::StorageUnavailable(Box::new(error))
    } else {
        UpgradeError::TypedValidation(Box::new(error))
    }
}

#[cfg(test)]
mod real_fixture_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

    #[test]
    fn platform_compatibility_bounds_upstream_backups_after_failed_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wallet.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(include_str!("fixtures/67d4ef3.sql"))
            .unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE wallets ADD COLUMN unsupported INTEGER;")
            .unwrap();
        let auto = platform_wallet_storage::default_auto_backup_dir(&path);
        std::fs::create_dir_all(&auto).unwrap();
        for attempt in 1..=3 {
            let old = auto.join(format!(
                "pre-migration-wallet-1-to-2-20260915T12000{attempt}Z.db"
            ));
            std::fs::write(&old, b"snapshot from an earlier attempt").unwrap();
            assert!(open(SqlitePersisterConfig::new(&path)).is_err());
            assert_eq!(std::fs::read_dir(&auto).unwrap().count(), 1);
            let retained = std::fs::read_dir(&auto)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            if retained != old {
                std::fs::rename(retained, old).unwrap();
            }
        }
    }

    #[test]
    fn platform_compatibility_post_open_retention_failure_keeps_open_result() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        let mut calls = 0;
        let persister = open_with_retention(SqlitePersisterConfig::new(&path), |_, _| {
            calls += 1;
            if calls == 1 {
                Ok(())
            } else {
                Err(std::io::Error::other("stray entry rejected by backup scan"))
            }
        })
        .expect("a post-open retention failure must not discard a successful open");
        assert_eq!(calls, 2);
        persister.load().unwrap();

        let bad = dir.path().join("corrupt.sqlite");
        std::fs::write(&bad, b"not a sqlite database, just candy wrappers").unwrap();
        let mut calls = 0;
        let error = match open_with_retention(SqlitePersisterConfig::new(&bad), |_, _| {
            calls += 1;
            if calls == 1 {
                Ok(())
            } else {
                Err(std::io::Error::other("retention failure"))
            }
        }) {
            Ok(_) => panic!("open unexpectedly succeeded on a corrupt database"),
            Err(error) => error,
        };
        assert_eq!(calls, 2);
        assert!(
            !matches!(&error, TaskError::FileSystem { source } if source.to_string() == "retention failure"),
            "the open error must not be masked by retention: {error:?}"
        );
    }

    fn failing_retention(
        calls: &mut usize,
    ) -> impl FnMut(&engine::BackupGuard, Option<&std::path::Path>) -> std::io::Result<()> + '_
    {
        move |_, _| {
            *calls += 1;
            Err(std::io::Error::other("stray entry rejected by backup scan"))
        }
    }

    #[test]
    fn platform_compatibility_pre_open_retention_failure_still_opens_current_database() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        let auto = platform_wallet_storage::default_auto_backup_dir(&path);
        // First a brand-new database, then the same database once it is current.
        for _ in 0..2 {
            let mut calls = 0;
            let persister = open_with_retention(
                SqlitePersisterConfig::new(&path),
                failing_retention(&mut calls),
            )
            .expect("failed housekeeping must not block a database that needs no snapshot");
            persister.load().unwrap();
            drop(persister);
            assert_eq!(calls, 1);
            assert!(
                !auto.exists() || std::fs::read_dir(&auto).unwrap().next().is_none(),
                "no snapshot may be taken while retention is failing"
            );
        }
    }

    #[test]
    fn platform_compatibility_pre_open_retention_failure_blocks_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(include_str!("fixtures/67d4ef3.sql"))
            .unwrap();
        let schema = || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
                .unwrap()
                .query_map([], |r| {
                    Ok(format!(
                        "{}|{}|{:?}",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let before = schema();
        let mut calls = 0;
        let error = match open_with_retention(
            SqlitePersisterConfig::new(&path),
            failing_retention(&mut calls),
        ) {
            Ok(_) => panic!("an upgrade must not run while backup retention is failing"),
            Err(error) => error,
        };
        assert_eq!(calls, 1);
        assert!(
            matches!(&error, TaskError::FileSystem { source } if source.to_string() == "stray entry rejected by backup scan"),
            "the strict retention failure must be reported: {error:?}"
        );
        assert_eq!(
            schema(),
            before,
            "the original database must stay untouched"
        );
        let auto = platform_wallet_storage::default_auto_backup_dir(&path);
        assert!(!auto.exists() || std::fs::read_dir(&auto).unwrap().next().is_none());
        assert!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .all(|name| !name.contains(".platform-67d4ef3-backup-")),
            "no bridge snapshot may be published while retention is failing"
        );
    }

    #[test]
    fn platform_compatibility_open_reports_lifecycle_lock_as_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wallet.sqlite");
        rusqlite::Connection::open(&path).unwrap();
        let _guard = engine::backup_lock(&path).unwrap();
        let error = match open(SqlitePersisterConfig::new(&path)) {
            Ok(_) => panic!("open unexpectedly acquired the held lifecycle lock"),
            Err(error) => error,
        };
        assert!(matches!(error, TaskError::WalletStorageNotReady));
        assert!(error.to_string().contains("try again"));
    }

    #[test]
    fn platform_compatibility_normal_open_upgrades_old_profile() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(include_str!("fixtures/67d4ef3.sql"))
            .unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(include_str!("fixtures/public-rows.sql"))
            .unwrap();
        let original_error = match SqlitePersister::open(
            SqlitePersisterConfig::new(&path)
                .with_auto_backup_dir(Some(dir.path().join("precondition-backup"))),
        ) {
            Ok(_) => panic!("The old pinned profile unexpectedly opened without compatibility."),
            Err(error) => error,
        };
        assert!(
            matches!(original_error, WalletStorageError::Migration(_)),
            "The old profile must reach migration validation, got {original_error:?}."
        );
        let persister = open(SqlitePersisterConfig::new(&path)).unwrap();
        let loaded = persister.load().unwrap();
        assert_eq!(loaded.wallets.len(), 1);
        let wallet = loaded.wallets.get(&[0xA1; 32]).unwrap();
        assert_eq!(wallet.wallet_info.balance.total(), 150_000);
        assert_eq!(
            wallet.identity_manager.wallet_identities[&[0xA1; 32]][&0]
                .identity
                .balance(),
            42
        );
        persister.load_unowned_identities().unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM identities", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM core_utxos", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(persister);
        open(SqlitePersisterConfig::new(&path)).unwrap();
        let upstream = platform_wallet_storage::default_auto_backup_dir(&path);
        assert_eq!(std::fs::read_dir(upstream).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use platform_wallet::changeset::PersistenceError;

    #[test]
    fn platform_compatibility_staged_resource_failures_remain_retryable() {
        for code in [
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_LOCKED,
            rusqlite::ffi::SQLITE_FULL,
            rusqlite::ffi::SQLITE_IOERR,
            rusqlite::ffi::SQLITE_NOMEM,
        ] {
            for persistence_wrapper in [false, true] {
                let storage = WalletStorageError::Sqlite(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(code),
                    None,
                ));
                let source = if persistence_wrapper {
                    validation_error(PersistenceError::from(storage))
                } else {
                    validation_error(storage)
                };
                assert!(source.is_retryable(), "{source:?}");
                let cause = std::error::Error::source(&source).unwrap();
                if persistence_wrapper {
                    assert!(cause.is::<PersistenceError>());
                } else {
                    assert!(cause.is::<WalletStorageError>());
                }
                assert!(!source.to_string().contains("previous application version"));
                assert!(!crate::backend_task::is_terminal_storage_open_error(
                    &TaskError::PlatformDatabaseUpgrade { source }
                ));
            }
        }
        let source = validation_error(WalletStorageError::Io(std::io::Error::from(
            std::io::ErrorKind::StorageFull,
        )));
        assert!(matches!(source, UpgradeError::StorageFull(_)));
    }

    #[test]
    fn platform_compatibility_staged_migration_disk_full_remains_retryable() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "max_page_count", 1).unwrap();
        let migration = refinery::Migration::unapplied(
            "V1__resource_fixture",
            "CREATE TABLE fixture (id INTEGER);",
        )
        .unwrap();
        let error = refinery::Runner::new(&[migration])
            .run(&mut connection)
            .unwrap_err();
        let source = validation_error(WalletStorageError::Migration(error));
        assert!(matches!(source, UpgradeError::StorageFull(_)), "{source:?}");
        assert!(!crate::backend_task::is_terminal_storage_open_error(
            &TaskError::PlatformDatabaseUpgrade { source }
        ));
    }

    #[test]
    fn platform_compatibility_staged_invalid_data_remains_terminal() {
        for storage in [
            WalletStorageError::SchemaHistoryMissing,
            WalletStorageError::BlobDecode {
                reason: "invalid fixture",
            },
            WalletStorageError::IntegrityCheckFailed {
                report: "invalid fixture".into(),
            },
        ] {
            let source = validation_error(PersistenceError::from(storage));
            assert!(matches!(source, UpgradeError::TypedValidation(_)));
            assert!(crate::backend_task::is_terminal_storage_open_error(
                &TaskError::PlatformDatabaseUpgrade { source }
            ));
        }
    }
}
