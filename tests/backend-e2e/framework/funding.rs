//! Faucet HTTP client and balance verification for test wallets on testnet.

use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use std::sync::Arc;
use std::time::Duration;

/// Test-only platform-receive-address helper for the e2e suite.
///
/// Production resolves the HD seed through the JIT chokepoint; the legacy
/// parked-seed `Wallet::platform_receive_address` was retired. This mirrors
/// the old "reuse-or-generate" behaviour without touching a seed in the test:
/// when `skip_known` is false and a watched platform-payment address already
/// exists, return it (seed-free); otherwise dispatch
/// [`WalletTask::GeneratePlatformReceiveAddress`], which derives the next one
/// through the chokepoint in the backend.
pub async fn derive_platform_receive_address(
    app_context: &Arc<AppContext>,
    seed_hash: WalletSeedHash,
    network: Network,
    skip_known: bool,
) -> PlatformAddress {
    if !skip_known {
        let existing = {
            let wallets = app_context.wallets().read().expect("wallets lock");
            let wallet = wallets.get(&seed_hash).expect("framework wallet missing");
            let guard = wallet.read().expect("wallet read lock");
            guard
                .platform_addresses(network)
                .into_iter()
                .next()
                .map(|(_, platform_addr)| platform_addr)
        };
        if let Some(platform_addr) = existing {
            return platform_addr;
        }
    }

    let result = run_task(
        app_context,
        BackendTask::WalletTask(WalletTask::GeneratePlatformReceiveAddress { seed_hash }),
    )
    .await
    .expect("GeneratePlatformReceiveAddress task failed");

    let BackendTaskSuccessResult::GeneratedPlatformReceiveAddress { address, .. } = result else {
        panic!("unexpected result for GeneratePlatformReceiveAddress: {result:?}");
    };
    PlatformAddress::from_bech32m_string(&address)
        .expect("derived platform address is not valid bech32m")
}

#[allow(dead_code)]
const FAUCET_BASE_URL: &str = "http://faucet.testnet.networks.dash.org";
const MIN_BALANCE_DUFFS: u64 = 1_000_000_000; // 10 DASH

/// Verify the framework wallet has at least `MIN_BALANCE_DUFFS`.
///
/// If the balance is below the threshold, panics with the receive address
/// and instructions for the user to fund it manually.
pub async fn verify_framework_funded(app_context: &Arc<AppContext>, wallet_hash: WalletSeedHash) {
    let (current_balance, address) = get_wallet_balance_and_address(app_context, wallet_hash).await;

    if current_balance >= MIN_BALANCE_DUFFS {
        tracing::info!(
            "Framework wallet balance: {} duffs (sufficient)",
            current_balance
        );
        return;
    }

    panic!(
        "Framework wallet balance is below minimum ({} duffs < {} duffs).\n\
         Fund this address manually: {}\n\
         Then set E2E_WALLET_MNEMONIC to the wallet's mnemonic.",
        current_balance, MIN_BALANCE_DUFFS, address
    );
}

/// Get the wallet's current total balance and a SPV-watched receive address.
async fn get_wallet_balance_and_address(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
) -> (u64, String) {
    let balance = app_context.snapshot_balance(&wallet_hash).total;
    let address = app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .next_receive_address(&wallet_hash)
        .await
        .expect("Failed to get receive address");
    (balance, address)
}

/// POST to the testnet faucet API with retries.
///
/// Available as a helper for manual use but not called during normal initialization.
#[allow(dead_code)]
pub async fn request_faucet_funds(address: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Pre-flight: check faucet is reachable
    client
        .get(format!("{}/api/status", FAUCET_BASE_URL))
        .send()
        .await
        .map_err(|e| format!("Faucet status check failed: {}", e))?;

    let body = serde_json::json!({ "address": address });

    let mut last_error = String::new();
    for attempt in 1..=3 {
        tracing::info!(
            "Faucet request attempt {}/3 for address {}",
            attempt,
            address
        );

        match client
            .post(format!("{}/api/drip", FAUCET_BASE_URL))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<unreadable>".to_string());

                if status.is_success() {
                    // Try to extract txid from response
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                        && let Some(txid) = json.get("txid").and_then(|v| v.as_str())
                    {
                        return Ok(txid.to_string());
                    }
                    // If no txid field, return the full response
                    return Ok(text);
                }

                last_error = format!("Faucet HTTP {}: {}", status, text);
            }
            Err(e) => {
                last_error = format!("Faucet request error: {}", e);
            }
        }

        if attempt < 3 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    Err(last_error)
}
