//! Faucet HTTP client for funding test wallets on testnet.

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;
use std::time::Duration;

const FAUCET_BASE_URL: &str = "http://faucet.testnet.networks.dash.org";
const MIN_BALANCE_DUFFS: u64 = 100_000_000; // 1 DASH

/// Ensure the framework wallet has at least `MIN_BALANCE_DUFFS`.
///
/// If the balance is below the threshold, request funds from the testnet faucet
/// and wait for them to arrive via SPV.
pub async fn ensure_framework_funded(app_context: &Arc<AppContext>, wallet_hash: WalletSeedHash) {
    let current_balance = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet_arc = wallets
            .get(&wallet_hash)
            .expect("framework wallet must exist");
        let wallet = wallet_arc.read().expect("wallet lock");
        wallet.total_balance_duffs()
    };

    if current_balance >= MIN_BALANCE_DUFFS {
        println!(
            "  Framework wallet balance: {} duffs (sufficient)",
            current_balance
        );
        return;
    }

    println!(
        "  Framework wallet balance: {} duffs (below {} minimum, requesting faucet)",
        current_balance, MIN_BALANCE_DUFFS
    );

    // Get a receive address for the framework wallet
    let address = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet_arc = wallets
            .get(&wallet_hash)
            .expect("framework wallet must exist");
        let mut wallet = wallet_arc.write().expect("wallet lock");
        wallet
            .receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                false,
                Some(app_context),
            )
            .expect("Failed to get receive address")
            .to_string()
    };

    match request_faucet_funds(&address).await {
        Ok(txid) => println!("  Faucet funded framework wallet, txid: {}", txid),
        Err(e) => {
            if current_balance == 0 {
                panic!(
                    "Faucet failed and framework wallet has zero balance: {}. \
                     Set E2E_WALLET_MNEMONIC to a pre-funded wallet or fund manually.",
                    e
                );
            }
            eprintln!(
                "  Faucet request failed ({}), proceeding with existing balance: {}",
                e, current_balance
            );
        }
    }
}

/// POST to the testnet faucet API with retries.
async fn request_faucet_funds(address: &str) -> Result<String, String> {
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
        println!(
            "  Faucet request attempt {}/3 for address {}",
            attempt, address
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
