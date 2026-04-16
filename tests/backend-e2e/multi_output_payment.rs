//! Test: Multi-output payment — proves that a single Core transaction sending
//! equal amounts to multiple never-used addresses only makes one output
//! spendable via InstantSend lock.

use crate::framework::harness::{MAX_TEST_TIMEOUT, ctx};
use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait::{wait_for_balance, wait_for_spendable_balance};
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};

/// Send one transaction with 3 equal outputs to 3 fresh wallets.
///
/// Expected (correct) behavior: all 3 wallets should have spendable balance.
/// Actual (buggy) behavior: only 1 wallet gets spendable funds; the other 2
/// see `total_balance` but `spendable_balance` remains 0.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn test_multi_output_payment_spendable() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let per_wallet_amount: u64 = 2_000_000;
    let total_send: u64 = per_wallet_amount * 3;

    // 1. Create a funded source wallet
    let (source_hash, source_wallet) = ctx.create_funded_test_wallet(10_000_000).await;
    tracing::info!("Source wallet funded");

    // 2. Create 3 empty destination wallets
    let mut dest_wallets = Vec::with_capacity(3);
    for i in 0..3 {
        let (hash, wallet) = ctx.create_empty_test_wallet().await;
        tracing::info!(wallet_idx = i, seed_hash = ?&hash[..4], "Destination wallet created");
        dest_wallets.push((hash, wallet));
    }

    // 3. Get receive addresses from each destination
    let recipients: Vec<PaymentRecipient> = dest_wallets
        .iter()
        .enumerate()
        .map(|(i, (_hash, wallet))| {
            let address = get_receive_address(app_context, wallet);
            tracing::info!(wallet_idx = i, address = %address, "Receive address derived");
            PaymentRecipient {
                address,
                amount_duffs: per_wallet_amount,
            }
        })
        .collect();

    // 4. Wait for source wallet to have spendable funds
    wait_for_spendable_balance(app_context, source_hash, total_send, MAX_TEST_TIMEOUT / 3)
        .await
        .expect("Source wallet funds should be spendable");

    // 5. Send ONE transaction to all 3 destinations
    let request = WalletPaymentRequest {
        recipients,
        subtract_fee_from_amount: false,
        memo: Some("E2E multi-output payment test".to_string()),
        override_fee: None,
    };

    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
        wallet: source_wallet.clone(),
        request,
    });

    let result = run_task(app_context, task)
        .await
        .expect("Multi-output payment should succeed");

    match &result {
        BackendTaskSuccessResult::WalletPayment {
            txid, total_amount, ..
        } => {
            tracing::info!(txid = %txid, total_amount, "Multi-output tx broadcast");
        }
        other => panic!("Expected WalletPayment result, got: {:?}", other),
    }

    // 6. Wait for each wallet to see total balance (unconfirmed is fine here)
    for (i, (hash, _wallet)) in dest_wallets.iter().enumerate() {
        let balance = wait_for_balance(app_context, *hash, per_wallet_amount, MAX_TEST_TIMEOUT / 3)
            .await
            .unwrap_or_else(|e| panic!("Wallet {i} should see total balance: {e}"));
        tracing::info!(wallet_idx = i, balance, "Total balance visible");
    }

    // 7. Check spendable balance for each wallet — this is where the bug manifests
    let mut spendable_results = Vec::with_capacity(3);
    for (i, (hash, _wallet)) in dest_wallets.iter().enumerate() {
        let spendable =
            wait_for_spendable_balance(app_context, *hash, per_wallet_amount, MAX_TEST_TIMEOUT / 3)
                .await;
        match &spendable {
            Ok(balance) => {
                tracing::info!(wallet_idx = i, balance, "Spendable balance confirmed");
            }
            Err(e) => {
                tracing::error!(wallet_idx = i, error = %e, "Spendable balance NOT reached");
            }
        }
        spendable_results.push((i, spendable));
    }

    // 8. Assert all wallets have spendable funds
    let mut failures = Vec::new();
    for (i, result) in &spendable_results {
        if let Err(e) = result {
            failures.push(format!("Wallet {i}: {e}"));
        }
    }

    assert!(
        failures.is_empty(),
        "Multi-output IS lock bug: only some wallets got spendable balance.\n\
         Failures:\n  {}",
        failures.join("\n  ")
    );

    tracing::info!("All 3 wallets have spendable balance — multi-output IS lock works correctly");
}
