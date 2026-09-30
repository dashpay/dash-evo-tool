//! Time-based expiry shared by every upgrade-backup location.
//!
//! One algorithm for the bridge, upstream `pre-migration-*` and legacy `data.db`
//! backups: delete those older than the retention period, except the newest backup
//! of the database, which is kept whatever its age. A wrong-forward clock can
//! therefore never delete the last recovery copy.

use super::file_deletion::{DeletionIntent, delete_file};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A published backup of one database, with its creation time or the error that
/// prevented reading it.
pub(crate) type Candidate = (PathBuf, std::io::Result<SystemTime>);

/// Delete every candidate older than `max_age` at `now`, except the newest one.
///
/// All candidates must be backups of the database named by `intent`. A candidate
/// whose creation time cannot be read is kept and reported. A candidate that is
/// already gone (e.g. pruned concurrently from another network's context) counts
/// as handled. Attempts every candidate, syncs the directories it deleted from,
/// and returns the first failure; `Ok` carries the number of backups deleted.
pub(crate) fn prune_expired(
    candidates: Vec<Candidate>,
    now: SystemTime,
    max_age: Duration,
    intent: DeletionIntent<'_>,
) -> std::io::Result<usize> {
    let mut first_error = None;
    let mut dated = Vec::with_capacity(candidates.len());
    for (path, created) in candidates {
        match created {
            Ok(created) => dated.push((created, path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    dated.sort();
    // The newest backup is the floor: it survives any age and any clock.
    dated.pop();
    let mut removed = 0;
    let mut touched = BTreeSet::new();
    for (created, path) in dated {
        if !crate::model::backup_retention::backup_expired(created, now, max_age) {
            continue;
        }
        match delete_file(&path, intent) {
            Ok(()) => {
                removed += 1;
                if let Some(directory) = path.parent() {
                    touched.insert(directory.to_owned());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    for directory in touched {
        if let Err(error) = sync_directory(&directory) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(removed), Err)
}

/// Persist directory-entry changes (unlinks) in `directory`.
///
/// Windows cannot open a directory as a `File` without extra flags and NTFS
/// journals metadata changes, so this is a no-op there.
pub(crate) fn sync_directory(directory: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}
