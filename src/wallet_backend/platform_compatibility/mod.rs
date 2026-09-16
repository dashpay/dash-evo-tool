//! Preserve databases created by the pinned Platform PR when opening the development release.

mod engine;
pub use engine::UpgradeError;
pub(crate) use engine::remove_backups;

use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig, WalletStorageError};

use crate::backend_task::error::TaskError;

pub(crate) fn open(config: SqlitePersisterConfig) -> Result<SqlitePersister, TaskError> {
    let prune = || {
        engine::retain_one_backup(&config.path, config.auto_backup_dir.as_deref())
            .map_err(|source| TaskError::FileSystem { source })
    };
    // Check retention before another attempt can create a snapshot, including after a restart.
    prune()?;
    let result = open_inner(&config);
    prune()?;
    result
}

fn open_inner(config: &SqlitePersisterConfig) -> Result<SqlitePersister, TaskError> {
    let original = match SqlitePersister::open(config.clone()) {
        Ok(persister) => return Ok(persister),
        Err(error @ WalletStorageError::Migration(_)) => error,
        Err(error) => return Err(TaskError::from_wallet_storage_open_error(error)),
    };
    let result = upgrade(config).map_err(|source| TaskError::PlatformDatabaseUpgrade { source })?;
    if !result {
        return Err(TaskError::from_wallet_storage_open_error(original));
    }
    SqlitePersister::open(config.clone()).map_err(TaskError::from_wallet_storage_open_error)
}

fn upgrade(config: &SqlitePersisterConfig) -> Result<bool, UpgradeError> {
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
    drop(
        SqlitePersister::open(SqlitePersisterConfig::new(&stage_path))
            .map_err(|e| UpgradeError::TypedValidation(Box::new(e)))?,
    );
    let backup = engine::upgrade(&config.path, &stage_path, |path| {
        let staged = SqlitePersister::open(SqlitePersisterConfig::new(path))
            .map_err(|e| UpgradeError::TypedValidation(Box::new(e)))?;
        staged
            .load()
            .map_err(|e| UpgradeError::TypedValidation(Box::new(e)))?;
        staged
            .load_unowned_identities()
            .map_err(|e| UpgradeError::TypedValidation(Box::new(e)))?;
        Ok(())
    })?;
    if let Some(backup) = backup {
        tracing::info!(backup = %backup.display(), "Upgraded pinned Platform database and retained original backup");
        Ok(true)
    } else {
        Ok(false)
    }
}

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
