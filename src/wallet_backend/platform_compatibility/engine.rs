use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use super::storage_failure::StorageFailure;

const OLD_SCHEMA: &str = include_str!("fixtures/67d4ef3.sql");
const TARGET_SCHEMA: &str = include_str!("fixtures/e3cd7cf.sql");
const HISTORY: &str = "refinery_schema_history";
const IDENTITY_INDEX_KEY: &str = "det:identity_index:v1";

/// A compatibility upgrade stopped before committing changes to the original database.
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    /// Another process holds the database (`SQLITE_BUSY` / `SQLITE_LOCKED`).
    #[error("{}", crate::backend_task::error::WALLET_DATA_IN_USE)]
    InUse(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(
        "Could not upgrade wallet data because the disk is full. Free up disk space and try again."
    )]
    StorageFull(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(
        "Could not access wallet storage. Check that the drive is connected and available, then try again."
    )]
    StorageUnavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(
        "Could not upgrade wallet data because memory is exhausted. Close other applications and try again."
    )]
    OutOfMemory(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(
        "Could not write wallet data. Allow write access to the app data folder, then restart the application."
    )]
    AccessDenied(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(
        "Could not upgrade wallet data, and your data was not changed. Restart the application to try again, or keep your data folder and reopen the previous application version."
    )]
    Sqlite(#[source] rusqlite::Error),
    #[error(
        "Could not back up wallet data. Check that the app data folder can be written to and try again."
    )]
    Io(#[source] std::io::Error),
    #[error(
        "Wallet data does not match a supported upgrade. Keep your data folder and reopen the previous application version."
    )]
    Unrecognized,
    /// The saved roster is missing, ambiguous, or does not decode.
    #[error(
        "The saved identity list could not be read. Keep your data folder and reopen the previous application version."
    )]
    IdentityRoster {
        #[source]
        source: Option<bincode::error::DecodeError>,
    },
    #[error(
        "Wallet data verification failed. Keep your data folder and reopen the previous application version."
    )]
    Verification,
    #[error(
        "The updated application could not read your wallet data. Keep your data folder and reopen the previous application version."
    )]
    TypedValidation(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl UpgradeError {
    /// Whether retrying can recover from contention or temporary resource exhaustion.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::InUse(_)
                | Self::StorageFull(_)
                | Self::StorageUnavailable(_)
                | Self::OutOfMemory(_)
        )
    }
}

impl UpgradeError {
    /// The variant for an environmental storage failure, keeping `error` as the source.
    pub(super) fn from_failure(
        failure: StorageFailure,
        error: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        match failure {
            StorageFailure::InUse => Self::InUse(error),
            StorageFailure::Full => Self::StorageFull(error),
            StorageFailure::Unavailable => Self::StorageUnavailable(error),
            StorageFailure::OutOfMemory => Self::OutOfMemory(error),
            StorageFailure::AccessDenied => Self::AccessDenied(error),
        }
    }
}

impl From<rusqlite::Error> for UpgradeError {
    fn from(error: rusqlite::Error) -> Self {
        match StorageFailure::of_sqlite(&error) {
            Some(failure) => Self::from_failure(failure, Box::new(error)),
            None => Self::Sqlite(error),
        }
    }
}

impl From<std::io::Error> for UpgradeError {
    fn from(error: std::io::Error) -> Self {
        match StorageFailure::of_io(&error) {
            Some(failure) => Self::from_failure(failure, Box::new(error)),
            None => Self::Io(error),
        }
    }
}

impl From<bincode::error::DecodeError> for UpgradeError {
    fn from(source: bincode::error::DecodeError) -> Self {
        Self::IdentityRoster {
            source: Some(source),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Object {
    kind: String,
    name: String,
    sql: String,
}

fn objects(conn: &Connection) -> Result<Vec<Object>, UpgradeError> {
    Ok(conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name",
        )?
        .query_map([], |r| {
            Ok(Object {
                kind: r.get(0)?,
                name: r.get(1)?,
                sql: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

fn history(conn: &Connection) -> Result<Vec<(i64, String, String)>, UpgradeError> {
    Ok(conn
        .prepare("SELECT version, name, checksum FROM refinery_schema_history ORDER BY version")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>, UpgradeError> {
    Ok(conn
        .prepare(&format!("PRAGMA table_info({})", quote(table)))?
        .query_map([], |r| r.get(1))?
        .collect::<Result<_, _>>()?)
}

fn verify(conn: &Connection) -> Result<(), UpgradeError> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result != "ok" || conn.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(UpgradeError::Verification);
    }
    Ok(())
}

fn old_reference() -> Result<Connection, UpgradeError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(OLD_SCHEMA)?;
    Ok(conn)
}

fn target_reference() -> Result<Connection, UpgradeError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(TARGET_SCHEMA)?;
    Ok(conn)
}

fn active_identities(conn: &Connection) -> Result<BTreeSet<Vec<u8>>, UpgradeError> {
    let value: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM meta_global WHERE key = ?1",
            [IDENTITY_INDEX_KEY],
            |r| r.get(0),
        )
        .optional()?;
    let Some(value) = value else {
        if conn.prepare("SELECT 1 FROM identities i JOIN meta_identity m ON i.identity_id = m.identity_id WHERE i.tombstoned = 1 AND m.key = 'det:identity:v1'")?.exists([])? {
            return Err(UpgradeError::IdentityRoster { source: None });
        }
        return Ok(BTreeSet::new());
    };
    let Some((&1, body)) = value.split_first() else {
        return Err(UpgradeError::IdentityRoster { source: None });
    };
    let (ids, consumed): (Vec<[u8; 32]>, usize) = bincode::serde::decode_from_slice(
        body,
        bincode::config::standard().with_limit::<16777216>(),
    )?;
    if consumed != body.len() {
        return Err(UpgradeError::IdentityRoster { source: None });
    }
    Ok(ids.into_iter().map(Vec::from).collect())
}

fn retired_identities(conn: &Connection) -> Result<Vec<Vec<u8>>, UpgradeError> {
    let active = active_identities(conn)?;
    Ok(conn
        .prepare("SELECT identity_id FROM identities WHERE tombstoned = 1")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|id| !active.contains(id))
        .collect())
}

fn retirement_column(table: &str) -> Option<&'static str> {
    match table {
        "identities"
        | "identity_keys"
        | "token_balances"
        | "dashpay_profiles"
        | "dashpay_payments_overlay" => Some("identity_id"),
        "contacts" | "ignored_senders" => Some("owner_id"),
        "pending_contact_crypto" => Some("owner_identity_id"),
        _ => None,
    }
}

fn selected_rows(table: &str) -> String {
    retirement_column(table)
        .map(|c| {
            format!(
                " WHERE {} NOT IN (SELECT id FROM temp.retired_identities)",
                quote(c)
            )
        })
        .unwrap_or_default()
}

fn backup_prefix(path: &Path) -> Option<String> {
    Some(format!(
        "{}.platform-67d4ef3-backup-",
        path.file_name()?.to_string_lossy()
    ))
}

/// Upstream snapshot names: `pre-migration-<stem>-<from>-to-<to>-<timestamp>.db`.
const UPSTREAM_PREFIX: &str = "pre-migration-";
const UPSTREAM_SUFFIX: &str = ".db";
/// UTC timestamp format in an upstream snapshot name.
const UPSTREAM_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// Whether `name` is a bridge snapshot of `database` (published `.sqlite` or
/// unpublished `.pending`), which lives next to the database.
pub(crate) fn is_bridge_backup_name(database: &Path, name: &str) -> bool {
    let Some(prefix) = backup_prefix(database) else {
        return false;
    };
    name.strip_prefix(&prefix)
        .and_then(|suffix| {
            suffix
                .strip_suffix(".sqlite")
                .or_else(|| suffix.strip_suffix(".pending"))
        })
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

/// The migration an upstream snapshot of `database` precedes, and when it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UpstreamSnapshot {
    from: u32,
    to: u32,
    taken: std::time::SystemTime,
}

fn parse_upstream_name(database: &Path, name: &str) -> Option<UpstreamSnapshot> {
    let stem = database.file_stem()?.to_str()?;
    // DET database names are already valid upstream stems; reject lossy/ambiguous names.
    if stem.is_empty()
        || stem.len() > 32
        || !stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let suffix = name
        .strip_prefix(UPSTREAM_PREFIX)?
        .strip_prefix(stem)?
        .strip_prefix('-')?
        .strip_suffix(UPSTREAM_SUFFIX)?;
    let (versions, timestamp) = suffix.rsplit_once('-')?;
    let (from, to) = versions.split_once("-to-")?;
    let taken = chrono::NaiveDateTime::parse_from_str(timestamp, UPSTREAM_TIMESTAMP_FORMAT).ok()?;
    Some(UpstreamSnapshot {
        from: from.parse().ok()?,
        to: to.parse().ok()?,
        taken: taken.and_utc().into(),
    })
}

/// When the upstream snapshot `name` of `database` was taken, or `None` when
/// `name` is not one.
pub(crate) fn upstream_backup_timestamp(
    database: &Path,
    name: &str,
) -> Option<std::time::SystemTime> {
    parse_upstream_name(database, name).map(|snapshot| snapshot.taken)
}

/// Retained upgrade backups of the database at `path`.
#[cfg(test)]
fn backups(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    backups_in(path, Some(&default_auto_dir(path)))
}

fn default_auto_dir(path: &Path) -> PathBuf {
    platform_wallet_storage::default_auto_backup_dir(path)
}

/// Backup candidates that passed validation, plus every rejected one with its reason.
///
/// Directory-level failures still abort the scan; a single rejected
/// candidate does not, so deletion can remove every valid snapshot before
/// reporting the rejection.
struct BackupScan {
    found: Vec<PathBuf>,
    rejected: Vec<(PathBuf, std::io::Error)>,
}

/// Strict scan: any rejected candidate fails the whole scan. Retention and
/// publication rely on this to never proceed past an unexpected file.
fn backups_in(path: &Path, auto_dir: Option<&Path>) -> std::io::Result<Vec<PathBuf>> {
    let scan = scan_backups_in(path, auto_dir)?;
    match scan.rejected.into_iter().next() {
        Some((_, error)) => Err(error),
        None => Ok(scan.found),
    }
}

fn scan_backups_in(path: &Path, auto_dir: Option<&Path>) -> std::io::Result<BackupScan> {
    let mut found = Vec::new();
    let mut rejected = Vec::new();
    let Some(parent) = path.parent() else {
        return Ok(BackupScan { found, rejected });
    };
    for directory in [Some(parent), auto_dir].into_iter().flatten() {
        let metadata = match std::fs::symlink_metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() {
            return Err(std::io::Error::other(
                "Backup directory is not a regular directory",
            ));
        }
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let bridge = directory == parent && is_bridge_backup_name(path, name);
            let upstream = Some(directory) == auto_dir && parse_upstream_name(path, name).is_some();
            if bridge || upstream {
                match validate_backup_file(&entry.path(), path) {
                    Ok(()) => found.push(entry.path()),
                    Err(error) => rejected.push((entry.path(), error)),
                }
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(BackupScan { found, rejected })
}

/// Scan-time classification of a candidate; the deletion itself is policed by the
/// [`delete_file`](crate::utils::file_deletion::delete_file) chokepoint.
fn validate_backup_file(backup: &Path, database: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(backup)?;
    if !metadata.is_file() || backup == database {
        return Err(std::io::Error::other(
            "Refusing to remove a non-regular backup file",
        ));
    }
    if let Ok(live_path) = database.canonicalize()
        && backup.canonicalize()? == live_path
    {
        return Err(std::io::Error::other(
            "Refusing to remove the live database",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(std::io::Error::other(
                "Refusing to remove a hard-linked backup file",
            ));
        }
    }
    Ok(())
}

fn remove_backup(backup: &Path, database: &Path) -> std::io::Result<()> {
    crate::utils::file_deletion::delete_file(
        backup,
        crate::utils::file_deletion::DeletionIntent::Backup { database },
    )
}

pub(super) struct BackupGuard {
    path: PathBuf,
    _file: Option<std::fs::File>,
}

/// How long a contended lifecycle lock is retried before reporting `WouldBlock`.
///
/// `flock` belongs to the open file description, and a child that any thread of this
/// process forks shares it until the child execs. A lock just released by its guard
/// can therefore still look held for that brief window.
const LOCK_GRACE: std::time::Duration = std::time::Duration::from_millis(500);
const LOCK_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);

fn lock_with_grace(file: &std::fs::File) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + LOCK_GRACE;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(super) fn backup_lock(path: &Path) -> std::io::Result<BackupGuard> {
    let Some(name) = path.file_name() else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
    let mut lock_name = name.to_os_string();
    lock_name.push(".platform-upgrade.lock");
    let lock_path = path.with_file_name(lock_name);
    let mut options = std::fs::File::options();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_backup_file(&lock_path, path)?;
            std::fs::File::options()
                .read(true)
                .write(true)
                .open(&lock_path)?
        }
        // A missing parent has no snapshots to clean up.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BackupGuard {
                path: path.to_owned(),
                _file: None,
            });
        }
        Err(error) => return Err(error),
    };
    validate_backup_file(&lock_path, path)?;
    lock_with_grace(&file)?;
    // Keep the pathname stable: unlinking it could let contenders lock different files.
    Ok(BackupGuard {
        path: path.to_owned(),
        _file: Some(file),
    })
}

/// [`tidy_backups_locked`] under a freshly taken lifecycle lock.
#[cfg(test)]
pub(super) fn tidy_backups(path: &Path, auto_dir: Option<&Path>) -> std::io::Result<()> {
    let guard = backup_lock(path)?;
    tidy_backups_locked(&guard, auto_dir)
}

/// Housekeeping that loses no recovery data: delete unpublished `.pending` copies left
/// by a crash, and published snapshots that are byte-identical to a newer retained
/// snapshot of the same migration (each failed open writes another copy of the
/// unchanged database). Every other snapshot is left to time-based retention.
///
/// The scan is strict, so an unexpected candidate fails the call before anything is
/// deleted.
pub(super) fn tidy_backups_locked(
    guard: &BackupGuard,
    auto_dir: Option<&Path>,
) -> std::io::Result<()> {
    let path = &guard.path;
    // Group published snapshots by the migration they precede; newest first.
    let mut groups: std::collections::BTreeMap<Option<(u32, u32)>, Vec<_>> =
        std::collections::BTreeMap::new();
    for backup in backups_in(path, auto_dir)? {
        let name = backup
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if backup
            .extension()
            .is_some_and(|extension| extension == "pending")
        {
            remove_backup(&backup, path)?;
            continue;
        }
        // Bridge snapshots all precede the same pinned-profile upgrade.
        let migration =
            parse_upstream_name(path, name).map(|snapshot| (snapshot.from, snapshot.to));
        let created = backup_created(path, &backup)?;
        groups.entry(migration).or_default().push((created, backup));
    }
    for mut group in groups.into_values() {
        group.sort_by(|a, b| b.cmp(a));
        let mut kept: Vec<PathBuf> = Vec::with_capacity(group.len());
        for (_, backup) in group {
            let mut duplicate = false;
            for newer in &kept {
                if same_contents(newer, &backup)? {
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                remove_backup(&backup, path)?;
            } else {
                kept.push(backup);
            }
        }
    }
    Ok(())
}

/// Whether two files hold the same bytes. Sizes are compared first, then the
/// contents are streamed in fixed-size chunks, stopping at the first difference.
fn same_contents(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    const CHUNK: usize = 64 * 1024;
    if std::fs::symlink_metadata(a)?.len() != std::fs::symlink_metadata(b)?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut left, mut right) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    loop {
        let read = a.read(&mut left)?;
        if read == 0 {
            // Equal lengths: `b` must be exhausted too, unless it grew meanwhile.
            return Ok(b.read(&mut right[..1])? == 0);
        }
        b.read_exact(&mut right[..read])?;
        if left[..read] != right[..read] {
            return Ok(false);
        }
    }
}

/// Delete every retained upgrade backup of the database at `path`.
///
/// Backups never contain vault secrets. Attempts every file and returns the first failure.
/// `Ok` means the deletions are durable: every existing backup directory is
/// synced on every call (including a retry that finds nothing left to delete),
/// so callers may retire their retry state afterwards.
pub(crate) fn remove_backups(path: &Path) -> std::io::Result<()> {
    remove_backups_with_sync(path, |_| true, crate::utils::backup_prune::sync_directory)
}

/// Delete the upgrade backups of the database at `path` that are older than `max_age`.
///
/// Covers both the bridge snapshots next to the database and upstream `pre-migration-*`
/// snapshots, and always keeps the newest one, so it is safe whether or not the database
/// currently opens. A database with no backups is skipped without taking its lock, and
/// one whose lock is held (another context is opening or upgrading it) is skipped too;
/// the next pass covers it.
///
/// Attempts every candidate and returns the first failure; a candidate that fails
/// validation is reported but never blocks removing the others. `Ok` carries the number
/// of backups deleted, and their directories are synced.
pub(crate) fn prune_expired_backups(
    path: &Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> std::io::Result<usize> {
    let auto_dir = default_auto_dir(path);
    let unlocked = scan_backups_in(path, Some(&auto_dir))?;
    if unlocked.found.is_empty() && unlocked.rejected.is_empty() {
        return Ok(0);
    }
    let _guard = match backup_lock(path) {
        Ok(guard) => guard,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            tracing::debug!(database = %path.display(), "Upgrade backup retention skipped a database in use");
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let scan = scan_backups_in(path, Some(&auto_dir))?;
    let rejected = scan.rejected.into_iter().next().map(|(_, error)| error);
    let candidates = scan
        .found
        .into_iter()
        .filter(|backup| {
            backup
                .extension()
                .is_none_or(|extension| extension != "pending")
        })
        .map(|backup| {
            let created = backup_created(path, &backup);
            (backup, created)
        })
        .collect();
    let pruned = crate::utils::backup_prune::prune_expired(
        candidates,
        now,
        max_age,
        crate::utils::file_deletion::DeletionIntent::Backup { database: path },
    );
    match (pruned, rejected) {
        (Err(error), _) | (Ok(_), Some(error)) => Err(error),
        (Ok(removed), None) => Ok(removed),
    }
}

/// When `backup` of `database` was created: the later of its modification time and
/// the timestamp in its name, if any. Taking the later one means a reset or skewed
/// signal only delays deletion, never hastens it.
fn backup_created(database: &Path, backup: &Path) -> std::io::Result<std::time::SystemTime> {
    let modified = std::fs::symlink_metadata(backup)?.modified()?;
    let named = backup
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| upstream_backup_timestamp(database, name));
    Ok(named.map_or(modified, |named| named.max(modified)))
}

fn remove_backups_with_sync(
    path: &Path,
    selected: impl Fn(&Path) -> bool,
    mut sync_dir: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let _guard = backup_lock(path)?;
    let auto_dir = default_auto_dir(path);
    let scan = scan_backups_in(path, Some(&auto_dir))?;
    let mut first_error = scan
        .rejected
        .into_iter()
        .find(|(backup, _)| selected(backup))
        .map(|(_, error)| error);
    for backup in scan.found.into_iter().filter(|backup| selected(backup)) {
        if let Err(error) = remove_backup(&backup, path) {
            first_error.get_or_insert(error);
        }
    }
    // Sync every candidate directory, not just those unlinked from in this call: an
    // earlier call may have unlinked successfully and then failed its sync, and a
    // retry that finds nothing left to delete must still make that deletion durable.
    for directory in [path.parent(), Some(auto_dir.as_path())]
        .into_iter()
        .flatten()
    {
        match std::fs::symlink_metadata(directory) {
            // No directory means no entry was ever removed from it.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
            // The scan already rejected a non-directory; never open it here.
            Ok(metadata) if !metadata.is_dir() => continue,
            Ok(_) => {}
        }
        if let Err(error) = sync_dir(directory) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
fn backup(path: &Path) -> Result<PathBuf, UpgradeError> {
    backup_with_hook(path, |_| Ok(()))
}

#[cfg(test)]
fn backup_with_hook(
    path: &Path,
    pending_created: impl FnOnce(&Path) -> Result<(), UpgradeError>,
) -> Result<PathBuf, UpgradeError> {
    let guard = backup_lock(path)?;
    backup_with_hook_locked(&guard, pending_created)
}

fn backup_with_hook_locked(
    guard: &BackupGuard,
    pending_created: impl FnOnce(&Path) -> Result<(), UpgradeError>,
) -> Result<PathBuf, UpgradeError> {
    let path = &guard.path;
    let parent = path.parent().ok_or(UpgradeError::Unrecognized)?;
    let prefix = backup_prefix(path).ok_or(UpgradeError::Unrecognized)?;
    // Not routed through `delete_file`: on failure the temp file removes only
    // the uniquely named `.pending` file it created itself, which keeps the
    // cleanup tied to drop on every error path.
    let file = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".pending")
        .tempfile_in(parent)?;
    pending_created(file.path())?;
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    let mut dest = Connection::open(file.path())?;
    // A separate reader sees the committed snapshot while the writer lock excludes changes.
    let copier = rusqlite::backup::Backup::new(&source, &mut dest)?;
    if !matches!(copier.step(-1)?, rusqlite::backup::StepResult::Done) {
        return Err(UpgradeError::Verification);
    }
    drop(copier);
    verify(&dest)?;
    drop(dest);
    file.as_file().sync_all()?;
    // Earlier snapshots stay: only time-based retention and open-time tidying remove them.
    let kept = file.path().with_extension("sqlite");
    file.persist_noclobber(&kept).map_err(|e| e.error)?;
    crate::utils::backup_prune::sync_directory(parent)?;
    Ok(kept)
}

fn copy_rows(
    source: &Connection,
    target: &Connection,
    table: &str,
    cols: &[String],
    filter: &str,
) -> Result<(), UpgradeError> {
    let names = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
    let placeholders = vec!["?"; cols.len()].join(",");
    let mut input = source.prepare(&format!("SELECT {names} FROM {}{filter}", quote(table)))?;
    let mut output = target.prepare(&format!(
        "INSERT INTO {} ({names}) VALUES ({placeholders})",
        quote(table)
    ))?;
    let mut rows = input.query([])?;
    while let Some(row) = rows.next()? {
        let values = (0..cols.len())
            .map(|i| row.get::<_, rusqlite::types::Value>(i))
            .collect::<Result<Vec<_>, _>>()?;
        output.execute(rusqlite::params_from_iter(values))?;
    }
    Ok(())
}

fn equal_rows(
    source: &Connection,
    target: &Connection,
    table: &str,
    cols: &[String],
    filter: &str,
) -> Result<(), UpgradeError> {
    let names = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
    let sql = format!("SELECT {names} FROM {}", quote(table));
    let order = format!(" ORDER BY {names}");
    let mut a = source.prepare(&format!("{sql}{filter}{order}"))?;
    let mut b = target.prepare(&format!("{sql}{order}"))?;
    let mut a = a.query([])?;
    let mut b = b.query([])?;
    loop {
        match (a.next()?, b.next()?) {
            (None, None) => return Ok(()),
            (Some(a), Some(b)) => {
                for i in 0..cols.len() {
                    if a.get_ref(i)? != b.get_ref(i)? {
                        return Err(UpgradeError::Verification);
                    }
                }
            }
            _ => return Err(UpgradeError::Verification),
        }
    }
}

fn allowed_columns(
    table: &str,
    old: &[String],
    new: &[String],
) -> Result<Vec<String>, UpgradeError> {
    let old_set = old.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let new_set = new.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let added = new_set.difference(&old_set).copied().collect::<Vec<_>>();
    let removed = old_set.difference(&new_set).copied().collect::<Vec<_>>();
    let expected_added: &[&str] = match table {
        "core_utxos" => &[
            "is_sweep_placeholder",
            "spent_in_txid",
            "winner_mined_height",
        ],
        "core_sync_state" => &["chainlock_height"],
        _ => &[],
    };
    let expected_removed: &[&str] = if table == "identities" {
        &["tombstoned"]
    } else {
        &[]
    };
    if added != expected_added || removed != expected_removed {
        return Err(UpgradeError::Unrecognized);
    }
    Ok(old
        .iter()
        .filter(|c| new_set.contains(c.as_str()))
        .cloned()
        .collect())
}

/// Translate only the exact pinned PR schema, preserving the original in a durable backup.
#[cfg(test)]
pub(super) fn upgrade(
    path: &Path,
    target_path: &Path,
    validate: impl FnOnce(&Path) -> Result<(), UpgradeError>,
) -> Result<Option<PathBuf>, UpgradeError> {
    let guard = backup_lock(path)?;
    upgrade_locked(&guard, target_path, validate)
}

pub(super) fn upgrade_locked(
    guard: &BackupGuard,
    target_path: &Path,
    validate: impl FnOnce(&Path) -> Result<(), UpgradeError>,
) -> Result<Option<PathBuf>, UpgradeError> {
    upgrade_with_hook_locked(guard, target_path, validate, || Ok(()))
}

#[cfg(test)]
fn upgrade_with_hook(
    path: &Path,
    target_path: &Path,
    validate: impl FnOnce(&Path) -> Result<(), UpgradeError>,
    before_commit: impl FnOnce() -> Result<(), UpgradeError>,
) -> Result<Option<PathBuf>, UpgradeError> {
    let guard = backup_lock(path)?;
    upgrade_with_hook_locked(&guard, target_path, validate, before_commit)
}

fn upgrade_with_hook_locked(
    guard: &BackupGuard,
    target_path: &Path,
    validate: impl FnOnce(&Path) -> Result<(), UpgradeError>,
    before_commit: impl FnOnce() -> Result<(), UpgradeError>,
) -> Result<Option<PathBuf>, UpgradeError> {
    let path = &guard.path;
    let old = old_reference()?;
    let reference = target_reference()?;
    let mut source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    source.busy_timeout(std::time::Duration::from_secs(5))?;
    source.pragma_update(None, "foreign_keys", false)?;
    source.pragma_update(None, "synchronous", "FULL")?;
    let source = source.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = objects(&source)?;
    if !existing.iter().any(|o| o.name == HISTORY) {
        return Ok(None);
    }
    if history(&source)? != history(&old)? {
        return Ok(None);
    }
    if existing != objects(&old)?
        || source.query_row("PRAGMA application_id", [], |r| r.get::<_, i64>(0))? != 0x504c5754
    {
        return Err(UpgradeError::Unrecognized);
    }
    verify(&source)?;
    let retired = retired_identities(&source)?;
    let mut target = Connection::open_with_flags(
        target_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    if objects(&target)? != objects(&reference)? || history(&target)? != history(&reference)? {
        return Err(UpgradeError::Unrecognized);
    }
    let target_objects = objects(&target)?;
    target.pragma_update(None, "foreign_keys", false)?;
    let target_tx = target.transaction()?;
    source.execute_batch("CREATE TEMP TABLE retired_identities (id BLOB PRIMARY KEY)")?;
    for id in retired {
        source.execute("INSERT INTO temp.retired_identities VALUES (?1)", [id])?;
    }
    for o in target_objects.iter().filter(|o| o.kind == "trigger") {
        target_tx.execute_batch(&format!("DROP TRIGGER {}", quote(&o.name)))?;
    }
    if target_tx.query_row(
        "SELECT count(*) FROM meta_store_generation WHERE id = 0 AND length(generation) = 16",
        [],
        |r| r.get::<_, i64>(0),
    )? != 1
    {
        return Err(UpgradeError::Unrecognized);
    }
    target_tx.execute("DELETE FROM meta_store_generation", [])?;
    let old_tables = existing
        .iter()
        .filter(|o| o.kind == "table")
        .map(|o| o.name.as_str())
        .collect::<BTreeSet<_>>();
    let new_tables = target_objects
        .iter()
        .filter(|o| o.kind == "table")
        .map(|o| o.name.as_str())
        .collect::<BTreeSet<_>>();
    let added = new_tables
        .difference(&old_tables)
        .copied()
        .collect::<Vec<_>>();
    if !old_tables.is_subset(&new_tables)
        || added != ["identity_scan_failed_indices", "identity_scan_states"]
    {
        return Err(UpgradeError::Unrecognized);
    }
    for table in new_tables.iter().filter(|t| **t != HISTORY) {
        if target_tx.query_row(&format!("SELECT count(*) FROM {}", quote(table)), [], |r| {
            r.get::<_, i64>(0)
        })? != 0
        {
            return Err(UpgradeError::Unrecognized);
        }
    }
    for table in old_tables.iter().filter(|t| **t != HISTORY) {
        let cols = allowed_columns(
            table,
            &columns(&source, table)?,
            &columns(&target_tx, table)?,
        )?;
        let filter = selected_rows(table);
        copy_rows(&source, &target_tx, table, &cols, &filter)?;
        equal_rows(&source, &target_tx, table, &cols, &filter)?;
    }
    for (old_domain, new_domain) in [
        ("wallet_metadata", "wallets"),
        ("account_address_pools", "core_address_pool"),
    ] {
        target_tx.execute(
            "INSERT INTO meta_data_versions (wallet_id, domain, seq) SELECT wallet_id, ?2, seq FROM meta_data_versions WHERE domain = ?1 ON CONFLICT(wallet_id, domain) DO UPDATE SET seq = MAX(seq, excluded.seq)",
            [old_domain, new_domain],
        )?;
    }
    for o in target_objects.iter().filter(|o| o.kind == "trigger") {
        target_tx.execute_batch(&o.sql)?;
    }
    verify(&target_tx)?;
    target_tx.commit()?;
    drop(target);
    validate(target_path)?;
    // The source transaction has written only temp tables, so a separate reader still
    // sees the committed original; earlier failures roll back and need no backup.
    let backup = backup_with_hook_locked(guard, |_| Ok(()))?;
    let target = Connection::open_with_flags(
        target_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    // Rebuild under one SQLite transaction; power loss exposes either complete schema.
    for o in existing.iter().filter(|o| o.kind == "trigger") {
        source.execute_batch(&format!("DROP TRIGGER {}", quote(&o.name)))?;
    }
    for table in &old_tables {
        source.execute_batch(&format!("DROP TABLE {}", quote(table)))?;
    }
    for o in target_objects.iter().filter(|o| o.kind == "table") {
        source.execute_batch(&o.sql)?;
    }
    for table in &new_tables {
        let cols = columns(&target, table)?;
        copy_rows(&target, &source, table, &cols, "")?;
        equal_rows(&target, &source, table, &cols, "")?;
    }
    for o in target_objects.iter().filter(|o| o.kind != "table") {
        source.execute_batch(&o.sql)?;
    }
    if objects(&source)? != target_objects {
        return Err(UpgradeError::Verification);
    }
    verify(&source)?;
    before_commit()?;
    source.commit()?;
    Ok(Some(backup))
}

#[cfg(test)]
#[path = "engine/tests.rs"]
mod tests;
