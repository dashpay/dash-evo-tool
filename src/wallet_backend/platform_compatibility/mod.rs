//! Open the wallet databases, refusing those only an earlier release can upgrade.

mod engine;
#[cfg(test)]
mod real_fixture_tests;
mod storage_failure;
#[cfg(test)]
pub(crate) use engine::{LOCK_SUFFIX, backup_lock_path, bridge_backup_path};
pub(crate) use engine::{
    is_bridge_backup_name, prune_expired_backups, remove_backups, upstream_backup_timestamp,
};
pub(crate) use storage_failure::StorageFailure;

use std::path::Path;

use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::backend_task::error::{TaskError, UpgradeReleaseNetworks};

/// Refinery history (version, name, checksum) of a database written at the
/// pinned Platform PR revision 67d4ef3 (DET v1.0.0-weekly.20260901 and
/// v1.0.0-weekly.20260908). Its numbering diverges from the released lineage
/// at version 3, and only v1.0.0-weekly.20261117 can still convert it.
const PINNED_PR_HISTORY: [(i64, &str, &str); 14] = [
    (1, "initial", "17047616343160871465"),
    (2, "address_height_pin", "9304453063389616695"),
    (3, "unified", "14737834287968956387"),
    (4, "invitations", "10484261583046002776"),
    (5, "pool_public_key", "1184376266715578615"),
    (6, "pool_reserved_at", "11787317857065590354"),
    (7, "drop_core_utxo_metadata", "8136185229636655136"),
    (8, "shielded_viewing_keys", "1296229231248300117"),
    (
        9,
        "single_source_core_confirmation_height",
        "18418537157120884496",
    ),
    (10, "asset_lock_recovered_status", "2416860902556468199"),
    (11, "dpns_name_states", "6272036451822234550"),
    (
        12,
        "purge_legacy_empty_script_spent_utxos",
        "5078144809172324082",
    ),
    (13, "tracked_masternodes", "989798159842040942"),
    (
        14,
        "identity_keys_null_scope_requires_existing_identity",
        "9865513418054625997",
    ),
];

/// How long the history read waits for another process's write lock.
const HISTORY_READ_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) fn open(config: SqlitePersisterConfig) -> Result<SqlitePersister, TaskError> {
    if needs_upgrade_release(&config.path) {
        let data_dir = config.path.parent().unwrap_or_else(|| Path::new(""));
        refuse_stores_needing_upgrade_release(data_dir)?;
        return Err(TaskError::AppDataNeedsUpgradeRelease {
            database: config.path,
        });
    }
    open_with_housekeeping(config, engine::tidy_backups_locked)
}

/// Refuses when any wallet store in `data_dir` is on the pinned-PR lineage,
/// naming every such network. Must run before `det-app.sqlite` is opened:
/// opening migrates it past what v1.0.0-weekly.20261117 can read, which would
/// leave the network stores that release still has to convert unconvertible.
pub(crate) fn refuse_stores_needing_upgrade_release(data_dir: &Path) -> Result<(), TaskError> {
    let networks: Vec<_> = super::all_networks()
        .into_iter()
        .filter(|network| needs_upgrade_release(&super::wallet_database_path(data_dir, *network)))
        .collect();
    if !networks.is_empty() {
        return Err(TaskError::WalletDataNeedsUpgradeRelease {
            networks: UpgradeReleaseNetworks(networks),
        });
    }
    let app_database = super::app_database_path(data_dir);
    if needs_upgrade_release(&app_database) {
        return Err(TaskError::AppDataNeedsUpgradeRelease {
            database: app_database,
        });
    }
    Ok(())
}

/// Whether the existing database at `path` is on the pinned-PR lineage. Reads
/// the migration history through a read-only connection, before anything else
/// opens the file. A database that cannot be read is left to the normal open,
/// which reports its own error.
fn needs_upgrade_release(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    match read_history(path) {
        Ok(history) => history.is_some_and(|history| {
            history.len() == PINNED_PR_HISTORY.len()
                && history.iter().zip(PINNED_PR_HISTORY).all(
                    |((version, name, checksum), expected)| {
                        (*version, name.as_str(), checksum.as_str()) == expected
                    },
                )
        }),
        Err(error) => {
            tracing::warn!(?error, database = %path.display(), "Could not read the wallet database history before opening it; opening it normally");
            false
        }
    }
}

/// At most one row more than [`PINNED_PR_HISTORY`]; `None` without a history table.
fn read_history(path: &Path) -> rusqlite::Result<Option<Vec<(i64, String, String)>>> {
    let conn = open_for_history(path)?;
    conn.busy_timeout(HISTORY_READ_BUSY_TIMEOUT)?;
    let has_history = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'refinery_schema_history'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !has_history {
        return Ok(None);
    }
    conn.prepare(
        "SELECT version, name, checksum FROM refinery_schema_history ORDER BY version LIMIT ?1",
    )?
    .query_map([PINNED_PR_HISTORY.len() + 1], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map(Some)
}

/// A read-only connection to `path`. Without a `-wal` file no commit waits in
/// one, so the file is opened immutable, which keeps SQLite from creating
/// `-wal` and `-shm` files beside it. A writer starting in between can only
/// make the read stale, and the normal open then handles the file.
fn open_for_history(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    if Path::new(&wal).exists() {
        return Connection::open_with_flags(path, flags);
    }
    match immutable_uri(path) {
        Some(uri) => Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI),
        None => Connection::open_with_flags(path, flags),
    }
}

/// `file:` URI opening `path` (made absolute) read-only and immutable; `None`
/// for paths a URI cannot carry plainly (not UTF-8, or a Windows network or
/// verbatim path).
fn immutable_uri(path: &Path) -> Option<String> {
    let path = std::path::absolute(path).ok()?;
    let path = path.to_str()?;
    let path = if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    if path.starts_with("//") {
        return None;
    }
    let mut uri = String::from(if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Some(uri)
}

/// Create a database at `path` holding only the pinned-PR migration history.
#[cfg(any(test, feature = "testing"))]
pub fn seed_pinned_pr_history(path: &Path) -> rusqlite::Result<()> {
    seed_history(&Connection::open(path)?)
}

#[cfg(any(test, feature = "testing"))]
fn seed_history(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA application_id = 1347180372; CREATE TABLE refinery_schema_history(version int4 PRIMARY KEY, name VARCHAR(255), applied_on VARCHAR(255), checksum VARCHAR(255));",
    )?;
    for (version, name, checksum) in PINNED_PR_HISTORY {
        conn.execute(
            "INSERT INTO refinery_schema_history VALUES (?1, ?2, '2026-09-10T00:00:00Z', ?3)",
            rusqlite::params![version, name, checksum],
        )?;
    }
    Ok(())
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
    let result =
        SqlitePersister::open(config.clone()).map_err(TaskError::from_wallet_storage_open_error);
    tidy_best_effort();
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use platform_wallet::changeset::PlatformWalletPersistence;

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
    fn storage_open_reports_a_denied_lock() {
        use std::os::unix::fs::PermissionsExt;
        if !crate::test_support::file_modes_deny_access() {
            eprintln!("skipped: file modes do not deny this process access");
            return;
        }
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
    }

    /// A lock another open holds is contention, not a permission problem.
    #[cfg(unix)]
    #[test]
    fn storage_open_reports_real_contention_as_not_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = dir.path().join("wallet.sqlite");
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
        // A current database whose last migration is unrecorded: re-running it
        // fails on the column it already added, on every open.
        drop(open(SqlitePersisterConfig::new(&path)).unwrap());
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "DELETE FROM refinery_schema_history WHERE version = (SELECT max(version) FROM refinery_schema_history)",
                [],
            )
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

    /// Every file in `dir` (recursively) with its bytes.
    pub(super) fn tree(
        dir: &std::path::Path,
    ) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        let mut files = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(tree(&path));
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
        files
    }

    /// A database on the pinned-PR lineage is refused with the dedicated error
    /// before upstream sees it: no file in the folder is created or changed.
    #[test]
    fn platform_compatibility_refuses_pinned_pr_database_untouched() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("det-app.sqlite");
        seed_pinned_pr_history(&path).unwrap();
        let before = tree(dir.path());

        let error = match open(SqlitePersisterConfig::new(&path)) {
            Ok(_) => panic!("a pinned-PR database must not open"),
            Err(error) => error,
        };

        assert!(
            matches!(&error, TaskError::AppDataNeedsUpgradeRelease { database } if database == &path),
            "{error:?}"
        );
        assert!(error.to_string().contains("v1.0.0-weekly.20261117"));
        assert!(error.to_string().contains("start it once"), "{error}");
        assert!(crate::backend_task::is_terminal_storage_open_error(&error));
        assert_eq!(
            tree(dir.path()),
            before,
            "the folder must be left unchanged"
        );
    }

    /// A WAL-mode database closed cleanly is read without creating `-wal` or
    /// `-shm` files, also in a folder whose name needs escaping in a URI.
    #[test]
    fn platform_compatibility_refuses_wal_database_without_creating_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("wallet data %#");
        crate::app_dir::ensure_data_dir_exists(&folder).unwrap();
        let path = folder.join("det-app.sqlite");
        seed_pinned_pr_history(&path).unwrap();
        let mode: String = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let before = tree(&folder);
        assert_eq!(before.len(), 1, "{:?}", before.keys());

        let error = match open(SqlitePersisterConfig::new(&path)) {
            Ok(_) => panic!("a pinned-PR database must not open"),
            Err(error) => error,
        };

        assert!(
            matches!(error, TaskError::AppDataNeedsUpgradeRelease { .. }),
            "{error:?}"
        );
        assert_eq!(tree(&folder), before, "no file may be created");
    }

    /// A relative path (e.g. a relative `DASH_EVO_DATA_DIR`) is resolved against
    /// the working directory, not read as a path from the filesystem root.
    #[test]
    fn platform_compatibility_immutable_uri_resolves_relative_paths() {
        let relative = std::path::Path::new("data dir/det-app.sqlite");
        let absolute = std::env::current_dir().unwrap().join(relative);

        let uri = immutable_uri(relative).expect("a UTF-8 relative path has a URI");

        assert_ne!(uri, "file:///data%20dir/det-app.sqlite?mode=ro&immutable=1");
        assert_eq!(Some(uri), immutable_uri(&absolute));
    }

    /// A current app database beside pinned-PR network stores is refused before
    /// the app database is opened (and migrated past what the older release can
    /// read), naming every affected network, including one without a wallet.
    #[test]
    fn platform_compatibility_mixed_profile_is_refused_before_the_app_database_opens() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        drop(
            open(SqlitePersisterConfig::new(super::super::app_database_path(
                dir.path(),
            )))
            .unwrap(),
        );
        for network in [Network::Devnet, Network::Testnet] {
            seed_pinned_pr_history(&super::super::wallet_database_path(dir.path(), network))
                .unwrap();
        }
        let before = tree(dir.path());

        let error = match crate::context::AppContext::open_app_kv(dir.path()) {
            Ok(_) => panic!("a mixed profile must be refused"),
            Err(error) => error,
        };

        assert!(
            matches!(&error, TaskError::WalletDataNeedsUpgradeRelease { networks }
                if networks.0 == [Network::Testnet, Network::Devnet]),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("open each of these networks and wait until each one finishes syncing: Testnet, Devnet."),
            "{message}"
        );
        assert_eq!(tree(dir.path()), before, "no store may be changed");
    }

    /// A pinned-PR network store opened on its own names its network.
    #[test]
    fn platform_compatibility_refused_network_store_names_its_network() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = super::super::wallet_database_path(dir.path(), Network::Regtest);
        seed_pinned_pr_history(&path).unwrap();

        let error = match open(SqlitePersisterConfig::new(&path)) {
            Ok(_) => panic!("a pinned-PR database must not open"),
            Err(error) => error,
        };

        assert!(
            matches!(&error, TaskError::WalletDataNeedsUpgradeRelease { networks }
                if networks.0 == [Network::Regtest]),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("finishes syncing: Local."),
            "{error}"
        );
    }

    /// A history still only in the `-wal` file of an open writer is seen.
    #[test]
    fn platform_compatibility_reads_history_still_in_the_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("det-app.sqlite");
        let writer = rusqlite::Connection::open(&path).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
            .unwrap();
        seed_history(&writer).unwrap();
        assert!(
            std::fs::metadata(path.with_extension("sqlite-wal"))
                .unwrap()
                .len()
                > 0
        );

        assert!(needs_upgrade_release(&path));
        drop(writer);
    }

    /// Any other divergent history keeps the existing incompatible-data mapping.
    #[test]
    fn platform_compatibility_other_divergent_history_keeps_its_mapping() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        seed_pinned_pr_history(&path).unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("DELETE FROM refinery_schema_history WHERE version = 14", [])
            .unwrap();

        let error = match open(SqlitePersisterConfig::new(&path)) {
            Ok(_) => panic!("a divergent database must not open"),
            Err(error) => error,
        };

        assert!(
            matches!(error, TaskError::WalletDataIncompatible { .. }),
            "{error:?}"
        );
    }

    /// A current database and a missing one open normally.
    #[test]
    fn platform_compatibility_current_and_new_databases_open() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_dir::ensure_data_dir_exists(dir.path()).unwrap();
        let path = dir.path().join("wallet.sqlite");
        drop(open(SqlitePersisterConfig::new(&path)).unwrap());
        drop(open(SqlitePersisterConfig::new(&path)).unwrap());
    }
}
