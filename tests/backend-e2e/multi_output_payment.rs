//! Test: Multi-output payment — proves that a single Core transaction sending
//! equal amounts to multiple never-used addresses only makes one output
//! spendable via InstantSend lock.

use crate::framework::harness::{MAX_TEST_TIMEOUT, ctx};
use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait::{wait_for_balance, wait_for_spendable_balance};
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::wallet::WalletTask;
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

/// Send one transaction with 3 outputs to 3 distinct addresses of the SAME wallet.
///
/// This is the primary bug reproduction: a single Core transaction sends equal
/// amounts to 3 never-used receive addresses within one wallet. Expected behavior
/// is that the wallet's spendable balance equals the sum of all 3 outputs.
/// Actual (buggy) behavior: only 1 output becomes spendable via InstantSend lock.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[tracing::instrument]
async fn test_multi_output_single_wallet_spendable() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let num_outputs: usize = 3;
    let per_output_amount: u64 = 1_000_000;
    let total_expected: u64 = per_output_amount * num_outputs as u64;

    // 1. Create a funded source wallet
    let (source_hash, source_wallet) = ctx.create_funded_test_wallet(10_000_000).await;
    tracing::info!("Source wallet funded");

    // 2. Create one empty destination wallet
    let (dest_hash, _dest_wallet) = ctx.create_empty_test_wallet().await;
    tracing::info!(seed_hash = ?&dest_hash[..4], "Destination wallet created");

    // 3. Generate 3 distinct receive addresses from the SAME wallet via backend task.
    //    Each call to GenerateReceiveAddress advances the BIP44 index, yielding a
    //    new never-used address.
    let mut addresses = Vec::with_capacity(num_outputs);
    for i in 0..num_outputs {
        let result = run_task(
            app_context,
            BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
                seed_hash: dest_hash,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("GenerateReceiveAddress {i} failed: {e}"));

        match result {
            BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => {
                tracing::info!(idx = i, address = %address, "Generated receive address");
                addresses.push(address);
            }
            other => panic!("Expected GeneratedReceiveAddress, got: {:?}", other),
        }
    }

    // Verify all addresses are distinct
    let unique: std::collections::HashSet<&String> = addresses.iter().collect();
    assert_eq!(
        unique.len(),
        num_outputs,
        "Expected {num_outputs} distinct addresses, got {} unique",
        unique.len()
    );

    // 4. Wait for source to have spendable funds
    wait_for_spendable_balance(
        app_context,
        source_hash,
        total_expected,
        MAX_TEST_TIMEOUT / 3,
    )
    .await
    .expect("Source wallet funds should be spendable");

    // 5. Send ONE transaction with 3 outputs to the same wallet's addresses
    let recipients: Vec<PaymentRecipient> = addresses
        .iter()
        .map(|addr| PaymentRecipient {
            address: addr.clone(),
            amount_duffs: per_output_amount,
        })
        .collect();

    let request = WalletPaymentRequest {
        recipients,
        subtract_fee_from_amount: false,
        memo: Some("E2E multi-output single-wallet test".to_string()),
        override_fee: None,
    };

    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
        wallet: source_wallet.clone(),
        request,
    });

    let result = run_task(app_context, task)
        .await
        .expect("Multi-output payment should succeed");

    let txid = match &result {
        BackendTaskSuccessResult::WalletPayment {
            txid, total_amount, ..
        } => {
            tracing::info!(txid = %txid, total_amount, "Multi-output tx broadcast");
            txid.clone()
        }
        other => panic!("Expected WalletPayment result, got: {:?}", other),
    };

    // 6. Wait for total balance to appear (unconfirmed is fine)
    let total_balance =
        wait_for_balance(app_context, dest_hash, total_expected, MAX_TEST_TIMEOUT / 3)
            .await
            .unwrap_or_else(|e| panic!("Dest wallet should see total balance: {e}"));
    tracing::info!(total_balance, "Total balance visible in destination wallet");

    // 7. Wait for spendable balance — this is where the bug manifests.
    //    If only 1 of 3 outputs is IS-locked, spendable will be per_output_amount
    //    instead of total_expected.
    let spendable_result =
        wait_for_spendable_balance(app_context, dest_hash, total_expected, MAX_TEST_TIMEOUT / 3)
            .await;

    match spendable_result {
        Ok(spendable) => {
            tracing::info!(spendable, "All outputs spendable — no bug!");
            assert!(
                spendable >= total_expected,
                "Spendable balance {spendable} should be >= {total_expected}"
            );
        }
        Err(e) => {
            tracing::error!(
                txid = %txid,
                expected = total_expected,
                num_outputs,
                error = %e,
                "BUG: multi-output IS lock — not all outputs spendable"
            );
            panic!(
                "Multi-output IS lock bug (single wallet): sent {num_outputs} outputs \
                 of {per_output_amount} duffs each to the same wallet, but spendable \
                 balance never reached {total_expected}. Error: {e}"
            );
        }
    }
}
