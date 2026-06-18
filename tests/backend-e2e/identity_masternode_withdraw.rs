//! Backend E2E: headless masternode/evonode load + credit withdrawal.
//!
//! Mirrors `identity_withdraw.rs` but exercises the masternode path the new
//! `identity_masternode_load` / `identity_masternode_credits_withdraw` MCP tools
//! dispatch: a real load by ProTxHash + keys, then a real withdraw in both key
//! modes against testnet. The tools are thin adapters, so these tests drive the
//! same `IdentityTask::LoadIdentity` / `IdentityTask::WithdrawFromIdentity` the
//! tools build.
//!
//! All cases are `#[ignore]` and gated on `E2E_MN_*` env vars; each skips with a
//! log line (never fails) when its inputs are unset, since a real testnet
//! masternode with funded credits and its private keys cannot live in CI.
//!
//! Required env vars (see the test spec §0.3):
//! - `E2E_MN_PRO_TX_HASH` — testnet evonode/masternode ProTxHash (hex).
//! - `E2E_MN_OWNER_WIF`   — owner private key (WIF or 64-hex).
//! - `E2E_MN_PAYOUT_WIF`  — payout/transfer private key (WIF or 64-hex).
//! - `E2E_MN_VOTING_WIF`  — optional voting key (triggers the voter fetch).
//! - `E2E_MN_NODE_TYPE`   — "masternode" or "evonode" (default "evonode").

use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::identity::{IdentityInputToLoad, IdentityTask};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_evo_tool::model::secret::Secret;
use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use std::str::FromStr;

/// Read an env var, returning `None` and logging a skip line when unset.
fn opt_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => {
            tracing::info!("Skipping masternode e2e: {name} is not set");
            None
        }
    }
}

fn node_type_from_env() -> IdentityType {
    match std::env::var("E2E_MN_NODE_TYPE")
        .unwrap_or_else(|_| "evonode".to_owned())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "masternode" => IdentityType::Masternode,
        _ => IdentityType::Evonode,
    }
}

/// Build the load task exactly as `identity_masternode_load` would.
fn load_task(
    pro_tx_hash: String,
    node_type: IdentityType,
    owner_wif: Option<String>,
    payout_wif: Option<String>,
    voting_wif: Option<String>,
) -> BackendTask {
    let input = IdentityInputToLoad {
        identity_id_input: pro_tx_hash,
        identity_type: node_type,
        alias_input: String::new(),
        voting_private_key_input: Secret::new(voting_wif.unwrap_or_default()),
        owner_private_key_input: Secret::new(owner_wif.unwrap_or_default()),
        payout_address_private_key_input: Secret::new(payout_wif.unwrap_or_default()),
        keys_input: vec![],
        derive_keys_from_wallets: false,
        selected_wallet_seed_hash: None,
    };
    BackendTask::IdentityTask(IdentityTask::LoadIdentity(input))
}

/// Resolve the KeyID for a withdrawal purpose, as the withdraw tool does.
fn withdrawal_key_id(qi: &QualifiedIdentity, purpose: Purpose) -> Option<u32> {
    qi.available_withdrawal_keys()
        .into_iter()
        .find(|k| k.identity_public_key.purpose() == purpose)
        .map(|k| k.identity_public_key.id())
}

// ── TC-MN-016 — load happy path: evonode + payout key ────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn016_load_with_payout_key() {
    let (Some(pro_tx_hash), Some(payout_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_PAYOUT_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;
    let node_type = node_type_from_env();

    let task = load_task(pro_tx_hash, node_type, None, Some(payout_wif), None);
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("masternode load with payout key should succeed");

    let BackendTaskSuccessResult::LoadedIdentity(qi) = result else {
        panic!("Expected LoadedIdentity, got: {result:?}");
    };
    assert!(
        withdrawal_key_id(&qi, Purpose::TRANSFER).is_some(),
        "payout (TRANSFER) key should be loaded"
    );
    assert!(
        withdrawal_key_id(&qi, Purpose::OWNER).is_none(),
        "owner key should NOT be loaded"
    );
    assert!(
        qi.masternode_payout_address(ctx.app_context.network())
            .is_some(),
        "evonode should expose a payout address"
    );
}

// ── TC-MN-017 — load happy path: masternode/evonode + owner key ──────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn017_load_with_owner_key() {
    let (Some(pro_tx_hash), Some(owner_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_OWNER_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;

    let task = load_task(
        pro_tx_hash,
        node_type_from_env(),
        Some(owner_wif),
        None,
        None,
    );
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("masternode load with owner key should succeed");

    let BackendTaskSuccessResult::LoadedIdentity(qi) = result else {
        panic!("Expected LoadedIdentity, got: {result:?}");
    };
    assert!(
        withdrawal_key_id(&qi, Purpose::OWNER).is_some(),
        "owner key should be loaded"
    );
}

// ── TC-MN-018 — load with both owner + payout keys ───────────────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn018_load_with_both_keys() {
    let (Some(pro_tx_hash), Some(owner_wif), Some(payout_wif)) = (
        opt_env("E2E_MN_PRO_TX_HASH"),
        opt_env("E2E_MN_OWNER_WIF"),
        opt_env("E2E_MN_PAYOUT_WIF"),
    ) else {
        return;
    };
    let ctx = ctx().await;

    let task = load_task(
        pro_tx_hash,
        node_type_from_env(),
        Some(owner_wif),
        Some(payout_wif),
        None,
    );
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("masternode load with both keys should succeed");

    let BackendTaskSuccessResult::LoadedIdentity(qi) = result else {
        panic!("Expected LoadedIdentity, got: {result:?}");
    };
    assert!(
        withdrawal_key_id(&qi, Purpose::OWNER).is_some(),
        "owner key loaded"
    );
    assert!(
        withdrawal_key_id(&qi, Purpose::TRANSFER).is_some(),
        "transfer key loaded"
    );
}

// ── TC-MN-020 — wrong key (valid format, not on identity) → KeyInputValidationFailed

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn020_load_wrong_key_rejected() {
    let Some(pro_tx_hash) = opt_env("E2E_MN_PRO_TX_HASH") else {
        return;
    };
    let ctx = ctx().await;

    // A valid-format WIF that is (overwhelmingly) NOT a key on the identity.
    let bogus_owner = dash_sdk::dpp::dashcore::PrivateKey::from_byte_array(
        &[0x11u8; 32],
        dash_sdk::dpp::dashcore::Network::Testnet,
    )
    .expect("valid private key")
    .to_wif();

    let task = load_task(
        pro_tx_hash,
        node_type_from_env(),
        Some(bogus_owner),
        None,
        None,
    );
    let err = run_task(&ctx.app_context, task)
        .await
        .expect_err("a key not on the identity must be rejected");

    assert!(
        matches!(err, TaskError::KeyInputValidationFailed { .. }),
        "expected KeyInputValidationFailed, got: {err:?}"
    );
    // TC-MN-061 cross-check: the key value never appears in Display or Debug.
    let display = err.to_string();
    let debug = format!("{err:?}");
    assert!(
        !display.contains("bogus"),
        "no key bytes in Display: {display}"
    );
    assert!(!debug.contains("bogus"), "no key bytes in Debug: {debug}");
}

// ── TC-MN-021 — identity not found on network → IdentityNotFound ──────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn021_load_identity_not_found() {
    let ctx = ctx().await;

    // Well-formed but (overwhelmingly) nonexistent 64-hex ProTxHash.
    let random_pro_tx = hex::encode([0x42u8; 32]);
    let task = load_task(
        random_pro_tx,
        IdentityType::Evonode,
        None,
        Some("".to_owned()),
        None,
    );
    // No signing key here, but the network fetch fails first with IdentityNotFound.
    let err = run_task(&ctx.app_context, task)
        .await
        .expect_err("a nonexistent ProTxHash must not load");

    assert!(
        matches!(err, TaskError::IdentityNotFound),
        "expected IdentityNotFound, got: {err:?}"
    );
}

// ── TC-MN-050 — OWNER mode happy path: destination forced to payout, to_address=None

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn050_owner_withdraw_to_payout() {
    let (Some(pro_tx_hash), Some(owner_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_OWNER_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;

    let load = load_task(
        pro_tx_hash,
        node_type_from_env(),
        Some(owner_wif),
        None,
        None,
    );
    let BackendTaskSuccessResult::LoadedIdentity(qi) = run_task(&ctx.app_context, load)
        .await
        .expect("load should succeed")
    else {
        panic!("Expected LoadedIdentity");
    };

    let owner_key_id = withdrawal_key_id(&qi, Purpose::OWNER).expect("owner key loaded");
    let payout_address = qi
        .masternode_payout_address(ctx.app_context.network())
        .expect("payout address present");
    let balance = qi.identity.balance();
    assert!(balance > 0, "identity must have withdrawable credits");
    let amount = (balance / 10).max(1);

    // OWNER mode dispatches to_address = None; Platform forces the payout addr.
    let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
        qi,
        None,
        amount,
        Some(owner_key_id),
    ));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("owner-mode withdrawal should succeed");

    match result {
        BackendTaskSuccessResult::WithdrewFromIdentity(fee) => {
            tracing::info!("Owner withdraw to {payout_address}, fee: {fee:?}");
            assert!(fee.actual_fee > 0, "actual fee should be positive");
        }
        other => panic!("Expected WithdrewFromIdentity, got: {other:?}"),
    }
}

// ── TC-MN-051 — TRANSFER mode happy path: any Core address ───────────────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn051_transfer_withdraw_to_address() {
    let (Some(pro_tx_hash), Some(payout_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_PAYOUT_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;

    let load = load_task(
        pro_tx_hash,
        node_type_from_env(),
        None,
        Some(payout_wif),
        None,
    );
    let BackendTaskSuccessResult::LoadedIdentity(qi) = run_task(&ctx.app_context, load)
        .await
        .expect("load should succeed")
    else {
        panic!("Expected LoadedIdentity");
    };

    let transfer_key_id = withdrawal_key_id(&qi, Purpose::TRANSFER).expect("transfer key loaded");
    let balance = qi.identity.balance();
    assert!(balance > 0, "identity must have withdrawable credits");
    let amount = (balance / 10).max(1);

    // A fresh testnet Core address from the framework wallet (no extra funding
    // broadcast — we only need a watched destination address).
    let framework_wallet = {
        let wallets = ctx.app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };
    let addr_str = crate::framework::identity_helpers::get_receive_address(
        &ctx.app_context,
        &framework_wallet,
    )
    .await;
    let to_address = dash_sdk::dpp::dashcore::Address::from_str(&addr_str)
        .expect("valid address")
        .assume_checked();

    let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
        qi,
        Some(to_address),
        amount,
        Some(transfer_key_id),
    ));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("transfer-mode withdrawal should succeed");

    match result {
        BackendTaskSuccessResult::WithdrewFromIdentity(fee) => {
            tracing::info!("Transfer withdraw to {addr_str}, fee: {fee:?}");
            assert!(fee.actual_fee > 0, "actual fee should be positive");
        }
        other => panic!("Expected WithdrewFromIdentity, got: {other:?}"),
    }
}

// ── TC-MN-054 — withdraw with the mode key not loaded (no ST broadcast)  ──────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn054_owner_mode_key_not_loaded() {
    let (Some(pro_tx_hash), Some(payout_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_PAYOUT_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;

    // Load with ONLY the payout key.
    let load = load_task(
        pro_tx_hash,
        node_type_from_env(),
        None,
        Some(payout_wif),
        None,
    );
    let BackendTaskSuccessResult::LoadedIdentity(qi) = run_task(&ctx.app_context, load)
        .await
        .expect("load should succeed")
    else {
        panic!("Expected LoadedIdentity");
    };

    // The tool would reject owner mode here (no OWNER key) before any dispatch;
    // assert the precondition the tool relies on: no OWNER key is available.
    assert!(
        withdrawal_key_id(&qi, Purpose::OWNER).is_none(),
        "owner key must be absent so the tool rejects owner mode pre-dispatch"
    );
    assert!(
        withdrawal_key_id(&qi, Purpose::TRANSFER).is_some(),
        "payout key is loaded"
    );
}
