//! Test: SPV sync and wallet creation using shared context.

use crate::harness::CTX;
use bip39::{Language, Mnemonic};
use dash_sdk::dpp::dashcore::Network;
use std::time::Duration;
use tokio::time::timeout;

/// Verify SPV is running and can register a new wallet.
///
/// Uses the shared `BackendTestContext` -- SPV is already started.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_spv_sync_and_create_wallet() {
    let ctx = &*CTX;
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

    // Verify in SPV (10s timeout)
    let wallet_in_spv = timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = app_context.spv_manager().det_wallets_snapshot();
            if snapshot.contains_key(&seed_hash) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    assert!(
        wallet_in_spv.unwrap_or(false),
        "Wallet should appear in SPV within 10s"
    );
}
