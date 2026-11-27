//! Lifecycle lock, housekeeping, retention and removal of the upgrade backups of a
//! wallet database: upstream `pre-migration-*` snapshots and the copies earlier
//! releases kept when converting a pinned-PR database.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn backup_prefix(path: &Path) -> Option<String> {
    Some(format!(
        "{}.platform-67d4ef3-backup-",
        path.file_name()?.to_string_lossy()
    ))
}

/// The published bridge snapshot of `database` named with `suffix`; a test fixture.
#[cfg(test)]
pub(crate) fn bridge_backup_path(database: &Path, suffix: &str) -> PathBuf {
    let prefix = backup_prefix(database).expect("database path has a file name");
    database.with_file_name(format!("{prefix}{suffix}.sqlite"))
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

fn default_auto_dir(path: &Path) -> PathBuf {
    platform_wallet_storage::default_auto_backup_dir(path)
}

/// Single-link regular files named like a backup of the database at `path`, next to it
/// or in `auto_dir`. Anything else is skipped; [`remove_backup`] refuses what must never go.
fn backups_in(path: &Path, auto_dir: Option<&Path>) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let Some(parent) = path.parent() else {
        return Ok(found);
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
            // Symlinks, directories and hard links (which may alias a live database)
            // are skipped; an entry whose metadata cannot be read surfaces on use.
            let other_type = std::fs::symlink_metadata(entry.path())
                .is_ok_and(|metadata| !is_single_link_file(&metadata));
            if (bridge || upstream) && !other_type {
                found.push(entry.path());
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
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

/// Suffix appended to a database's file name to form its lifecycle lock file.
pub(crate) const LOCK_SUFFIX: &str = ".platform-upgrade.lock";

/// The lifecycle lock file of the database at `database`; `None` when the path has
/// no file name.
pub(crate) fn backup_lock_path(database: &Path) -> Option<PathBuf> {
    let mut name = database.file_name()?.to_os_string();
    name.push(LOCK_SUFFIX);
    Some(database.with_file_name(name))
}

/// Whether `metadata` (not followed) is a regular file with no other hard link.
fn is_single_link_file(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    let single_link = std::os::unix::fs::MetadataExt::nlink(metadata) == 1;
    #[cfg(not(unix))]
    let single_link = true;
    metadata.is_file() && single_link
}

/// Refuse an existing lock path that is a symlink, not a regular file, or hard-linked.
fn check_lock_file(lock: &Path) -> std::io::Result<()> {
    if is_single_link_file(&std::fs::symlink_metadata(lock)?) {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "Upgrade lock path is not a single-link regular file",
        ))
    }
}

pub(super) fn backup_lock(path: &Path) -> std::io::Result<BackupGuard> {
    let Some(lock_path) = backup_lock_path(path) else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
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
            check_lock_file(&lock_path)?;
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

/// Delete crash-left `.pending` copies and published snapshots byte-identical to a newer
/// one of the same migration (each failed open writes another); the rest is left to retention.
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

/// Delete every upgrade backup of the database at `path` and sync the directories
/// deleted from; attempts all and returns the first failure.
pub(crate) fn remove_backups(path: &Path) -> std::io::Result<()> {
    let _guard = backup_lock(path)?;
    let mut first_error = None;
    let mut touched = BTreeSet::new();
    for backup in backups_in(path, Some(&default_auto_dir(path)))? {
        match remove_backup(&backup, path) {
            Ok(()) => touched.extend(backup.parent().map(Path::to_owned)),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    for directory in touched {
        if let Err(error) = crate::utils::backup_prune::sync_directory(&directory) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// [`prune_expired`](crate::utils::backup_prune::prune_expired) over the bridge and
/// upstream backups of the database at `path`. Skips a database without backups or
/// whose lifecycle lock is held (the next pass covers it); returns the number deleted.
pub(crate) fn prune_expired_backups(
    path: &Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> std::io::Result<usize> {
    let auto_dir = default_auto_dir(path);
    if backups_in(path, Some(&auto_dir))?.is_empty() {
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
    let candidates = backups_in(path, Some(&auto_dir))?
        .into_iter()
        .filter(|backup| {
            backup
                .extension()
                .is_none_or(|extension| extension != "pending")
        })
        .map(|backup| {
            let created = backup_created(path, &backup);
            // A later migration is a later snapshot, whatever the clock said.
            let sequence = backup
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| parse_upstream_name(path, name))
                .map(|snapshot| (snapshot.to, snapshot.from));
            crate::utils::backup_prune::Candidate {
                path: backup,
                created,
                sequence,
            }
        })
        .collect();
    crate::utils::backup_prune::prune_expired(
        candidates,
        now,
        max_age,
        crate::utils::file_deletion::DeletionIntent::Backup { database: path },
    )
}

fn backup_created(database: &Path, backup: &Path) -> std::io::Result<std::time::SystemTime> {
    let named = backup
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| upstream_backup_timestamp(database, name));
    crate::utils::backup_prune::created_at(backup, named)
}

#[cfg(test)]
mod tests;
