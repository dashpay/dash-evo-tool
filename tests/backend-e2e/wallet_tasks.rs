// Tests implemented in Task 2 (WalletTask tests: TC-012 to TC-019)

use crate::framework::harness;
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::core::CoreTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::WalletSeedHash;
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

/// TC-012b (FUNDS-SAFETY): the address the Receive flow hands out via
/// `GenerateReceiveAddress` must be one SPV actually watches.
///
/// A real user lost a deposit because the old Receive "New Address" button
/// derived addresses past the gap window (index 32), outside the SPV-watched
/// pool, so the funds never appeared. This pins the invariant the fix
/// guarantees: every generated receive address is in
/// `monitored_receive_addresses` — the SPV-watched gap-limit window. RED on
/// the legacy DET-side derivation, GREEN once the flow routes through the
/// upstream watched pool.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_012b_receive_address_is_spv_watched() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-012b: GenerateReceiveAddress failed");
    let address = match result {
        BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => address,
        other => panic!(
            "TC-012b: expected GeneratedReceiveAddress, got: {:?}",
            other
        ),
    };

    // `monitored_receive_addresses` takes the manager's blocking read lock, so
    // it must run outside the async task. `block_in_place` is valid on this
    // multi-thread runtime.
    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    let watched = tokio::task::block_in_place(|| backend.monitored_receive_addresses(&seed_hash))
        .expect("monitored_receive_addresses");

    assert!(
        watched.contains(&address),
        "TC-012b: generated receive address {address} is not in the SPV-watched pool \
         (funds sent there would be invisible); watched window has {} addresses",
        watched.len()
    );
    tracing::info!(
        "TC-012b passed: {address} is one of {} SPV-watched addresses",
        watched.len()
    );
}

/// TC-012c (FUNDS-SAFETY, asset-lock funding / H1): the Create-Asset-Lock
/// screen shows a deposit address (QR + Copy) for the user to send DASH to;
/// the asset lock is then built from the resulting watched UTXO. That deposit
/// address must therefore be SPV-watched, or the deposit is invisible and the
/// asset lock can never be built.
///
/// The screen now derives the deposit address through the same
/// `GenerateReceiveAddress` task as the Receive flow (upstream watched pool),
/// not the legacy unbounded `Wallet::receive_address(skip=true)`. This pins the
/// funding-address ∈ watched-pool invariant for that scenario. RED on the
/// legacy path, GREEN on the fix.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore]
async fn tc_012c_asset_lock_funding_address_is_spv_watched() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-012c: GenerateReceiveAddress failed");
    let address = match result {
        BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => address,
        other => panic!(
            "TC-012c: expected GeneratedReceiveAddress, got: {:?}",
            other
        ),
    };

    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    let watched = tokio::task::block_in_place(|| backend.monitored_receive_addresses(&seed_hash))
        .expect("monitored_receive_addresses");

    assert!(
        watched.contains(&address),
        "TC-012c: asset-lock deposit address {address} is not in the SPV-watched pool \
         (a deposit there would be invisible and the asset lock could never be built)"
    );
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

    let platform_addr = crate::framework::funding::derive_platform_receive_address(
        &ctx.app_context,
        seed_hash,
        dash_sdk::dpp::dashcore::Network::Testnet,
        false,
    )
    .await;

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

    // Platform address funding (FundPlatformAddressFromWalletUtxos) is safe
    // outside FUNDING_MUTEX because it uses DIP-17 derivation path UTXOs
    // (m/9'/coin_type'/17'/...) which are disjoint from the BIP44 UTXOs
    // used by create_funded_test_wallet. No double-spend risk.
    let result = run_task_with_nonce_retry(&ctx.app_context, task)
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

/// Fetch platform address balances and assert the address funded in step 1 has credits.
async fn step_fetch_balances(
    ctx: &crate::framework::harness::BackendTestContext,
    seed_hash: WalletSeedHash,
) {
    tracing::info!("=== Step 2: Fetch platform address balances after funding ===");

    // Re-derive the same platform address that step 1 funded (reuse=false
    // returns the same address as long as it hasn't been marked used).
    let expected_addr = crate::framework::funding::derive_platform_receive_address(
        &ctx.app_context,
        seed_hash,
        dash_sdk::dpp::dashcore::Network::Testnet,
        false,
    )
    .await;

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
            let specific_balance = balances.get(&expected_addr).map(|(b, _)| *b).unwrap_or(0);
            assert!(
                specific_balance > 0,
                "step_fetch_balances: expected address {:?} should have credits after funding, \
                 got 0. All balances: {:?}",
                expected_addr,
                balances
            );
            tracing::info!(
                "step_fetch_balances passed: address {:?} has {} credits",
                expected_addr,
                specific_balance
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

    // Derive the first platform address (the one step 1 funded) so it is
    // guaranteed to be in watched_addresses. Then derive a fresh second one
    // as the transfer destination.
    let source_candidate = crate::framework::funding::derive_platform_receive_address(
        &ctx.app_context,
        seed_hash,
        dash_sdk::dpp::dashcore::Network::Testnet,
        false, // reuse existing — same address step 1 funded
    )
    .await;
    let dest_addr = crate::framework::funding::derive_platform_receive_address(
        &ctx.app_context,
        seed_hash,
        dash_sdk::dpp::dashcore::Network::Testnet,
        true, // skip_known — derive a fresh one
    )
    .await;

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

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
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

    // TODO: This step fails because sync_address_balances returns a balance
    // (~485M credits) that doesn't match what Platform's state transition
    // processor sees (1 credit). The full tree scan proof says 485M but the
    // withdrawal is rejected with AddressesNotEnoughFundsError. This is a
    // Platform/SDK bug — the sync proof and the state transition processor
    // disagree on the balance, possibly due to node height differences or
    // proof verification issues. Needs investigation upstream.

    // Fund a fresh platform address so we have credits to withdraw,
    // regardless of what step 3 did to the original address.
    let fresh_addr = crate::framework::funding::derive_platform_receive_address(
        &ctx.app_context,
        seed_hash,
        dash_sdk::dpp::dashcore::Network::Testnet,
        true,
    )
    .await;

    let fund_task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
        seed_hash,
        amount: 500_000,
        destination: fresh_addr,
        fee_deduct_from_output: true,
    });
    run_task_with_nonce_retry(&ctx.app_context, fund_task)
        .await
        .expect("step_withdraw: FundPlatformAddressFromWalletUtxos failed");

    // Poll until the fresh address has credits on Platform.
    let poll_timeout = harness::MAX_TEST_TIMEOUT;
    let poll_interval = Duration::from_secs(5);
    let start = std::time::Instant::now();

    // Reset again so the next sync picks up the new funding
    if let Err(e) = ctx.app_context.set_platform_sync_info(&seed_hash, 0, 0) {
        tracing::warn!("Failed to reset platform sync info: {}", e);
    }

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

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
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
///
/// TODO(#799): This test fails because CreateRegistrationAssetLock generates a
/// one-time key address for the credit output that is NOT registered in
/// `known_addresses`. When the IS lock arrives, `received_asset_lock_finality`
/// skips the wallet (address not recognized), so `unused_asset_locks` is never
/// populated and the test times out waiting for the proof. Fix is tracked in
/// issue #799 (unify asset lock paths). The workaround would be to register
/// the one-time key address in known_addresses during asset lock creation.
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

    // Step 2: Wait for a tracked asset lock with the expected amount and a
    // ready proof from the upstream `AssetLockManager`.
    tracing::info!("TC-018: waiting for tracked asset lock IS proof...");
    let proof_timeout = harness::MAX_TEST_TIMEOUT;
    let min_credits: u64 = 90_000_000;
    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("TC-018: wallet backend not ready");
    let tracked_out_point = tokio::time::timeout(proof_timeout, async {
        loop {
            let maybe = backend
                .list_tracked_asset_locks(&seed_hash)
                .await
                .ok()
                .and_then(|locks| {
                    locks.into_iter().find_map(|l| {
                        if l.amount >= min_credits && l.proof.is_some() {
                            Some(l.out_point)
                        } else {
                            None
                        }
                    })
                });
            if let Some(op) = maybe {
                return op;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .expect("TC-018: timed out waiting for tracked asset lock IS proof");

    tracing::info!(
        "TC-018: tracked asset lock ready, out_point={}",
        tracked_out_point
    );

    // Step 3: Derive a fresh platform address for funding
    let platform_addr = crate::framework::funding::derive_platform_receive_address(
        &ctx.app_context,
        seed_hash,
        dash_sdk::dpp::dashcore::Network::Testnet,
        true, // skip_known — get a fresh one
    )
    .await;

    let mut outputs = BTreeMap::new();
    outputs.insert(platform_addr, None); // None = distribute evenly

    // Step 4: Fund platform address from the asset lock
    tracing::info!(
        "TC-018: funding platform address {:?} from asset lock",
        platform_addr
    );
    let fund_task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromAssetLock {
        seed_hash,
        out_point: tracked_out_point,
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
