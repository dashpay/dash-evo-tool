//! Test: Withdraw credits from an identity to a Core address.

use crate::harness::ctx;
use crate::identity_helpers::{build_identity_registration, get_receive_address};
use crate::task_runner::run_task;
use crate::wait::wait_for_spendable_balance;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::str::FromStr;
use std::time::Duration;

/// Create identity, then withdraw some credits to a Core address.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_withdraw_from_identity() {
    let ctx = ctx().await;

    // Create funded test wallet
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(10_000_000).await;

    // Wait for test wallet funds to become spendable before identity registration
    wait_for_spendable_balance(&ctx.app_context, seed_hash, 1, Duration::from_secs(60))
        .await
        .expect("Test wallet funds should become spendable");

    // Register identity (with retry for transient chain sync issues)
    let mut last_error = String::new();
    let mut reg_result = None;
    for attempt in 1..=3 {
        let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
            build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash),
        ));
        match run_task(&ctx.app_context, task).await {
            Ok(r) => {
                reg_result = Some(r);
                break;
            }
            Err(e) => {
                let err_str = e.to_string();
                if attempt < 3
                    && (err_str.contains("chain height")
                        || err_str.contains("Timeout waiting for asset lock"))
                {
                    println!(
                        "  Identity registration attempt {}/3 failed ({}), retrying in 30s...",
                        attempt, err_str
                    );
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    last_error = err_str;
                    continue;
                }
                panic!("Identity registration should succeed: {}", e);
            }
        }
    }
    let result = reg_result.unwrap_or_else(|| {
        panic!(
            "Identity registration failed after 3 attempts: {}",
            last_error
        )
    });

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
            println!("  Withdrawal successful, fee: {:?}", fee_result);
        }
        other => panic!("Expected WithdrewFromIdentity, got: {:?}", other),
    }
}
