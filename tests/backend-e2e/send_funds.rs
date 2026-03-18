//! Test: Send and receive Core payments between wallets.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait::{wait_for_balance, wait_for_spendable_balance};
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Send a wallet payment with retry logic for "Insufficient funds" errors.
///
/// After a previous send, the change output may not be spendable yet (waiting for
/// InstantSend lock or block confirmation). This helper retries with backoff.
async fn send_with_retry(
    app_context: &Arc<AppContext>,
    wallet: &Arc<RwLock<Wallet>>,
    wallet_hash: WalletSeedHash,
    address: String,
    amount_duffs: u64,
    subtract_fee: bool,
    memo: &str,
) -> BackendTaskSuccessResult {
    const MAX_RETRIES: u32 = 5;
    const RETRY_DELAY: Duration = Duration::from_secs(10);

    for attempt in 1..=MAX_RETRIES {
        // On retries, wait for spendable balance
        if attempt > 1 {
            tracing::info!(
                "Retry {}/{}: waiting for wallet UTXOs to become spendable...",
                attempt,
                MAX_RETRIES
            );
            let _ = wait_for_spendable_balance(
                app_context,
                wallet_hash,
                if subtract_fee { 1 } else { amount_duffs },
                RETRY_DELAY,
            )
            .await;
        }

        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: address.clone(),
                amount_duffs,
            }],
            subtract_fee_from_amount: subtract_fee,
            memo: Some(memo.to_string()),
            override_fee: None,
        };

        let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: wallet.clone(),
            request,
        });

        match run_task(app_context, task).await {
            Ok(result) => return result,
            Err(e)
                if (e.to_string().contains("Insufficient")
                    || e.to_string().contains("No UTXOs")
                    || e.to_string().contains("No spendable funds")
                    || e.to_string().contains("spendable"))
                    && attempt < MAX_RETRIES =>
            {
                tracing::info!(
                    "Send attempt {}/{} failed ({}), will retry...",
                    attempt,
                    MAX_RETRIES,
                    e
                );
                tokio::time::sleep(RETRY_DELAY).await;
                continue;
            }
            Err(e) => panic!(
                "Payment '{}' failed after {} attempts: {}",
                memo, attempt, e
            ),
        }
    }
    panic!("Payment '{}' failed after {} retries", memo, MAX_RETRIES);
}

/// Send DASH between two test wallets and verify balances.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_send_and_receive_funds() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Create two funded test wallets
    let (hash_a, wallet_a) = ctx.create_funded_test_wallet(5_000_000).await;
    let (hash_b, wallet_b) = ctx.create_funded_test_wallet(1_000_000).await;

    let initial_b_balance = {
        let w = wallet_b.read().expect("lock");
        w.total_balance_duffs()
    };

    // Send 2,000,000 duffs from A to B
    let send_amount: u64 = 2_000_000;
    let b_address = get_receive_address(app_context, &wallet_b);

    let result = send_with_retry(
        app_context,
        &wallet_a,
        hash_a,
        b_address,
        send_amount,
        false,
        "E2E test A->B",
    )
    .await;

    match &result {
        BackendTaskSuccessResult::WalletPayment {
            txid, total_amount, ..
        } => {
            tracing::info!("A->B payment txid: {}, amount: {}", txid, total_amount);
        }
        other => panic!("Expected WalletPayment, got: {:?}", other),
    }

    // Wait for B to see the funds
    let new_b_balance = wait_for_balance(
        app_context,
        hash_b,
        initial_b_balance + send_amount,
        Duration::from_secs(120),
    )
    .await
    .expect("B should receive funds");

    tracing::info!("B balance after receiving: {}", new_b_balance);
    assert!(
        new_b_balance >= initial_b_balance + send_amount,
        "B should have received funds"
    );

    // Wait for B's funds to become spendable before sending back
    wait_for_spendable_balance(app_context, hash_b, send_amount, Duration::from_secs(120))
        .await
        .expect("Wallet B funds should become spendable");

    // Send funds back from B to A
    let a_address = get_receive_address(app_context, &wallet_a);

    let _result = send_with_retry(
        app_context,
        &wallet_b,
        hash_b,
        a_address,
        send_amount,
        true,
        "E2E test B->A return",
    )
    .await;

    // Verify A received the return payment
    let a_balance_after_return = wait_for_balance(
        app_context,
        hash_a,
        send_amount, // A should have at least send_amount back (minus fee from B)
        Duration::from_secs(120),
    )
    .await
    .expect("A should receive return funds from B");

    tracing::info!(
        "A balance after round-trip: {} duffs",
        a_balance_after_return
    );

    tracing::info!("Round-trip payment completed successfully");
}
