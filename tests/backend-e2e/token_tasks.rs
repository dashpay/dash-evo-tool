//! Token task backend E2E tests (TC-045 to TC-065).
//!
//! Tests the full token lifecycle: registration, querying, minting, burning,
//! transfer, freeze/unfreeze, destroy, pause/resume, pricing, purchase,
//! config update, and error paths.

use crate::framework::fixtures::{shared_identity, shared_token};
use crate::framework::harness;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use crate::framework::token_helpers;
use dash_evo_tool::backend_task::tokens::TokenTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::qualified_contract::QualifiedContract;
use dash_evo_tool::ui::tokens::tokens_screen::{
    IdentityTokenIdentifier, IdentityTokenInfo, TokenInfo,
};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;

/// Module-level storage for a second identity used across freeze/transfer/purchase tests.
static SECOND_IDENTITY: tokio::sync::OnceCell<SecondIdentity> = tokio::sync::OnceCell::const_new();

struct SecondIdentity {
    qualified_identity: dash_evo_tool::model::qualified_identity::QualifiedIdentity,
    signing_key: dash_sdk::platform::IdentityPublicKey,
    #[allow(dead_code)]
    signing_key_bytes: Vec<u8>,
}

/// Register and return a second identity for use in transfer/freeze/purchase tests.
async fn ensure_second_identity() -> &'static SecondIdentity {
    SECOND_IDENTITY
        .get_or_init(|| async {
            let ctx = harness::ctx().await;
            tracing::info!("SecondIdentity: creating funded test wallet (10M duffs)...");
            let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(30_000_000).await;

            let (reg_info, master_key_bytes) =
                build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);

            let task = BackendTask::IdentityTask(
                dash_evo_tool::backend_task::identity::IdentityTask::RegisterIdentity(reg_info),
            );
            let result = run_task(&ctx.app_context, task)
                .await
                .expect("SecondIdentity: registration failed");

            let qi = match result {
                BackendTaskSuccessResult::RegisteredIdentity(qi, _fee) => qi,
                other => panic!(
                    "SecondIdentity: expected RegisteredIdentity, got: {:?}",
                    other
                ),
            };

            let signing_key = find_auth_public_key(&qi);

            SecondIdentity {
                qualified_identity: qi,
                signing_key,
                signing_key_bytes: master_key_bytes,
            }
        })
        .await
}

/// Find the first AUTHENTICATION public key from a QualifiedIdentity.
///
/// Tries CRITICAL first — it can do everything HIGH can, and some
/// operations (e.g. token minting) require CRITICAL specifically.
fn find_auth_public_key(
    qi: &dash_evo_tool::model::qualified_identity::QualifiedIdentity,
) -> dash_sdk::platform::IdentityPublicKey {
    use dash_evo_tool::model::qualified_identity::PrivateKeyTarget;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::{Purpose, SecurityLevel};

    for target_level in [
        SecurityLevel::CRITICAL,
        SecurityLevel::HIGH,
        SecurityLevel::MASTER,
    ] {
        for ((target, _key_id), (qualified_key, _)) in qi.private_keys.private_keys.iter() {
            if *target != PrivateKeyTarget::PrivateKeyOnMainIdentity {
                continue;
            }
            let ipk = &qualified_key.identity_public_key;
            if ipk.purpose() == Purpose::AUTHENTICATION && ipk.security_level() == target_level {
                return ipk.clone();
            }
        }
    }
    panic!("find_auth_public_key: no AUTHENTICATION key found");
}

// ---------------------------------------------------------------------------
// TC-045: RegisterTokenContract
// ---------------------------------------------------------------------------

/// TC-045: Registers a token contract via shared_token() fixture.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_045_register_token_contract() {
    let st = shared_token().await;

    // shared_token() already asserted RegisteredTokenContract and fetched from DB.
    assert!(
        !st.data_contract.tokens().is_empty(),
        "Token contract should have at least one token"
    );
    assert_eq!(st.token_position, 0, "Token position should be 0");
    tracing::info!(
        "TC-045: token contract registered, contract_id={:?}, token_id={:?}",
        st.data_contract.id(),
        st.token_id
    );
}

// ---------------------------------------------------------------------------
// TC-046: QueryMyTokenBalances
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_046_query_my_token_balances() {
    let ctx = harness::ctx().await;
    let _st = shared_token().await;

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        run_task(&ctx.app_context, task),
    )
    .await
    .expect("TC-046: QueryMyTokenBalances timed out after 120s")
    .expect("TC-046: QueryMyTokenBalances failed");

    assert!(
        matches!(result, BackendTaskSuccessResult::FetchedTokenBalances),
        "TC-046: expected FetchedTokenBalances, got: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// TC-047: QueryIdentityTokenBalance
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_047_query_identity_token_balance() {
    let ctx = harness::ctx().await;
    let si = shared_identity().await;
    let st = shared_token().await;

    let iti = IdentityTokenIdentifier {
        identity_id: si.qualified_identity.identity.id(),
        token_id: st.token_id,
    };

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(iti)));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-047: QueryIdentityTokenBalance failed");

    assert!(
        matches!(result, BackendTaskSuccessResult::FetchedTokenBalances),
        "TC-047: expected FetchedTokenBalances, got: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// TC-048: FetchTokenByContractId
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_048_fetch_token_by_contract_id() {
    let ctx = harness::ctx().await;
    let st = shared_token().await;
    let contract_id = st.data_contract.id();

    let task = BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByContractId(contract_id)));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-048: FetchTokenByContractId failed");

    // FetchTokenByContractId returns FetchedContract (not FetchedContractWithTokenPosition,
    // which is returned by FetchTokenByTokenId that can resolve the position).
    match result {
        BackendTaskSuccessResult::FetchedContract(contract) => {
            assert_eq!(contract.id(), contract_id, "Contract ID should match");
            assert!(
                !contract.tokens().is_empty(),
                "Contract should have token data"
            );
        }
        other => panic!("TC-048: expected FetchedContract, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// TC-049: FetchTokenByTokenId
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_049_fetch_token_by_token_id() {
    let ctx = harness::ctx().await;
    let st = shared_token().await;

    let task = BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByTokenId(st.token_id)));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-049: FetchTokenByTokenId failed");

    match result {
        BackendTaskSuccessResult::FetchedContractWithTokenPosition(contract, position) => {
            assert_eq!(
                contract.id(),
                st.data_contract.id(),
                "Contract ID should match"
            );
            assert_eq!(position, st.token_position, "Token position should match");
        }
        other => panic!(
            "TC-049: expected FetchedContractWithTokenPosition, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// TC-050: SaveTokenLocally
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_050_save_token_locally() {
    let ctx = harness::ctx().await;
    let st = shared_token().await;

    let token_config = st
        .data_contract
        .tokens()
        .get(&st.token_position)
        .expect("Token config at position 0")
        .clone();

    let token_info = TokenInfo {
        token_id: st.token_id,
        token_name: "E2ETestToken".to_string(),
        data_contract_id: st.data_contract.id(),
        token_position: st.token_position,
        token_configuration: token_config,
        description: Some("Token created by backend E2E tests".to_string()),
    };

    let task = BackendTask::TokenTask(Box::new(TokenTask::SaveTokenLocally(token_info)));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-050: SaveTokenLocally failed");

    assert!(
        matches!(result, BackendTaskSuccessResult::SavedToken),
        "TC-050: expected SavedToken, got: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// TC-051: QueryDescriptionsByKeyword
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_051_query_descriptions_by_keyword() {
    let ctx = harness::ctx().await;
    let _st = shared_token().await;

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryDescriptionsByKeyword(
        "e2e".to_string(),
        None,
    )));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-051: QueryDescriptionsByKeyword failed");

    match result {
        BackendTaskSuccessResult::DescriptionsByKeyword(_descriptions, _next_cursor) => {
            // Platform indexing may not have caught up yet, so we accept empty results
            tracing::info!("TC-051: got DescriptionsByKeyword result");
        }
        other => panic!("TC-051: expected DescriptionsByKeyword, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// TC-052: QueryTokenPricing
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_052_query_token_pricing() {
    let ctx = harness::ctx().await;
    let st = shared_token().await;

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryTokenPricing(st.token_id)));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("TC-052: QueryTokenPricing failed");

    match result {
        BackendTaskSuccessResult::TokenPricing {
            token_id,
            prices: _,
        } => {
            assert_eq!(token_id, st.token_id, "Token ID should match");
        }
        other => panic!("TC-052: expected TokenPricing, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// TC-053: Token lifecycle (mint → burn → transfer → freeze → unfreeze →
//          destroy_frozen → pause → resume → set_price → purchase → update_config)
// ---------------------------------------------------------------------------

async fn step_mint(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
) {
    tracing::info!("=== Step 1: Mint 500_000 tokens ===");
    let result = token_helpers::mint_tokens(
        &ctx.app_context,
        &si.qualified_identity,
        &st.data_contract,
        st.token_position,
        &si.signing_key,
        500_000,
    )
    .await;

    match result {
        BackendTaskSuccessResult::MintedTokens(fee) => {
            assert!(fee.actual_fee > 0, "Minting fee should be positive");
            tracing::info!("Step 1: minted 500_000 tokens (fee: {:?})", fee);
        }
        other => panic!("Step 1: expected MintedTokens, got: {:?}", other),
    }
}

async fn step_burn(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
) {
    tracing::info!("=== Step 2: Burn 100 tokens ===");
    let task = BackendTask::TokenTask(Box::new(TokenTask::BurnTokens {
        owner_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E burn test".to_string()),
        amount: 100,
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 2: BurnTokens failed");

    match result {
        BackendTaskSuccessResult::BurnedTokens(fee) => {
            assert!(fee.actual_fee > 0, "Burn fee should be positive");
            tracing::info!("Step 2: burned 100 tokens (fee: {:?})", fee);
        }
        other => panic!("Step 2: expected BurnedTokens, got: {:?}", other),
    }
}

async fn step_transfer(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
    second: &SecondIdentity,
) {
    tracing::info!("=== Step 3: Transfer 100 tokens to second identity ===");
    let task = BackendTask::TokenTask(Box::new(TokenTask::TransferTokens {
        sending_identity: si.qualified_identity.clone(),
        recipient_id: second.qualified_identity.identity.id(),
        amount: 100,
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E transfer test".to_string()),
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 3: TransferTokens failed");

    match result {
        BackendTaskSuccessResult::TransferredTokens(fee) => {
            assert!(fee.actual_fee > 0, "Transfer fee should be positive");
            tracing::info!("Step 3: transferred 100 tokens (fee: {:?})", fee);
        }
        other => panic!("Step 3: expected TransferredTokens, got: {:?}", other),
    }
}

async fn step_freeze(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
    second: &SecondIdentity,
) {
    tracing::info!("=== Step 4: Freeze second identity ===");
    let task = BackendTask::TokenTask(Box::new(TokenTask::FreezeTokens {
        actor_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E freeze test".to_string()),
        freeze_identity: second.qualified_identity.identity.id(),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 4: FreezeTokens failed");

    match result {
        BackendTaskSuccessResult::FrozeTokens(fee) => {
            assert!(fee.actual_fee > 0, "Freeze fee should be positive");
            tracing::info!("Step 4: froze tokens for second identity (fee: {:?})", fee);
        }
        other => panic!("Step 4: expected FrozeTokens, got: {:?}", other),
    }
}

async fn step_unfreeze(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
    second: &SecondIdentity,
) {
    tracing::info!("=== Step 5: Unfreeze second identity ===");
    let task = BackendTask::TokenTask(Box::new(TokenTask::UnfreezeTokens {
        actor_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E unfreeze test".to_string()),
        unfreeze_identity: second.qualified_identity.identity.id(),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 5: UnfreezeTokens failed");

    match result {
        BackendTaskSuccessResult::UnfrozeTokens(fee) => {
            assert!(fee.actual_fee > 0, "Unfreeze fee should be positive");
            tracing::info!(
                "Step 5: unfroze tokens for second identity (fee: {:?})",
                fee
            );
        }
        other => panic!("Step 5: expected UnfrozeTokens, got: {:?}", other),
    }
}

async fn step_destroy_frozen(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
    second: &SecondIdentity,
) {
    tracing::info!("=== Step 6: Re-freeze then destroy frozen funds ===");

    // Re-freeze the second identity first (unfrozen by step 5)
    let freeze_task = BackendTask::TokenTask(Box::new(TokenTask::FreezeTokens {
        actor_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E re-freeze for destroy".to_string()),
        freeze_identity: second.qualified_identity.identity.id(),
        group_info: None,
    }));
    let freeze_result = run_task_with_nonce_retry(&ctx.app_context, freeze_task)
        .await
        .expect("Step 6: re-freeze failed");
    assert!(
        matches!(freeze_result, BackendTaskSuccessResult::FrozeTokens(_)),
        "Step 6: re-freeze should succeed"
    );

    let task = BackendTask::TokenTask(Box::new(TokenTask::DestroyFrozenFunds {
        actor_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E destroy frozen funds".to_string()),
        frozen_identity: second.qualified_identity.identity.id(),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 6: DestroyFrozenFunds failed");

    match result {
        BackendTaskSuccessResult::DestroyedFrozenFunds(fee) => {
            assert!(fee.actual_fee > 0, "Destroy fee should be positive");
            tracing::info!("Step 6: destroyed frozen funds (fee: {:?})", fee);
        }
        other => panic!("Step 6: expected DestroyedFrozenFunds, got: {:?}", other),
    }
}

async fn step_pause(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
) {
    tracing::info!("=== Step 7: Pause token ===");
    let task = BackendTask::TokenTask(Box::new(TokenTask::PauseTokens {
        actor_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E pause test".to_string()),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 7: PauseTokens failed");

    match result {
        BackendTaskSuccessResult::PausedTokens(fee) => {
            assert!(fee.actual_fee > 0, "Pause fee should be positive");
            tracing::info!("Step 7: paused token (fee: {:?})", fee);
        }
        other => panic!("Step 7: expected PausedTokens, got: {:?}", other),
    }
}

async fn step_resume(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
) {
    tracing::info!("=== Step 8: Resume token ===");
    let task = BackendTask::TokenTask(Box::new(TokenTask::ResumeTokens {
        actor_identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E resume test".to_string()),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 8: ResumeTokens failed");

    match result {
        BackendTaskSuccessResult::ResumedTokens(fee) => {
            assert!(fee.actual_fee > 0, "Resume fee should be positive");
            tracing::info!("Step 8: resumed token (fee: {:?})", fee);
        }
        other => panic!("Step 8: expected ResumedTokens, got: {:?}", other),
    }
}

async fn step_set_price(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
) {
    tracing::info!("=== Step 9: Set direct purchase price ===");
    let pricing = TokenPricingSchedule::SinglePrice(1_000);

    let task = BackendTask::TokenTask(Box::new(TokenTask::SetDirectPurchasePrice {
        identity: si.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: si.signing_key.clone(),
        token_pricing_schedule: Some(pricing),
        public_note: Some("E2E set price".to_string()),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 9: SetDirectPurchasePrice failed");

    match result {
        BackendTaskSuccessResult::SetTokenPrice(fee) => {
            assert!(fee.actual_fee > 0, "Set price fee should be positive");
            tracing::info!("Step 9: set token price (fee: {:?})", fee);
        }
        other => panic!("Step 9: expected SetTokenPrice, got: {:?}", other),
    }
}

async fn step_purchase(
    ctx: &crate::framework::harness::BackendTestContext,
    st: &crate::framework::fixtures::SharedToken,
    second: &SecondIdentity,
) {
    tracing::info!("=== Step 10: Purchase 10 tokens ===");
    // Purchase 10 tokens at 1_000 credits each = 10_000 total
    let task = BackendTask::TokenTask(Box::new(TokenTask::PurchaseTokens {
        identity: second.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: second.signing_key.clone(),
        amount: 10,
        total_agreed_price: 10_000,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 10: PurchaseTokens failed");

    match result {
        BackendTaskSuccessResult::PurchasedTokens(fee) => {
            assert!(fee.actual_fee > 0, "Purchase fee should be positive");
            tracing::info!("Step 10: purchased 10 tokens (fee: {:?})", fee);
        }
        other => panic!("Step 10: expected PurchasedTokens, got: {:?}", other),
    }
}

async fn step_update_config(
    ctx: &crate::framework::harness::BackendTestContext,
    si: &crate::framework::fixtures::SharedIdentity,
    st: &crate::framework::fixtures::SharedToken,
) {
    tracing::info!("=== Step 11: Update token config ===");
    let token_config = st
        .data_contract
        .tokens()
        .get(&st.token_position)
        .expect("Token config at position 0")
        .clone();

    let identity_token_info = IdentityTokenInfo {
        token_id: st.token_id,
        token_alias: "E2ETestToken".to_string(),
        identity: si.qualified_identity.clone(),
        data_contract: QualifiedContract {
            contract: (*st.data_contract).clone(),
            alias: Some("E2E Token Contract".to_string()),
        },
        token_config,
        token_position: st.token_position,
    };

    let change_item = TokenConfigurationChangeItem::MaxSupply(Some(2_000_000_000_000_000));

    let task = BackendTask::TokenTask(Box::new(TokenTask::UpdateTokenConfig {
        identity_token_info: Box::new(identity_token_info),
        change_item,
        signing_key: si.signing_key.clone(),
        public_note: Some("E2E config update".to_string()),
        group_info: None,
    }));

    let result = run_task_with_nonce_retry(&ctx.app_context, task)
        .await
        .expect("Step 11: UpdateTokenConfig failed");

    match result {
        BackendTaskSuccessResult::UpdatedTokenConfig(description, fee) => {
            assert!(fee.actual_fee > 0, "Config update fee should be positive");
            tracing::info!(
                "Step 11: updated token config: {} (fee: {:?})",
                description,
                fee
            );
        }
        other => panic!("Step 11: expected UpdatedTokenConfig, got: {:?}", other),
    }
}

/// TC-053: Full token lifecycle in a single sequential test.
///
/// Steps: mint → burn → transfer → freeze → unfreeze → destroy_frozen →
///        pause → resume → set_price → purchase → update_config
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_053_token_lifecycle() {
    let ctx = harness::ctx().await;
    let si = shared_identity().await;
    let st = shared_token().await;
    let second = ensure_second_identity().await;

    step_mint(ctx, si, st).await;
    step_burn(ctx, si, st).await;
    step_transfer(ctx, si, st, second).await;
    step_freeze(ctx, si, st, second).await;
    step_unfreeze(ctx, si, st, second).await;
    step_destroy_frozen(ctx, si, st, second).await;
    step_pause(ctx, si, st).await;
    step_resume(ctx, si, st).await;
    step_set_price(ctx, si, st).await;
    step_purchase(ctx, st, second).await;
    step_update_config(ctx, si, st).await;

    tracing::info!("TC-053: token lifecycle complete");
}

// ---------------------------------------------------------------------------
// TC-064: EstimatePerpetualTokenRewardsWithExplanation
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_064_estimate_perpetual_rewards() {
    let ctx = harness::ctx().await;
    let si = shared_identity().await;
    let st = shared_token().await;

    let task = BackendTask::TokenTask(Box::new(
        TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
            identity_id: si.qualified_identity.identity.id(),
            token_id: st.token_id,
        },
    ));

    let result = run_task(&ctx.app_context, task).await;

    // No perpetual distribution configured, so this may return an error or zero amount
    match result {
        Ok(BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(
            iti,
            amount,
            _explanation,
        )) => {
            assert_eq!(iti.token_id, st.token_id, "Token ID should match");
            tracing::info!(
                "TC-064: estimated perpetual rewards = {} for identity {:?}",
                amount,
                iti.identity_id
            );
        }
        Err(e) => {
            // Graceful error is acceptable when no perpetual distribution is configured
            tracing::info!(
                "TC-064: got expected error (no perpetual distribution): {:?}",
                e
            );
        }
        Ok(other) => {
            panic!(
                "TC-064: expected TokenEstimatedNonClaimed... or error, got: {:?}",
                other
            );
        }
    }
}

// ---------------------------------------------------------------------------
// TC-065: TokenTask error -- mint with unauthorized identity
// ---------------------------------------------------------------------------

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_065_mint_unauthorized() {
    let ctx = harness::ctx().await;
    let st = shared_token().await;
    let second = ensure_second_identity().await;

    // Second identity is NOT the contract owner and not in any minting group
    let task = BackendTask::TokenTask(Box::new(TokenTask::MintTokens {
        sending_identity: second.qualified_identity.clone(),
        data_contract: st.data_contract.clone(),
        token_position: st.token_position,
        signing_key: second.signing_key.clone(),
        public_note: Some("E2E unauthorized mint attempt".to_string()),
        amount: 1_000,
        recipient_id: None,
        group_info: None,
    }));

    let result = run_task(&ctx.app_context, task).await;
    assert!(
        result.is_err(),
        "TC-065: unauthorized minting should fail, got Ok({:?})",
        result.unwrap()
    );
    tracing::info!(
        "TC-065: unauthorized mint correctly rejected: {:?}",
        result.unwrap_err()
    );
}
