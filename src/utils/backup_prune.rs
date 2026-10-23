//! Time-based expiry shared by every upgrade-backup location (bridge, upstream
//! `pre-migration-*`, legacy `data.db`).
//!
//! Backups older than the retention period are deleted, except the newest usable one
//! of each database by creation time (preferring those not dated in the future) and by
//! upgrade-history position. Copies whose usability is undetermined (unreadable, or
//! with a non-empty rollback journal beside them) are never deleted. With no usable
//! backup, the newest files are kept anyway.

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
    usability: Usability,
}

/// Whether a backup can be restored, as far as a cheap structural check can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Usability {
    /// A complete SQLite database.
    Usable,
    /// Empty, truncated, header-only or not SQLite at all.
    Unusable,
    /// Not judged: the file could not be read, or it looks complete but has a
    /// non-empty rollback journal beside it (a torn write or a harmless leftover).
    Undetermined,
}

/// Delete expired `candidates` (all backups of `intent`'s database) as the module docs
/// describe. Attempts every one and returns the first failure, or the number deleted.
pub(crate) fn prune_expired(
    candidates: Vec<Candidate>,
    now: SystemTime,
    max_age: Duration,
    intent: DeletionIntent<'_>,
) -> std::io::Result<usize> {
    let mut first_error = None;
    let mut dated = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let checked = candidate
            .created
            .map(|created| (created, check_snapshot(&candidate.path)));
        let (created, usability) = match checked {
            Ok((created, Ok(usability))) => (created, usability),
            Ok((created, Err(error))) if error.kind() != std::io::ErrorKind::NotFound => {
                // Unreadable is not proof of unusable: keep the file and retry later.
                first_error.get_or_insert(error);
                (created, Usability::Undetermined)
            }
            // Already gone, e.g. pruned concurrently from another network's context.
            Ok((_, Err(_))) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        dated.push(Dated {
            created,
            path: candidate.path,
            sequence: candidate.sequence,
            usability,
        });
    }
    let floors = floors(&dated, now);
    let mut removed = 0;
    let mut touched = BTreeSet::new();
    for (index, backup) in dated.iter().enumerate() {
        if floors.contains(&index)
            || backup.usability == Usability::Undetermined
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
    let usable = |backup: &Dated| backup.usability == Usability::Usable;
    let any_usable = dated.iter().any(usable);
    let eligible = || {
        dated
            .iter()
            .enumerate()
            .filter(move |(_, backup)| usable(backup) || !any_usable)
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

/// When `backup` was created: the later of its modification time and `named` (the
/// timestamp in its name), so a reset or skewed signal only delays deletion.
pub(crate) fn created_at(backup: &Path, named: Option<SystemTime>) -> std::io::Result<SystemTime> {
    let modified = std::fs::symlink_metadata(backup)?.modified()?;
    Ok(named.map_or(modified, |named| named.max(modified)))
}

/// Length of the SQLite database header.
const SQLITE_HEADER_LEN: usize = 100;
/// Magic string every SQLite 3 database file starts with.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Whether `backup` is proven to be a complete SQLite database.
#[cfg(test)]
pub(crate) fn usable_snapshot(backup: &Path) -> bool {
    matches!(check_snapshot(backup), Ok(Usability::Usable))
}

/// How restorable `backup` looks.
///
/// A structural check that reads only the 100-byte header and never opens the file
/// through SQLite, which could create `-wal`/`-shm` sidecars or roll back a journal.
/// Usable requires the magic string, a valid page size and file-format version, and
/// a file size covering every page the header claims. A complete-looking file with a
/// non-empty `-journal` beside it is [`Usability::Undetermined`]. A file that cannot
/// be read is an error, never a verdict.
fn check_snapshot(backup: &Path) -> std::io::Result<Usability> {
    use std::io::Read;
    let mut file = std::fs::File::open(backup)?;
    let length = file.metadata()?.len();
    if length < SQLITE_HEADER_LEN as u64 {
        return Ok(Usability::Unusable);
    }
    let mut header = [0u8; SQLITE_HEADER_LEN];
    file.read_exact(&mut header)?;
    if !header.starts_with(SQLITE_MAGIC) {
        return Ok(Usability::Unusable);
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
        _ => return Ok(Usability::Unusable),
    };
    // File-format write/read versions: 1 = rollback journal, 2 = WAL.
    if !matches!(header[18], 1 | 2) || !matches!(header[19], 1 | 2) {
        return Ok(Usability::Unusable);
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
        return Ok(Usability::Unusable);
    }
    let mut journal = backup.as_os_str().to_owned();
    journal.push("-journal");
    match std::fs::symlink_metadata(journal) {
        Ok(metadata) if metadata.len() > 0 => {
            tracing::debug!(backup = %backup.display(), "Upgrade backup has a rollback journal beside it; kept");
            Ok(Usability::Undetermined)
        }
        Ok(_) => Ok(Usability::Usable),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Usability::Usable),
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
            usability: check_snapshot(path).unwrap_or(Usability::Undetermined),
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
            assert_eq!(
                check_snapshot(&path).unwrap(),
                Usability::Unusable,
                "{case}"
            );
        }
        assert!(!usable_snapshot(&dir.path().join("missing.db")), "missing");
    }

    #[test]
    fn hot_journal_leaves_a_copy_undetermined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("torn.db");
        write_database(&path);
        let journal = dir.path().join("torn.db-journal");
        std::fs::write(&journal, b"").unwrap();
        assert_eq!(
            check_snapshot(&path).unwrap(),
            Usability::Usable,
            "an empty journal is not hot"
        );
        std::fs::write(&journal, b"uncommitted pages").unwrap();
        assert_eq!(check_snapshot(&path).unwrap(), Usability::Undetermined);
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

    /// A newer copy of undetermined usability does not displace the newest usable one.
    #[test]
    fn floor_ignores_undetermined_newer_files() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let [valid, journaled] = ["valid.db", "journaled.db"].map(|name| dir.path().join(name));
        write_database(&valid);
        write_database(&journaled);
        std::fs::write(dir.path().join("journaled.db-journal"), b"pages").unwrap();
        let backups = [
            dated(&valid, now - DAY * 200, None),
            dated(&journaled, now - DAY * 100, Some((2, 1))),
        ];
        assert_eq!(backups[1].usability, Usability::Undetermined);
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
        // Once real time catches up with a skewed date, that backup is trusted
        // again and, being dated later, is the time floor once more. A limit of
        // any clock-based order; only the migration order is immune.
        let skewed = now + DAY * 30;
        let backups = [
            dated(&paths[0], skewed, None),
            dated(&paths[1], now - DAY * 100, None),
        ];
        assert_eq!(floors(&backups, skewed), BTreeSet::from([0]));
        // Only future-dated backups: the newest of them.
        let backups = [
            dated(&paths[0], now + DAY * 30, None),
            dated(&paths[1], now + DAY * 20, None),
        ];
        assert_eq!(floors(&backups, now), BTreeSet::from([0]));
    }
}
