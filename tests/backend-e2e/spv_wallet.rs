//! Test scenario: connect to testnet via SPV, sync, create a wallet.

use crate::helpers::BackendTestContext;
use bip39::{Language, Mnemonic};
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::spv::SpvStatus;
use dash_sdk::dpp::dashcore::Network;
use std::time::Duration;
use tokio::time::timeout;

/// Connect to Dash testnet via SPV, wait for peer connections, create a new
/// wallet, verify it is persisted to the database and loaded into the SPV
/// subsystem, then shut down gracefully.
///
/// # Requirements
///
/// - Network access to Dash testnet peers (uses DAPI addresses from `.env.example`)
/// - No running Dash Core node required (SPV-only)
#[ignore] // Requires network access to Dash testnet peers
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_spv_sync_and_create_wallet() {
    // 1. Set up headless AppContext for testnet
    let ctx = BackendTestContext::new(Network::Testnet);

    // 2. Start SPV sync
    ctx.start_spv()
        .expect("SPV start should succeed");

    // 3. Wait for at least one peer to connect (60s timeout for CI environments)
    let status = ctx
        .wait_for_spv_peers(Duration::from_secs(60))
        .await
        .expect("Should connect to at least one testnet peer");
    assert!(
        matches!(status, SpvStatus::Syncing | SpvStatus::Running),
        "SPV should be syncing or running after connecting peers, got: {:?}",
        status
    );

    // 4. Generate a new wallet from a random mnemonic
    let mnemonic = Mnemonic::generate_in(Language::English, 12)
        .expect("Mnemonic generation should succeed");
    let seed = mnemonic.to_seed("");

    let wallet = Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("Backend E2E Test Wallet".to_string()),
        None, // no password
    )
    .expect("Wallet::new_from_seed should succeed");

    // Verify the wallet has a first receive address
    assert!(
        !wallet.known_addresses.is_empty(),
        "New wallet should have at least one known address"
    );

    // 5. Register the wallet (persists to DB, loads into SPV)
    let (seed_hash, _wallet_arc) = ctx
        .app_context
        .register_wallet(wallet)
        .expect("register_wallet should succeed");

    // 6. Verify wallet is in the in-memory map
    {
        let wallets = ctx.app_context.wallets.read().expect("wallets lock");
        assert!(
            wallets.contains_key(&seed_hash),
            "Wallet should be registered in AppContext.wallets"
        );
    }

    // 7. Verify wallet is persisted in the database
    {
        let db_wallets = ctx
            .app_context
            .db
            .get_wallets(&Network::Testnet)
            .expect("DB query should succeed");
        assert!(
            db_wallets.iter().any(|w| w.seed_hash() == seed_hash),
            "Wallet should be persisted in the database"
        );
    }

    // 8. Wait for wallet to appear in SPV subsystem (10s timeout)
    let wallet_in_spv = timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = ctx.app_context.spv_manager().det_wallets_snapshot();
            if snapshot.contains_key(&seed_hash) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    assert!(
        wallet_in_spv.unwrap_or(false),
        "Wallet should appear in SPV subsystem within 10 seconds"
    );

    // 9. Graceful shutdown happens in BackendTestContext::drop
}
