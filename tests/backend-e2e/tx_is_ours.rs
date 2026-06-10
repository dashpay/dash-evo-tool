//! Test: Verify `is_ours` flag is set correctly for SPV transactions.
//!
//! The upstream `platform-wallet` engine matches transactions against watched
//! addresses and emits wallet events; DET's `EventBridge` accumulates them
//! into the per-wallet snapshot read here via
//! `WalletBackend::transaction_history`. Both the sender and receiver wallet
//! must see the transaction with `is_ours: true`.
//!
//! This test sends funds between two wallets and verifies that both the sender
//! and receiver have `is_ours: true` on the resulting transaction.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait::{wait_for_balance, wait_for_spendable_balance};
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};

/// After an SPV send, both sender and receiver wallets must have `is_ours: true`
/// on the resulting transaction.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_spv_transactions_is_ours_flag() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Create two funded wallets
    let (hash_a, wallet_a) = ctx.create_funded_test_wallet(3_000_000).await;
    let (hash_b, wallet_b) = ctx.create_funded_test_wallet(1_000_000).await;

    let send_amount: u64 = 500_000;
    let b_address = get_receive_address(app_context, &wallet_b);

    // Capture B's balance BEFORE sending, so we know the exact target to
    // wait for. Reading this after the send risks including the send amount
    // (via reconciliation), which inflates the target and causes a timeout.
    let initial_b = app_context.snapshot_balance(&hash_b).total;
    tracing::info!("initial_b balance = {} duffs", initial_b);

    // Wait for A to have spendable funds
    wait_for_spendable_balance(
        app_context,
        hash_a,
        send_amount,
        crate::framework::harness::MAX_TEST_TIMEOUT / 3,
    )
    .await
    .expect("Wallet A should have spendable funds");

    // Allow bloom filter to propagate to peers so B's addresses are
    // monitored before we broadcast A→B. Without this, peers may not
    // relay the tx back through B's filter.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Send from A to B
    let request = WalletPaymentRequest {
        recipients: vec![PaymentRecipient {
            address: b_address.clone(),
            amount_duffs: send_amount,
        }],
        subtract_fee_from_amount: false,
        override_fee: None,
    };

    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
        wallet: wallet_a.clone(),
        request,
    });

    let result = run_task(app_context, task)
        .await
        .expect("Payment A->B should succeed");

    let payment_txid = match &result {
        BackendTaskSuccessResult::WalletPayment { txid, .. } => {
            tracing::info!("Payment txid: {txid}");
            txid.clone()
        }
        other => panic!("Expected WalletPayment, got: {other:?}"),
    };

    // Wait for B to receive the funds (ensures SPV has propagated the tx)
    wait_for_balance(
        app_context,
        hash_b,
        initial_b + send_amount,
        crate::framework::harness::MAX_TEST_TIMEOUT / 3,
    )
    .await
    .expect("B should receive funds");

    let wallet_backend = app_context
        .wallet_backend()
        .expect("wallet backend available");

    // Check is_ours on wallet A (sender) — should be true
    {
        let history = wallet_backend.transaction_history(&hash_a);
        let tx = history
            .iter()
            .find(|t| t.txid.to_string() == payment_txid)
            .unwrap_or_else(|| panic!("Wallet A should have tx {payment_txid}"));
        assert!(
            tx.is_ours,
            "Sender wallet should have is_ours=true for outgoing tx {payment_txid}"
        );
        assert!(
            tx.net_amount < 0,
            "Sender tx should have negative net_amount"
        );
    }

    // Check is_ours on wallet B (receiver) — should be true
    {
        let history = wallet_backend.transaction_history(&hash_b);
        let tx = history
            .iter()
            .find(|t| t.txid.to_string() == payment_txid)
            .unwrap_or_else(|| panic!("Wallet B should have tx {payment_txid}"));
        assert!(
            tx.is_ours,
            "Receiver wallet should have is_ours=true for incoming tx {payment_txid}"
        );
        assert!(
            tx.net_amount > 0,
            "Receiver tx should have positive net_amount"
        );
    }

    tracing::info!("is_ours flag verified for both sender and receiver");
}
