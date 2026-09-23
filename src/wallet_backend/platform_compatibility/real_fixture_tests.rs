//! Real-data regression test for the bridge: a profile the released
//! `v1.0.0-weekly.20260908` build (Platform pin 67d4ef3) wrote on testnet,
//! published as a migration-fixture artifact (`tests/migration-fixtures/`).
//!
//! The synthetic tests next door prove the bridge on rows written for them.
//! This one feeds it bytes a real build wrote: a funded wallet's UTXO,
//! transaction, address pool and sync state, and an uncheckpointed WAL left by
//! the app's own clean shutdown.
//!
//! Env-gated. It runs only when `MIGRATION_FIXTURES_DIR` points at the
//! downloaded fixture archives, as the migration-matrix workflow does, and
//! otherwise reports the skip and passes. With the variable set, a missing or
//! altered archive fails: silently testing nothing is the failure mode the
//! fixture exists to prevent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig, WalletStorageError};
use sha2::{Digest, Sha256};

use super::open;

/// Directory of fixture archives, shared with the migration-matrix harness.
const FIXTURES_DIR_ENV: &str = "MIGRATION_FIXTURES_DIR";
/// The manifest entry this test reads.
const FIXTURE_ID: &str = "v1.0.0-weekly.20260908-wallet-only";
/// The committed manifest: the archive name and checksum the download is held to.
const MANIFEST: &str = include_str!("../../../tests/migration-fixtures/manifest.json");
/// The fixture wallet's balance at capture: one faucet payment of 1 tDASH.
const CAPTURED_BALANCE_DUFFS: u64 = 100_000_000;
/// Per-network store holding the fixture wallet.
const NETWORK_DB: &str = "det-testnet.sqlite";
/// Cross-network app store.
const APP_DB: &str = "det-app.sqlite";
/// Version bookkeeping the upgrade rewrites by design; every other table must
/// keep its rows.
const VERSION_TABLES: [&str; 2] = ["refinery_schema_history", "meta_data_versions"];
/// Marker in the name of the backup the bridge retains next to an upgraded
/// database.
const BACKUP_MARKER: &str = ".platform-67d4ef3-backup-";

#[test]
fn platform_compatibility_upgrades_the_real_weekly_20260908_profile() {
    let Some(staged) = stage() else {
        return;
    };

    assert_eq!(
        assert_upgrade(&staged, NETWORK_DB),
        [CAPTURED_BALANCE_DUFFS],
        "{NETWORK_DB} must still load its one captured wallet, with the captured balance"
    );
    assert_upgrade(&staged, APP_DB);
}

/// The fixture unpacked into an owner-only temporary data directory.
struct Staged {
    root: tempfile::TempDir,
    data_dir: PathBuf,
}

impl Staged {
    /// A fresh directory for copies, so reading a database never checkpoints
    /// the WAL of the file under test.
    fn scratch(&self, label: &str) -> PathBuf {
        let dir = self.root.path().join("scratch").join(label);
        // The wallet store refuses a database below a group- or
        // other-writable directory, so every level is created owner-only
        // whatever the umask.
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir).expect("scratch directory");
        dir
    }
}

/// Unpacks the fixture named by [`FIXTURE_ID`], or returns `None` when
/// [`FIXTURES_DIR_ENV`] is unset.
fn stage() -> Option<Staged> {
    let Some(fixtures_dir) = std::env::var_os(FIXTURES_DIR_ENV).filter(|dir| !dir.is_empty())
    else {
        eprintln!(
            "skipped: {FIXTURES_DIR_ENV} is unset; the migration-matrix workflow points it at the downloaded fixture archives"
        );
        return None;
    };
    let manifest: serde_json::Value =
        serde_json::from_str(MANIFEST).expect("the committed manifest parses");
    let entry = manifest["fixtures"]
        .as_array()
        .expect("the manifest lists fixtures")
        .iter()
        .find(|fixture| fixture["id"] == FIXTURE_ID)
        .unwrap_or_else(|| panic!("the manifest has no `{FIXTURE_ID}` entry"));
    let artifact = &entry["artifact"];
    let archive_name = artifact["archive_filename"]
        .as_str()
        .expect("the entry names its archive");
    let expected_sha256 = artifact["sha256"]
        .as_str()
        .expect("the entry records the archive checksum");

    let archive = Path::new(&fixtures_dir).join(archive_name);
    let bytes = std::fs::read(&archive).unwrap_or_else(|error| {
        panic!(
            "{FIXTURES_DIR_ENV} is set, but {} cannot be read: {error}",
            archive.display()
        )
    });
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        expected_sha256,
        "{} does not match the manifest checksum",
        archive.display()
    );

    // Owner-only whatever the umask, like the bridge's own staging directory:
    // the wallet store refuses a database below a group-writable ancestor.
    let mut builder = tempfile::Builder::new();
    builder.prefix("platform-real-fixture-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let root = builder.tempdir().expect("temporary directory");
    let data_dir = root.path().join("data");
    crate::app_dir::ensure_data_dir_exists(&data_dir).expect("owner-only data directory");
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&data_dir)
        .status()
        .expect("tar runs");
    assert!(
        status.success(),
        "tar could not unpack {}",
        archive.display()
    );
    Some(Staged { root, data_dir })
}

/// Opens `db_name` through the bridge exactly as the app does and asserts the
/// bridge's contract on it. Returns the balance of every wallet the upgraded
/// store loads.
///
/// The contract: the captured file really is the divergent 67d4ef3 schema (it
/// does not open without the bridge); the bridge opens it and the typed loads
/// succeed; no row is lost (see [`assert_rows_kept`]); one backup of the
/// original is retained with the same rows; and a second open changes nothing.
fn assert_upgrade(staged: &Staged, db_name: &str) -> Vec<u64> {
    let path = staged.data_dir.join(db_name);
    let before = table_counts(&path, &staged.scratch(&format!("{db_name}-before")));
    assert!(
        !before.is_empty(),
        "{db_name} is missing from the fixture or has no tables"
    );
    assert_needs_bridge(&path, &staged.scratch(&format!("{db_name}-precondition")));

    let persister = open(SqlitePersisterConfig::new(&path))
        .unwrap_or_else(|error| panic!("the bridge could not open {db_name}: {error:?}"));
    let loaded = persister.load().unwrap_or_else(|error| {
        panic!("{db_name}: typed load failed after the upgrade: {error:?}")
    });
    persister.load_unowned_identities().unwrap_or_else(|error| {
        panic!("{db_name}: identity load failed after the upgrade: {error:?}")
    });
    let balances = loaded
        .wallets
        .values()
        .map(|wallet| wallet.wallet_info.balance.total())
        .collect();
    drop(persister);

    let after = table_counts(&path, &staged.scratch(&format!("{db_name}-after")));
    assert_rows_kept(db_name, &before, &after);

    let backups = backup_files(&path);
    let [backup] = &backups.iter().collect::<Vec<_>>()[..] else {
        panic!("{db_name}: expected exactly one retained backup, found {backups:?}");
    };
    assert_eq!(
        table_counts(backup, &staged.scratch(&format!("{db_name}-backup"))),
        before,
        "{db_name}: the retained backup must hold the original rows"
    );

    drop(
        open(SqlitePersisterConfig::new(&path))
            .unwrap_or_else(|error| panic!("{db_name}: a second open failed: {error:?}")),
    );
    assert_eq!(
        backup_files(&path),
        backups,
        "{db_name}: a second open must not upgrade again"
    );

    eprintln!("{db_name}: upgraded, rows kept {:?}", data_tables(&after));
    balances
}

/// The captured file must fail the plain open with a migration error: proof
/// that it carries the divergent history the bridge exists for, rather than a
/// schema the current pin already accepts. Checked on a copy, because the open
/// may checkpoint the WAL of the file under test.
fn assert_needs_bridge(path: &Path, scratch: &Path) {
    let copy = copy_database(path, scratch);
    let config = SqlitePersisterConfig::new(&copy)
        .with_auto_backup_dir(Some(scratch.join("precondition-backup")));
    match SqlitePersister::open(config) {
        Err(WalletStorageError::Migration(_)) => {}
        Ok(_) => panic!(
            "{} opened without the bridge; the fixture no longer exercises it",
            path.display()
        ),
        Err(error) => panic!(
            "{} must fail with a migration error without the bridge, got {error:?}",
            path.display()
        ),
    }
}

/// Row count of every table, read from a copy of the database and its WAL.
fn table_counts(path: &Path, scratch: &Path) -> BTreeMap<String, i64> {
    let copy = copy_database(path, scratch);
    let conn = rusqlite::Connection::open(&copy).expect("open the copy");
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()
        })
        .expect("list tables");
    tables
        .into_iter()
        .map(|table| {
            let count = conn
                .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .expect("count rows");
            (table, count)
        })
        .collect()
}

/// The upgrade loses no row: every data table keeps its row count (a table
/// the target schema drops may disappear only if it was empty), and every
/// table the target schema adds starts empty.
fn assert_rows_kept(db_name: &str, before: &BTreeMap<String, i64>, after: &BTreeMap<String, i64>) {
    for (table, rows) in data_tables(before) {
        assert_eq!(
            after.get(table).copied().unwrap_or(0),
            rows,
            "{db_name}: `{table}` must keep its {rows} rows through the upgrade"
        );
    }
    let filled_new_tables: BTreeMap<&str, i64> = data_tables(after)
        .into_iter()
        .filter(|(table, rows)| !before.contains_key(*table) && *rows != 0)
        .collect();
    assert!(
        filled_new_tables.is_empty(),
        "{db_name}: tables the upgrade created must start empty, found {filled_new_tables:?}"
    );
}

/// `counts` without the version bookkeeping tables.
fn data_tables(counts: &BTreeMap<String, i64>) -> BTreeMap<&str, i64> {
    counts
        .iter()
        .filter(|(table, _)| !VERSION_TABLES.contains(&table.as_str()))
        .map(|(table, count)| (table.as_str(), *count))
        .collect()
}

/// Copies a database with its `-wal` and `-shm` sidecars into `scratch`.
fn copy_database(path: &Path, scratch: &Path) -> PathBuf {
    let name = path.file_name().expect("database file name");
    let copy = scratch.join(name);
    std::fs::copy(path, &copy)
        .unwrap_or_else(|error| panic!("could not copy {}: {error}", path.display()));
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let sidecar = PathBuf::from(sidecar);
        if sidecar.exists() {
            let mut target = copy.as_os_str().to_owned();
            target.push(suffix);
            std::fs::copy(&sidecar, PathBuf::from(target))
                .unwrap_or_else(|error| panic!("could not copy {}: {error}", sidecar.display()));
        }
    }
    copy
}

/// Backups the bridge retained next to `path`.
fn backup_files(path: &Path) -> BTreeSet<PathBuf> {
    let prefix = format!(
        "{}{BACKUP_MARKER}",
        path.file_name().expect("file name").to_string_lossy()
    );
    std::fs::read_dir(path.parent().expect("parent directory"))
        .expect("read the data directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|candidate| {
            candidate
                .file_name()
                .map(|name| name.to_string_lossy())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".sqlite"))
        })
        .collect()
}
