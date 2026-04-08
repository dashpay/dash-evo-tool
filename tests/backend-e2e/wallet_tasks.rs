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
use tokio::sync::OnceCell;

// Module-level state for the TC-014 → TC-015 → TC-016 → TC-017 sequence.
// Tests run serially within this file; the funded platform address is reused.
struct FundedPlatformState {
    seed_hash: WalletSeedHash,
    funded_address: PlatformAddress,
    funded_balance: u64,
}

static FUNDED_PLATFORM: OnceCell<FundedPlatformState> = OnceCell::const_new();

// ─── TC-012 ───────────────────────────────────────────────────────────────────

/// TC-012: GenerateReceiveAddress — basic derivation and uniqueness.
#[tokio_shared_rt::test(shared)]
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

/// TC-013: FetchPlatformAddressBalances — no platform addresses funded (baseline).
#[tokio_shared_rt::test(shared)]
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
            // Balances may be empty or contain zero-balance entries — both are valid
            let all_zero = balances.values().all(|(balance, _nonce)| *balance == 0);
            assert!(
                balances.is_empty() || all_zero,
                "TC-013: expected no funded platform addresses at baseline, got: {:?}",
                balances
            );
            tracing::info!(
                "TC-013 passed: {} platform addresses (all zero or empty)",
                balances.len()
            );
        }
        other => panic!("TC-013: expected PlatformAddressBalances, got: {:?}", other),
    }
}

// ─── TC-014 ───────────────────────────────────────────────────────────────────

/// TC-014: FundPlatformAddressFromWalletUtxos — funds a platform address and verifies balance.
#[tokio_shared_rt::test(shared)]
#[ignore]
async fn tc_014_fund_platform_address_from_wallet_utxos() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    // Derive a platform receive address from the framework wallet
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
            .expect("TC-014: failed to derive platform receive address");
        PlatformAddress::try_from(addr).expect("TC-014: failed to convert to PlatformAddress")
    };

    tracing::info!("TC-014: funding platform address {:?}", platform_addr);

    let task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
        seed_hash,
        amount: 1_000_000, // 0.01 DASH in duffs
        destination: platform_addr,
        fee_deduct_from_output: true,
    });

    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-014: FundPlatformAddressFromWalletUtxos failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressFunded { seed_hash: sh } => {
            assert_eq!(sh, seed_hash, "TC-014: seed_hash mismatch");
            tracing::info!("TC-014: PlatformAddressFunded confirmed");
        }
        other => panic!("TC-014: expected PlatformAddressFunded, got: {:?}", other),
    }

    // Verify balance — fetch platform address balances and confirm > 0
    let fetch_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let fetch_result = run_task(&ctx.app_context, fetch_task)
        .await
        .expect("TC-014: FetchPlatformAddressBalances failed");

    let (funded_address, funded_balance) = match fetch_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => balances
            .iter()
            .find(|(_, (balance, _))| *balance > 0)
            .map(|(addr, (balance, _))| (*addr, *balance))
            .expect("TC-014: expected at least one funded platform address"),
        other => panic!("TC-014: expected PlatformAddressBalances, got: {:?}", other),
    };

    tracing::info!(
        "TC-014 passed: funded_address={:?}, balance={} credits",
        funded_address,
        funded_balance
    );

    // Store state for TC-015 → TC-016 → TC-017
    FUNDED_PLATFORM
        .set(FundedPlatformState {
            seed_hash,
            funded_address,
            funded_balance,
        })
        .ok(); // Ignore if already set (test re-run scenario)
}

// ─── TC-015 ───────────────────────────────────────────────────────────────────

/// TC-015: FetchPlatformAddressBalances — after TC-014 funding, at least one address has credits.
#[tokio_shared_rt::test(shared)]
#[ignore]
async fn tc_015_fetch_platform_address_balances_after_funding() {
    let ctx = harness::ctx().await;

    // This test depends on TC-014 having run. In serial execution the OnceCell
    // should be set; if it isn't, we fall back to the framework wallet's
    // current platform state.
    let seed_hash = match FUNDED_PLATFORM.get() {
        Some(state) => state.seed_hash,
        None => ctx.framework_wallet_hash,
    };

    let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-015: FetchPlatformAddressBalances failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash: sh,
            balances,
            network,
        } => {
            assert_eq!(sh, seed_hash, "TC-015: seed_hash mismatch");
            assert_eq!(
                network,
                dash_sdk::dpp::dashcore::Network::Testnet,
                "TC-015: expected testnet"
            );
            let any_funded = balances.values().any(|(balance, _)| *balance > 0);
            assert!(
                any_funded,
                "TC-015: expected at least one funded platform address after TC-014, got: {:?}",
                balances
            );
            tracing::info!(
                "TC-015 passed: {} platform addresses, at least one with credits",
                balances.len()
            );
        }
        other => panic!("TC-015: expected PlatformAddressBalances, got: {:?}", other),
    }
}

// ─── TC-016 ───────────────────────────────────────────────────────────────────

/// TC-016: TransferPlatformCredits — transfer half the funded balance to a second platform address.
#[tokio_shared_rt::test(shared)]
#[ignore]
async fn tc_016_transfer_platform_credits() {
    let ctx = harness::ctx().await;

    let state = FUNDED_PLATFORM
        .get()
        .expect("TC-016: FUNDED_PLATFORM not set — TC-014 must run first");
    let seed_hash = state.seed_hash;
    let source_addr = state.funded_address;

    // Derive a second platform address as the destination
    let wallet_arc = {
        let wallets = ctx.app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&seed_hash)
            .expect("framework wallet missing")
            .clone()
    };

    let dest_addr = {
        let mut wallet = wallet_arc.write().expect("wallet write lock");
        let addr = wallet
            .platform_receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                true, // skip_known_addresses — derive a fresh one
                Some(&ctx.app_context),
            )
            .expect("TC-016: failed to derive second platform address");
        PlatformAddress::try_from(addr).expect("TC-016: failed to convert to PlatformAddress")
    };

    assert_ne!(
        source_addr, dest_addr,
        "TC-016: source and destination must differ"
    );

    // Re-fetch current balance of the source address to get an accurate amount
    let fetch_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let fetch_result = run_task(&ctx.app_context, fetch_task)
        .await
        .expect("TC-016: pre-transfer FetchPlatformAddressBalances failed");

    let current_source_balance = match fetch_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => balances
            .get(&source_addr)
            .map(|(bal, _)| *bal)
            .unwrap_or(state.funded_balance),
        _ => state.funded_balance,
    };

    // Transfer half the balance
    let transfer_amount = current_source_balance / 2;
    assert!(
        transfer_amount > 0,
        "TC-016: source balance too low to transfer"
    );

    let mut inputs = BTreeMap::new();
    inputs.insert(source_addr, transfer_amount);

    let mut outputs = BTreeMap::new();
    outputs.insert(dest_addr, transfer_amount);

    tracing::info!(
        "TC-016: transferring {} credits from {:?} to {:?}",
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
        .expect("TC-016: TransferPlatformCredits failed");

    match result {
        BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash: sh } => {
            assert_eq!(sh, seed_hash, "TC-016: seed_hash mismatch");
            tracing::info!("TC-016: PlatformCreditsTransferred confirmed");
        }
        other => panic!(
            "TC-016: expected PlatformCreditsTransferred, got: {:?}",
            other
        ),
    }

    // Verify both addresses have credits after transfer
    let verify_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let verify_result = run_task(&ctx.app_context, verify_task)
        .await
        .expect("TC-016: post-transfer FetchPlatformAddressBalances failed");

    if let BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } = verify_result {
        let dest_credits = balances.get(&dest_addr).map(|(b, _)| *b).unwrap_or(0);
        assert!(
            dest_credits > 0,
            "TC-016: destination address should have credits after transfer, got 0"
        );
        tracing::info!("TC-016 passed: dest credits = {}", dest_credits);
    }
}

// ─── TC-017 ───────────────────────────────────────────────────────────────────

/// TC-017: WithdrawFromPlatformAddress — withdraw remaining balance back to Core.
#[tokio_shared_rt::test(shared)]
#[ignore]
async fn tc_017_withdraw_from_platform_address() {
    let ctx = harness::ctx().await;

    let state = FUNDED_PLATFORM
        .get()
        .expect("TC-017: FUNDED_PLATFORM not set — TC-014 must run first");
    let seed_hash = state.seed_hash;

    // Fetch current platform address balances to find one with credits
    let fetch_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let fetch_result = run_task(&ctx.app_context, fetch_task)
        .await
        .expect("TC-017: FetchPlatformAddressBalances failed");

    let (withdrawal_addr, withdrawal_balance) = match fetch_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => balances
            .iter()
            .find(|(_, (balance, _))| *balance > 0)
            .map(|(addr, (balance, _))| (*addr, *balance))
            .expect("TC-017: no funded platform address found for withdrawal"),
        other => panic!("TC-017: expected PlatformAddressBalances, got: {:?}", other),
    };

    tracing::info!(
        "TC-017: withdrawing {} credits from {:?}",
        withdrawal_balance,
        withdrawal_addr
    );

    // Get a Core receive address for the withdrawal output
    let receive_addr_task =
        BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let receive_result = run_task(&ctx.app_context, receive_addr_task)
        .await
        .expect("TC-017: GenerateReceiveAddress failed");

    let core_address_str = match receive_result {
        BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => address,
        other => panic!("TC-017: expected GeneratedReceiveAddress, got: {:?}", other),
    };

    let core_address: dash_sdk::dpp::dashcore::Address = core_address_str
        .parse::<dash_sdk::dpp::dashcore::Address<
            dash_sdk::dpp::dashcore::address::NetworkUnchecked,
        >>()
        .expect("TC-017: failed to parse core address")
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
        .expect("TC-017: WithdrawFromPlatformAddress failed");

    match result {
        BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash: sh } => {
            assert_eq!(sh, seed_hash, "TC-017: seed_hash mismatch");
            tracing::info!("TC-017: PlatformAddressWithdrawal confirmed");
        }
        other => panic!(
            "TC-017: expected PlatformAddressWithdrawal, got: {:?}",
            other
        ),
    }

    // Verify the source address balance is reduced
    let verify_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let verify_result = run_task(&ctx.app_context, verify_task)
        .await
        .expect("TC-017: post-withdrawal FetchPlatformAddressBalances failed");

    if let BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } = verify_result {
        let remaining = balances.get(&withdrawal_addr).map(|(b, _)| *b).unwrap_or(0);
        assert!(
            remaining < withdrawal_balance,
            "TC-017: withdrawal address balance should decrease after withdrawal (was {}, now {})",
            withdrawal_balance,
            remaining
        );
        tracing::info!("TC-017 passed: remaining credits = {}", remaining);
    }
}

// ─── TC-018 ───────────────────────────────────────────────────────────────────

/// TC-018: FundPlatformAddressFromAssetLock — create an asset lock via CoreTask and then
/// fund a platform address directly from it.
#[tokio_shared_rt::test(shared)]
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
    let proof_timeout = Duration::from_secs(120);
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
#[tokio_shared_rt::test(shared)]
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
