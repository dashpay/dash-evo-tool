//! Test: Verify `is_ours` flag is set correctly for SPV transactions.
//!
//! SPV transactions pass through bloom filter → `check_transaction()` (address
//! matching) → `record_transaction()`. The upstream library sets `is_ours` only
//! for sends (`net_amount < 0`). We override to `true` for all matched
//! transactions in the SPV reconcile path, since `check_transaction` already
//! verified address ownership (bloom filter FPs are filtered there).
//!
//! This test sends funds between two wallets and verifies that both the sender
//! and receiver have `is_ours: true` on the resulting transaction.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait::{wait_for_balance, wait_for_spendable_balance};
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use std::time::Duration;

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

    // Wait for A to have spendable funds
    wait_for_spendable_balance(app_context, hash_a, send_amount, Duration::from_secs(120))
        .await
        .expect("Wallet A should have spendable funds");

    // Send from A to B
    let request = WalletPaymentRequest {
        recipients: vec![PaymentRecipient {
            address: b_address.clone(),
            amount_duffs: send_amount,
        }],
        subtract_fee_from_amount: false,
        memo: Some("is_ours test".to_string()),
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
    let initial_b = {
        let w = wallet_b.read().expect("lock");
        w.total_balance_duffs()
    };
    wait_for_balance(
        app_context,
        hash_b,
        initial_b + send_amount,
        Duration::from_secs(120),
    )
    .await
    .expect("B should receive funds");

    // Force a reconcile to ensure latest SPV state is reflected
    app_context
        .reconcile_spv_wallets()
        .await
        .expect("reconcile should succeed");

    // Check is_ours on wallet A (sender) — should be true
    {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet = wallets
            .get(&hash_a)
            .expect("wallet A")
            .read()
            .expect("lock");
        let transactions = wallet.get_transactions();
        let tx = transactions
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
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet = wallets
            .get(&hash_b)
            .expect("wallet B")
            .read()
            .expect("lock");
        let transactions = wallet.get_transactions();
        let tx = transactions
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
