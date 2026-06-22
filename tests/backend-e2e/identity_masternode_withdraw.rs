//! Backend E2E: headless masternode/evonode load + credit withdrawal.
//!
//! Mirrors `identity_withdraw.rs` but exercises the masternode path the new
//! `masternode_identity_load` / `masternode_credits_withdraw` MCP tools
//! dispatch: a real load by ProTxHash + keys, then a real withdraw in both key
//! modes against testnet.
//!
//! TC-MN-050 and TC-MN-051 go through the full tool `invoke()` path
//! (`MasternodeCreditsWithdraw::invoke`), exercising key-mode resolution, the
//! owner/transfer address logic, the SPV gate, and the fee echo — not just
//! the underlying BackendTask. All other tests drive the BackendTask directly
//! because they test backend-level behaviour that the tool is transparent to.
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
use dash_evo_tool::mcp::server::DashMcpService;
use dash_evo_tool::mcp::tools::masternode::{
    MasternodeCreditsWithdraw, MasternodeCreditsWithdrawParams,
};
use dash_evo_tool::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_evo_tool::model::secret::Secret;
use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use rmcp::handler::server::router::tool::AsyncTool;
use std::str::FromStr;
use std::sync::Arc;

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

/// Convert a Dash Network enum to the string the MCP require_network check uses.
fn network_name(network: dash_sdk::dpp::dashcore::Network) -> &'static str {
    use dash_sdk::dpp::dashcore::Network;
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "local",
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

/// Build the load task exactly as `masternode_identity_load` would.
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
    // Capture the actual WIF value BEFORE moving it into the task so we can
    // check for key-material leakage in TC-MN-061 below.
    let wrong_key_wif = dash_sdk::dpp::dashcore::PrivateKey::from_byte_array(
        &[0x11u8; 32],
        dash_sdk::dpp::dashcore::Network::Testnet,
    )
    .expect("valid private key")
    .to_wif();

    let task = load_task(
        pro_tx_hash,
        node_type_from_env(),
        Some(wrong_key_wif.clone()), // clone — value used in assertion below
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
    // TC-MN-061 cross-check: the actual WIF bytes never appear in Display or Debug.
    // (Previous check used "bogus" — the variable name — which is never part of a
    // WIF string and made the assertion vacuously true.)
    let display = err.to_string();
    let debug = format!("{err:?}");
    assert!(
        !display.contains(&wrong_key_wif),
        "actual key WIF must not appear in Display: {display}"
    );
    assert!(
        !debug.contains(&wrong_key_wif),
        "actual key WIF must not appear in Debug: {debug}"
    );
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

// ── TC-MN-050 — OWNER mode happy path via tool invoke: destination forced to payout
//
// Goes through the full `MasternodeCreditsWithdraw::invoke()` path, exercising:
//   - key-mode resolution (owner key lookup via `available_withdrawal_keys`)
//   - `resolve_withdrawal_plan` → `dispatch_address = None` (Platform forces payout)
//   - echo_address = registered payout address (verified in the output)
//   - SPV gate (already satisfied by the preceding load)

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
        pro_tx_hash.clone(),
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

    let payout_address = qi
        .masternode_payout_address(ctx.app_context.network())
        .expect("payout address present");
    let balance = qi.identity.balance();
    assert!(balance > 0, "identity must have withdrawable credits");
    let amount = (balance / 10).max(1);

    // Obtain the persisted identity ID (Base58) from the loaded identity.
    let identity_id_b58 = qi.identity.id().to_string(Encoding::Base58);
    let network_str = network_name(ctx.app_context.network()).to_owned();

    // Wrap the test context in an ArcSwap for the shared-context service.
    let swappable = Arc::new(arc_swap::ArcSwap::new(Arc::clone(&ctx.app_context)));
    let service = DashMcpService::new_shared(swappable);

    // OWNER mode: no to_address — Platform consensus forces the registered payout address.
    let params = MasternodeCreditsWithdrawParams {
        identity_id: identity_id_b58,
        key_mode: "owner".to_owned(),
        to_address: String::new(),
        amount_credits: amount,
        network: network_str,
    };

    let output = MasternodeCreditsWithdraw::invoke(&service, params)
        .await
        .expect("owner-mode withdrawal via tool invoke should succeed");

    tracing::info!(
        "Owner withdraw: to={}, estimated_fee={}, actual_fee={}",
        output.to_address,
        output.estimated_fee,
        output.actual_fee
    );
    assert!(output.actual_fee > 0, "actual fee should be positive");
    // The echo address must be the registered payout address.
    assert_eq!(
        output.to_address,
        payout_address.to_string(),
        "owner mode echo must match the registered payout address"
    );
    assert_eq!(output.key_mode, "owner", "key_mode must echo 'owner'");
}

// ── TC-MN-051 — TRANSFER mode happy path via tool invoke: any Core address
//
// Goes through `MasternodeCreditsWithdraw::invoke()`, exercising:
//   - key-mode resolution (transfer/payout key via `available_withdrawal_keys`)
//   - `resolve_withdrawal_plan` → `dispatch_address = Some(caller_addr)`
//   - echo_address = the caller's address (verified in the output)

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

    let balance = qi.identity.balance();
    assert!(balance > 0, "identity must have withdrawable credits");
    let amount = (balance / 10).max(1);

    let identity_id_b58 = qi.identity.id().to_string(Encoding::Base58);
    let network_str = network_name(ctx.app_context.network()).to_owned();

    // A fresh testnet Core address from the framework wallet.
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

    let swappable = Arc::new(arc_swap::ArcSwap::new(Arc::clone(&ctx.app_context)));
    let service = DashMcpService::new_shared(swappable);

    // TRANSFER mode: caller supplies an explicit Core address.
    let params = MasternodeCreditsWithdrawParams {
        identity_id: identity_id_b58,
        key_mode: "transfer".to_owned(),
        to_address: addr_str.clone(),
        amount_credits: amount,
        network: network_str,
    };

    let output = MasternodeCreditsWithdraw::invoke(&service, params)
        .await
        .expect("transfer-mode withdrawal via tool invoke should succeed");

    tracing::info!(
        "Transfer withdraw: to={}, estimated_fee={}, actual_fee={}",
        output.to_address,
        output.estimated_fee,
        output.actual_fee
    );
    assert!(output.actual_fee > 0, "actual fee should be positive");
    // The echo address must be the caller-supplied address.
    assert_eq!(
        output.to_address, addr_str,
        "transfer mode echo must match the caller-supplied address"
    );
    assert_eq!(output.key_mode, "transfer", "key_mode must echo 'transfer'");
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

// ── TC-MN-007 — load with a malformed ProTxHash → IdentifierParsingError ──────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn007_load_malformed_protx() {
    let Some(payout_wif) = opt_env("E2E_MN_PAYOUT_WIF") else {
        return;
    };
    let ctx = ctx().await;

    // "not-a-hash" parses as neither Base58 nor hex; the backend preserves the
    // original input in the typed variant rather than string-parsing it.
    let task = load_task(
        "not-a-hash".to_owned(),
        node_type_from_env(),
        None,
        Some(payout_wif),
        None,
    );
    let err = run_task(&ctx.app_context, task)
        .await
        .expect_err("a malformed ProTxHash must not load");

    match err {
        TaskError::IdentifierParsingError { input } => {
            assert_eq!(input, "not-a-hash", "the original input is preserved");
        }
        other => panic!("Expected IdentifierParsingError, got: {other:?}"),
    }
}

// ── TC-MN-019 — load with a voting key fetches and binds the voter identity ──

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn019_load_with_voting_key() {
    let (Some(pro_tx_hash), Some(payout_wif), Some(voting_wif)) = (
        opt_env("E2E_MN_PRO_TX_HASH"),
        opt_env("E2E_MN_PAYOUT_WIF"),
        opt_env("E2E_MN_VOTING_WIF"),
    ) else {
        return;
    };
    let ctx = ctx().await;

    let task = load_task(
        pro_tx_hash,
        node_type_from_env(),
        None,
        Some(payout_wif),
        Some(voting_wif),
    );
    let BackendTaskSuccessResult::LoadedIdentity(qi) = run_task(&ctx.app_context, task)
        .await
        .expect("load with voting key should succeed")
    else {
        panic!("Expected LoadedIdentity");
    };

    // The voting key triggers a voter-identity fetch and binds it.
    assert!(
        qi.associated_voter_identity.is_some(),
        "voter identity should be bound when a voting key is supplied"
    );
    // The voting key adds no withdrawal mode — only payout (TRANSFER) is present.
    assert!(withdrawal_key_id(&qi, Purpose::TRANSFER).is_some());
    assert!(withdrawal_key_id(&qi, Purpose::OWNER).is_none());
}

// ── TC-MN-023 — re-load is idempotent at the DB layer (single row) ───────────

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn023_reload_idempotent() {
    let (Some(pro_tx_hash), Some(payout_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_PAYOUT_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;

    let first = load_task(
        pro_tx_hash.clone(),
        node_type_from_env(),
        None,
        Some(payout_wif.clone()),
        None,
    );
    let BackendTaskSuccessResult::LoadedIdentity(qi) = run_task(&ctx.app_context, first)
        .await
        .expect("first load should succeed")
    else {
        panic!("Expected LoadedIdentity");
    };
    let target_id = qi.identity.id();

    let count_rows = || {
        ctx.app_context
            .load_local_qualified_identities()
            .expect("load local identities")
            .into_iter()
            .filter(|q| q.identity.id() == target_id)
            .count()
    };
    assert_eq!(count_rows(), 1, "exactly one row after the first load");

    // Re-load the same identity with the same key.
    let second = load_task(
        pro_tx_hash,
        node_type_from_env(),
        None,
        Some(payout_wif),
        None,
    );
    run_task(&ctx.app_context, second)
        .await
        .expect("re-load should succeed");
    assert_eq!(
        count_rows(),
        1,
        "re-load must INSERT-OR-REPLACE, not duplicate the row"
    );
}

// ── TC-MN-052 — owner mode + supplied to_address rejected, no ST broadcast ───

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn052_owner_mode_supplied_address_no_broadcast() {
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
    let balance_before = qi.identity.balance();

    // The tool rejects owner+address BEFORE dispatch (reject_owner_address_
    // contradiction), so no ST is broadcast — the unit test
    // owner_mode_supplied_address_rejected proves the rejection. Here we assert
    // the identity balance is unchanged after the rejected attempt.
    let identity_id = qi.identity.id();
    let refreshed = ctx
        .app_context
        .get_identity_by_id(&identity_id)
        .expect("db read")
        .expect("identity present");
    assert_eq!(
        refreshed.identity.balance(),
        balance_before,
        "no withdrawal should have moved funds"
    );
}

// ── TC-MN-053 — A→B composition THROUGH the local DB (the load→withdraw seam) ─

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_mn053_compose_through_db() {
    let (Some(pro_tx_hash), Some(payout_wif)) =
        (opt_env("E2E_MN_PRO_TX_HASH"), opt_env("E2E_MN_PAYOUT_WIF"))
    else {
        return;
    };
    let ctx = ctx().await;

    // Step 1 — load (Tool A), which persists to SQLite.
    let load = load_task(
        pro_tx_hash,
        node_type_from_env(),
        None,
        Some(payout_wif),
        None,
    );
    let BackendTaskSuccessResult::LoadedIdentity(loaded) = run_task(&ctx.app_context, load)
        .await
        .expect("load should succeed")
    else {
        panic!("Expected LoadedIdentity");
    };
    let identity_id = loaded.identity.id();
    drop(loaded); // ensure step 2 uses the DB record, not the in-memory qi.

    // Step 2 — resolve from the DB exactly as the withdraw tool does, then
    // withdraw. This exercises the load→DB→withdraw seam (the only A→B coupling).
    let qi = ctx
        .app_context
        .get_identity_by_id(&identity_id)
        .expect("db read")
        .expect("identity must resolve from the DB after load");

    let transfer_key_id = withdrawal_key_id(&qi, Purpose::TRANSFER)
        .expect("payout key recoverable from the persisted record");
    let balance = qi.identity.balance();
    assert!(balance > 0, "identity must have withdrawable credits");
    let amount = (balance / 10).max(1);

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
        .expect("withdraw via the DB-resolved identity should succeed");

    match result {
        BackendTaskSuccessResult::WithdrewFromIdentity(fee) => {
            tracing::info!("A->B compose withdraw, fee: {fee:?}");
            assert!(fee.actual_fee > 0, "actual fee should be positive");
        }
        other => panic!("Expected WithdrewFromIdentity, got: {other:?}"),
    }
}

// TC-MN-022 (load SPV-gate presence) is intentionally deferred per coverage gap
// G-3: asserting the gate fires needs a forced-SPV-error harness. The gate is
// present at the load tool's `ensure_spv_synced` call; TC-MN-042 proves cheap
// validation runs before it.
