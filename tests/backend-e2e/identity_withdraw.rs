//! Test: Withdraw credits from an identity to a Core address.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::{build_identity_registration, get_receive_address};
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::str::FromStr;

/// Create identity, then withdraw some credits to a Core address.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_withdraw_from_identity() {
    let ctx = ctx().await;

    // Asset lock (1M) + withdrawal state transition fees. 3M provides margin.
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(3_000_000).await;

    // Register identity on Platform
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
        build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash),
    ));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("Identity registration should succeed");

    let qualified_identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };

    let initial_balance = qualified_identity.identity.balance();
    tracing::info!("Identity balance before withdrawal: {}", initial_balance);

    // Get a Core address to withdraw to
    let withdraw_address_str = get_receive_address(&ctx.app_context, &wallet_arc);
    let withdraw_address = Address::from_str(&withdraw_address_str)
        .expect("Valid address")
        .assume_checked();

    // Withdraw a tenth of the credits
    let withdraw_amount = initial_balance / 10;
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
            tracing::info!("Withdrawal successful, fee: {:?}", fee_result);
        }
        other => panic!("Expected WithdrewFromIdentity, got: {:?}", other),
    }
}
