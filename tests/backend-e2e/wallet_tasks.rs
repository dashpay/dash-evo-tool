// Tests implemented in Task 2 (WalletTask tests: TC-012 to TC-019)

use crate::framework::harness;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::core::CoreTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::identity::core_script::CoreScript;
use std::collections::BTreeMap;
use std::time::Duration;

// ─── TC-012 ───────────────────────────────────────────────────────────────────

/// TC-012: GenerateReceiveAddress — basic derivation and uniqueness.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_012_generate_receive_address() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let task1 = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let result1 = run_task(&ctx.app_context, task1)
        .await
        .expect("TC-012: first GenerateReceiveAddress failed");

    let address1 = match result1 {
        BackendTaskSuccessResult::GeneratedReceiveAddress {
            seed_hash: sh,
            address,
        } => {
            assert_eq!(sh, seed_hash, "TC-012: seed_hash mismatch");
            address
        }
        other => panic!("TC-012: expected GeneratedReceiveAddress, got: {:?}", other),
    };

    // Testnet addresses start with 'y' or '8'
    let first_char = address1.chars().next().unwrap_or_default();
    assert!(
        first_char == 'y' || first_char == '8',
        "TC-012: expected testnet address starting with 'y' or '8', got: {}",
        address1
    );

    // Second call should produce a different address (key derivation advances)
    let task2 = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let result2 = run_task(&ctx.app_context, task2)
        .await
        .expect("TC-012: second GenerateReceiveAddress failed");

    let address2 = match result2 {
        BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => address,
        other => panic!("TC-012: expected GeneratedReceiveAddress, got: {:?}", other),
    };

    assert_ne!(
        address1, address2,
        "TC-012: second call should return a different address"
    );

    tracing::info!("TC-012 passed: addr1={} addr2={}", address1, address2);
}

// ─── TC-013 ───────────────────────────────────────────────────────────────────

/// TC-013: FetchPlatformAddressBalances — verify task returns valid result.
///
/// The framework wallet may have funded platform addresses from previous
/// test runs (the workdir is persistent), so we cannot assume empty balances.
/// We only verify the task succeeds and returns the correct result type.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_013_fetch_platform_address_balances_empty() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-013: FetchPlatformAddressBalances failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash: sh,
            balances,
            network,
        } => {
            assert_eq!(sh, seed_hash, "TC-013: seed_hash mismatch");
            assert_eq!(
                network,
                dash_sdk::dpp::dashcore::Network::Testnet,
                "TC-013: expected testnet network"
            );
            tracing::info!(
                "TC-013 passed: {} platform addresses returned",
                balances.len()
            );
        }
        other => panic!("TC-013: expected PlatformAddressBalances, got: {:?}", other),
    }
}

// ─── TC-014: wallet platform lifecycle (fund → fetch → transfer → withdraw) ──

/// Fund a platform address from wallet UTXOs and return the seed hash.
async fn step_fund_platform_address(
    ctx: &crate::framework::harness::BackendTestContext,
) -> WalletSeedHash {
    tracing::info!("=== Step 1: Fund platform address from wallet UTXOs ===");
    let seed_hash = ctx.framework_wallet_hash;

    let wallet_arc = {
        let wallets = ctx.app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&seed_hash)
            .expect("framework wallet missing")
            .clone()
    };

    let platform_addr = {
        let mut wallet = wallet_arc.write().expect("wallet write lock");
        let addr = wallet
            .platform_receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                false,
                Some(&ctx.app_context),
            )
            .expect("step_fund_platform_address: failed to derive platform receive address");
        PlatformAddress::try_from(addr)
            .expect("step_fund_platform_address: failed to convert to PlatformAddress")
    };

    tracing::info!(
        "step_fund_platform_address: funding platform address {:?}",
        platform_addr
    );

    let task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
        seed_hash,
        amount: 1_000_000, // 0.01 DASH in duffs
        destination: platform_addr,
        fee_deduct_from_output: true,
    });

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("step_fund_platform_address: FundPlatformAddressFromWalletUtxos failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressFunded { seed_hash: sh } => {
            assert_eq!(
                sh, seed_hash,
                "step_fund_platform_address: seed_hash mismatch"
            );
            tracing::info!("step_fund_platform_address: PlatformAddressFunded confirmed");
        }
        other => panic!(
            "step_fund_platform_address: expected PlatformAddressFunded, got: {:?}",
            other
        ),
    }

    seed_hash
}

/// Fetch platform address balances and assert at least one is funded.
async fn step_fetch_balances(
    ctx: &crate::framework::harness::BackendTestContext,
    seed_hash: WalletSeedHash,
) {
    tracing::info!("=== Step 2: Fetch platform address balances after funding ===");

    let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("step_fetch_balances: FetchPlatformAddressBalances failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash: sh,
            balances,
            network,
        } => {
            assert_eq!(sh, seed_hash, "step_fetch_balances: seed_hash mismatch");
            assert_eq!(
                network,
                dash_sdk::dpp::dashcore::Network::Testnet,
                "step_fetch_balances: expected testnet"
            );
            let any_funded = balances.values().any(|(balance, _)| *balance > 0);
            assert!(
                any_funded,
                "step_fetch_balances: expected at least one funded platform address after funding, got: {:?}",
                balances
            );
            tracing::info!(
                "step_fetch_balances passed: {} platform addresses, at least one with credits",
                balances.len()
            );
        }
        other => panic!(
            "step_fetch_balances: expected PlatformAddressBalances, got: {:?}",
            other
        ),
    }
}

/// Transfer half the funded balance to a second platform address.
async fn step_transfer_credits(
    ctx: &crate::framework::harness::BackendTestContext,
    seed_hash: WalletSeedHash,
) {
    tracing::info!("=== Step 3: Transfer platform credits to a second address ===");

    let wallet_arc = {
        let wallets = ctx.app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&seed_hash)
            .expect("framework wallet missing")
            .clone()
    };

    // Derive the first platform address (the one step 1 funded) so it is
    // guaranteed to be in watched_addresses. Then derive a fresh second one
    // as the transfer destination.
    let (source_candidate, dest_addr) = {
        let mut wallet = wallet_arc.write().expect("wallet write lock");
        let src = wallet
            .platform_receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                false, // reuse existing — same address step 1 funded
                Some(&ctx.app_context),
            )
            .expect("step_transfer_credits: failed to derive source platform address");
        let dst = wallet
            .platform_receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                true, // skip_known — derive a fresh one
                Some(&ctx.app_context),
            )
            .expect("step_transfer_credits: failed to derive second platform address");
        (
            PlatformAddress::try_from(src).expect("step_transfer_credits: src PlatformAddress"),
            PlatformAddress::try_from(dst).expect("step_transfer_credits: dst PlatformAddress"),
        )
    };

    // Fetch current platform address balances to get the funded amount.
    let fetch_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let fetch_result = run_task(&ctx.app_context, fetch_task)
        .await
        .expect("step_transfer_credits: pre-transfer FetchPlatformAddressBalances failed");

    let (source_addr, current_source_balance) = match fetch_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
            // Prefer the address we derived (guaranteed in watched_addresses).
            // Fall back to any funded address if the derived one has no balance.
            if let Some((bal, _)) = balances.get(&source_candidate) {
                if *bal > 0 {
                    (source_candidate, *bal)
                } else {
                    balances
                        .iter()
                        .find(|(_, (balance, _))| *balance > 0)
                        .map(|(addr, (balance, _))| (*addr, *balance))
                        .expect("step_transfer_credits: no funded platform address found")
                }
            } else {
                balances
                    .iter()
                    .find(|(_, (balance, _))| *balance > 0)
                    .map(|(addr, (balance, _))| (*addr, *balance))
                    .expect("step_transfer_credits: no funded platform address found")
            }
        }
        other => panic!(
            "step_transfer_credits: expected PlatformAddressBalances, got: {:?}",
            other
        ),
    };

    assert_ne!(
        source_addr, dest_addr,
        "step_transfer_credits: source and destination must differ"
    );

    // Transfer half the balance
    let transfer_amount = current_source_balance / 2;
    assert!(
        transfer_amount > 0,
        "step_transfer_credits: source balance too low to transfer"
    );

    let mut inputs = BTreeMap::new();
    inputs.insert(source_addr, transfer_amount);

    let mut outputs = BTreeMap::new();
    outputs.insert(dest_addr, transfer_amount);

    tracing::info!(
        "step_transfer_credits: transferring {} credits from {:?} to {:?}",
        transfer_amount,
        source_addr,
        dest_addr
    );

    let task = BackendTask::WalletTask(WalletTask::TransferPlatformCredits {
        seed_hash,
        inputs,
        outputs,
        fee_payer_index: 0,
    });

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("step_transfer_credits: TransferPlatformCredits failed");

    match result {
        BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash: sh } => {
            assert_eq!(sh, seed_hash, "step_transfer_credits: seed_hash mismatch");
            tracing::info!("step_transfer_credits: PlatformCreditsTransferred confirmed");
        }
        other => panic!(
            "step_transfer_credits: expected PlatformCreditsTransferred, got: {:?}",
            other
        ),
    }

    // Verify destination has credits after transfer
    let verify_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let verify_result = run_task(&ctx.app_context, verify_task)
        .await
        .expect("step_transfer_credits: post-transfer FetchPlatformAddressBalances failed");

    if let BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } = verify_result {
        let dest_credits = balances.get(&dest_addr).map(|(b, _)| *b).unwrap_or(0);
        assert!(
            dest_credits > 0,
            "step_transfer_credits: destination address should have credits after transfer, got 0"
        );
        tracing::info!(
            "step_transfer_credits passed: dest credits = {}",
            dest_credits
        );
    }
}

/// Fund a fresh platform address and withdraw its balance back to Core.
async fn step_withdraw(
    ctx: &crate::framework::harness::BackendTestContext,
    seed_hash: WalletSeedHash,
) {
    tracing::info!("=== Step 4: Withdraw from platform address back to Core ===");

    let wallet_arc = {
        let wallets = ctx.app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&seed_hash)
            .expect("framework wallet missing")
            .clone()
    };

    // Fund a fresh platform address so we have credits to withdraw,
    // regardless of what step 3 did to the original address.
    let fresh_addr = {
        let mut wallet = wallet_arc.write().expect("wallet write lock");
        let addr = wallet
            .platform_receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                true,
                Some(&ctx.app_context),
            )
            .expect("step_withdraw: failed to derive platform address");
        PlatformAddress::try_from(addr)
            .expect("step_withdraw: failed to convert to PlatformAddress")
    };

    let fund_task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
        seed_hash,
        amount: 500_000,
        destination: fresh_addr,
        fee_deduct_from_output: true,
    });
    run_task(&ctx.app_context, fund_task)
        .await
        .expect("step_withdraw: FundPlatformAddressFromWalletUtxos failed");

    // Poll until the fresh address has credits on Platform.
    let poll_timeout = Duration::from_secs(90);
    let poll_interval = Duration::from_secs(3);
    let start = std::time::Instant::now();

    let (withdrawal_addr, withdrawal_balance) = loop {
        let fetch_task =
            BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
        let fetch_result = run_task(&ctx.app_context, fetch_task)
            .await
            .expect("step_withdraw: FetchPlatformAddressBalances failed");

        let found = match fetch_result {
            BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
                if let Some((bal, _)) = balances.get(&fresh_addr) {
                    if *bal > 0 {
                        Some((fresh_addr, *bal))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            other => panic!(
                "step_withdraw: expected PlatformAddressBalances, got: {:?}",
                other
            ),
        };

        if let Some(entry) = found {
            break entry;
        }

        if start.elapsed() > poll_timeout {
            panic!(
                "step_withdraw: funded platform address {:?} not found for withdrawal within {:?}",
                fresh_addr, poll_timeout
            );
        }

        tracing::info!(
            "step_withdraw: fresh address not yet funded on Platform, retrying in {:?}...",
            poll_interval
        );
        tokio::time::sleep(poll_interval).await;
    };

    tracing::info!(
        "step_withdraw: withdrawing {} credits from {:?}",
        withdrawal_balance,
        withdrawal_addr
    );

    // Get a Core receive address for the withdrawal output
    let receive_addr_task =
        BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let receive_result = run_task(&ctx.app_context, receive_addr_task)
        .await
        .expect("step_withdraw: GenerateReceiveAddress failed");

    let core_address_str = match receive_result {
        BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => address,
        other => panic!(
            "step_withdraw: expected GeneratedReceiveAddress, got: {:?}",
            other
        ),
    };

    let core_address: dash_sdk::dpp::dashcore::Address = core_address_str
        .parse::<dash_sdk::dpp::dashcore::Address<
            dash_sdk::dpp::dashcore::address::NetworkUnchecked,
        >>()
        .expect("step_withdraw: failed to parse core address")
        .assume_checked();

    let output_script = CoreScript::new(core_address.script_pubkey());

    let mut inputs = BTreeMap::new();
    inputs.insert(withdrawal_addr, withdrawal_balance);

    let task = BackendTask::WalletTask(WalletTask::WithdrawFromPlatformAddress {
        seed_hash,
        inputs,
        output_script,
        core_fee_per_byte: 1,
        fee_payer_index: 0,
    });

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("step_withdraw: WithdrawFromPlatformAddress failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash: sh } => {
            assert_eq!(sh, seed_hash, "step_withdraw: seed_hash mismatch");
            tracing::info!("step_withdraw: PlatformAddressWithdrawal confirmed");
        }
        other => panic!(
            "step_withdraw: expected PlatformAddressWithdrawal, got: {:?}",
            other
        ),
    }

    // Verify the source address balance is reduced
    let verify_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let verify_result = run_task(&ctx.app_context, verify_task)
        .await
        .expect("step_withdraw: post-withdrawal FetchPlatformAddressBalances failed");

    if let BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } = verify_result {
        let remaining = balances.get(&withdrawal_addr).map(|(b, _)| *b).unwrap_or(0);
        assert!(
            remaining < withdrawal_balance,
            "step_withdraw: withdrawal address balance should decrease after withdrawal (was {}, now {})",
            withdrawal_balance,
            remaining
        );
        tracing::info!("step_withdraw passed: remaining credits = {}", remaining);
    }
}

/// TC-014: Wallet platform lifecycle — fund → fetch → transfer → withdraw.
///
/// Covers the full TC-014 → TC-015 → TC-016 → TC-017 dependency chain in a
/// single sequenced test so shared state flows naturally between steps.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_014_wallet_platform_lifecycle() {
    let ctx = harness::ctx().await;

    let seed_hash = step_fund_platform_address(ctx).await;
    step_fetch_balances(ctx, seed_hash).await;
    step_transfer_credits(ctx, seed_hash).await;
    step_withdraw(ctx, seed_hash).await;

    tracing::info!("TC-014 wallet platform lifecycle passed");
}

// ─── TC-018 ───────────────────────────────────────────────────────────────────

/// TC-018: FundPlatformAddressFromAssetLock — create an asset lock via CoreTask and then
/// fund a platform address directly from it.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_018_fund_platform_address_from_asset_lock() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let wallet_arc = {
        let wallets = ctx.app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&seed_hash)
            .expect("framework wallet missing")
            .clone()
    };

    // Step 1: Broadcast an asset lock registration transaction via CoreTask
    tracing::info!("TC-018: broadcasting CreateRegistrationAssetLock (identity_index=98)...");
    let create_task = BackendTask::CoreTask(CoreTask::CreateRegistrationAssetLock(
        wallet_arc.clone(),
        100_000_000, // credits
        98,          // use an unused identity index
    ));

    let create_result = run_task(&ctx.app_context, create_task)
        .await
        .expect("TC-018: CreateRegistrationAssetLock failed");

    // The task broadcasts the tx and returns a Message (broadcast confirmation).
    // The IS lock arrives asynchronously via SPV and populates unused_asset_locks.
    assert!(
        matches!(create_result, BackendTaskSuccessResult::Message(_)),
        "TC-018: expected Message from CreateRegistrationAssetLock, got: {:?}",
        create_result
    );

    // Step 2: Wait for the asset lock proof to appear in unused_asset_locks
    tracing::info!("TC-018: waiting for asset lock IS proof in unused_asset_locks...");
    let proof_timeout = Duration::from_secs(360);
    let (asset_lock_address, asset_lock_proof) = tokio::time::timeout(proof_timeout, async {
        loop {
            let maybe_lock = {
                let wallet = wallet_arc.read().expect("wallet read lock");
                wallet
                    .unused_asset_locks
                    .iter()
                    .find_map(|(_tx, addr, _amount, _islock, proof)| {
                        proof.as_ref().map(|proof| (addr.clone(), proof.clone()))
                    })
            };
            if let Some(found) = maybe_lock {
                return found;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .expect("TC-018: timed out waiting for asset lock IS proof");

    tracing::info!(
        "TC-018: asset lock proof ready, address={:?}",
        asset_lock_address
    );

    // Step 3: Derive a fresh platform address for funding
    let platform_addr = {
        let mut wallet = wallet_arc.write().expect("wallet write lock");
        let addr = wallet
            .platform_receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                true, // skip_known — get a fresh one
                Some(&ctx.app_context),
            )
            .expect("TC-018: failed to derive platform address");
        PlatformAddress::try_from(addr).expect("TC-018: failed to convert to PlatformAddress")
    };

    let mut outputs = BTreeMap::new();
    outputs.insert(platform_addr, None); // None = distribute evenly

    // Step 4: Fund platform address from the asset lock
    tracing::info!(
        "TC-018: funding platform address {:?} from asset lock",
        platform_addr
    );
    let fund_task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromAssetLock {
        seed_hash,
        asset_lock_proof: Box::new(asset_lock_proof),
        asset_lock_address,
        outputs,
    });

    let result = run_task(&ctx.app_context, fund_task)
        .await
        .expect("TC-018: FundPlatformAddressFromAssetLock failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressFunded { seed_hash: sh } => {
            assert_eq!(sh, seed_hash, "TC-018: seed_hash mismatch");
            tracing::info!("TC-018 passed: PlatformAddressFunded confirmed");
        }
        other => panic!("TC-018: expected PlatformAddressFunded, got: {:?}", other),
    }
}

// ─── TC-019 ───────────────────────────────────────────────────────────────────

/// TC-019: WalletTask error path — unknown seed hash returns a typed error, not a panic.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_019_wallet_task_error_unknown_seed_hash() {
    let ctx = harness::ctx().await;

    // Construct a seed hash that does not match any loaded wallet
    let fake_seed_hash: WalletSeedHash = [
        0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa,
        0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
        0x0b, 0x0c,
    ];

    let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
        seed_hash: fake_seed_hash,
    });

    let result = run_task(&ctx.app_context, task).await;

    assert!(
        result.is_err(),
        "TC-019: expected Err for unknown seed hash, got Ok"
    );

    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            dash_evo_tool::backend_task::error::TaskError::WalletNotFound
        ),
        "TC-019: expected WalletNotFound, got: {:?}",
        err
    );

    tracing::info!("TC-019 passed: WalletNotFound error confirmed");
}
