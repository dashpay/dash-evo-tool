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
fn platform_compatibility_repeated_failed_rebuilds_keep_one_backup() {
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
    let backups = backup_files(dir.path());
    assert_eq!(
        backups.len(),
        1,
        "Retried upgrades must not accumulate backups."
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
            "det-testnet.sqlite.platform-upgrade.lock",
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

#[test]
fn platform_compatibility_prune_failure_preserves_original() {
    let (dir, path, target) = fixture();
    let before = snapshot(&path);
    std::fs::create_dir(
        dir.path()
            .join("wallet.sqlite.platform-67d4ef3-backup-blocked.sqlite"),
    )
    .unwrap();
    assert!(upgrade(&path, &target, |_| Ok(())).is_err());
    assert_eq!(snapshot(&path), before);
}

#[cfg(unix)]
#[test]
fn platform_compatibility_rejects_backup_links_to_live_databases() {
    let (dir, path, target) = fixture();
    for extension in ["sqlite", "pending"] {
        let candidate = dir.path().join(format!(
            "wallet.sqlite.platform-67d4ef3-backup-link.{extension}"
        ));
        for live in [&path, &target] {
            std::os::unix::fs::symlink(live, &candidate).unwrap();
            assert!(remove_backups(&path).is_err());
            assert!(live.exists());
            assert!(candidate.symlink_metadata().is_ok());
            std::fs::remove_file(&candidate).unwrap();
            std::fs::hard_link(live, &candidate).unwrap();
            assert!(remove_backups(&path).is_err());
            assert!(live.exists());
            std::fs::remove_file(&candidate).unwrap();
        }
    }
    let live_alias = dir.path().join("alias.sqlite");
    let live_target = dir
        .path()
        .join("alias.sqlite.platform-67d4ef3-backup-live.sqlite");
    std::fs::write(&live_target, b"live database").unwrap();
    std::os::unix::fs::symlink(&live_target, &live_alias).unwrap();
    assert!(remove_backups(&live_alias).is_err());
    assert!(live_target.exists());
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
    retain_one_backup(&path, None).unwrap();
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
fn platform_compatibility_transient_sqlite_errors_can_retry() {
    for code in [rusqlite::ffi::SQLITE_IOERR, rusqlite::ffi::SQLITE_NOMEM] {
        let source = UpgradeError::from(sqlite_failure(code));
        assert!(source.is_retryable(), "{source:?}");
        assert!(!crate::backend_task::is_terminal_storage_open_error(
            &crate::backend_task::error::TaskError::PlatformDatabaseUpgrade { source }
        ));
    }
}

#[test]
fn platform_compatibility_deterministic_io_errors_do_not_retry() {
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::InvalidInput,
    ] {
        let source = UpgradeError::from(std::io::Error::from(kind));
        assert!(!source.is_retryable(), "{source:?}");
    }
}

#[test]
fn platform_compatibility_transient_io_errors_preserve_causes() {
    use std::io::ErrorKind;
    for kind in [
        ErrorKind::Interrupted,
        ErrorKind::WouldBlock,
        ErrorKind::TimedOut,
        ErrorKind::ResourceBusy,
        ErrorKind::StorageFull,
        ErrorKind::OutOfMemory,
    ] {
        let error = UpgradeError::from(std::io::Error::from(kind));
        assert!(error.is_retryable(), "{error:?}");
        let cause = std::error::Error::source(&error).unwrap();
        assert_eq!(cause.downcast_ref::<std::io::Error>().unwrap().kind(), kind);
        if kind == ErrorKind::OutOfMemory {
            assert!(error.to_string().contains("Close other applications"));
        }
    }
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
        let retained = retain_one_backup(&path, None);
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
            retain_one_backup(&path, None).unwrap();
            assert!(!pending.exists());
            continue;
        }
        assert!(status.success());
        let published = pending.with_extension("sqlite");
        assert_eq!(snapshot(&published), snapshot(&path));
        retain_one_backup(&path, None).unwrap();
        assert!(published.exists());
        remove_backups(&path).unwrap();
        assert!(!published.exists());
    }
}
