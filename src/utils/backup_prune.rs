//! Time-based expiry shared by every upgrade-backup location.
//!
//! One algorithm for the bridge, upstream `pre-migration-*` and legacy `data.db`
//! backups: delete those older than the retention period, except the newest usable
//! backup of the database, which is kept whatever its age. Neither a wrong clock nor
//! an interrupted copy can therefore delete the last recovery copy.
//!
//! "Newest" is judged by every ordering signal a backup carries, and the newest by
//! each signal is kept:
//! - its creation time, preferring backups not dated in the future, so a copy taken
//!   while the clock ran ahead cannot displace a genuinely newer one;
//! - its position in the database's upgrade history (upstream snapshots name the
//!   migration they precede), which no clock can skew.
//!
//! "Usable" is a cheap structural check ([`usable_snapshot`]). Empty, truncated,
//! header-only and torn files fail it. When no backup of a database passes, the
//! newest files are kept anyway, so a false negative of the check can never delete
//! every backup.

use super::file_deletion::{DeletionIntent, delete_file};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How far ahead of `now` a creation time may be and still count as not in the
/// future when choosing the newest backup. Absorbs ordinary clock drift.
const FUTURE_TOLERANCE: Duration = Duration::from_secs(24 * 60 * 60);

/// A published backup of one database.
#[derive(Debug)]
pub(crate) struct Candidate {
    /// The backup file.
    pub(crate) path: PathBuf,
    /// When the backup was created, or the error that prevented reading it.
    pub(crate) created: std::io::Result<SystemTime>,
    /// Clock-independent position in the database's upgrade history; a larger value
    /// is a later snapshot. `None` when the name carries no such signal.
    pub(crate) sequence: Option<(u32, u32)>,
}

/// A candidate whose creation time is known.
struct Dated {
    created: SystemTime,
    path: PathBuf,
    sequence: Option<(u32, u32)>,
    usable: bool,
}

/// Delete every candidate older than `max_age` at `now`, except the newest usable one
/// by each ordering signal (see the module docs).
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
    for candidate in candidates {
        match candidate.created {
            Ok(created) => dated.push(Dated {
                created,
                usable: usable_snapshot(&candidate.path),
                path: candidate.path,
                sequence: candidate.sequence,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    let floors = floors(&dated, now);
    let mut removed = 0;
    let mut touched = BTreeSet::new();
    for (index, backup) in dated.iter().enumerate() {
        if floors.contains(&index)
            || !crate::model::backup_retention::backup_expired(backup.created, now, max_age)
        {
            continue;
        }
        match delete_file(&backup.path, intent) {
            Ok(()) => {
                removed += 1;
                if let Some(directory) = backup.path.parent() {
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

/// Indices of the backups retention keeps whatever their age: the newest usable one
/// by creation time (preferring those not dated in the future) and the newest usable
/// one by upgrade-history position. Ties break on the path, so every pass and every
/// concurrent context picks the same files. With no usable backup at all, the same
/// choice is made among every backup instead.
fn floors(dated: &[Dated], now: SystemTime) -> BTreeSet<usize> {
    let any_usable = dated.iter().any(|backup| backup.usable);
    let eligible = || {
        dated
            .iter()
            .enumerate()
            .filter(move |(_, backup)| backup.usable || !any_usable)
    };
    let latest_trusted = now.checked_add(FUTURE_TOLERANCE);
    let by_time = eligible()
        .max_by_key(|(_, backup)| {
            let trusted = latest_trusted.is_none_or(|latest| backup.created <= latest);
            (trusted, backup.created, &backup.path)
        })
        .map(|(index, _)| index);
    let by_sequence = eligible()
        .filter_map(|(index, backup)| Some((index, backup.sequence?, backup)))
        .max_by_key(|(_, sequence, backup)| (*sequence, backup.created, &backup.path))
        .map(|(index, _, _)| index);
    by_time.into_iter().chain(by_sequence).collect()
}

/// Length of the SQLite database header.
const SQLITE_HEADER_LEN: usize = 100;
/// Magic string every SQLite 3 database file starts with.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Whether `backup` looks like a complete SQLite database that can be restored.
///
/// A structural check that reads only the 100-byte header and never opens the file
/// through SQLite, which could create `-wal`/`-shm` sidecars or roll back a journal.
/// It requires the magic string, a valid page size and file-format version, a file
/// size covering every page the header claims, and no non-empty `-journal` beside
/// the file: a hot rollback journal means the file holds a torn, uncommitted write.
/// Any read error counts as unusable.
pub(crate) fn usable_snapshot(backup: &Path) -> bool {
    match check_snapshot(backup) {
        Ok(usable) => usable,
        Err(error) => {
            tracing::debug!(backup = %backup.display(), ?error, "Upgrade backup could not be checked");
            false
        }
    }
}

fn check_snapshot(backup: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut file = std::fs::File::open(backup)?;
    let length = file.metadata()?.len();
    if length < SQLITE_HEADER_LEN as u64 {
        return Ok(false);
    }
    let mut header = [0u8; SQLITE_HEADER_LEN];
    file.read_exact(&mut header)?;
    if !header.starts_with(SQLITE_MAGIC) {
        return Ok(false);
    }
    let be_u32 = |offset: usize| {
        u32::from_be_bytes([
            header[offset],
            header[offset + 1],
            header[offset + 2],
            header[offset + 3],
        ])
    };
    let page_size = match u16::from_be_bytes([header[16], header[17]]) {
        1 => 65_536,
        size if (512..=32_768).contains(&size) && size.is_power_of_two() => u64::from(size),
        _ => return Ok(false),
    };
    // File-format write/read versions: 1 = rollback journal, 2 = WAL.
    if !matches!(header[18], 1 | 2) || !matches!(header[19], 1 | 2) {
        return Ok(false);
    }
    // The in-header page count is valid only when "version-valid-for" matches the
    // change counter, as every SQLite since 3.7.0 writes it; otherwise require
    // whole pages.
    let (change_counter, pages, valid_for) = (be_u32(24), be_u32(28), be_u32(92));
    let complete = if pages > 0 && change_counter == valid_for {
        length >= u64::from(pages) * page_size
    } else {
        length >= page_size && length % page_size == 0
    };
    if !complete {
        return Ok(false);
    }
    let mut journal = backup.as_os_str().to_owned();
    journal.push("-journal");
    match std::fs::symlink_metadata(journal) {
        Ok(metadata) => Ok(metadata.len() == 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    /// A small, complete SQLite database at `path`.
    fn write_database(path: &Path) {
        rusqlite::Connection::open(path)
            .unwrap()
            .execute_batch("CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
    }

    fn dated(path: &Path, created: SystemTime, sequence: Option<(u32, u32)>) -> Dated {
        Dated {
            created,
            path: path.to_owned(),
            sequence,
            usable: usable_snapshot(path),
        }
    }

    #[test]
    fn complete_databases_are_usable() {
        let dir = tempfile::tempdir().unwrap();
        let rollback = dir.path().join("rollback.db");
        write_database(&rollback);
        assert!(usable_snapshot(&rollback), "rollback-journal database");

        let wal = dir.path().join("wal.db");
        let conn = rusqlite::Connection::open(&wal).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
        drop(conn);
        assert!(usable_snapshot(&wal), "WAL database");
    }

    #[test]
    fn interrupted_copies_are_not_usable() {
        let dir = tempfile::tempdir().unwrap();
        let complete = dir.path().join("complete.db");
        write_database(&complete);
        let bytes = std::fs::read(&complete).unwrap();
        let zeroed = vec![0u8; bytes.len()];
        let cases: [(&str, &[u8]); 6] = [
            ("empty", b""),
            ("header only", &bytes[..SQLITE_HEADER_LEN]),
            ("one byte short", &bytes[..bytes.len() - 1]),
            ("half", &bytes[..bytes.len() / 2]),
            ("zero-filled", &zeroed),
            ("not sqlite", b"candy wrappers, not a database"),
        ];
        for (case, content) in cases {
            let path = dir.path().join(format!("{case}.db"));
            std::fs::write(&path, content).unwrap();
            assert!(!usable_snapshot(&path), "{case}");
        }
        assert!(!usable_snapshot(&dir.path().join("missing.db")), "missing");
    }

    #[test]
    fn hot_journal_makes_a_copy_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("torn.db");
        write_database(&path);
        let journal = dir.path().join("torn.db-journal");
        std::fs::write(&journal, b"").unwrap();
        assert!(usable_snapshot(&path), "an empty journal is not hot");
        std::fs::write(&journal, b"uncommitted pages").unwrap();
        assert!(!usable_snapshot(&path));
    }

    /// A newer unusable file never becomes the floor; the newest usable one does.
    #[test]
    fn floor_skips_unusable_newer_files() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let valid = dir.path().join("valid.db");
        write_database(&valid);
        let killed = dir.path().join("killed.db");
        std::fs::write(&killed, b"").unwrap();
        let backups = [
            dated(&valid, now - DAY * 200, None),
            dated(&killed, now, Some((9, 8))),
        ];
        assert_eq!(floors(&backups, now), BTreeSet::from([0]));
    }

    /// Without any usable backup the newest file is still kept.
    #[test]
    fn floor_falls_back_to_newest_file_when_none_is_usable() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let [older, newer] = ["older.db", "newer.db"].map(|name| dir.path().join(name));
        for path in [&older, &newer] {
            std::fs::write(path, b"junk").unwrap();
        }
        let backups = [
            dated(&older, now - DAY * 20, None),
            dated(&newer, now - DAY * 10, None),
        ];
        assert_eq!(floors(&backups, now), BTreeSet::from([1]));
    }

    /// A backup dated in the future does not displace the newest backup dated in the
    /// past, and the latest migration's snapshot is kept whatever its date.
    #[test]
    fn floor_resists_clock_skew() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let paths = ["a.db", "b.db", "c.db"].map(|name| dir.path().join(name));
        for path in &paths {
            write_database(path);
        }
        let backups = [
            dated(&paths[0], now + DAY * 30, None),
            dated(&paths[1], now - DAY * 100, Some((2, 1))),
            dated(&paths[2], now - DAY * 400, Some((3, 2))),
        ];
        assert_eq!(floors(&backups, now), BTreeSet::from([1, 2]));
        // Ordinary drift is not the future.
        let backups = [
            dated(&paths[0], now + Duration::from_secs(60), None),
            dated(&paths[1], now - DAY, None),
        ];
        assert_eq!(floors(&backups, now), BTreeSet::from([0]));
        // Only future-dated backups: the newest of them.
        let backups = [
            dated(&paths[0], now + DAY * 30, None),
            dated(&paths[1], now + DAY * 20, None),
        ];
        assert_eq!(floors(&backups, now), BTreeSet::from([0]));
    }
}
