#![expect(
    clippy::disallowed_methods,
    reason = "test fixture setup/teardown outside any production deletion path"
)]

use super::*;

/// Every protected base name in a data directory, as the app lays them out.
fn protected_files(data_dir: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        data_dir.join("det-app.sqlite"),
        data_dir.join("data.db"),
        data_dir.join(".env"),
        data_dir.join("secrets/det-secrets.pwsvault"),
        data_dir.join("data.db.premigration"),
    ];
    for network in crate::wallet_backend::all_networks() {
        files.push(crate::wallet_backend::wallet_database_path(
            data_dir, network,
        ));
        files.push(crate::wallet_backend::shielded_database_path(
            data_dir, network,
        ));
    }
    let with_sidecars: Vec<PathBuf> = files
        .iter()
        .flat_map(|file| {
            SQLITE_SIDECAR_SUFFIXES.iter().map(move |suffix| {
                let mut name = file.as_os_str().to_owned();
                name.push(suffix);
                PathBuf::from(name)
            })
        })
        .collect();
    files.extend(with_sidecars);
    for file in &files {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"live").unwrap();
    }
    files
}

/// Every intent, each scoped to reach `data_dir` as widely as it can.
fn intents<'a>(data_dir: &'a Path, app_db: &'a Path) -> Vec<DeletionIntent<'a>> {
    vec![
        DeletionIntent::Backup { database: app_db },
        DeletionIntent::NetworkClear {
            data_dir,
            network: Network::Testnet,
        },
        DeletionIntent::Log {
            dir: data_dir,
            stem: "det",
        },
    ]
}

fn refusal(error: &std::io::Error) -> &DeletionRefused {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<DeletionRefused>())
        .unwrap_or_else(|| panic!("expected a refusal, got {error:?}"))
}

fn assert_protected(result: std::io::Result<()>, what: &str) {
    let error = result.expect_err(what);
    assert!(
        matches!(refusal(&error), DeletionRefused::ProtectedFile { .. }),
        "{what}: {error:?}"
    );
}

#[test]
fn delete_file_refuses_every_protected_file_under_every_intent() {
    let dir = tempfile::tempdir().unwrap();
    let files = protected_files(dir.path());
    let app_db = dir.path().join("det-app.sqlite");
    for intent in intents(dir.path(), &app_db) {
        for file in &files {
            assert_protected(delete_file(file, intent), &format!("{intent:?} {file:?}"));
            assert!(file.exists(), "{file:?} must survive");
        }
    }
}

#[test]
fn delete_file_refuses_protected_names_in_any_directory() {
    let dir = tempfile::tempdir().unwrap();
    // A protected name inside the network-clear scope is still refused.
    let spv = dir.path().join("spv/testnet");
    std::fs::create_dir_all(&spv).unwrap();
    for name in [
        "det-testnet.sqlite",
        "DET-MAINNET.SQLITE-wal",
        "det-app.sqlite-journal",
        "wallet.db.premigration",
        "other.pwsvault",
    ] {
        let file = spv.join(name);
        std::fs::write(&file, b"live").unwrap();
        assert_protected(
            delete_file(
                &file,
                DeletionIntent::NetworkClear {
                    data_dir: dir.path(),
                    network: Network::Testnet,
                },
            ),
            name,
        );
        assert!(file.exists(), "{name} must survive");
    }
}

#[test]
fn delete_file_refuses_protected_files_via_dot_dot_paths() {
    let dir = tempfile::tempdir().unwrap();
    let files = protected_files(dir.path());
    std::fs::create_dir_all(dir.path().join("backups/auto")).unwrap();
    let app_db = dir.path().join("det-app.sqlite");
    for intent in intents(dir.path(), &app_db) {
        for file in &files {
            let relative = file.strip_prefix(dir.path()).unwrap();
            let alias = dir.path().join("backups/auto/../..").join(relative);
            assert_protected(delete_file(&alias, intent), &format!("{alias:?}"));
            assert!(file.exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn delete_file_refuses_protected_files_via_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let files = protected_files(dir.path());
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    // A symlink to the live file, named like an ordinary backup.
    let link = backups.join("data_backup_20000101_000000.db");
    // A symlinked directory aliasing the data directory.
    let dir_link = dir.path().join("spv");
    std::os::unix::fs::symlink(dir.path(), &dir_link).unwrap();
    for file in &files {
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(file, &link).unwrap();
        let database = dir.path().join("data.db");
        let error = delete_file(
            &link,
            DeletionIntent::Backup {
                database: &database,
            },
        )
        .expect_err("a symlink must be refused");
        assert!(matches!(
            refusal(&error),
            DeletionRefused::NotRegularFile { .. }
        ));

        let through_dir = dir_link.join(file.strip_prefix(dir.path()).unwrap());
        for intent in intents(dir.path(), &database) {
            assert_protected(
                delete_file(&through_dir, intent),
                &format!("{through_dir:?}"),
            );
        }
        assert!(file.exists());
    }
}

#[cfg(unix)]
#[test]
fn delete_file_refuses_hard_links_to_protected_files() {
    let dir = tempfile::tempdir().unwrap();
    let files = protected_files(dir.path());
    let auto = dir.path().join("backups/auto");
    std::fs::create_dir_all(&auto).unwrap();
    let alias = auto.join("pre-migration-det-app-1-to-2-20000101T000000Z.db");
    let database = dir.path().join("det-app.sqlite");
    for file in &files {
        let _ = std::fs::remove_file(&alias);
        std::fs::hard_link(file, &alias).unwrap();
        let error = delete_file(
            &alias,
            DeletionIntent::Backup {
                database: &database,
            },
        )
        .expect_err("a hard link to a protected file must be refused");
        // Files protected by exact location are matched by inode; the rest
        // (e.g. `*.premigration`, protected by name only) by the link count.
        assert!(
            matches!(
                refusal(&error),
                DeletionRefused::ProtectedFile { .. } | DeletionRefused::HardLinked { .. }
            ),
            "hard link to {file:?}: {error:?}"
        );
        assert!(alias.exists() && file.exists());
    }
}

#[cfg(unix)]
#[test]
fn delete_file_refuses_any_hard_linked_file() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("det-app.sqlite");
    let backup = dir
        .path()
        .join("det-app.sqlite.platform-67d4ef3-backup-a.sqlite");
    std::fs::write(&backup, b"copy").unwrap();
    std::fs::hard_link(&backup, dir.path().join("elsewhere")).unwrap();
    let error = delete_file(
        &backup,
        DeletionIntent::Backup {
            database: &database,
        },
    )
    .unwrap_err();
    assert!(matches!(
        refusal(&error),
        DeletionRefused::HardLinked { .. }
    ));
    assert!(backup.exists());
}

#[test]
fn delete_file_refuses_targets_outside_the_intent_scope() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("det-app.sqlite");
    let legacy = dir.path().join("data.db");
    let cases = [
        (
            dir.path()
                .join("elsewhere/det-app.sqlite.platform-67d4ef3-backup-a.sqlite"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path().join("spv/mainnet/peers.dat"),
            DeletionIntent::NetworkClear {
                data_dir: dir.path(),
                network: Network::Testnet,
            },
        ),
        (
            dir.path().join("spv/mainnet.lock"),
            DeletionIntent::NetworkClear {
                data_dir: dir.path(),
                network: Network::Testnet,
            },
        ),
        (
            dir.path().join("notes.txt"),
            DeletionIntent::NetworkClear {
                data_dir: dir.path(),
                network: Network::Testnet,
            },
        ),
        (
            dir.path().join("app.ron"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            crate::wallet_backend::platform_compatibility::backup_lock_path(&database).unwrap(),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path()
                .join("det-mainnet.sqlite.platform-67d4ef3-backup-a.sqlite"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path().join("backups/data_backup_20000101_000000.db"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path()
                .join("backups/data_backup_20000101_000000.db.a1B2c3.pending"),
            DeletionIntent::Backup { database: &legacy },
        ),
        (
            dir.path()
                .join("backups/auto/pre-migration-det-mainnet-1-to-2-20000101T000000Z.db"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path().join("det.log"),
            DeletionIntent::Log {
                dir: dir.path(),
                stem: "det",
            },
        ),
        (
            dir.path().join("other.0000000001.log"),
            DeletionIntent::Log {
                dir: dir.path(),
                stem: "det",
            },
        ),
        (
            dir.path().join("logs/det.0000000001.log"),
            DeletionIntent::Log {
                dir: dir.path(),
                stem: "det",
            },
        ),
    ];
    for (file, intent) in cases {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"keep").unwrap();
        let error = delete_file(&file, intent).unwrap_err();
        assert!(
            matches!(refusal(&error), DeletionRefused::OutsideScope { .. }),
            "{file:?}: {error:?}"
        );
        assert!(file.exists(), "{file:?}");
    }
}

#[test]
fn delete_file_deletes_targets_in_scope() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("det-testnet.sqlite");
    let legacy = dir.path().join("data.db");
    let cases = [
        (
            dir.path()
                .join("det-testnet.sqlite.platform-67d4ef3-backup-a.sqlite"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path()
                .join("det-testnet.sqlite.platform-67d4ef3-backup-b.pending"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path().join("backups/data_backup_20000101_000000.db"),
            DeletionIntent::Backup { database: &legacy },
        ),
        (
            dir.path()
                .join("backups/auto/pre-migration-det-testnet-1-to-2-20000101T000000Z.db"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path().join("spv/testnet/filters/segment.dat"),
            DeletionIntent::NetworkClear {
                data_dir: dir.path(),
                network: Network::Testnet,
            },
        ),
        (
            dir.path().join("spv/testnet.lock"),
            DeletionIntent::NetworkClear {
                data_dir: dir.path(),
                network: Network::Testnet,
            },
        ),
        (
            dir.path().join("det.0000000001.log"),
            DeletionIntent::Log {
                dir: dir.path(),
                stem: "det",
            },
        ),
    ];
    for (file, intent) in cases {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"expendable").unwrap();
        delete_file(&file, intent).unwrap_or_else(|e| panic!("{file:?}: {e:?}"));
        assert!(!file.exists(), "{file:?}");
    }
}

#[test]
fn delete_file_reports_a_missing_target_as_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let error = delete_file(
        &dir.path().join("spv/testnet/peers.dat"),
        DeletionIntent::NetworkClear {
            data_dir: dir.path(),
            network: Network::Testnet,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn delete_file_refuses_directories() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("spv/testnet/blocks");
    std::fs::create_dir_all(&target).unwrap();
    let error = delete_file(
        &target,
        DeletionIntent::NetworkClear {
            data_dir: dir.path(),
            network: Network::Testnet,
        },
    )
    .unwrap_err();
    assert!(matches!(
        refusal(&error),
        DeletionRefused::NotRegularFile { .. }
    ));
    assert!(target.is_dir());
}

#[test]
fn delete_tree_removes_a_cache_tree_but_stops_at_a_protected_file() {
    let dir = tempfile::tempdir().unwrap();
    let intent = DeletionIntent::NetworkClear {
        data_dir: dir.path(),
        network: Network::Testnet,
    };
    let cache = dir.path().join("spv/testnet/blocks");
    std::fs::create_dir_all(cache.join("nested")).unwrap();
    std::fs::write(cache.join("a.dat"), b"x").unwrap();
    std::fs::write(cache.join("nested/b.dat"), b"x").unwrap();
    delete_tree(&cache, intent).unwrap();
    assert!(!cache.exists());

    std::fs::create_dir_all(&cache).unwrap();
    let planted = cache.join("det-app.sqlite");
    std::fs::write(&planted, b"live").unwrap();
    assert_protected(delete_tree(&cache, intent), "planted live database");
    assert!(planted.exists());
}

/// A network clear unlinks a symlinked cache entry itself (e.g. chain data moved to
/// another disk) without following it; other intents still refuse symlinks.
#[cfg(unix)]
#[test]
fn network_clear_unlinks_symlinks_without_following_them() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.dat"), b"keep").unwrap();
    let spv = dir.path().join("spv/testnet");
    std::fs::create_dir_all(&spv).unwrap();
    let intent = DeletionIntent::NetworkClear {
        data_dir: dir.path(),
        network: Network::Testnet,
    };
    let dir_link = spv.join("block_headers");
    std::os::unix::fs::symlink(outside.path(), &dir_link).unwrap();
    let file_link = spv.join("peers.dat");
    std::os::unix::fs::symlink(outside.path().join("keep.dat"), &file_link).unwrap();
    for link in [&dir_link, &file_link] {
        delete_file(link, intent).unwrap_or_else(|e| panic!("{link:?}: {e:?}"));
        assert!(
            link.symlink_metadata().is_err(),
            "{link:?} must be unlinked"
        );
    }
    assert!(outside.path().join("keep.dat").exists());

    let backup_link = dir
        .path()
        .join("det-app.sqlite.platform-67d4ef3-backup-a.sqlite");
    std::os::unix::fs::symlink(outside.path().join("keep.dat"), &backup_link).unwrap();
    let database = dir.path().join("det-app.sqlite");
    let error = delete_file(
        &backup_link,
        DeletionIntent::Backup {
            database: &database,
        },
    )
    .unwrap_err();
    assert!(matches!(
        refusal(&error),
        DeletionRefused::NotRegularFile { .. }
    ));
}

#[test]
fn delete_tree_refuses_directories_outside_its_scope() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("det-app.sqlite");
    let clear = DeletionIntent::NetworkClear {
        data_dir: dir.path(),
        network: Network::Testnet,
    };
    let cases = [
        (dir.path().join("spv/testnet"), clear),
        (dir.path().join("spv/mainnet/blocks"), clear),
        (
            dir.path().join("backups/auto"),
            DeletionIntent::Backup {
                database: &database,
            },
        ),
        (
            dir.path().join("logs"),
            DeletionIntent::Log {
                dir: dir.path(),
                stem: "det",
            },
        ),
    ];
    for (target, intent) in cases {
        std::fs::create_dir_all(&target).unwrap();
        let error = delete_tree(&target, intent).unwrap_err();
        assert!(
            matches!(refusal(&error), DeletionRefused::OutsideScope { .. }),
            "{target:?}: {error:?}"
        );
        assert!(target.is_dir(), "{target:?}");
    }
}

#[test]
fn delete_tree_handles_deep_trees_without_recursion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("spv/testnet/filters");
    // Short components keep the full path within OS limits.
    let mut deepest = root.clone();
    for _ in 0..1000 {
        deepest.push("d");
    }
    std::fs::create_dir_all(&deepest).unwrap();
    std::fs::write(deepest.join("x.dat"), b"x").unwrap();
    delete_tree(
        &root,
        DeletionIntent::NetworkClear {
            data_dir: dir.path(),
            network: Network::Testnet,
        },
    )
    .unwrap();
    assert!(!root.exists());
}

#[test]
fn windows_alias_names_are_recognised() {
    for name in [
        "DET-AP~1.SQL",
        "det-app.sqlite::$DATA",
        "det-app.sqlite.",
        "det-app.sqlite ",
    ] {
        assert!(is_windows_alias_name(name), "{name}");
    }
    for name in [
        "det-app.sqlite.platform-67d4ef3-backup-a1.sqlite",
        "pre-migration-det-app-1-to-2-20000101T000000Z.db",
        "det.0000000001.log",
    ] {
        assert!(!is_windows_alias_name(name), "{name}");
    }
}

#[cfg(unix)]
#[test]
fn delete_tree_never_follows_a_symlinked_directory() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.dat"), b"keep").unwrap();
    let cache = dir.path().join("spv/testnet/blocks");
    std::fs::create_dir_all(&cache).unwrap();
    std::os::unix::fs::symlink(&outside, cache.join("link")).unwrap();
    let intent = DeletionIntent::NetworkClear {
        data_dir: dir.path(),
        network: Network::Testnet,
    };
    let error = delete_tree(&cache.join("link"), intent).unwrap_err();
    assert!(matches!(
        refusal(&error),
        DeletionRefused::NotRegularFile { .. }
    ));
    // Inside the tree the link itself is unlinked; its target is never entered.
    delete_tree(&cache, intent).unwrap();
    assert!(!cache.exists());
    assert!(outside.join("keep.dat").exists());
}

#[test]
fn rotated_log_names_are_exact() {
    let rotated = |name: &str, stem: &str| crate::logging::parse_rotated_ts(name, stem).is_some();
    assert!(rotated("det.0000000001.log", "det"));
    for name in [
        "det.log",
        "det..log",
        "det.12a.log",
        "det.-5.log",
        "det.1.log.bak",
        "detx.1.log",
        "other.1.log",
    ] {
        assert!(!rotated(name, "det"), "{name}");
    }
    assert!(!rotated(".1.log", ""));
}
