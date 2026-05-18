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
        .register_wallet(wallet)
        .expect("register_wallet should succeed");

    // Verify in-memory
    {
        let wallets = app_context.wallets().read().expect("wallets lock");
        assert!(
            wallets.contains_key(&seed_hash),
            "Wallet should be registered"
        );
    }

    // Verify in DB
    {
        let db_wallets = app_context
            .db()
            .get_wallets(&Network::Testnet)
            .expect("DB query should succeed");
        assert!(
            db_wallets.iter().any(|w| w.seed_hash() == seed_hash),
            "Wallet should be persisted in DB"
        );
    }

    // TODO(P0.5): re-enable in P2 — chain sync is owned by upstream
    // platform-wallet; wallet registration is observed via the EventBridge.
    let _ = (&app_context, &seed_hash);
}
