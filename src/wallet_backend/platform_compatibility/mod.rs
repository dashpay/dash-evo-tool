//! Preserve databases created by the pinned Platform PR when opening the development release.

mod engine;
mod storage_failure;
pub use engine::UpgradeError;
#[cfg(test)]
pub(crate) use engine::{LOCK_SUFFIX, backup_lock_path, bridge_backup_path};
pub(crate) use engine::{
    is_bridge_backup_name, prune_expired_backups, remove_backups, upstream_backup_timestamp,
};
pub(crate) use storage_failure::StorageFailure;

use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig, WalletStorageError};

use crate::backend_task::error::TaskError;

pub(crate) fn open(config: SqlitePersisterConfig) -> Result<SqlitePersister, TaskError> {
    open_with_housekeeping(config, engine::tidy_backups_locked)
}

fn open_with_housekeeping(
    config: SqlitePersisterConfig,
    mut tidy: impl FnMut(&engine::BackupGuard, Option<&std::path::Path>) -> std::io::Result<()>,
) -> Result<SqlitePersister, TaskError> {
    let guard = engine::backup_lock(&config.path).map_err(lock_error)?;
    let auto_dir = config.auto_backup_dir.as_deref();
    // Best-effort both times: tidy before a snapshot can be taken, and again to drop a
    // copy a failed open just duplicated. A failure never decides the open's result.
    let mut tidy_best_effort = || {
        if let Err(error) = tidy(&guard, auto_dir) {
            tracing::warn!(
                ?error,
                "Upgrade backup housekeeping failed; retrying on the next open"
            );
        }
    };
    tidy_best_effort();
    let result = open_inner(&config, &guard);
    tidy_best_effort();
    result
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
    engine::initialize_stage(&stage_path)?;
    let reference_path = stage_dir.path().join("current-reference.sqlite");
    drop(
        SqlitePersister::open(SqlitePersisterConfig::new(&reference_path))
            .map_err(validation_error)?,
    );
    let backup = engine::upgrade_locked(guard, &stage_path, |path| {
        let staged =
            SqlitePersister::open(SqlitePersisterConfig::new(path)).map_err(validation_error)?;
        staged.load().map_err(validation_error)?;
        staged.load_unowned_identities().map_err(validation_error)?;
        drop(staged);
        engine::verify_current_schema(path, &reference_path)
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
    } else if StorageFailure::of_io(&source) == Some(StorageFailure::AccessDenied) {
        TaskError::WalletStorageLockAccessDenied { source }
    } else {
        TaskError::FileSystem { source }
    }
}

fn validation_error(error: impl std::error::Error + Send + Sync + 'static) -> UpgradeError {
    use platform_wallet::changeset::PersistenceError;

    if let Some(failure) = StorageFailure::in_chain(&error) {
        return UpgradeError::from_failure(failure, Box::new(error));
    }
    let mut transient = false;
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = cause {
        transient |= current
            .downcast_ref::<PersistenceError>()
            .is_some_and(PersistenceError::is_transient);
        cause = current.source();
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

    /// A storage lock the app may not open gets the data-folder permission
    /// advice; a held lock stays "not ready".
    #[test]
    fn lock_error_gives_access_failures_the_data_folder_advice() {
        use std::io::{Error, ErrorKind};
        for kind in [ErrorKind::PermissionDenied, ErrorKind::ReadOnlyFilesystem] {
            let error = lock_error(Error::from(kind));
            assert!(
                matches!(error, TaskError::WalletStorageLockAccessDenied { .. }),
                "{error:?}"
            );
        }
        let held = lock_error(Error::from(ErrorKind::WouldBlock));
        assert!(matches!(held, TaskError::WalletStorageNotReady), "{held:?}");
        let other = lock_error(Error::from(ErrorKind::NotFound));
        assert!(matches!(other, TaskError::FileSystem { .. }), "{other:?}");
    }

    #[cfg(unix)]
    #[test]
    fn storage_open_reports_denied_lock_and_real_contention() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let database = dir.path().join("wallet.sqlite");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500))
            .expect("deny writes");
        let denied = open(SqlitePersisterConfig::new(&database)).err();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restore");
        let denied = denied.expect("cannot create lock");
        assert!(
            matches!(&denied, TaskError::WalletStorageLockAccessDenied { source }
            if source.kind() == std::io::ErrorKind::PermissionDenied),
            "{denied:?}"
        );
        assert!(
            denied.to_string().contains("permission to change"),
            "{denied}"
        );
        let _guard = engine::backup_lock(&database).expect("hold lock");
        let held = open(SqlitePersisterConfig::new(&database))
            .err()
            .expect("held lock");
        assert!(matches!(held, TaskError::WalletStorageNotReady), "{held:?}");
        assert_eq!(
            held.to_string(),
            "The storage update is still running. Please wait a moment and try again."
        );
    }

    /// Every failed open of an unsupported database makes upstream write another
    /// `pre-migration-*` copy of the unchanged database; the open's own tidying
    /// collapses them to the newest, even when the open fails.
    ///
    /// Upstream refuses a group- or world-writable data or backup directory before
    /// taking any snapshot, so the data directory is made owner-only and the backup
    /// directory is left for upstream to create; neither may depend on the umask.
    #[test]
    fn platform_compatibility_bounds_upstream_backups_after_failed_opens() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
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
        let mut previous: Option<String> = None;
        for attempt in 1..=3 {
            assert!(open(SqlitePersisterConfig::new(&path)).is_err());
            let retained = dir_entries(&auto);
            assert_eq!(
                retained.len(),
                1,
                "attempt {attempt}: one snapshot must remain, got {retained:?}"
            );
            let snapshot = retained[0].clone();
            assert!(
                snapshot.starts_with("pre-migration-wallet-") && snapshot.ends_with(".db"),
                "attempt {attempt}: unexpected entry {snapshot}"
            );
            assert!(
                crate::utils::backup_prune::usable_snapshot(&auto.join(&snapshot)),
                "attempt {attempt}: a real upstream snapshot must count as usable"
            );
            if let Some(previous) = &previous {
                assert_ne!(
                    &snapshot, previous,
                    "attempt {attempt}: the newest copy must be the one kept"
                );
            }
            // Age the kept copy's name so the next open's copy gets a distinct,
            // newer name even within the same second.
            let (head, _) = snapshot.rsplit_once('-').unwrap();
            let aged = format!("{head}-2020010{attempt}T000000Z.db");
            std::fs::rename(auto.join(&snapshot), auto.join(&aged)).unwrap();
            previous = Some(aged);
        }
    }

    /// Opening never deletes a published snapshot that differs from the others: only
    /// the retention setting removes those, so "Keep forever" keeps every one.
    #[test]
    fn platform_compatibility_repeated_opens_keep_every_distinct_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        drop(open(SqlitePersisterConfig::new(&path)).unwrap());
        let auto = platform_wallet_storage::default_auto_backup_dir(&path);
        std::fs::create_dir_all(&auto).unwrap();
        let snapshots = [
            dir.path()
                .join("wallet.sqlite.platform-67d4ef3-backup-first.sqlite"),
            auto.join("pre-migration-wallet-1-to-2-20200101T000000Z.db"),
            auto.join("pre-migration-wallet-2-to-3-20210101T000000Z.db"),
            auto.join("pre-migration-wallet-2-to-3-20220101T000000Z.db"),
        ];
        for (index, snapshot) in snapshots.iter().enumerate() {
            std::fs::write(snapshot, format!("snapshot {index}")).unwrap();
        }
        for _ in 0..3 {
            drop(open(SqlitePersisterConfig::new(&path)).unwrap());
        }
        for snapshot in &snapshots {
            assert!(snapshot.exists(), "{} must be kept", snapshot.display());
        }
    }

    /// Housekeeping is best-effort before and after the open: its failure neither
    /// blocks nor discards a successful open, nor masks the open's own error.
    #[test]
    fn platform_compatibility_housekeeping_failure_never_decides_the_open() {
        fn failing(
            calls: &mut usize,
        ) -> impl FnMut(&engine::BackupGuard, Option<&std::path::Path>) -> std::io::Result<()> + '_
        {
            move |_, _| {
                *calls += 1;
                Err(std::io::Error::other("housekeeping failure"))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let mut calls = 0;
        let persister = open_with_housekeeping(
            SqlitePersisterConfig::new(dir.path().join("wallet.sqlite")),
            failing(&mut calls),
        )
        .expect("failed housekeeping must not block or discard an open");
        assert_eq!(calls, 2);
        persister.load().unwrap();

        let bad = dir.path().join("corrupt.sqlite");
        std::fs::write(&bad, b"not a sqlite database, just candy wrappers").unwrap();
        let mut calls = 0;
        let error =
            match open_with_housekeeping(SqlitePersisterConfig::new(&bad), failing(&mut calls)) {
                Ok(_) => panic!("open unexpectedly succeeded on a corrupt database"),
                Err(error) => error,
            };
        assert_eq!(calls, 2);
        assert!(
            !matches!(&error, TaskError::FileSystem { source } if source.to_string() == "housekeeping failure"),
            "the open error must not be masked by housekeeping: {error:?}"
        );
    }

    fn dir_entries(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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

    // The historical public fixture uses a placeholder txid; V022 validates body hashes.
    fn populate_valid_public_rows(conn: &rusqlite::Connection) {
        use dash_sdk::dpp::dashcore::{OutPoint, ScriptBuf, TxOut};
        use dash_sdk::dpp::key_wallet::managed_account::transaction_record::TransactionRecord;

        conn.execute_batch(include_str!("fixtures/public-rows.sql"))
            .unwrap();
        let (blob, script): (Vec<u8>, Vec<u8>) = conn
            .query_row(
                "SELECT record_blob, script FROM core_transactions CROSS JOIN core_utxos",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let (mut record, _): (TransactionRecord, _) =
            bincode::serde::decode_from_slice(&blob, bincode::config::standard()).unwrap();
        record.transaction.output = vec![TxOut {
            value: 150_000,
            script_pubkey: ScriptBuf::from_bytes(script),
        }];
        record.txid = record.transaction.txid();
        let blob = bincode::serde::encode_to_vec(&record, bincode::config::standard()).unwrap();
        let outpoint = bincode::serde::encode_to_vec(
            OutPoint {
                txid: record.txid,
                vout: 0,
            },
            bincode::config::standard(),
        )
        .unwrap();
        conn.execute(
            "UPDATE core_transactions SET txid = ?1, record_blob = ?2",
            rusqlite::params![record.txid.as_byte_array().as_slice(), blob],
        )
        .unwrap();
        conn.execute("UPDATE core_utxos SET outpoint = ?1", [outpoint])
            .unwrap();
    }

    #[test]
    fn platform_compatibility_refuses_invalid_history_without_rewriting_source() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("fixtures/67d4ef3.sql"))
            .unwrap();
        conn.execute_batch(include_str!("fixtures/public-rows.sql"))
            .unwrap();
        let original: Vec<u8> = conn
            .query_row("SELECT record_blob FROM core_transactions", [], |r| {
                r.get(0)
            })
            .unwrap();
        let version: i64 = conn
            .query_row(
                "SELECT max(version) FROM refinery_schema_history",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(conn);
        let error = match open(SqlitePersisterConfig::new(&path)) {
            Ok(_) => panic!("an invalid historical transaction must be refused"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error,
                TaskError::PlatformDatabaseUpgrade {
                    source: UpgradeError::TypedValidation(_)
                }
            ),
            "{error:?}"
        );
        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("SELECT record_blob FROM core_transactions", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            original
        );
        assert_eq!(
            conn.query_row(
                "SELECT max(version) FROM refinery_schema_history",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            version
        );
    }

    #[test]
    fn platform_compatibility_runs_accounting_migration_on_copied_history() {
        use dash_sdk::dpp::dashcore::{Address, Network, ScriptBuf, Transaction, TxIn, TxOut};
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
            OutputDetail, OutputRole, TransactionDirection, TransactionRecord,
        };
        use dash_sdk::dpp::key_wallet::transaction_checking::{
            TransactionContext, TransactionType,
        };

        for start_at_v020 in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
            let path = dir.path().join("wallet.sqlite");
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("fixtures/67d4ef3.sql"))
                .unwrap();
            populate_valid_public_rows(&conn);
            let script: Vec<u8> = conn
                .query_row("SELECT script FROM core_utxos", [], |r| r.get(0))
                .unwrap();
            let address =
                Address::from_script(&ScriptBuf::from_bytes(script), Network::Testnet).unwrap();
            let record = TransactionRecord::new(
                Transaction {
                    version: 1,
                    lock_time: 0,
                    input: vec![TxIn::default()],
                    output: vec![TxOut {
                        value: 10,
                        script_pubkey: address.script_pubkey(),
                    }],
                    special_transaction_payload: None,
                },
                AccountType::Standard {
                    index: 0,
                    standard_account_type: StandardAccountType::BIP44Account,
                },
                TransactionContext::Mempool,
                TransactionType::Standard,
                TransactionDirection::Incoming,
                vec![],
                vec![OutputDetail {
                    index: 0,
                    role: OutputRole::Received,
                    address: Some(address),
                    value: 10,
                }],
                999,
            );
            let txid = record.txid.as_byte_array().as_slice();
            let original =
                bincode::serde::encode_to_vec(&record, bincode::config::standard()).unwrap();
            conn.execute(
                "INSERT INTO core_transactions (wallet_id, txid, finalized, record_blob) VALUES (?1, ?2, 0, ?3)",
                rusqlite::params![&[0xA1u8; 32][..], txid, &original],
            ).unwrap();
            drop(conn);
            if start_at_v020 {
                let stage = dir.path().join("v020.sqlite");
                engine::initialize_stage(&stage).unwrap();
                assert!(
                    engine::upgrade(&path, &stage, |_| Ok(()))
                        .unwrap()
                        .is_some()
                );
            }

            drop(open(SqlitePersisterConfig::new(&path)).unwrap());
            let conn = rusqlite::Connection::open(&path).unwrap();
            let preserved: Vec<u8> = conn
                .query_row(
                    "SELECT record_blob FROM core_transaction_record_originals WHERE txid = ?1",
                    [txid],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(preserved, original, "retain the pre-repair record");
            let repaired: Vec<u8> = conn
                .query_row(
                    "SELECT record_blob FROM core_transactions WHERE txid = ?1",
                    [txid],
                    |r| r.get(0),
                )
                .unwrap();
            let (repaired_record, _): (TransactionRecord, _) =
                bincode::serde::decode_from_slice(&repaired, bincode::config::standard()).unwrap();
            assert_eq!(
                repaired_record.net_amount, 10,
                "wallet history uses the owned output amount"
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM core_transaction_inputs WHERE txid = ?1",
                    [txid],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap(),
                1,
                "index the historical input"
            );
            assert_eq!(
                conn.query_row(
                    "SELECT max(version) FROM refinery_schema_history",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap(),
                23
            );
            assert_eq!(
                conn.query_row("SELECT synced_height FROM core_sync_state", [], |r| r
                    .get::<_, i64>(0),)
                    .unwrap(),
                200,
                "accounting repair must not reset chain sync"
            );
            drop(conn);
            drop(open(SqlitePersisterConfig::new(&path)).unwrap());
            assert_eq!(
                rusqlite::Connection::open(&path)
                    .unwrap()
                    .query_row(
                        "SELECT record_blob FROM core_transactions WHERE txid = ?1",
                        [txid],
                        |r| r.get::<_, Vec<u8>>(0),
                    )
                    .unwrap(),
                repaired,
                "reopening must preserve the repaired record"
            );
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
        populate_valid_public_rows(&rusqlite::Connection::open(&path).unwrap());
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
        let upstream = platform_wallet_storage::default_auto_backup_dir(&path);
        let upstream_snapshots = || std::fs::read_dir(&upstream).unwrap().count();
        let before_reopen = upstream_snapshots();
        drop(open(SqlitePersisterConfig::new(&path)).unwrap());
        // Both recovery points stay: the bridge copy of the pinned profile and any
        // upstream copy taken before its own migration. A current database adds none.
        assert_eq!(upstream_snapshots(), before_reopen);
        assert!(
            std::fs::read_dir(dir.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".platform-67d4ef3-backup-")),
            "the bridge snapshot is kept"
        );
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use platform_wallet::changeset::PersistenceError;

    /// Staged-validation failures keep their storage class through any wrapper:
    /// resource failures stay retryable, invalid data stays terminal.
    #[test]
    fn platform_compatibility_staged_validation_errors_keep_their_class() {
        let full = || {
            WalletStorageError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                None,
            ))
        };
        for source in [
            validation_error(full()),
            validation_error(PersistenceError::from(full())),
        ] {
            assert!(matches!(source, UpgradeError::StorageFull(_)), "{source:?}");
            assert!(std::error::Error::source(&source).is_some());
            assert!(!crate::backend_task::is_terminal_storage_open_error(
                &TaskError::PlatformDatabaseUpgrade { source }
            ));
        }
        let source = validation_error(PersistenceError::from(
            WalletStorageError::SchemaHistoryMissing,
        ));
        assert!(matches!(source, UpgradeError::TypedValidation(_)));
        assert!(crate::backend_task::is_terminal_storage_open_error(
            &TaskError::PlatformDatabaseUpgrade { source }
        ));
    }
}
