//! Faucet HTTP client and balance verification for test wallets on testnet.

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;
use std::time::Duration;

#[allow(dead_code)]
const FAUCET_BASE_URL: &str = "http://faucet.testnet.networks.dash.org";
const MIN_BALANCE_DUFFS: u64 = 1_000_000_000; // 10 DASH

/// Verify the framework wallet has at least `MIN_BALANCE_DUFFS`.
///
/// If the balance is below the threshold, panics with the receive address
/// and instructions for the user to fund it manually.
pub async fn verify_framework_funded(app_context: &Arc<AppContext>, wallet_hash: WalletSeedHash) {
    // Read balance from lock-free atomics (no lock needed).
    let current_balance = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&wallet_hash)
            .map(|w| w.read().expect("wallet lock").total_balance_duffs())
            .unwrap_or(0)
    };

    if current_balance >= MIN_BALANCE_DUFFS {
        tracing::info!(
            "Framework wallet balance: {} duffs (sufficient)",
            current_balance
        );
        return;
    }

    // Only derive address in the panic path (rare).
    let address = get_wallet_balance_and_address(app_context, wallet_hash).await.1;
    panic!(
        "Framework wallet balance is below minimum ({} duffs < {} duffs).\n\
         Fund this address manually: {}\n\
         Then set E2E_WALLET_MNEMONIC to the wallet's mnemonic.",
        current_balance, MIN_BALANCE_DUFFS, address
    );
}

/// Get the wallet's current total balance and receive address.
async fn get_wallet_balance_and_address(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
) -> (u64, String) {
    // Extract balance and PlatformWallet under short sync lock, then drop
    // the guard before any .await to avoid holding std::sync::RwLock across
    // an async yield point (which would deadlock with SPV reconcile).
    let (balance, pw) = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet_arc = wallets
            .get(&wallet_hash)
            .expect("framework wallet must exist");
        let wallet = wallet_arc.read().expect("wallet lock");
        let balance = wallet.total_balance_duffs();
        let pw = wallet.platform_wallet.clone().expect("platform wallet");
        (balance, pw)
    };

    let address = pw
        .core()
        .next_receive_address()
        .await
        .expect("Failed to get receive address")
        .to_string();
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
