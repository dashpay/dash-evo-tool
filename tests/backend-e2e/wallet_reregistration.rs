//! PROJ-010 regression: wallets must re-register with the upstream SPV
//! backend so received funds are visible.
//!
//! Background: commit `e6c6c017` replaced the seed-based re-registration
//! loader with a read-only seedless loader, leaving NO code that ever
//! populated the upstream `platform-wallet.sqlite` persistor. An empty
//! persistor means an empty SPV watch set, so received Core funds stayed
//! invisible at 100% sync. The fix re-introduces the persistor write at the
//! create/import (W1) and cold-boot (W2) seed-bearing moments, with a
//! genesis birth-height floor for imported/recovered wallets so deposits made
//! before registration are still found.
//!
//! These tests run against a live Dash testnet via SPV and require a funded
//! framework wallet; they are `#[ignore]` like the rest of the backend-e2e
//! suite. See `tests/backend-e2e/README.md`.

use crate::framework::harness;
use crate::framework::wait;
use std::sync::Arc;
use std::time::Duration;

/// W1 below-tip visibility: a freshly created+funded wallet (registered with
/// the genesis birth-height floor via the `Imported` origin the harness uses)
/// must surface its received balance.
///
/// This is the miniature of the real-world 1.0 DASH-at-block-1492173 repro:
/// the deposit lands and, because the wallet's addresses are actually watched
/// (persistor populated by W1), the balance becomes visible. Pre-fix the
/// persistor was never written, the watch set was empty, and this balance
/// never appeared — `create_funded_test_wallet` would time out waiting for it.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn funded_wallet_balance_is_visible_after_registration() {
    let ctx = harness::ctx().await;

    // 100k duffs is comfortably above dust and below the framework wallet's
    // balance; enough to prove the watch set sees a real deposit.
    let amount_duffs: u64 = 100_000;
    let (seed_hash, _wallet_arc) = ctx.create_funded_test_wallet(amount_duffs).await;

    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the funded wallet must be registered with the upstream SPV backend"
    );

    // The balance must become visible — the core PROJ-010 assertion. The
    // deposit is below the SPV tip by the time it is matched, so this only
    // passes when the wallet's addresses are actually watched.
    let balance = wait::wait_for_balance(
        &ctx.app_context,
        seed_hash,
        amount_duffs,
        Duration::from_secs(120),
    )
    .await
    .expect("received funds must be visible once the wallet is registered (PROJ-010)");

    assert!(
        balance >= amount_duffs,
        "visible balance {balance} must be at least the funded amount {amount_duffs}"
    );
}

/// W2 cold-boot reconciliation idempotency on the live backend: re-driving the
/// registered-wallet reconciliation (`ensure_wallets_registered`, the
/// seedless load pass) against an already-registered, funded wallet leaves it
/// watched and does not disturb its visible balance.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn re_registration_is_idempotent_for_funded_wallet() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");

    // Framework wallet is funded historically (below the current tip). It must
    // already be registered and its balance visible.
    wait::wait_for_wallet_in_spv(&ctx.app_context, seed_hash, Duration::from_secs(30))
        .await
        .expect("framework wallet must be registered with the backend");
    let before = ctx.app_context.snapshot_balance(&seed_hash).total;

    // Re-run the reconciliation pass; it must not double-register or change the
    // visible balance.
    backend
        .ensure_wallets_registered(&ctx.app_context)
        .await
        .expect("re-registration pass must not error");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the wallet must stay registered after a reconciliation pass"
    );
    let after = ctx.app_context.snapshot_balance(&seed_hash).total;
    assert_eq!(
        before, after,
        "a reconciliation pass must not disturb the visible balance"
    );
}

/// F140 (cold-process boot from migrated on-disk state — the smoke test that
/// would have caught the "wallet still loading forever" field report).
///
/// The live repro: a user upgrades, their wallet exists ONLY as legacy
/// `data.db` rows (no upstream persistor, no DET sidecars). On the next launch
/// the wallet backend wires and runs its cold-boot reconciliation BEFORE the
/// cold-start `FinishUnwire` migration imports the wallet — so the
/// reconciliation sees zero wallets and registers nothing. The migration then
/// imports the seed + meta, but if it does not RE-RUN the reconciliation the
/// upstream `id_map` stays empty and `resolve_wallet` returns `WalletNotLoaded`
/// for 20+ minutes until a restart.
///
/// This drives that exact sequence on a fresh, isolated `AppContext` /
/// `WalletBackend` (a true cold process boundary — no in-process
/// `register_wallet`): seed the framework wallet's seed as a legacy `data.db`
/// row, wire a fresh backend (sidecars empty → nothing registered), run the
/// migration, and assert the wallet is registered AND its historical balance
/// becomes visible — all without a second backend build.
///
/// Fund-safe: it only reads the framework wallet's pre-existing balance; no
/// funds move. It shares the already-synced framework backend's SPV peer set
/// via a fresh backend over an isolated workdir, so it does not disturb the
/// singleton harness context.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn cold_process_boot_from_migrated_state_registers_and_shows_balance() {
    use dash_evo_tool::app::TaskResult;
    use dash_evo_tool::app_dir::ensure_env_file;
    use dash_evo_tool::context::AppContext;
    use dash_evo_tool::context::connection_status::ConnectionStatus;
    use dash_evo_tool::database::test_helpers::{
        create_database_at_path, seed_legacy_unprotected_hd_wallet_row,
    };
    use dash_evo_tool::utils::egui_mpsc::EguiMpscAsync;
    use dash_evo_tool::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
    use dash_sdk::dpp::key_wallet::bip32::{
        ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
    };

    // Ensure the shared framework backend is up first — it owns the funded
    // framework wallet and a synced SPV view; reading the mnemonic from the
    // same env guarantees the seed we migrate is the funded one.
    let _shared = harness::ctx().await;
    let mnemonic_phrase = std::env::var("E2E_WALLET_MNEMONIC")
        .expect("E2E_WALLET_MNEMONIC must be set for E2E tests");
    let mnemonic = bip39::Mnemonic::parse_in(bip39::Language::English, &mnemonic_phrase)
        .expect("valid mnemonic");
    let seed = mnemonic.to_seed("");

    // Isolated cold-boot workdir: no sidecars, no upstream persistor — only the
    // legacy `data.db` row we seed below, exactly the migrated-on-disk shape.
    let workdir =
        std::env::temp_dir().join(format!("dash-evo-e2e-coldboot-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("create cold-boot workdir");
    ensure_env_file(&workdir);

    let db_path = workdir.join("data.db");
    let db = Arc::new(create_database_at_path(&db_path).expect("create cold-boot database"));

    // Compute the wallet's seed hash and BIP44 account-0 xpub so the legacy row
    // is internally consistent (the W2 fund-routing gate matches the published
    // xpub against the seed's derivation).
    let seed_hash = {
        let w =
            dash_evo_tool::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet for hash");
        w.seed_hash()
    };
    let epk = {
        let secp = Secp256k1::new();
        let master = ExtendedPrivKey::new_master(Network::Testnet, &seed).expect("master key");
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
        ]);
        let account = master.derive_priv(&secp, &path).expect("derive account");
        ExtendedPubKey::from_priv(&secp, &account).encode().to_vec()
    };
    seed_legacy_unprotected_hd_wallet_row(
        &db,
        &seed_hash,
        &seed,
        &epk,
        "cold-boot-migrated",
        Network::Testnet,
    )
    .expect("insert legacy wallet row");

    // Build a fresh, independent AppContext over the migrated-on-disk state.
    let subtasks = Arc::new(TaskManager::new());
    let connection_status = Arc::new(ConnectionStatus::new());
    let egui_ctx = egui::Context::default();
    let app_kv = AppContext::open_app_kv(&workdir).expect("open app k/v");
    let secret_store = AppContext::open_secret_store(&workdir).expect("open secret store");
    let app_context = AppContext::new(
        workdir.clone(),
        Network::Testnet,
        db,
        subtasks,
        connection_status,
        egui_ctx,
        app_kv,
        secret_store,
    )
    .expect("create cold-boot AppContext");

    let (task_result_sender, _task_result_rx) =
        tokio::sync::mpsc::channel::<TaskResult>(256).with_egui_ctx(app_context.egui_ctx().clone());

    // Wire the fresh backend: its cold-boot reconciliation runs NOW, against the
    // empty sidecars (the migration has not run yet), so nothing is registered.
    app_context
        .ensure_wallet_backend(task_result_sender)
        .await
        .expect("ensure_wallet_backend on cold-boot context");
    let backend = app_context
        .wallet_backend()
        .expect("cold-boot backend wired");
    assert!(
        !backend.is_wallet_registered(&seed_hash),
        "precondition: the migrated wallet is not registered at cold wiring (sidecars empty)"
    );

    // Run the cold-start migration — it must import the wallet AND re-run the
    // reconciliation so the wallet is registered without a restart.
    dash_evo_tool::backend_task::migration::finish_unwire::run(&app_context)
        .await
        .expect("cold-start migration should succeed");

    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the migrated wallet must be upstream-registered right after migration (no restart)"
    );

    // Start chain sync and confirm the historical balance becomes visible — the
    // end-to-end PROJ-010 assertion on a genuinely cold-booted, migrated wallet.
    backend.start().await.expect("start cold-boot chain sync");
    wait::wait_for_spv_peers(&app_context, Duration::from_secs(60))
        .await
        .expect("cold-boot SPV failed to connect to peers");
    wait::wait_for_balance(&app_context, seed_hash, 1, Duration::from_secs(600))
        .await
        .expect("migrated framework wallet must surface its historical balance");

    backend.shutdown().await;
}

// TODO(issue #7): fund-routing-gate xpub mismatch on a fresh DB (the REAL bug).
//
// On a fresh DB the upstream persistor stores the BIP44 account-0 row at DEPTH-1
// (BIP32 `m/0'`) instead of DEPTH-3 (`m/44'/coin'/0'`): `Default` creates both a
// BIP32 and a BIP44 account-0, both map to the same persistor primary key
// (`account_type_db_label` collapses both `StandardAccountType` variants to
// "standard"), and the BIP32 row overwrites the BIP44 row via
// `ON CONFLICT DO UPDATE`. The seedless cold-boot reload then reads the depth-1
// xpub, it matches no depth-3 DET sidecar bridge entry, and the fund-routing gate
// rejects every wallet -> systematic `WalletNotLoaded`. This is an upstream
// `platform-wallet-storage` data-loss bug, NOT legacy format drift.
//
// The earlier #251 "drift self-heal" was REVERTED — it treated a non-cause and
// would be re-clobbered by the same collision on the next persist. The real fix
// is upstream (distinguish BIP32 vs BIP44 standard accounts in the persistor key)
// + pin bump + a re-derive migration.
//
// Reproduced by the `#[ignore]`d `issue7_fresh_persistor_bip44_xpub_matches_det_bridge`
// in `src/context/wallet_lifecycle.rs` (field-level diff of the persisted vs
// expected xpub); the sound DET-vs-upstream derivation is guarded by
// `fresh_bip44_account0_xpub_is_stable_across_gate_sides` and
// `wallet_id_is_independent_of_account_creation` in `src/wallet_backend/mod.rs`.
// A full live cold-boot e2e through `load_from_persistor` is blocked offline by
// the persistor's single-open advisory lock (see the in-test fallback); add it
// once the upstream fix lands.
