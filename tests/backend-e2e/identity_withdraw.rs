//! Test: Withdraw credits from an identity to a Core address.

use crate::harness::ctx;
use crate::identity_helpers::{build_identity_registration, get_receive_address};
use crate::task_runner::run_task;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::str::FromStr;

/// Create identity, then withdraw some credits to a Core address.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_withdraw_from_identity() {
    let ctx = ctx().await;

    // Create funded test wallet
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(2_000_000).await;

    // Register identity
    let registration_info = build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(registration_info));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("Identity registration should succeed");

    let qualified_identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };

    let initial_balance = qualified_identity.identity.balance();
    println!("  Identity balance before withdrawal: {}", initial_balance);

    // Get a Core address to withdraw to
    let withdraw_address_str = get_receive_address(&ctx.app_context, &wallet_arc);
    let withdraw_address = Address::from_str(&withdraw_address_str)
        .expect("Valid address")
        .assume_checked();

    // Withdraw half the credits
    let withdraw_amount = initial_balance / 2;
    let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
        qualified_identity,
        Some(withdraw_address),
        withdraw_amount,
        None,
    ));

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("Withdrawal should succeed");

    match result {
        BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) => {
            println!("  Withdrawal successful, fee: {:?}", fee_result);
        }
        other => panic!("Expected WithdrewFromIdentity, got: {:?}", other),
    }
}
