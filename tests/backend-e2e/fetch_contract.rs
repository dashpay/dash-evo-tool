//! Test: Fetch data contracts from Platform.

use crate::harness::CTX;
use crate::task_runner::run_task;
use dash_evo_tool::backend_task::contract::ContractTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::platform::Identifier;

/// Fetch the DashPay system contract and verify its structure.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_fetch_data_contract() {
    let ctx = &*CTX;
    let app_context = &ctx.app_context;

    // Use DashPay contract ID (well-known system contract)
    let dashpay_contract_id = app_context.dashpay_contract_id();

    // Fetch contract
    let task = BackendTask::ContractTask(Box::new(ContractTask::FetchContracts(vec![
        dashpay_contract_id,
    ])));
    let result = run_task(app_context, task)
        .await
        .expect("FetchContracts should succeed");

    match result {
        BackendTaskSuccessResult::FetchedContracts(contracts) => {
            assert_eq!(contracts.len(), 1, "Should return one contract");
            let contract = contracts[0].as_ref().expect("Contract should exist");
            assert_eq!(
                contract.id(),
                dashpay_contract_id,
                "Contract ID should match"
            );
            println!("  Fetched contract: {:?}", contract.id());
        }
        other => panic!("Expected FetchedContracts, got: {:?}", other),
    }

    // Fetch non-existent contract -> should return None
    let fake_id = Identifier::random();
    let task = BackendTask::ContractTask(Box::new(ContractTask::FetchContracts(vec![fake_id])));
    let result = run_task(app_context, task)
        .await
        .expect("FetchContracts for non-existent should not error");

    match result {
        BackendTaskSuccessResult::FetchedContracts(contracts) => {
            assert_eq!(contracts.len(), 1);
            assert!(
                contracts[0].is_none(),
                "Non-existent contract should return None"
            );
            println!("  Non-existent contract correctly returned None");
        }
        other => panic!("Expected FetchedContracts, got: {:?}", other),
    }

    // Fetch with descriptions
    let task = BackendTask::ContractTask(Box::new(ContractTask::FetchContractsWithDescriptions(
        vec![dashpay_contract_id],
    )));
    let result = run_task(app_context, task)
        .await
        .expect("FetchContractsWithDescriptions should succeed");

    match result {
        BackendTaskSuccessResult::ContractsWithDescriptions(map) => {
            assert!(
                map.contains_key(&dashpay_contract_id),
                "Should contain the requested contract"
            );
            println!("  Fetched contract with descriptions successfully");
        }
        other => panic!("Expected ContractsWithDescriptions, got: {:?}", other),
    }
}
