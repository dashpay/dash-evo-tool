//! Legacy `data.db` migration backups (`backups/data_backup_<timestamp>.db`).
//!
//! The legacy migration ladder copies `data.db` before upgrading it. These copies are
//! subject to the same time-based retention as the storage-upgrade backups.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// Directory, next to the database, that holds the legacy backups.
pub(crate) const BACKUP_DIR: &str = "backups";
const PREFIX: &str = "data_backup_";
const SUFFIX: &str = ".db";
/// UTC timestamp format embedded in the backup name.
const TIMESTAMP_FORMAT: &str = "%Y%m%d_%H%M%S";

/// File name of a legacy backup taken at `timestamp` (UTC).
pub(crate) fn backup_file_name(timestamp: chrono::DateTime<chrono::Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", timestamp.format(TIMESTAMP_FORMAT))
}

/// The UTC creation time embedded in a legacy backup name, or `None` for any other file.
fn named_timestamp(name: &str) -> Option<SystemTime> {
    let timestamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let parsed = chrono::NaiveDateTime::parse_from_str(timestamp, TIMESTAMP_FORMAT).ok()?;
    Some(parsed.and_utc().into())
}

/// Delete legacy backups in `<data_dir>/backups` older than `max_age`.
///
/// Only regular files with the exact legacy name are candidates; anything else,
/// including symlinks and subdirectories, is left alone. Age is measured from the
/// later of the name timestamp and the modification time, so a reset or skewed signal
/// only delays deletion. A `backups` path that is a symlink or not a directory is
/// refused. A candidate that vanishes mid-pass (e.g. a concurrent prune from another
/// network's context) counts as already handled, not as a failure. Attempts every
/// candidate and returns the first failure; `Ok` carries the number of backups this
/// call deleted.
pub(crate) fn prune_expired(
    data_dir: &Path,
    max_age: Duration,
    now: SystemTime,
) -> std::io::Result<usize> {
    let directory = data_dir.join(BACKUP_DIR);
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(std::io::Error::other(
                "Legacy backup directory is not a regular directory",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    }
    let entries = std::fs::read_dir(&directory)?;
    let mut removed = 0;
    let mut first_error = None;
    for entry in entries {
        let outcome = entry.and_then(|entry| {
            let Some(named) = entry.file_name().to_str().and_then(named_timestamp) else {
                return Ok(false);
            };
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() {
                return Ok(false);
            }
            let created = named.max(metadata.modified()?);
            if !crate::model::backup_retention::backup_expired(created, now, max_age) {
                return Ok(false);
            }
            std::fs::remove_file(entry.path()).map(|()| true)
        });
        match outcome {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(removed), Err)
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
        assert_eq!(named_timestamp(&name), Some(taken.into()));
        for other in [
            "data_backup_20240305_060708.db-journal",
            "data_backup_latest.db",
            "data.db",
        ] {
            assert_eq!(named_timestamp(other), None, "{other}");
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
        assert_eq!(deleted, 200, "each backup is counted once");
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 0);
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
