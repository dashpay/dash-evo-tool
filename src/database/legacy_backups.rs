//! Legacy `data.db` migration backups (`backups/data_backup_<timestamp>.db`).
//!
//! Earlier app versions copied `data.db` here before upgrading it; this version opens an
//! existing `data.db` read-only and never writes new copies. The copies already on disk
//! are subject to the same time-based retention as the storage-upgrade backups.

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

/// File name of a legacy backup taken at `timestamp` (UTC). Tests only: this version
/// never writes legacy backups, it only recognises and expires existing ones.
#[cfg(test)]
pub(crate) fn backup_file_name(timestamp: chrono::DateTime<chrono::Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", timestamp.format(TIMESTAMP_FORMAT))
}

/// The UTC creation time embedded in a legacy backup name, or `None` for any other
/// file. Also the deletion chokepoint's definition of a legacy backup name.
pub(crate) fn backup_timestamp(name: &str) -> Option<SystemTime> {
    let timestamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let parsed = chrono::NaiveDateTime::parse_from_str(timestamp, TIMESTAMP_FORMAT).ok()?;
    Some(parsed.and_utc().into())
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
