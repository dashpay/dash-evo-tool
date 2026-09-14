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
    assert_eq!(snapshot(&backup_files(dir.path())[0]), before);
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
        Err(UpgradeError::IdentityRoster)
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
