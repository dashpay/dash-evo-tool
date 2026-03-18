//! Test: Send and receive Core payments between wallets.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait::{wait_for_balance, wait_for_spendable_balance};
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use std::time::Duration;

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

    // Ensure wallet A has spendable balance before sending
    wait_for_spendable_balance(app_context, hash_a, send_amount, Duration::from_secs(120))
        .await
        .expect("Wallet A funds should be spendable");

    let request = WalletPaymentRequest {
        recipients: vec![PaymentRecipient {
            address: b_address.clone(),
            amount_duffs: send_amount,
        }],
        subtract_fee_from_amount: false,
        memo: Some("E2E test A->B".to_string()),
        override_fee: None,
    };

    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
        wallet: wallet_a.clone(),
        request,
    });

    let result = run_task(app_context, task)
        .await
        .expect("Payment A->B should succeed");

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

    let request = WalletPaymentRequest {
        recipients: vec![PaymentRecipient {
            address: a_address.clone(),
            amount_duffs: send_amount,
        }],
        subtract_fee_from_amount: true,
        memo: Some("E2E test B->A return".to_string()),
        override_fee: None,
    };

    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
        wallet: wallet_b.clone(),
        request,
    });

    let _result = run_task(app_context, task)
        .await
        .expect("Payment B->A should succeed");

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
