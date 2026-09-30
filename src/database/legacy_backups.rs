//! Legacy `data.db` migration backups (`backups/data_backup_<timestamp>.db`).
//!
//! The legacy migration ladder copies `data.db` before upgrading it. These copies are
//! subject to the same time-based retention as the storage-upgrade backups.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Directory, next to the database, that holds the legacy backups.
pub(crate) const BACKUP_DIR: &str = "backups";
/// The legacy database these backups copy.
pub(crate) const LEGACY_DATABASE: &str = "data.db";
const PREFIX: &str = "data_backup_";
const SUFFIX: &str = ".db";
/// UTC timestamp format embedded in the backup name.
const TIMESTAMP_FORMAT: &str = "%Y%m%d_%H%M%S";
/// Suffix of the unpublished copy [`write_backup`] writes before renaming it.
const PENDING_SUFFIX: &str = ".pending";
/// How old an unpublished copy must be before it counts as left behind by a crash
/// rather than still being written. Copying `data.db` takes seconds.
const PENDING_GRACE: Duration = Duration::from_secs(24 * 60 * 60);

/// File name of a legacy backup taken at `timestamp` (UTC).
pub(crate) fn backup_file_name(timestamp: chrono::DateTime<chrono::Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", timestamp.format(TIMESTAMP_FORMAT))
}

/// Copy `database` to `<its directory>/backups/` under the legacy name for `taken`.
///
/// The copy goes to a unique `.pending` file first, is synced, and only then is
/// renamed to its final name (never replacing an existing backup), after which the
/// directory is synced. A crash or power loss therefore never leaves a torn copy
/// under a backup name, where it could pass as a complete backup.
pub(crate) fn write_backup(
    database: &Path,
    taken: chrono::DateTime<chrono::Utc>,
) -> std::io::Result<PathBuf> {
    let directory = database
        .parent()
        .ok_or(std::io::ErrorKind::InvalidInput)?
        .join(BACKUP_DIR);
    std::fs::create_dir_all(&directory)?;
    let name = backup_file_name(taken);
    // Not routed through `delete_file`: on failure the temp file removes only the
    // uniquely named `.pending` file it created itself.
    let pending = tempfile::Builder::new()
        .prefix(&format!("{name}."))
        .suffix(PENDING_SUFFIX)
        .tempfile_in(&directory)?;
    std::fs::copy(database, pending.path())?;
    pending.as_file().sync_all()?;
    let published = directory.join(name);
    pending
        .persist_noclobber(&published)
        .map_err(|error| error.error)?;
    crate::utils::backup_prune::sync_directory(&directory)?;
    Ok(published)
}

/// The UTC creation time embedded in a legacy backup name, or `None` for any other
/// file. Also the deletion chokepoint's definition of a legacy backup name.
pub(crate) fn backup_timestamp(name: &str) -> Option<SystemTime> {
    let timestamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let parsed = chrono::NaiveDateTime::parse_from_str(timestamp, TIMESTAMP_FORMAT).ok()?;
    Some(parsed.and_utc().into())
}

/// Whether `name` is an unpublished copy written by [`write_backup`]:
/// `<legacy backup name>.<random letters and digits>.pending`. Also the deletion
/// chokepoint's definition of such a copy.
pub(crate) fn is_pending_backup_name(name: &str) -> bool {
    name.strip_suffix(PENDING_SUFFIX)
        .and_then(|rest| rest.rsplit_once('.'))
        .is_some_and(|(published, random)| {
            !random.is_empty()
                && random.bytes().all(|b| b.is_ascii_alphanumeric())
                && backup_timestamp(published).is_some()
        })
}

/// `<data_dir>/backups` if it exists; an error if it is a symlink or not a directory.
fn backup_directory(data_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let directory = data_dir.join(BACKUP_DIR);
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() => Ok(Some(directory)),
        Ok(_) => Err(std::io::Error::other(
            "Legacy backup directory is not a regular directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Delete unpublished copies [`write_backup`] left behind when the app stopped
/// mid-copy.
///
/// Only regular files matching [`is_pending_backup_name`] whose modification time is
/// more than a day before `now` are removed, so a copy another process is still
/// writing is left alone; one dated in the future is kept. They never held a
/// published backup, so every retention policy removes them. Deletion goes through
/// the chokepoint. Attempts every file and returns the first failure; `Ok` carries
/// the number removed.
pub(crate) fn sweep_pending(data_dir: &Path, now: SystemTime) -> std::io::Result<usize> {
    let Some(directory) = backup_directory(data_dir)? else {
        return Ok(0);
    };
    let database = data_dir.join(LEGACY_DATABASE);
    let mut first_error = None;
    let mut removed = 0;
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_str()
            .is_some_and(is_pending_backup_name)
        {
            continue;
        }
        let outcome = match abandoned(&entry.path(), now) {
            Ok(true) => crate::utils::file_deletion::delete_file(
                &entry.path(),
                crate::utils::file_deletion::DeletionIntent::Backup {
                    database: &database,
                },
            ),
            Ok(false) => continue,
            Err(error) => Err(error),
        };
        match outcome {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if removed > 0
        && let Err(error) = crate::utils::backup_prune::sync_directory(&directory)
    {
        first_error.get_or_insert(error);
    }
    first_error.map_or(Ok(removed), Err)
}

/// Whether the unpublished copy at `path` is a regular file last written more than
/// [`PENDING_GRACE`] before `now`.
fn abandoned(path: &Path, now: SystemTime) -> std::io::Result<bool> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(metadata.is_file()
        && crate::model::backup_retention::backup_expired(metadata.modified()?, now, PENDING_GRACE))
}

/// Delete legacy backups in `<data_dir>/backups` older than `max_age`, always
/// keeping the newest usable one.
///
/// Deletion goes through the [`delete_file`](crate::utils::file_deletion::delete_file)
/// chokepoint, which refuses live databases, aliases and hard links.
/// Only regular files with the exact legacy name are candidates; anything else,
/// including symlinks and subdirectories, is left alone. Age is measured from the
/// later of the name timestamp and the modification time, so a reset or skewed signal
/// only delays deletion. A `backups` path that is a symlink or not a directory is
/// refused. Expiry itself is [`prune_expired`](crate::utils::backup_prune::prune_expired);
/// `Ok` carries the number of backups this call deleted.
pub(crate) fn prune_expired(
    data_dir: &Path,
    max_age: Duration,
    now: SystemTime,
) -> std::io::Result<usize> {
    let Some(directory) = backup_directory(data_dir)? else {
        return Ok(0);
    };
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(named) = entry.file_name().to_str().and_then(backup_timestamp) else {
            continue;
        };
        let metadata = match std::fs::symlink_metadata(entry.path()) {
            Ok(metadata) if !metadata.is_file() => continue,
            other => other,
        };
        let created = metadata.and_then(|metadata| Ok(named.max(metadata.modified()?)));
        candidates.push(crate::utils::backup_prune::Candidate {
            path: entry.path(),
            created,
            sequence: None,
        });
    }
    crate::utils::backup_prune::prune_expired(
        candidates,
        now,
        max_age,
        crate::utils::file_deletion::DeletionIntent::Backup {
            database: &data_dir.join(LEGACY_DATABASE),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn set_mtime(path: &Path, time: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    /// The published copy is complete, no temporary file is left, and an existing
    /// backup of the same second is never overwritten.
    #[test]
    fn legacy_backup_is_published_complete_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join(LEGACY_DATABASE);
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch("CREATE TABLE settings (v INTEGER); INSERT INTO settings VALUES (1);")
            .unwrap();
        let taken = chrono::Utc::now();

        let published = write_backup(&database, taken).unwrap();

        assert_eq!(
            published,
            dir.path().join(BACKUP_DIR).join(backup_file_name(taken))
        );
        assert_eq!(
            std::fs::read(&published).unwrap(),
            std::fs::read(&database).unwrap()
        );
        assert!(crate::utils::backup_prune::usable_snapshot(&published));
        let error = write_backup(&database, taken).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        let entries: Vec<_> = std::fs::read_dir(dir.path().join(BACKUP_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [published.file_name().unwrap().to_owned()]);
    }

    #[test]
    fn pending_copy_names_follow_write_backup_exactly() {
        assert!(is_pending_backup_name(
            "data_backup_20200101_000000.db.a1B2c3.pending"
        ));
        for other in [
            "data_backup_20200101_000000.db.pending",
            "data_backup_20200101_000000.db..pending",
            "data_backup_20200101_000000.db.a-1.pending",
            "data_backup_20200101_000000.db.abc.pending-journal",
            "data_backup_latest.db.abc123.pending",
            "notes.db.abc123.pending",
            "data.db.abc123.pending",
            "data_backup_20200101_000000.db",
        ] {
            assert!(!is_pending_backup_name(other), "{other}");
        }
    }

    /// Only old regular files with the exact temporary name are swept, through the
    /// chokepoint; lookalikes, directories, symlinks and published backups stay.
    #[test]
    fn sweep_removes_only_abandoned_pending_copies() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let now = SystemTime::now();
        let write = |name: &str, age: Duration| {
            let path = backups.join(name);
            std::fs::write(&path, b"partial").unwrap();
            set_mtime(&path, now - age);
            path
        };
        let abandoned = write("data_backup_20200101_000000.db.a1B2c3.pending", DAY * 2);
        let fresh = write("data_backup_20200102_000000.db.Z9y8X7.pending", DAY / 24);
        let published = write("data_backup_20200101_000000.db", DAY * 400);
        let lookalike = write("data_backup_20200101_000000.db.pending", DAY * 400);
        let unrelated = write("notes.db.abc123.pending", DAY * 400);
        let directory = backups.join("data_backup_20200103_000000.db.dir123.pending");
        std::fs::create_dir(&directory).unwrap();

        assert_eq!(sweep_pending(dir.path(), now).unwrap(), 1);

        assert!(!abandoned.exists());
        for kept in [&fresh, &published, &lookalike, &unrelated, &directory] {
            assert!(kept.exists(), "{}", kept.display());
        }
        assert_eq!(sweep_pending(dir.path(), now).unwrap(), 0);
        assert_eq!(
            sweep_pending(tempfile::tempdir().unwrap().path(), now).unwrap(),
            0,
            "no backups directory"
        );
    }

    #[test]
    fn legacy_backup_names_round_trip() {
        let taken = chrono::DateTime::parse_from_rfc3339("2024-03-05T06:07:08Z")
            .unwrap()
            .to_utc();
        let name = backup_file_name(taken);
        assert_eq!(name, "data_backup_20240305_060708.db");
        assert_eq!(backup_timestamp(&name), Some(taken.into()));
        for other in [
            "data_backup_20240305_060708.db-journal",
            "data_backup_latest.db",
            "data.db",
        ] {
            assert_eq!(backup_timestamp(other), None, "{other}");
        }
    }

    #[test]
    fn legacy_backups_expire_by_later_of_name_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(backups.join("auto")).unwrap();
        let now = SystemTime::now();
        let long_ago = now - DAY * 200;
        let old_name = backup_file_name(chrono::DateTime::from(long_ago));
        let expired = backups.join(&old_name);
        std::fs::write(&expired, b"old").unwrap();
        set_mtime(&expired, long_ago);
        let recently_written =
            backups.join(backup_file_name(chrono::DateTime::from(long_ago - DAY)));
        std::fs::write(&recently_written, b"restored copy").unwrap();
        let fresh = backups.join(backup_file_name(chrono::DateTime::from(now)));
        std::fs::write(&fresh, b"new").unwrap();
        set_mtime(&fresh, long_ago);
        let unrelated = backups.join("notes.db");
        std::fs::write(&unrelated, b"keep").unwrap();
        set_mtime(&unrelated, long_ago);

        assert_eq!(prune_expired(dir.path(), DAY * 90, now).unwrap(), 1);

        assert!(!expired.exists());
        assert!(recently_written.exists());
        assert!(fresh.exists());
        assert!(unrelated.exists());
        assert!(backups.join("auto").is_dir());
    }

    #[test]
    fn missing_legacy_backup_directory_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            prune_expired(dir.path(), DAY, SystemTime::now()).unwrap(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn legacy_backup_directory_symlink_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("data_backup_20000101_000000.db");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(BACKUP_DIR)).unwrap();

        prune_expired(dir.path(), DAY, SystemTime::now()).unwrap_err();

        assert!(
            victim.exists(),
            "a symlinked backups directory is never entered"
        );
    }

    #[test]
    fn legacy_backup_path_that_is_a_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(BACKUP_DIR), b"not a directory").unwrap();
        prune_expired(dir.path(), DAY, SystemTime::now()).unwrap_err();
    }

    /// Two contexts pruning the shared data directory at once must not report a
    /// backup the other one already deleted as a failure.
    #[test]
    fn concurrent_legacy_backup_prunes_do_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let long_ago = SystemTime::now() - DAY * 400;
        for second in 0..200 {
            let path = backups.join(format!(
                "data_backup_20000101_00{:02}{:02}.db",
                second / 60,
                second % 60
            ));
            std::fs::write(&path, b"old").unwrap();
            set_mtime(&path, long_ago);
        }
        let barrier = std::sync::Barrier::new(4);
        let results: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        prune_expired(dir.path(), DAY, SystemTime::now())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let deleted: usize = results
            .into_iter()
            .map(|result| result.expect("a concurrently deleted backup is not a failure"))
            .sum();
        assert_eq!(
            deleted, 199,
            "each backup is counted once; the newest stays"
        );
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);
    }

    /// A clock far in the future must never delete the last legacy backup.
    #[test]
    fn newest_legacy_backup_survives_a_far_future_clock() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let older = backups.join("data_backup_20200101_000000.db");
        let newest = backups.join("data_backup_20210101_000000.db");
        for (path, age) in [(&older, DAY * 30), (&newest, DAY * 2)] {
            std::fs::write(path, b"backup").unwrap();
            set_mtime(path, SystemTime::now() - age);
        }
        let far_future = SystemTime::now() + DAY * 365 * 100;

        assert_eq!(prune_expired(dir.path(), DAY, far_future).unwrap(), 1);
        assert!(!older.exists());
        assert!(newest.exists(), "the newest backup is the floor");
        assert_eq!(prune_expired(dir.path(), DAY, far_future).unwrap(), 0);
        assert!(newest.exists());
    }

    /// An interrupted legacy copy is never the floor: the newest complete backup
    /// survives, and the expired interrupted copy is deleted.
    #[test]
    fn newest_legacy_backup_floor_must_be_a_complete_database() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let complete = backups.join("data_backup_20200101_000000.db");
        rusqlite::Connection::open(&complete)
            .unwrap()
            .execute_batch("CREATE TABLE settings (v INTEGER); INSERT INTO settings VALUES (1);")
            .unwrap();
        let bytes = std::fs::read(&complete).unwrap();
        let truncated = backups.join("data_backup_20200601_000000.db");
        std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
        let now = SystemTime::now();
        set_mtime(&complete, now - DAY * 200);
        set_mtime(&truncated, now - DAY * 100);

        assert_eq!(prune_expired(dir.path(), DAY * 90, now).unwrap(), 1);
        assert!(complete.exists(), "the only complete backup is the floor");
        assert!(!truncated.exists());
    }

    #[cfg(unix)]
    #[test]
    fn legacy_backup_symlinks_are_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let target = dir.path().join("precious.db");
        std::fs::write(&target, b"keep").unwrap();
        let link = backups.join("data_backup_20000101_000000.db");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert_eq!(
            prune_expired(dir.path(), DAY, SystemTime::now()).unwrap(),
            0
        );
        assert!(link.symlink_metadata().is_ok());
        assert!(target.exists());
    }
}
