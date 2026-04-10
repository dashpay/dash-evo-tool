//! Helpers for token contract registration and minting in tests.

// TODO(production-reuse): This helper parallels `src/backend_task/tokens/mod.rs::run_token_task`
// (RegisterTokenContract, MintTokens).
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::tokens::TokenTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::TokenKeepsHistoryRules;
use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::v0::TokenKeepsHistoryRulesV0;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::platform::IdentityPublicKey;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Build a `BackendTask` that registers a token contract with permissive rules.
///
/// The contract allows the owner to mint, burn, freeze, unfreeze, destroy frozen
/// funds, pause, resume, and set marketplace prices without group approval.
pub fn build_register_token_task(
    identity: &QualifiedIdentity,
    signing_key: &IdentityPublicKey,
) -> BackendTask {
    let owner_only = ChangeControlRules::V0(ChangeControlRulesV0 {
        authorized_to_make_change: AuthorizedActionTakers::ContractOwner,
        admin_action_takers: AuthorizedActionTakers::ContractOwner,
        changing_authorized_action_takers_to_no_one_allowed: false,
        changing_admin_action_takers_to_no_one_allowed: false,
        self_changing_admin_action_takers_allowed: false,
    });

    let keeps_history = TokenKeepsHistoryRules::V0(
        TokenKeepsHistoryRulesV0::default_for_keeping_all_history(true),
    );

    let distribution_rules = TokenDistributionRules::V0(TokenDistributionRulesV0 {
        perpetual_distribution: None,
        perpetual_distribution_rules: owner_only.clone(),
        pre_programmed_distribution: None,
        new_tokens_destination_identity: Some(identity.identity.id()),
        new_tokens_destination_identity_rules: owner_only.clone(),
        minting_allow_choosing_destination: true,
        minting_allow_choosing_destination_rules: owner_only.clone(),
        change_direct_purchase_pricing_rules: owner_only.clone(),
    });

    BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract {
        identity: identity.clone(),
        signing_key: Box::new(signing_key.clone()),
        token_names: vec![(
            "E2ETestToken".to_string(),
            "E2ETK".to_string(),
            "en".to_string(),
        )],
        // No keywords — each keyword costs ~10B credits (0.1 DASH) due to
        // the keyword search contract registration fee, which would exceed
        // the test wallet's budget and cause registration to fail.
        contract_keywords: vec![],
        token_description: Some("Token created by backend E2E tests".to_string()),
        should_capitalize: false,
        decimals: 8,
        base_supply: 0,
        max_supply: Some(1_000_000_000_000_000),
        start_paused: false,
        allow_transfers_to_frozen_identities: false,
        keeps_history,
        main_control_group: None,
        manual_minting_rules: owner_only.clone(),
        manual_burning_rules: owner_only.clone(),
        freeze_rules: owner_only.clone(),
        unfreeze_rules: Box::new(owner_only.clone()),
        destroy_frozen_funds_rules: Box::new(owner_only.clone()),
        emergency_action_rules: Box::new(owner_only.clone()),
        max_supply_change_rules: Box::new(owner_only.clone()),
        conventions_change_rules: Box::new(owner_only.clone()),
        main_control_group_change_authorized: AuthorizedActionTakers::ContractOwner,
        distribution_rules,
        groups: BTreeMap::new(),
        document_schemas: None,
        marketplace_trade_mode: 1,
        marketplace_rules: owner_only,
    }))
}

/// Mint tokens via `TokenTask::MintTokens`.
///
/// Mints `amount` tokens to the sending identity (self-mint).
pub async fn mint_tokens(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    data_contract: &Arc<DataContract>,
    token_position: TokenContractPosition,
    signing_key: &IdentityPublicKey,
    amount: TokenAmount,
) -> BackendTaskSuccessResult {
    let task = BackendTask::TokenTask(Box::new(TokenTask::MintTokens {
        sending_identity: identity.clone(),
        data_contract: data_contract.clone(),
        token_position,
        signing_key: signing_key.clone(),
        public_note: Some("E2E test mint".to_string()),
        amount,
        recipient_id: Some(identity.identity.id()),
        group_info: None,
    }));

    run_task(app_context, task)
        .await
        .expect("mint_tokens: minting failed")
}
