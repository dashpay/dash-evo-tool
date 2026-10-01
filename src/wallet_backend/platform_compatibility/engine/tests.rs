use super::*;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wallet.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch(OLD_SCHEMA)
        .unwrap();
    let target = dir.path().join("target.sqlite");
    Connection::open(&target)
        .unwrap()
        .execute_batch(TARGET_SCHEMA)
        .unwrap();
    (dir, path, target)
}

#[test]
fn platform_compatibility_upgrades_old_empty_database() {
    let (_dir, path, target) = fixture();
    let backup = upgrade(&path, &target, |_| Ok(())).unwrap().unwrap();
    assert_eq!(
        history(&Connection::open(&backup).unwrap()).unwrap(),
        history(&old_reference().unwrap()).unwrap()
    );
    assert_eq!(
        objects(&Connection::open(&path).unwrap()).unwrap(),
        objects(&target_reference().unwrap()).unwrap()
    );
    assert!(
        upgrade(&path, &target, |_| panic!("Already current"))
            .unwrap()
            .is_none()
    );
}

fn snapshot(path: &Path) -> String {
    let conn = Connection::open(path).unwrap();
    let mut text = format!("{:?}", objects(&conn).unwrap());
    for o in objects(&conn).unwrap().iter().filter(|o| o.kind == "table") {
        let cols = columns(&conn, &o.name).unwrap();
        let order = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT * FROM {} ORDER BY {order}",
                quote(&o.name)
            ))
            .unwrap();
        for row in stmt
            .query_map([], |r| {
                (0..cols.len())
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
        {
            text.push_str(&format!("{:?}", row.unwrap()));
        }
    }
    text
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
fn platform_compatibility_preserves_active_tombstones_and_all_local_metadata() {
    let (dir, path, target) = fixture();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO wallets VALUES (zeroblob(32), 'testnet', 0)",
        [],
    )
    .unwrap();
    let ids = [[1_u8; 32], [2_u8; 32], [3_u8; 32]];
    for (i, id) in ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO identities VALUES (?1, NULL, NULL, ?2, ?3)",
            rusqlite::params![id.as_slice(), &[0_u8, 255, i as u8], i != 2],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meta_identity VALUES (?1, 'det:identity:v1', ?2, 31)",
            rusqlite::params![id.as_slice(), &[0_u8, 255, i as u8]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meta_token VALUES (?1, ?2, 'det:token:v1', X'00ff80', 32)",
            rusqlite::params![id.as_slice(), [5_u8; 32].as_slice()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ignored_senders VALUES (zeroblob(32), ?1, zeroblob(32), 30)",
            [id.as_slice()],
        )
        .unwrap();
    }
    let mut index = vec![1];
    index.extend(bincode::serde::encode_to_vec(vec![ids[0]], bincode::config::standard()).unwrap());
    conn.execute(
        "INSERT INTO meta_global VALUES (?1, ?2, 33)",
        rusqlite::params![IDENTITY_INDEX_KEY, index],
    )
    .unwrap();
    let before = snapshot(&path);
    let backup = upgrade(&path, &target, |_| Ok(())).unwrap().unwrap();
    assert_eq!(snapshot(&backup), before);
    let migrated = Connection::open(&path).unwrap();
    let found = migrated
        .prepare("SELECT identity_id FROM identities ORDER BY identity_id")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(found, vec![ids[0].to_vec(), ids[2].to_vec()]);
    for table in [
        "meta_global",
        "meta_identity",
        "meta_token",
        "meta_store_generation",
    ] {
        equal_rows(
            &Connection::open(&backup).unwrap(),
            &migrated,
            table,
            &columns(&migrated, table).unwrap(),
            "",
        )
        .unwrap();
    }
    assert_eq!(
        migrated
            .query_row("SELECT count(*) FROM ignored_senders", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(
        upgrade(&path, &target, |_| panic!("Already upgraded"))
            .unwrap()
            .is_none()
    );
    assert_eq!(backup_files(dir.path()).len(), 1);
}

#[test]
fn platform_compatibility_maps_version_domains_with_maximum_collision() {
    let (_dir, path, target) = fixture();
    Connection::open(&path).unwrap().execute_batch("INSERT INTO meta_data_versions VALUES (X'01', 'wallet_metadata', 17), (X'01', 'wallets', 3), (X'01', 'account_address_pools', 9), (X'02', 'wallet_metadata', 4), (X'02', 'wallets', 30);").unwrap();
    upgrade(&path, &target, |_| Ok(())).unwrap();
    let conn = Connection::open(&path).unwrap();
    for (id, domain, seq) in [
        (1, "wallets", 17),
        (1, "core_address_pool", 9),
        (2, "wallets", 30),
        (1, "wallet_metadata", 17),
        (1, "account_address_pools", 9),
    ] {
        assert_eq!(
            conn.query_row(
                "SELECT seq FROM meta_data_versions WHERE wallet_id = ?1 AND domain = ?2",
                rusqlite::params![vec![id as u8], domain],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            seq
        );
    }
}

#[test]
fn platform_compatibility_failure_before_commit_restores_original_and_keeps_backup() {
    let (dir, path, target) = fixture();
    let before = snapshot(&path);
    let error = upgrade_with_hook(
        &path,
        &target,
        |_| Ok(()),
        || Err(UpgradeError::Verification),
    )
    .unwrap_err();
    assert!(matches!(error, UpgradeError::Verification));
    assert_eq!(snapshot(&path), before);
    let backups = backup_files(dir.path());
    assert_eq!(backups.len(), 1);
    assert_eq!(snapshot(&backups[0]), before);
    // A restart uses a fresh stage, so an interrupted attempt cannot poison the next one.
    let next = dir.path().join("next.sqlite");
    Connection::open(&next)
        .unwrap()
        .execute_batch(TARGET_SCHEMA)
        .unwrap();
    assert!(upgrade(&path, &next, |_| Ok(())).unwrap().is_some());
}

#[test]
fn platform_compatibility_typed_validation_failure_never_rewrites_original() {
    let (dir, path, target) = fixture();
    let before = snapshot(&path);
    let error = upgrade(&path, &target, |_| {
        Err(UpgradeError::TypedValidation(Box::new(
            std::io::Error::other("synthetic invalid public blob"),
        )))
    })
    .unwrap_err();
    assert!(matches!(error, UpgradeError::TypedValidation(_)));
    assert_eq!(snapshot(&path), before);
    assert!(
        backup_files(dir.path()).is_empty(),
        "A failure before the rebuild leaves the original untouched, so no backup may remain."
    );
}

#[test]
fn platform_compatibility_repeated_failed_rebuilds_are_deduplicated() {
    let (dir, path, _target) = fixture();
    let before = snapshot(&path);
    for attempt in 0..3 {
        let stage = dir.path().join(format!("stage-{attempt}.sqlite"));
        Connection::open(&stage)
            .unwrap()
            .execute_batch(TARGET_SCHEMA)
            .unwrap();
        let error = upgrade_with_hook(
            &path,
            &stage,
            |_| Ok(()),
            || Err(UpgradeError::Verification),
        )
        .unwrap_err();
        assert!(matches!(error, UpgradeError::Verification));
    }
    assert_eq!(
        backup_files(dir.path()).len(),
        3,
        "publication never deletes an earlier snapshot"
    );
    tidy_backups(&path, None).unwrap();
    let backups = backup_files(dir.path());
    assert_eq!(
        backups.len(),
        1,
        "identical copies of the unchanged original collapse to one"
    );
    assert_eq!(snapshot(&backups[0]), before);
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

/// The previous recovery snapshot must survive until its replacement is
/// published: a failed publication must not leave zero snapshots behind.
#[test]
fn platform_compatibility_failed_publication_keeps_previous_snapshot() {
    let (_dir, path, _target) = fixture();
    let previous = backup(&path).unwrap();
    let error = backup_with_hook(&path, |pending| {
        // Occupy the name the replacement would be published under.
        std::fs::write(pending.with_extension("sqlite"), b"collision")?;
        Ok(())
    })
    .unwrap_err();
    assert!(
        matches!(&error, UpgradeError::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists),
        "{error:?}"
    );
    assert!(
        previous.exists(),
        "the published snapshot must survive a failed replacement"
    );
    assert_eq!(snapshot(&previous), snapshot(&path));
}

/// Publication keeps earlier snapshots: only retention and tidying remove them.
#[test]
fn platform_compatibility_publication_keeps_earlier_snapshots() {
    let (dir, path, _target) = fixture();
    let previous = backup(&path).unwrap();
    let replacement = backup(&path).unwrap();
    assert_ne!(previous, replacement);
    assert!(previous.exists());
    assert!(replacement.exists());
    assert_eq!(backup_files(dir.path()).len(), 2);
}

/// Tidying removes a snapshot only when a newer one of the same migration has the
/// same bytes; a snapshot that differs by even one byte, or precedes another
/// migration, is never removed.
#[test]
fn platform_compatibility_tidy_removes_only_identical_copies() {
    let (dir, path, _target) = fixture();
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let original = backup(&path).unwrap();
    let copy = backup(&path).unwrap();
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
    let (dir, path, _target) = fixture();
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
    let (dir, path, _target) = fixture();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::write(&elsewhere, b"keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, backup_lock_path(&path).unwrap()).unwrap();
    assert!(backup_lock(&path).is_err());
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"keep");
}

fn sqlite_failure(code: std::ffi::c_int) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
}

#[test]
fn platform_compatibility_errors_name_their_actual_cause() {
    let in_use = [
        UpgradeError::from(sqlite_failure(rusqlite::ffi::SQLITE_BUSY)),
        UpgradeError::from(sqlite_failure(rusqlite::ffi::SQLITE_LOCKED)),
    ];
    for error in &in_use {
        assert!(matches!(error, UpgradeError::InUse(_)), "{error:?}");
        assert!(error.is_retryable());
        let text = error.to_string();
        assert!(text.contains("another Dash Evo Tool"), "{text}");
        assert!(!text.contains("disk"), "{text}");
    }
    let full = [
        UpgradeError::from(sqlite_failure(rusqlite::ffi::SQLITE_FULL)),
        UpgradeError::from(std::io::Error::from(std::io::ErrorKind::StorageFull)),
    ];
    for error in &full {
        assert!(matches!(error, UpgradeError::StorageFull(_)), "{error:?}");
        assert!(error.is_retryable());
        assert!(error.to_string().contains("disk"));
    }
    let io = UpgradeError::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    assert!(matches!(io, UpgradeError::AccessDenied(_)) && !io.is_retryable());
    assert!(!io.to_string().contains("disk"), "{io}");

    let generic = UpgradeError::from(sqlite_failure(rusqlite::ffi::SQLITE_CORRUPT));
    assert!(matches!(generic, UpgradeError::Sqlite(_)));
    assert!(!generic.is_retryable());
    assert!(!generic.to_string().contains("disk"), "{generic}");
    for error in [
        std::error::Error::source(&in_use[0]),
        std::error::Error::source(&full[1]),
        std::error::Error::source(&generic),
    ] {
        assert!(
            error.is_some(),
            "The technical cause must stay in the source chain."
        );
    }

    for error in [
        UpgradeError::Unrecognized,
        UpgradeError::IdentityRoster { source: None },
        UpgradeError::Verification,
        UpgradeError::TypedValidation(Box::new(std::io::Error::other("fixture"))),
    ] {
        assert!(!error.is_retryable(), "{error:?}");
    }
}

#[test]
fn platform_compatibility_malformed_roster_reports_one_message() {
    let (_dir, path, target) = fixture();
    Connection::open(&path)
        .unwrap()
        .execute_batch("INSERT INTO meta_global VALUES ('det:identity_index:v1', X'01ff', 0)")
        .unwrap();
    let decode = upgrade(&path, &target, |_| Ok(())).unwrap_err();
    assert!(
        matches!(decode, UpgradeError::IdentityRoster { source: Some(_) }),
        "{decode:?}"
    );
    assert_eq!(
        decode.to_string(),
        UpgradeError::IdentityRoster { source: None }.to_string()
    );
}

#[test]
fn platform_compatibility_rejects_unknown_history_schema_and_roster_without_changes() {
    for sql in [
        "UPDATE refinery_schema_history SET checksum = '1' WHERE version = 4",
        "ALTER TABLE wallets ADD COLUMN unknown TEXT",
        "INSERT INTO meta_global VALUES ('det:identity_index:v1', X'02ff', 0)",
        "INSERT INTO meta_global VALUES ('det:identity_index:v1', X'01ff', 0)",
        "INSERT INTO meta_global VALUES ('det:identity_index:v1', X'010000', 0)",
    ] {
        let (dir, path, target) = fixture();
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        let before = snapshot(&path);
        assert!(!matches!(
            upgrade(&path, &target, |_| panic!(
                "Unknown input must not reach validation"
            )),
            Ok(Some(_))
        ));
        assert_eq!(snapshot(&path), before);
        assert!(backup_files(dir.path()).is_empty());
    }
}

#[test]
fn platform_compatibility_fresh_database_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.sqlite");
    Connection::open(&path).unwrap();
    let before = snapshot(&path);
    assert!(
        upgrade(&path, &dir.path().join("unused.sqlite"), |_| panic!(
            "Fresh database"
        ))
        .unwrap()
        .is_none()
    );
    assert_eq!(snapshot(&path), before);
    assert!(backup_files(dir.path()).is_empty());
}

#[test]
fn platform_compatibility_wal_snapshot_includes_committed_uncheckpointed_data() {
    let (_dir, path, target) = fixture();
    let live = Connection::open(&path).unwrap();
    live.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO meta_global VALUES ('wal-fixture', X'00ff', 42)").unwrap();
    let before = snapshot(&path);
    let backup = upgrade(&path, &target, |_| Ok(())).unwrap().unwrap();
    assert!(
        crate::utils::backup_prune::usable_snapshot(&backup),
        "a snapshot of a WAL database must count as usable"
    );
    assert_eq!(snapshot(&backup), before);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT value FROM meta_global WHERE key='wal-fixture'",
            [],
            |r| r.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        [0, 255]
    );
}

#[test]
fn platform_compatibility_missing_roster_with_identity_sidecar_is_ambiguous() {
    let (dir, path, target) = fixture();
    Connection::open(&path).unwrap().execute_batch("INSERT INTO identities VALUES (zeroblob(32), NULL, NULL, X'00', 1); INSERT INTO meta_identity VALUES (zeroblob(32), 'det:identity:v1', X'00', 0);").unwrap();
    let before = snapshot(&path);
    assert!(matches!(
        upgrade(&path, &target, |_| Ok(())),
        Err(UpgradeError::IdentityRoster { source: None })
    ));
    assert_eq!(snapshot(&path), before);
    assert!(backup_files(dir.path()).is_empty());
}

#[test]
fn platform_compatibility_process_exit_rolls_back_and_keeps_valid_backup() {
    const CHILD_DIR: &str = "DET_PLATFORM_COMPAT_CRASH_FIXTURE_DIR";
    if let Some(dir) = std::env::var_os(CHILD_DIR) {
        let dir = PathBuf::from(dir);
        upgrade_with_hook(
            &dir.join("wallet.sqlite"),
            &dir.join("target.sqlite"),
            |_| Ok(()),
            || std::process::exit(77),
        )
        .unwrap();
        panic!("The crash hook must exit the process.");
    }
    let (dir, path, _target) = fixture();
    let before = snapshot(&path);
    let test_name = std::thread::current().name().unwrap().to_owned();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test_name, "--nocapture"])
        .env(CHILD_DIR, dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(77));
    assert_eq!(snapshot(&path), before);
    assert_eq!(snapshot(&backup_files(dir.path())[0]), before);
}

#[test]
fn platform_compatibility_writer_exclusion_covers_staged_validation() {
    let (_dir, path, target) = fixture();
    upgrade(&path, &target, |_| {
        let other = Connection::open(&path)?;
        other.busy_timeout(std::time::Duration::ZERO)?;
        let error = other.execute("INSERT INTO meta_global VALUES ('concurrent', X'00', 0)", []).unwrap_err();
        assert!(matches!(error, rusqlite::Error::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::DatabaseBusy));
        Ok(())
    }).unwrap();
}

#[test]
fn platform_compatibility_pending_cleanup_preserves_published_snapshot() {
    let (dir, path, _target) = fixture();
    let published = backup(&path).unwrap();
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
    let (dir, path, _target) = fixture();
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

#[test]
fn platform_compatibility_active_snapshot_survives_concurrent_cleanup() {
    use std::io::{BufRead, Read, Write};
    const CHILD_DIR: &str = "DET_PLATFORM_COMPAT_ACTIVE_FIXTURE_DIR";
    if let Some(dir) = std::env::var_os(CHILD_DIR) {
        let path = PathBuf::from(dir).join("wallet.sqlite");
        backup_with_hook(&path, |pending| {
            // Pause the real writer after creation, before it copies the database.
            println!("PENDING:{}", pending.display());
            std::io::stdout().flush()?;
            std::io::stdin().read_exact(&mut [0])?;
            Ok(())
        })
        .unwrap();
        return;
    }
    for crash in [false, true] {
        let (dir, path, _target) = fixture();
        let test_name = std::thread::current().name().unwrap().to_owned();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD_DIR, dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let pending = loop {
            let mut line = String::new();
            assert_ne!(
                reader.read_line(&mut line).unwrap(),
                0,
                "Writer exited before creating a snapshot"
            );
            if let Some((_, path)) = line.trim().split_once("PENDING:") {
                break PathBuf::from(path);
            }
        };
        let retained = tidy_backups(&path, None);
        let removed = remove_backups(&path);
        let exists = pending.exists();
        if crash {
            child.kill().unwrap();
        } else {
            child.stdin.take().unwrap().write_all(&[1]).unwrap();
        }
        let status = child.wait().unwrap();
        assert!(exists, "Cleanup deleted another process's active snapshot");
        assert_eq!(retained.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(removed.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        if crash {
            assert!(!status.success());
            assert!(pending.exists());
            tidy_backups(&path, None).unwrap();
            assert!(!pending.exists());
            continue;
        }
        assert!(status.success());
        let published = pending.with_extension("sqlite");
        assert_eq!(snapshot(&published), snapshot(&path));
        tidy_backups(&path, None).unwrap();
        assert!(published.exists());
        remove_backups(&path).unwrap();
        assert!(!published.exists());
    }
}

#[test]
fn platform_compatibility_one_guard_covers_retention_and_upgrade() {
    let (_dir, path, target) = fixture();
    let before = snapshot(&path);
    let guard = backup_lock(&path).unwrap();
    tidy_backups_locked(&guard, None).unwrap();
    let backup = upgrade_locked(&guard, &target, |_| Ok(()))
        .unwrap()
        .unwrap();
    tidy_backups_locked(&guard, None).unwrap();
    assert_eq!(snapshot(&backup), before);
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
    let (dir, path, _target) = fixture();
    let before = snapshot(&path);
    let valid = backup(&path).unwrap();
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
        assert_eq!(snapshot(&valid), before, "{case}");
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
    let (_dir, path, _target) = fixture();
    let older = backup(&path).unwrap();
    let newest = backup(&path).unwrap();
    set_mtime(&older, -200 * 24 * 60 * 60);
    set_mtime(&newest, -100 * 24 * 60 * 60);
    std::fs::set_permissions(&newest, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&newest).is_ok() {
        // Running as root: permissions cannot make it unreadable.
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
    let (_dir, path, _target) = fixture();
    let skewed = backup(&path).unwrap();
    let current = backup(&path).unwrap();
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
    let (dir, path, _target) = fixture();
    let valid = backup(&path).unwrap();
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
    let (_dir, path, _target) = fixture();
    let real = backup(&path).unwrap();
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
