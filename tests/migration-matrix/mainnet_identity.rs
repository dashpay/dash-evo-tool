//! The captured mainnet profile must retain usable identity keys across upgrades.

use std::path::Path;
use std::sync::Arc;

use dash_evo_tool::backend_task::migration::finish_unwire::{
    MigrationCompletion, dapi_refresh_sentinel_key_for,
};
use dash_evo_tool::backend_task::{BackendTaskSuccessResult, migration::MigrationTask};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::database::Database;
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use dash_evo_tool::utils::egui_mpsc::SenderAsync;
use dash_evo_tool::utils::tasks::TaskManager;
use dash_evo_tool::wallet_backend::KV_SCHEMA_VERSION;
use dash_evo_tool::wallet_backend::{DetScope, IdentityKeyView};
use dash_sdk::dpp::dashcore::{Network, signer};
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::public_identities::StoredIdentity;
use crate::{DEFAULT_BOOT_TIMEOUT, assertions, cli, manifest::Fixture, stage};

const CAPTURE: &[u8] = include_bytes!("../migration-fixtures/v11-mainnet-identity/data.db");
const CAPTURE_WITH_IMPORTED_KEY: &[u8] =
    include_bytes!("../migration-fixtures/v11-mainnet-identity/data-with-imported-key.db");
// Operator-authorized public test key; see the fixture's PROVENANCE.md.
const IMPORTED_PRIVATE_KEY: &str =
    "394195a3ed1d404bd673484e1274a4c4d5040d46705cdca3337f3dbd999760af";

#[test]
fn mainnet_imported_key_fixture_is_the_authorized_capture() {
    assert_eq!(CAPTURE_WITH_IMPORTED_KEY.len(), 180_224);
    assert_eq!(
        hex::encode(Sha256::digest(CAPTURE_WITH_IMPORTED_KEY)),
        "23fc06e82df316dca56cb6ae338aaa470790e6e1cdf19f35cdcd777d1d9485d3"
    );
}

#[test]
fn mainnet_identity_fixture_is_the_authorized_capture() {
    assert_eq!(CAPTURE.len(), 180_224);
    assert_eq!(
        hex::encode(Sha256::digest(CAPTURE)),
        "81309e1fb1f8158aa1b5bb0b61a7c662d5b0926115cda629bc863496d91590c9"
    );
}

async fn open_context(dir: &Path) -> (Arc<AppContext>, Arc<TaskManager>) {
    let db = Database::new(dir.join("data.db")).unwrap();
    db.execute("PRAGMA query_only = ON", []).unwrap();
    let tasks = Arc::new(TaskManager::default());
    let app_kv = AppContext::open_app_kv(dir).unwrap();
    // Signing and restore are local checks; skip the unrelated node discovery.
    app_kv
        .put(
            DetScope::Global,
            &dapi_refresh_sentinel_key_for(Network::Mainnet),
            &MigrationCompletion {
                completed_at: 0,
                sha: "offline-signing-test".into(),
                network_count: 0,
            },
        )
        .unwrap();
    let context = AppContext::new(
        dir.to_path_buf(),
        Network::Mainnet,
        Arc::new(db),
        Arc::clone(&tasks),
        Default::default(),
        egui::Context::default(),
        app_kv,
        AppContext::open_secret_store(dir).unwrap(),
        Default::default(),
    )
    .expect("mainnet context");
    let (tx, _rx) = tokio::sync::mpsc::channel(128);
    let sender = SenderAsync::new(tx, context.egui_ctx().clone());
    context.prepare_storage(sender.clone()).await.unwrap();
    // Join the queued discovery no-op before exercising another gated task.
    context.prepare_storage(sender).await.unwrap();
    (context, tasks)
}

fn stored_identity(dir: &Path) -> (Connection, StoredIdentity) {
    let conn = Connection::open(dir.join("det-mainnet.sqlite")).unwrap();
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT value FROM meta_identity WHERE key = 'det:identity:v1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bytes[0], KV_SCHEMA_VERSION);
    let (stored, consumed) =
        bincode::serde::decode_from_slice(&bytes[1..], bincode::config::standard()).unwrap();
    assert_eq!(consumed, bytes.len() - 1);
    (conn, stored)
}

#[test]
fn mainnet_identity_fixture_migrates_and_signs() {
    check_migration_and_signing(CAPTURE, 4);
}

#[test]
fn mainnet_imported_key_fixture_migrates_and_signs() {
    check_migration_and_signing(CAPTURE_WITH_IMPORTED_KEY, 5);
}

fn check_migration_and_signing(original: &[u8], key_count: usize) {
    if !cfg!(feature = "cli") && std::env::var_os(cli::BINARY_ENV).is_none() {
        println!("Skipping mainnet CLI migration: enable cli or set DET_CLI_BIN");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "v11-mainnet-identity", "network": "mainnet",
        "expect": {"derive_address": false, "starting_db_version": 11},
        "contents": {"wallets": [{"alias": "e2e-test-mainnet"}]}
    }))
    .unwrap();
    let source_dir = root.path().join(&fixture.id);
    std::fs::create_dir(&source_dir).unwrap();
    std::fs::write(source_dir.join("data.db"), original).unwrap();
    let staged = stage::stage(root.path(), &fixture).unwrap();
    let data_db = staged.data_dir().join("data.db");
    let legacy = Connection::open(&data_db).unwrap();
    let (bytes, wallet_hash, wallet_index): (Vec<u8>, Vec<u8>, u32) = legacy
        .query_row("SELECT data, wallet, wallet_index FROM identity WHERE is_local = 1 AND network = 'dash'", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).unwrap();
    let original_identity = QualifiedIdentity::from_bytes(&bytes).unwrap();
    assert_eq!(wallet_index, 3);
    assert_eq!(original_identity.identity.public_keys().len(), key_count);
    assert_eq!(original_identity.private_keys.len(), key_count);
    for (_, (key, data)) in original_identity.private_keys.iter() {
        if key.identity_public_key.id() == 4 {
            assert_eq!(key.identity_public_key.key_type(), KeyType::ECDSA_SECP256K1);
            let PrivateKeyData::Clear(secret) = data else {
                panic!("captured imported key must be plaintext")
            };
            assert_eq!(hex::encode(secret), IMPORTED_PRIVATE_KEY);
        } else {
            assert!(matches!(data, PrivateKeyData::AtWalletDerivationPath(_)));
            assert_eq!(key.identity_public_key.key_type(), KeyType::ECDSA_HASH160);
        }
        assert_eq!(key.identity_public_key.data().len(), 33);
    }
    drop(legacy);

    let cli = cli::DetCli::new(&staged).unwrap();
    let mut canonical_bytes = None;
    for boot in 1..=3 {
        let run = cli.wallets_list(DEFAULT_BOOT_TIMEOUT).unwrap();
        assertions::check_boot(&run, "core-wallets-list").unwrap();
        assertions::check_wallets(&["e2e-test-mainnet".to_owned()], &run).unwrap();
        let info = cli.network_info(DEFAULT_BOOT_TIMEOUT).unwrap();
        assertions::check_network(Network::Mainnet, &info).unwrap();
        let (_, stored) = stored_identity(staged.data_dir());
        if let Some(expected) = &canonical_bytes {
            assert!(
                stored.qi_bytes == *expected,
                "repair and subsequent boots must produce the same identity record"
            );
        } else {
            canonical_bytes = Some(stored.qi_bytes.clone());
        }
        let persisted = QualifiedIdentity::from_bytes(&stored.qi_bytes).unwrap();
        assert_eq!(persisted.private_keys.len(), key_count);
        for (key, data) in persisted.private_keys.values() {
            if key.identity_public_key.id() == 4 {
                assert!(matches!(data, PrivateKeyData::InVault));
                assert_eq!(key.identity_public_key.data().len(), 33);
            } else {
                assert_eq!(key.identity_public_key.data().len(), 20);
            }
        }

        // In-process backend handles can retain advisory locks after shutdown.
        // Sign from an exact copy so only CLI processes open the boot fixture.
        let snapshot_fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": staged.data_dir().file_name().unwrap().to_str().unwrap(),
            "network": "mainnet"
        }))
        .unwrap();
        let signing_copy =
            stage::stage(staged.data_dir().parent().unwrap(), &snapshot_fixture).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (context, tasks) = open_context(signing_copy.data_dir()).await;
            if boot == 3 {
                let mut damaged = context.load_local_qualified_identities().unwrap().remove(0);
                // Remove the actual vault bytes too: restoring metadata alone
                // must not accidentally pass by reusing a surviving secret.
                let secrets = context.secret_store();
                let keys = IdentityKeyView::new(&secrets, damaged.identity.id().to_buffer());
                for ((target, id), (_, data)) in damaged.private_keys.iter() {
                    if matches!(data, PrivateKeyData::InVault) {
                        keys.delete(target, *id).unwrap();
                        assert!(keys.get(target, *id).unwrap().is_none());
                    }
                }
                damaged.private_keys = Default::default();
                context.update_local_qualified_identity(&damaged).unwrap();
                context
                    .wallet_backend()
                    .unwrap()
                    .wallet_seeds()
                    .delete_raw(&wallet_hash.clone().try_into().unwrap())
                    .unwrap();
                let result = context
                    .run_migration_task(MigrationTask::RestoreFromPreviousVersion)
                    .await
                    .unwrap();
                let BackendTaskSuccessResult::PreviousVersionRestored(summary) = result else {
                    panic!("restore summary")
                };
                assert_eq!(summary.wallets_restored, 1);
                assert_eq!(summary.identity_keys_restored as usize, key_count);
                assert!(!summary.has_problems());
            }
            let identities = context.load_local_qualified_identities().unwrap();
            assert_eq!(identities.len(), 1);
            let identity = &identities[0];
            assert_eq!(identity.identity, original_identity.identity);
            assert_eq!(identity.wallet_index, Some(wallet_index));
            assert_eq!(identity.network, Network::Mainnet);
            assert_eq!(identity.associated_wallets.len(), 1);
            assert!(
                identity
                    .associated_wallets
                    .contains_key(wallet_hash.as_slice())
            );
            assert_eq!(
                identity.private_keys.keys_set(),
                original_identity.private_keys.keys_set()
            );
            for ((placement, (key, data)), (original_placement, (_, original_data))) in identity
                .private_keys
                .iter()
                .zip(original_identity.private_keys.iter())
            {
                assert_eq!(placement, original_placement);
                if key.identity_public_key.id() == 4 {
                    assert!(matches!(data, PrivateKeyData::InVault));
                    assert_eq!(key.identity_public_key.data().len(), 33);
                } else {
                    assert!(
                        data == original_data,
                        "wallet derivation references must survive unchanged"
                    );
                    assert_eq!(key.identity_public_key.data().len(), 20);
                }
            }
            for key in identity.identity.public_keys().values() {
                if key.id() == 4 {
                    let (_, secret) = identity
                        .resolve_private_key_bytes(key)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(hex::encode(*secret), IMPORTED_PRIVATE_KEY);
                }
                assert!(
                    identity.can_sign_with(key),
                    "key {} must remain available",
                    key.id()
                );
                let message = format!("DET offline migration check: boot {boot}, key {}", key.id());
                let signature = identity.sign(key, message.as_bytes()).await.unwrap();
                let verify = |data: &[u8]| match key.key_type() {
                    KeyType::ECDSA_SECP256K1 => signer::verify_data_signature(
                        data,
                        signature.as_slice(),
                        key.data().as_slice(),
                    ),
                    KeyType::ECDSA_HASH160 => signer::verify_hash_signature(
                        &signer::double_sha(data),
                        signature.as_slice(),
                        key.data().as_slice(),
                    ),
                    other => panic!("unexpected captured key type: {other:?}"),
                };
                verify(message.as_bytes())
                    .unwrap_or_else(|error| panic!("boot {boot}, key {}: {error}", key.id()));
                assert!(verify(b"different message").is_err());
                println!(
                    "boot {boot}: key {} ({:?}, {:?}) signature verified",
                    key.id(),
                    key.purpose(),
                    key.security_level()
                );
            }
            context.wallet_backend().unwrap().shutdown().await;
            tasks.shutdown_async().await.unwrap();
        });
        drop(runtime);
        assert!(
            std::fs::read(&data_db).unwrap() == original,
            "legacy database must stay byte-identical"
        );
        if boot == 1 {
            // An earlier upgrade imported the old snapshots but set all drain
            // sentinels. The next boot must repair them without a fresh import.
            let (conn, mut stored) = stored_identity(staged.data_dir());
            stored.qi_bytes = bytes.clone();
            let mut encoded = vec![KV_SCHEMA_VERSION];
            encoded.extend(
                bincode::serde::encode_to_vec(&stored, bincode::config::standard()).unwrap(),
            );
            assert_eq!(
                conn.execute(
                    "UPDATE meta_identity SET value = ?1 WHERE key = 'det:identity:v1'",
                    [encoded]
                )
                .unwrap(),
                1
            );
        }
    }
}
