//! Sentinel for the file-deletion chokepoint: every production deletion path
//! runs against a realistic data directory, and every file it does not target
//! must survive byte-for-byte.

use super::*;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn plant(path: &Path, age: Duration) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, path.to_string_lossy().as_bytes()).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - age)
        .unwrap();
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// Live files the testnet context itself writes while the operations run:
/// they must survive, but their bytes may legitimately change.
fn written_by_context(data_dir: &Path, path: &Path) -> bool {
    if path.starts_with(data_dir.join("secrets")) {
        return true;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let live = [
        "det-app.sqlite",
        "det-testnet.sqlite",
        "det-testnet-shielded.sqlite",
        "data.db",
    ];
    path.parent() == Some(data_dir)
        && live.iter().any(|base| {
            name == *base
                || ["-wal", "-shm", "-journal", ".platform-upgrade.lock"]
                    .iter()
                    .any(|suffix| name == format!("{base}{suffix}"))
        })
}

/// Retention prune, wallet removal, identity removal, log rotation, SPV-cache
/// clear and a full testnet network clear run against a data directory holding
/// every network's databases. Only the files each operation targets disappear;
/// every other file keeps its exact bytes, and the testnet clear never reaches
/// another network's data.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_paths_leave_every_untargeted_file_intact() {
    let (ctx, sender, tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender.clone())
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let dir = tmp.path();
    let old = DAY * 400;
    let fresh = DAY;

    // Other networks' live databases and sidecars: never deletable.
    let mut sentinels = Vec::new();
    for network in [Network::Mainnet, Network::Devnet, Network::Regtest] {
        let wallet = wallet_database_path(dir, network);
        let shielded = crate::wallet_backend::shielded_database_path(dir, network);
        for base in [wallet, shielded] {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut name = base.as_os_str().to_owned();
                name.push(suffix);
                sentinels.push(PathBuf::from(name));
            }
        }
    }
    // Other networks' SPV data and backups, config, a pre-migration copy,
    // current logs and a foreign-stem log.
    sentinels.extend(
        [
            "spv/mainnet/blocks/segment.dat",
            "spv/mainnet/peers.dat",
            "spv/mainnet/det-shielded.sqlite",
            "spv/mainnet.lock",
            "det-mainnet.sqlite.platform-67d4ef3-backup-old.sqlite",
            "backups/auto/pre-migration-det-mainnet-1-to-2-20000101T000000Z.db",
            "data.db.premigration",
            "det.log",
            "det-stderr.0000000001.log",
            "notes.txt",
        ]
        .map(|relative| dir.join(relative)),
    );
    for path in &sentinels {
        plant(path, old);
    }
    // The current log is rotated to a name from its mtime; keep it recent so the
    // rotated copy is retained rather than expiring in the same pass.
    plant(&dir.join("det.log"), fresh);

    let targeted: BTreeSet<PathBuf> = [
        // Retention prune: expired backups of the app and testnet databases.
        "det-app.sqlite.platform-67d4ef3-backup-old.sqlite",
        "det-testnet.sqlite.platform-67d4ef3-backup-old.sqlite",
        "backups/auto/pre-migration-det-testnet-1-to-2-20000101T000000Z.db",
        "backups/data_backup_20000101_000000.db",
        // Network clear: every remaining app/testnet upgrade backup.
        "det-app.sqlite.platform-67d4ef3-backup-new.sqlite",
        "det-testnet.sqlite.platform-67d4ef3-backup-new.sqlite",
        // Network clear (SPV cache + retired shielded files): testnet only.
        "spv/testnet/blocks/segment.dat",
        "spv/testnet/filters/nested/segment.dat",
        "spv/testnet/peers.dat",
        "spv/testnet.lock",
        "spv/testnet/det-shielded.sqlite",
        "spv/testnet/shielded-commitment-tree.sqlite",
        // Log rotation: an expired rotated log of the rotated stem.
        "det.0000000001.log",
    ]
    .map(|relative| dir.join(relative))
    .into_iter()
    .collect();
    for path in &targeted {
        let age = if path.to_string_lossy().contains("-new.") {
            fresh
        } else {
            old
        };
        plant(path, age);
    }

    // Wallet and identity to remove.
    let seed = [0xC4u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");
    let identity = wallet_owned_qualified_identity(None);
    ctx.insert_local_qualified_identity(&identity, &None)
        .expect("insert identity");

    let before = snapshot(dir);

    let report = ctx.prune_expired_upgrade_backups();
    assert!(report.failure.is_none(), "{:?}", report.failure);
    assert_eq!(report.deleted, 4, "retention prune");
    ctx.remove_wallet(&seed_hash).expect("remove wallet");
    ctx.delete_local_qualified_identity(&identity.identity.id())
        .expect("remove identity");
    crate::logging::rotate_log_in_dir(dir, "det");
    ctx.clear_spv_data().expect("clear SPV data");
    ctx.clear_network_database()
        .await
        .expect("clear network database");
    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;

    let after = snapshot(dir);
    for path in &targeted {
        assert!(!path.exists(), "{path:?} should have been deleted");
    }
    for (path, bytes) in &before {
        if targeted.contains(path) {
            continue;
        }
        if written_by_context(dir, path) {
            assert!(after.contains_key(path), "live file {path:?} was deleted");
        } else if path == &dir.join("det.log") {
            // Rotation moves the current log aside rather than deleting it.
            assert!(
                after.values().any(|after| after == bytes),
                "the current log's contents must survive rotation"
            );
        } else {
            assert_eq!(after.get(path), Some(bytes), "{path:?} must be unchanged");
        }
    }
    for path in &sentinels {
        if path != &dir.join("det.log") {
            assert!(path.exists(), "sentinel {path:?} missing");
        }
    }
}
