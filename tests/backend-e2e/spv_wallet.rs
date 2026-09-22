//! Test: SPV sync and wallet creation using shared context.

use crate::framework::harness::ctx;
use bip39::{Language, Mnemonic};
use dash_sdk::dpp::dashcore::Network;

/// Verify SPV is running and can register a new wallet.
///
/// Uses the shared `BackendTestContext` -- SPV is already started.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_spv_sync_and_create_wallet() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Generate a new wallet from a random mnemonic
    let mnemonic =
        Mnemonic::generate_in(Language::English, 12).expect("Mnemonic generation should succeed");
    let seed = mnemonic.to_seed("");

    let wallet = dash_evo_tool::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("SPV E2E Test Wallet".to_string()),
        None,
    )
    .expect("Wallet::new_from_seed should succeed");

    assert!(
        !wallet.known_addresses.is_empty(),
        "New wallet should have at least one known address"
    );

    // Register the wallet
    let (seed_hash, _wallet_arc) = app_context
        .register_wallet(
            wallet,
            &seed,
            dash_evo_tool::model::wallet::birth_height::WalletOrigin::Imported,
        )
        .expect("register_wallet should succeed");

    // Verify in-memory
    {
        let wallets = app_context.wallets().read().expect("wallets lock");
        assert!(
            wallets.contains_key(&seed_hash),
            "Wallet should be registered"
        );
    }

    // Verify persistence in the system of record (the wallet-meta sidecar).
    // DET no longer writes the legacy `data.db.wallet` row — the upstream
    // persistor plus the wallet-meta/seed-envelope sidecars own wallet state.
    {
        let meta = dash_evo_tool::wallet_backend::WalletMetaView::new(&app_context.app_kv())
            .get(Network::Testnet, &seed_hash);
        assert!(
            meta.is_some(),
            "Wallet should be persisted in the wallet-meta sidecar"
        );
    }

    // The wallet must register with the upstream manager so the SpvRuntime
    // watches its addresses (chain sync is upstream-owned; registration is
    // driven via the wallet backend, observed through the EventBridge).
    crate::framework::wait::wait_for_wallet_in_spv(
        app_context,
        seed_hash,
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("New wallet should register with the wallet backend");

    let backend = app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "New wallet should be registered with the upstream manager"
    );
}
