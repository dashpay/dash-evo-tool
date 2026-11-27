use super::*;

/// A small live database at `wallet.sqlite`.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wallet.sqlite");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
        .unwrap();
    (dir, path)
}

/// A copy of the database under a new bridge backup name, as earlier releases left.
fn backup(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let copy = bridge_backup_path(path, &format!("copy{n}"));
    std::fs::copy(path, &copy).unwrap();
    copy
}

fn backup_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".platform-67d4ef3-backup-")
        })
        .collect()
}

#[test]
fn platform_compatibility_backup_removal_only_touches_the_named_database() {
    let dir = tempfile::tempdir().unwrap();
    let names = [
        "det-app.sqlite.platform-67d4ef3-backup-a1.sqlite",
        "det-testnet.sqlite.platform-67d4ef3-backup-b2.sqlite",
        "det-mainnet.sqlite.platform-67d4ef3-backup-c3.sqlite",
        "det-testnet.sqlite",
        "det-testnet-shielded.sqlite",
    ];
    for name in names {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    remove_backups(&dir.path().join("det-testnet.sqlite")).unwrap();
    let mut left = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    left.sort();
    assert_eq!(
        left,
        [
            "det-app.sqlite.platform-67d4ef3-backup-a1.sqlite",
            "det-mainnet.sqlite.platform-67d4ef3-backup-c3.sqlite",
            "det-testnet-shielded.sqlite",
            "det-testnet.sqlite",
            format!("det-testnet.sqlite{LOCK_SUFFIX}").as_str(),
        ]
    );
    remove_backups(&dir.path().join("absent.sqlite")).unwrap();
}

#[test]
fn platform_compatibility_removes_only_matching_upstream_backups() {
    let dir = tempfile::tempdir().unwrap();
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let removed = auto.join("pre-migration-det-testnet-1-to-2-20260915T120000Z.db");
    let kept = auto.join("pre-migration-det-testnet-other-1-to-2-20260915T120000Z.db");
    for path in [&removed, &kept] {
        std::fs::write(path, b"backup").unwrap();
    }
    remove_backups(&dir.path().join("det-testnet.sqlite")).unwrap();
    assert!(!removed.exists());
    assert!(kept.exists());
}

/// Tidying removes a snapshot only when a newer one of the same migration has the
/// same bytes; a snapshot that differs by even one byte, or precedes another
/// migration, is never removed.
#[test]
fn platform_compatibility_tidy_removes_only_identical_copies() {
    let (dir, path) = fixture();
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let original = backup(&path);
    let copy = backup(&path);
    set_mtime(&original, -3600);
    let bytes = std::fs::read(&copy).unwrap();
    let mut differing = bytes.clone();
    *differing.last_mut().unwrap() ^= 0xFF;
    let upstream = |versions: &str, second: u32| {
        auto.join(format!(
            "pre-migration-wallet-{versions}-20260915T12000{second}Z.db"
        ))
    };
    let (same_a, same_b, other_migration, changed) = (
        upstream("1-to-2", 1),
        upstream("1-to-2", 2),
        upstream("2-to-3", 3),
        upstream("1-to-2", 4),
    );
    for (file, contents) in [
        (&same_a, &bytes),
        (&same_b, &bytes),
        (&other_migration, &bytes),
        (&changed, &differing),
    ] {
        std::fs::write(file, contents).unwrap();
    }
    set_mtime(&same_a, -3600);

    tidy_backups(&path, Some(&auto)).unwrap();

    assert!(!original.exists(), "the older identical bridge copy goes");
    assert!(copy.exists(), "the newest identical copy stays");
    assert!(!same_a.exists(), "the older identical upstream copy goes");
    assert!(same_b.exists());
    assert!(
        other_migration.exists(),
        "another migration is never merged"
    );
    assert!(changed.exists(), "a differing snapshot is never removed");
    tidy_backups(&path, Some(&auto)).unwrap();
    assert_eq!(backup_files(dir.path()), vec![copy]);
}

/// Removal deletes every backup and skips entries named like one that are directories,
/// symlinks or hard links (here to the live database), which are never followed.
#[cfg(unix)]
#[test]
fn platform_compatibility_removal_skips_links_and_directories() {
    let (dir, path) = fixture();
    let bridge = |suffix: &str| {
        dir.path()
            .join(format!("wallet.sqlite.platform-67d4ef3-backup-{suffix}"))
    };
    let skipped = [
        bridge("link.sqlite"),
        bridge("dir.pending"),
        bridge("symlink.sqlite"),
    ];
    std::fs::hard_link(&path, &skipped[0]).unwrap();
    std::fs::create_dir(&skipped[1]).unwrap();
    std::os::unix::fs::symlink(&path, &skipped[2]).unwrap();
    let valid = [bridge("b1.sqlite"), bridge("c2.pending")];
    for backup in &valid {
        std::fs::write(backup, b"backup").unwrap();
    }
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let upstream = auto.join("pre-migration-wallet-1-to-2-20260915T120000Z.db");
    std::fs::write(&upstream, b"backup").unwrap();

    remove_backups(&path).unwrap();
    for backup in valid.iter().chain([&upstream]) {
        assert!(!backup.exists(), "{} must be removed", backup.display());
    }
    assert!(path.exists());
    for entry in &skipped {
        assert!(entry.symlink_metadata().is_ok(), "{}", entry.display());
    }
}

/// An existing lifecycle lock path that is a symlink is refused, never followed.
#[cfg(unix)]
#[test]
fn platform_compatibility_lock_refuses_a_symlinked_lock_file() {
    let (dir, path) = fixture();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::write(&elsewhere, b"keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, backup_lock_path(&path).unwrap()).unwrap();
    assert!(backup_lock(&path).is_err());
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"keep");
}

#[test]
fn platform_compatibility_pending_cleanup_preserves_published_snapshot() {
    let (dir, path) = fixture();
    let published = backup(&path);
    let pending = dir
        .path()
        .join("wallet.sqlite.platform-67d4ef3-backup-crash.pending");
    std::fs::copy(&path, &pending).unwrap();
    tidy_backups(&path, None).unwrap();
    assert!(!pending.exists());
    assert!(published.exists());
    assert!(path.exists());
}

#[test]
fn platform_compatibility_deletion_removes_pending_snapshots() {
    let (dir, path) = fixture();
    let pending = dir
        .path()
        .join("wallet.sqlite.platform-67d4ef3-backup-crash.pending");
    let unrelated = dir
        .path()
        .join("other.sqlite.platform-67d4ef3-backup-crash.pending");
    std::fs::copy(&path, &pending).unwrap();
    std::fs::copy(&path, &unrelated).unwrap();
    remove_backups(&path).unwrap();
    assert!(!pending.exists());
    assert!(unrelated.exists());
    assert!(path.exists());
}

fn set_mtime(path: &Path, seconds_from_now: i64) {
    let now = std::time::SystemTime::now();
    let offset = std::time::Duration::from_secs(seconds_from_now.unsigned_abs());
    let time = if seconds_from_now < 0 {
        now - offset
    } else {
        now + offset
    };
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

/// An interrupted copy left under a final snapshot name (empty or truncated) differs
/// from the complete snapshot, so tidying keeps both, in either format.
#[test]
fn platform_compatibility_tidy_keeps_differing_snapshots() {
    let (dir, path) = fixture();
    let before = std::fs::read(&path).unwrap();
    let valid = backup(&path);
    let full = std::fs::read(&valid).unwrap();
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let bridge = dir
        .path()
        .join("wallet.sqlite.platform-67d4ef3-backup-killed.sqlite");
    let upstream = auto.join("pre-migration-wallet-1-to-2-20260915T120000Z.db");
    let cases: [(&str, &[u8], &PathBuf); 4] = [
        ("empty bridge", b"", &bridge),
        ("truncated bridge", &full[..full.len() / 2], &bridge),
        ("header-only bridge", &full[..100], &bridge),
        ("empty upstream", b"", &upstream),
    ];
    set_mtime(&valid, -3600);
    for (case, bytes, incomplete) in cases {
        std::fs::write(incomplete, bytes).unwrap();
        tidy_backups(&path, Some(&auto)).unwrap();
        assert_eq!(std::fs::read(&valid).unwrap(), before, "{case}");
        assert!(incomplete.exists(), "{case}: a differing copy is kept");
    }
}

const RETENTION_DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Expiry covers both backup formats, measures age from the later of the file's
/// modification time and its name timestamp, and never touches the database itself.
#[test]
fn platform_compatibility_prune_expired_backups_by_age() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("det-testnet.sqlite");
    std::fs::write(&path, b"live database").unwrap();
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let old_bridge = dir
        .path()
        .join("det-testnet.sqlite.platform-67d4ef3-backup-old.sqlite");
    let fresh_bridge = dir
        .path()
        .join("det-testnet.sqlite.platform-67d4ef3-backup-fresh.sqlite");
    let old_upstream = auto.join("pre-migration-det-testnet-1-to-2-20200101T000000Z.db");
    // An old name whose file was rewritten recently (e.g. restored from a copy).
    let old_name_fresh_file = auto.join("pre-migration-det-testnet-2-to-3-20200102T000000Z.db");
    // A recent name whose modification time was reset to long ago.
    let fresh_name_old_file = auto.join("pre-migration-det-testnet-3-to-4-20991231T000000Z.db");
    let unrelated = auto.join("pre-migration-det-mainnet-1-to-2-20200101T000000Z.db");
    for file in [
        &old_bridge,
        &fresh_bridge,
        &old_upstream,
        &old_name_fresh_file,
        &fresh_name_old_file,
        &unrelated,
    ] {
        std::fs::write(file, b"backup").unwrap();
    }
    let hundred_days = -100 * 24 * 60 * 60;
    for file in [&old_bridge, &old_upstream, &fresh_name_old_file, &unrelated] {
        set_mtime(file, hundred_days);
    }
    set_mtime(&path, hundred_days);

    let removed =
        prune_expired_backups(&path, RETENTION_DAY * 90, std::time::SystemTime::now()).unwrap();

    assert_eq!(removed, 2);
    assert!(!old_bridge.exists());
    assert!(!old_upstream.exists());
    assert!(fresh_bridge.exists());
    assert!(
        old_name_fresh_file.exists(),
        "a recent modification time keeps an old-named snapshot"
    );
    assert!(
        fresh_name_old_file.exists(),
        "a recent name timestamp keeps a snapshot with an old modification time"
    );
    assert!(unrelated.exists(), "another database's snapshot is kept");
    assert!(path.exists(), "the live database is never a candidate");
}

/// An unreadable newest snapshot is kept and reported, and never hands the floor to an
/// older copy that would then be the only one left.
#[cfg(unix)]
#[test]
fn platform_compatibility_prune_keeps_unreadable_snapshots() {
    use std::os::unix::fs::PermissionsExt;
    if !crate::test_support::file_modes_deny_access() {
        eprintln!("skipped: file modes do not deny this process access");
        return;
    }
    let (_dir, path) = fixture();
    let older = backup(&path);
    let newest = backup(&path);
    set_mtime(&older, -200 * 24 * 60 * 60);
    set_mtime(&newest, -100 * 24 * 60 * 60);
    std::fs::set_permissions(&newest, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&newest).is_ok() {
        // The process can bypass read permissions independently of write permissions.
        return;
    }

    let result = prune_expired_backups(&path, RETENTION_DAY * 90, std::time::SystemTime::now());

    std::fs::set_permissions(&newest, std::fs::Permissions::from_mode(0o600)).unwrap();
    result.unwrap_err();
    assert!(newest.exists(), "an unjudged snapshot is kept");
    assert!(older.exists(), "the newest proven snapshot is kept");
}

/// A snapshot written while the clock ran ahead is dated in the future; it must not
/// displace the genuinely newer snapshot taken after the clock was corrected.
#[test]
fn platform_compatibility_prune_floor_resists_forward_clock_skew() {
    let (_dir, path) = fixture();
    let skewed = backup(&path);
    let current = backup(&path);
    set_mtime(&skewed, 30 * 24 * 60 * 60);
    set_mtime(&current, -100 * 24 * 60 * 60);

    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY * 90, std::time::SystemTime::now()).unwrap(),
        0
    );
    assert!(
        current.exists(),
        "the newest snapshot dated in the past is kept"
    );
    assert!(skewed.exists(), "a future-dated snapshot is never expired");
}

/// Upstream snapshots name the migration they precede, a clock-independent order:
/// the snapshot of the latest migration survives even when an older migration's
/// snapshot carries later dates.
#[test]
fn platform_compatibility_prune_floor_keeps_latest_migration_snapshot() {
    let (dir, path) = fixture();
    let valid = backup(&path);
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let latest_migration = auto.join("pre-migration-wallet-2-to-3-20200101T000000Z.db");
    let earlier_migration = auto.join("pre-migration-wallet-1-to-2-20200601T000000Z.db");
    std::fs::copy(&valid, &latest_migration).unwrap();
    std::fs::copy(&valid, &earlier_migration).unwrap();
    set_mtime(&latest_migration, -400 * 24 * 60 * 60);
    set_mtime(&earlier_migration, -300 * 24 * 60 * 60);
    set_mtime(&valid, -500 * 24 * 60 * 60);

    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY * 90, std::time::SystemTime::now()).unwrap(),
        1
    );
    assert!(!valid.exists());
    assert!(latest_migration.exists(), "newest by migration order");
    assert!(earlier_migration.exists(), "newest by creation time");
}

/// A backup-named hard link (here to the live database) is never a candidate, so it
/// cannot become the floor and let the real newest backup expire.
#[cfg(unix)]
#[test]
fn platform_compatibility_prune_skips_hard_linked_backup_names() {
    let (_dir, path) = fixture();
    let real = backup(&path);
    set_mtime(&real, -200 * 24 * 60 * 60);
    let linked = bridge_backup_path(&path, "linked");
    std::fs::hard_link(&path, &linked).unwrap();

    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY * 90, std::time::SystemTime::now()).unwrap(),
        0
    );
    assert!(real.exists(), "the only real backup is the floor");
    assert!(linked.exists() && path.exists());
}

/// A held lifecycle lock means an open or upgrade is running: pruning skips the
/// database without failing, and a later pass covers it.
#[test]
fn platform_compatibility_prune_backs_off_while_lock_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("det-testnet.sqlite");
    let expired = dir
        .path()
        .join("det-testnet.sqlite.platform-67d4ef3-backup-old.sqlite");
    std::fs::write(&expired, b"backup").unwrap();
    set_mtime(&expired, -10 * 24 * 60 * 60);
    std::fs::write(
        dir.path()
            .join("det-testnet.sqlite.platform-67d4ef3-backup-new.sqlite"),
        b"newer backup",
    )
    .unwrap();
    let guard = backup_lock(&path).unwrap();
    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY, std::time::SystemTime::now()).unwrap(),
        0
    );
    assert!(expired.exists());
    drop(guard);
    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY, std::time::SystemTime::now()).unwrap(),
        1
    );
}

/// A database with no backups is skipped before its lock file would be created.
#[test]
fn platform_compatibility_prune_skips_database_without_backups() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("det-devnet.sqlite");
    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY, std::time::SystemTime::now()).unwrap(),
        0
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// Only a well-formed UTC timestamp counts as a creation time. A malformed name is
/// not an upstream snapshot at all, so pruning never touches it however old it is.
#[test]
fn platform_compatibility_prune_rejects_malformed_upstream_timestamps() {
    assert_eq!(
        upstream_backup_timestamp(
            Path::new("det-testnet.sqlite"),
            "pre-migration-det-testnet-1-to-2-20200101T000000Z.db"
        ),
        Some(
            chrono::NaiveDate::from_ymd_opt(2020, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .into()
        )
    );
    let malformed = [
        "pre-migration-det-testnet-1-to-2-20201301T000000Z.db",
        "pre-migration-det-testnet-1-to-2-20200101T000000.db",
        "pre-migration-det-testnet-1-to-2-2020-01-01.db",
        "pre-migration-det-testnet-1-to-2-.db",
    ];
    for name in malformed {
        assert_eq!(
            upstream_backup_timestamp(Path::new("det-testnet.sqlite"), name),
            None,
            "{name}"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("det-testnet.sqlite");
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    for name in malformed {
        let file = auto.join(name);
        std::fs::write(&file, b"backup").unwrap();
        set_mtime(&file, -400 * 24 * 60 * 60);
    }
    assert_eq!(
        prune_expired_backups(&path, RETENTION_DAY, std::time::SystemTime::now()).unwrap(),
        0
    );
    for name in malformed {
        assert!(auto.join(name).exists(), "{name}");
    }
}

/// A child forked by another thread briefly shares the lock file's open description,
/// so a lock this thread just released can still look held; the grace period must
/// absorb that instead of reporting the database as busy.
#[test]
fn platform_compatibility_lock_survives_concurrent_process_spawns() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wallet.sqlite");
    let stop = AtomicBool::new(false);
    let spawned = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let _ = std::process::Command::new(std::env::current_exe().unwrap())
                        .arg("--help")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                    spawned.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        // Keep re-acquiring while at least 40 processes are spawned alongside.
        let mut outcome = Ok(());
        while outcome.is_ok() && spawned.load(Ordering::Relaxed) < 40 {
            outcome = backup_lock(&path).map(drop);
        }
        stop.store(true, Ordering::Relaxed);
        outcome.expect("a released lock must be re-acquirable while processes spawn");
    });
}
