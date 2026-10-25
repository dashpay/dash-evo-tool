use super::{BackendTaskSuccessResult, FeeResult};
use crate::backend_task::error::TaskError;
use crate::backend_task::{NETWORK_REQUEST_TIMEOUT, await_network_request_with_timeout};
use crate::ui::tokens::tokens_screen::{IdentityTokenIdentifier, IdentityTokenInfo, TokenInfo};
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::Fetch;
use dash_sdk::{
    Sdk,
    dpp::{
        ProtocolError,
        data_contract::{
            TokenConfiguration, TokenContractPosition,
            associated_token::{
                token_configuration::v0::TokenConfigurationV0,
                token_configuration_convention::TokenConfigurationConvention,
                token_configuration_localization::{
                    TokenConfigurationLocalization, v0::TokenConfigurationLocalizationV0,
                },
                token_distribution_key::TokenDistributionType,
                token_distribution_rules::TokenDistributionRules,
                token_keeps_history_rules::TokenKeepsHistoryRules,
                token_marketplace_rules::{
                    TokenMarketplaceRules,
                    v0::{TokenMarketplaceRulesV0, TokenTradeMode},
                },
            },
            change_control_rules::{
                ChangeControlRules, authorized_action_takers::AuthorizedActionTakers,
            },
            config::DataContractConfig,
            group::Group,
            v1::DataContractV1,
        },
        identity::accessors::IdentityGettersV0,
    },
    platform::{
        DataContract, Identifier, IdentityPublicKey,
        proto::get_documents_request::get_documents_request_v0::Start,
    },
};
use std::future::Future;
use std::{collections::BTreeMap, sync::Arc};

mod burn_tokens;
mod claim_tokens;
mod destroy_frozen_funds;
mod freeze_tokens;
mod mint_tokens;
mod pause_tokens;
mod purchase_tokens;
mod query_my_token_balances;
mod query_token_non_claimed_perpetual_distribution_rewards;
mod query_token_pricing;
mod query_tokens;
mod resume_tokens;
mod set_token_price;
mod transfer_tokens;
mod unfreeze_tokens;
mod update_token_config;

/// All token-configuration inputs for
/// [`AppContext::build_data_contract_v1_with_one_token`], grouped to keep the
/// builder and its task variant from carrying two dozen positional fields.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenContractParams {
    pub token_names: Vec<(String, String, String)>,
    pub contract_keywords: Vec<String>,
    pub token_description: Option<String>,
    pub should_capitalize: bool,
    pub decimals: u8,
    pub base_supply: TokenAmount,
    pub max_supply: Option<TokenAmount>,
    pub start_paused: bool,
    pub allow_transfers_to_frozen_identities: bool,
    pub keeps_history: TokenKeepsHistoryRules,
    pub main_control_group: Option<GroupContractPosition>,

    pub manual_minting_rules: ChangeControlRules,
    pub manual_burning_rules: ChangeControlRules,
    pub freeze_rules: ChangeControlRules,
    pub unfreeze_rules: ChangeControlRules,
    pub destroy_frozen_funds_rules: ChangeControlRules,
    pub emergency_action_rules: ChangeControlRules,
    pub max_supply_change_rules: ChangeControlRules,
    pub conventions_change_rules: ChangeControlRules,

    pub main_control_group_change_authorized: AuthorizedActionTakers,

    pub distribution_rules: TokenDistributionRules,
    pub groups: BTreeMap<GroupContractPosition, Group>,
    pub document_schemas: Option<BTreeMap<String, serde_json::Value>>,
    pub marketplace_rules: ChangeControlRules,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum TokenTask {
    RegisterTokenContract {
        identity: QualifiedIdentity,
        signing_key: Box<IdentityPublicKey>,
        params: Box<TokenContractParams>,
    },
    QueryMyTokenBalances,
    QueryIdentityTokenBalance(IdentityTokenIdentifier),
    /// Stop tracking one `(identity, token)` balance: un-watch it upstream so
    /// the background sync stops fetching it, drop it from the My Tokens
    /// ordering so the row disappears, and record the dismissal so later
    /// refreshes do not re-watch the pair.
    StopTrackingTokenBalance(IdentityTokenIdentifier),
    QueryDescriptionsByKeyword(String, Option<Start>),
    FetchTokenByContractId(Identifier),
    FetchTokenByTokenId(Identifier),
    SaveTokenLocally(TokenInfo),
    QueryTokenPricing(Identifier),
    MintTokens {
        sending_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: TokenAmount,
        recipient_id: Option<Identifier>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    TransferTokens {
        sending_identity: QualifiedIdentity,
        recipient_id: Identifier,
        amount: TokenAmount,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
    },
    BurnTokens {
        owner_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: TokenAmount,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    DestroyFrozenFunds {
        actor_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        frozen_identity: Identifier,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    FreezeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        freeze_identity: Identifier,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    UnfreezeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        unfreeze_identity: Identifier,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    PauseTokens {
        actor_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    ResumeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    ClaimTokens {
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        actor_identity: QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
    },
    EstimatePerpetualTokenRewardsWithExplanation {
        identity_id: Identifier,
        token_id: Identifier,
    },
    UpdateTokenConfig {
        identity_token_info: Box<IdentityTokenInfo>,
        change_item: TokenConfigurationChangeItem,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
    PurchaseTokens {
        identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        amount: TokenAmount,
        total_agreed_price: Credits,
    },
    SetDirectPurchasePrice {
        identity: QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        token_pricing_schedule: Option<TokenPricingSchedule>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    },
}

impl AppContext {
    /// Awaits a token state-transition SDK call, applies its post-broadcast
    /// side effects (balance updates, audit logging), and returns the success
    /// result carrying an estimated-only fee.
    ///
    /// The platform does not report a settled fee for token ops, so the fee is
    /// always the pre-flight estimate (see [`FeeResult::estimated_only`]).
    async fn execute_token_op<R>(
        &self,
        call: impl Future<Output = Result<R, TaskError>>,
        post_broadcast: impl FnOnce(R),
        make_success: fn(FeeResult) -> BackendTaskSuccessResult,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let result = call.await?;
        post_broadcast(result);
        let estimated_fee = self.fee_estimator().estimate_document_batch(1);
        Ok(make_success(FeeResult::estimated_only(estimated_fee)))
    }

    pub async fn run_token_task(
        self: &Arc<Self>,
        task: TokenTask,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            TokenTask::RegisterTokenContract {
                identity,
                signing_key,
                params,
            } => {
                params
                    .contract_keywords
                    .iter()
                    .try_for_each(|keyword| crate::model::token::validate_contract_keyword(keyword))
                    .map_err(|source| TaskError::InvalidContractKeywordLength { source })?;
                let alias = params.token_names[0].0.clone();
                let data_contract = self
                    .build_data_contract_v1_with_one_token(identity.identity.id(), *params)
                    .map_err(|e| TaskError::from(dash_sdk::Error::Protocol(e)))?;

                self.register_data_contract(
                    data_contract,
                    alias,
                    identity,
                    *signing_key,
                    sdk,
                    sender,
                )
                .await
                .map(|_| BackendTaskSuccessResult::RegisteredTokenContract)
            }
            TokenTask::QueryMyTokenBalances => self.query_my_token_balances(sdk, sender).await,
            TokenTask::MintTokens {
                sending_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                amount,
                recipient_id,
                group_info,
            } => {
                self.mint_tokens(
                    &sending_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    amount,
                    recipient_id,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::QueryDescriptionsByKeyword(keyword, cursor) => {
                self.query_descriptions_by_keyword(&keyword, &cursor, sdk)
                    .await
            }
            TokenTask::TransferTokens {
                sending_identity,
                recipient_id,
                amount,
                data_contract,
                token_position,
                signing_key,
                public_note,
            } => {
                self.transfer_tokens(
                    &sending_identity,
                    recipient_id,
                    amount,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::BurnTokens {
                owner_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                amount,
                group_info,
            } => {
                self.burn_tokens(
                    &owner_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    amount,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::DestroyFrozenFunds {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                frozen_identity,
                group_info,
            } => {
                self.destroy_frozen_funds(
                    &actor_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    frozen_identity,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::FreezeTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                freeze_identity,
                group_info,
            } => {
                self.freeze_tokens(
                    &actor_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    freeze_identity,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::UnfreezeTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                unfreeze_identity,
                group_info,
            } => {
                self.unfreeze_tokens(
                    &actor_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    unfreeze_identity,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::PauseTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                group_info,
            } => {
                self.pause_tokens(
                    &actor_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::ResumeTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                public_note,
                group_info,
            } => {
                self.resume_tokens(
                    &actor_identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::ClaimTokens {
                data_contract,
                token_position,
                actor_identity,
                distribution_type,
                signing_key,
                public_note,
            } => {
                self.claim_tokens(
                    data_contract,
                    token_position,
                    &actor_identity,
                    distribution_type,
                    signing_key,
                    public_note,
                    sdk,
                )
                .await
            }
            TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
                identity_id,
                token_id,
            } => {
                self.query_token_non_claimed_perpetual_distribution_rewards_with_explanation(
                    identity_id,
                    token_id,
                    sdk,
                )
                .await
            }
            TokenTask::QueryIdentityTokenBalance(identity_token_pair) => {
                self.query_token_balance(sdk, identity_token_pair, sender)
                    .await
            }
            TokenTask::StopTrackingTokenBalance(identity_token_pair) => {
                self.stop_tracking_token_balance(identity_token_pair, sender)
                    .await
            }
            TokenTask::FetchTokenByContractId(contract_id) => {
                match await_network_request_with_timeout(
                    NETWORK_REQUEST_TIMEOUT,
                    DataContract::fetch_by_identifier(sdk, contract_id),
                    |source| TaskError::TokenLookupTimeout { source },
                )
                .await?
                {
                    Ok(Some(data_contract)) => {
                        Ok(BackendTaskSuccessResult::FetchedContract(data_contract))
                    }
                    Ok(None) => Ok(BackendTaskSuccessResult::ContractNotFound),
                    Err(e) => Err(TaskError::from(e)),
                }
            }
            TokenTask::FetchTokenByTokenId(token_id) => {
                use dash_sdk::dpp::tokens::contract_info::TokenContractInfo;
                use dash_sdk::dpp::tokens::contract_info::v0::TokenContractInfoV0Accessors;

                match await_network_request_with_timeout(
                    NETWORK_REQUEST_TIMEOUT,
                    TokenContractInfo::fetch(sdk, token_id),
                    |source| TaskError::TokenLookupTimeout { source },
                )
                .await?
                {
                    Ok(Some(token_contract_info)) => {
                        // Extract the contract ID and token position from token_contract_info
                        let (contract_id, token_position) = match &token_contract_info {
                            TokenContractInfo::V0(info) => {
                                (info.contract_id(), info.token_contract_position())
                            }
                        };

                        // Fetch the full contract
                        match await_network_request_with_timeout(
                            NETWORK_REQUEST_TIMEOUT,
                            DataContract::fetch_by_identifier(sdk, contract_id),
                            |source| TaskError::TokenLookupTimeout { source },
                        )
                        .await?
                        {
                            Ok(Some(data_contract)) => {
                                // Return the contract with the specific token position
                                Ok(BackendTaskSuccessResult::FetchedContractWithTokenPosition(
                                    data_contract,
                                    token_position,
                                ))
                            }
                            Ok(None) => Ok(BackendTaskSuccessResult::ContractNotFound),
                            Err(e) => Err(TaskError::from(e)),
                        }
                    }
                    Ok(None) => Ok(BackendTaskSuccessResult::TokenNotFound),
                    Err(e) => Err(TaskError::from(e)),
                }
            }
            TokenTask::SaveTokenLocally(token_info) => {
                let token_id = token_info.token_id;
                self.insert_token(
                    &token_id,
                    &token_info.token_name,
                    token_info.token_configuration,
                    &token_info.data_contract_id,
                    token_info.token_position,
                )?;
                // Importing a token is intent to track it, so it overrides an
                // earlier "stop tracking" of the same token.
                self.clear_untracked_token(&token_id)?;

                Ok(BackendTaskSuccessResult::SavedToken)
            }
            TokenTask::UpdateTokenConfig {
                identity_token_info,
                change_item,
                signing_key,
                public_note,
                group_info,
            } => {
                self.update_token_config(
                    *identity_token_info,
                    change_item,
                    &signing_key,
                    public_note,
                    group_info,
                    sdk,
                )
                .await
            }
            TokenTask::PurchaseTokens {
                identity,
                data_contract,
                token_position,
                signing_key,
                amount,
                total_agreed_price,
            } => {
                self.purchase_tokens(
                    &identity,
                    data_contract,
                    token_position,
                    signing_key,
                    amount,
                    total_agreed_price,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::SetDirectPurchasePrice {
                identity,
                data_contract,
                token_position,
                signing_key,
                token_pricing_schedule,
                public_note,
                group_info,
            } => {
                self.set_direct_purchase_price(
                    &identity,
                    data_contract,
                    token_position,
                    signing_key,
                    public_note,
                    token_pricing_schedule,
                    group_info,
                    sdk,
                    sender,
                )
                .await
            }
            TokenTask::QueryTokenPricing(token_id) => {
                self.query_token_pricing(token_id, sdk, sender).await
            }
        }
    }

    /// Constructs a DataContract::V1 with:
    /// - contract_id (random)
    /// - version = 1
    /// - the specified owner_id
    /// - an empty set of documents, groups, schema_defs
    /// - a single token in tokens[0] with fields derived from your parameters.
    #[allow(clippy::result_large_err)]
    pub fn build_data_contract_v1_with_one_token(
        &self,
        owner_id: Identifier,
        params: TokenContractParams,
    ) -> Result<DataContract, ProtocolError> {
        let TokenContractParams {
            token_names,
            contract_keywords,
            token_description,
            should_capitalize,
            decimals,
            base_supply,
            max_supply,
            start_paused: start_as_paused,
            allow_transfers_to_frozen_identities: allow_transfer_to_frozen_balance,
            keeps_history,
            main_control_group,
            manual_minting_rules,
            manual_burning_rules,
            freeze_rules,
            unfreeze_rules,
            destroy_frozen_funds_rules,
            emergency_action_rules,
            max_supply_change_rules,
            conventions_change_rules,
            main_control_group_change_authorized,
            distribution_rules,
            groups,
            document_schemas,
            marketplace_rules,
        } = params;

        // 1) Create the V1 struct first to get the contract ID
        let contract_id = Identifier::random();
        let mut contract_v1 = DataContractV1 {
            id: contract_id,
            version: 1,
            owner_id,
            document_types: BTreeMap::new(), // Initialize empty, will populate below
            config: DataContractConfig::default_for_version(self.platform_version())?,
            schema_defs: None,
            groups,
            tokens: BTreeMap::new(),
            keywords: contract_keywords,
            created_at: None,
            updated_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            created_at_epoch: None,
            updated_at_epoch: None,
            description: token_description.clone(),
        };

        // 2) Parse document schemas if provided and add them to the contract
        if let Some(schemas) = document_schemas {
            for (name, schema_json) in schemas {
                // Convert serde_json::Value to platform_value::Value
                let platform_value = schema_json.into();

                // Convert JSON schema to DocumentType using the proper parameters
                let mut validation_operations = Vec::new();
                match dash_sdk::dpp::data_contract::document_type::DocumentType::try_from_schema(
                    contract_id,
                    0,
                    0,
                    &name,
                    platform_value,
                    None, // schema_defs
                    &contract_v1.tokens,
                    &contract_v1.config,
                    true, // validate_required
                    &mut validation_operations,
                    self.platform_version(),
                ) {
                    Ok(document_type) => {
                        contract_v1.document_types.insert(name, document_type);
                    }
                    Err(e) => {
                        return Err(ProtocolError::Generic(format!(
                            "Failed to convert document schema '{}' to DocumentType: {}",
                            name, e
                        )));
                    }
                }
            }
        }

        // 3) Build a single TokenConfiguration in V0 format
        let mut token_config_v0 = TokenConfigurationV0::default_most_restrictive();

        let TokenConfigurationConvention::V0(ref mut conv_v0) = token_config_v0.conventions;
        conv_v0.decimals = decimals;
        for (token_name, token_plural, language) in token_names {
            conv_v0.localizations.insert(
                language,
                TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                    should_capitalize,
                    singular_form: token_name,
                    plural_form: token_plural,
                }),
            );
        }

        token_config_v0.base_supply = base_supply;
        token_config_v0.max_supply = max_supply;
        token_config_v0.start_as_paused = start_as_paused;
        token_config_v0.allow_transfer_to_frozen_balance = allow_transfer_to_frozen_balance;
        token_config_v0.keeps_history = keeps_history;
        token_config_v0.main_control_group = main_control_group;
        token_config_v0.manual_minting_rules = manual_minting_rules;
        token_config_v0.manual_burning_rules = manual_burning_rules;
        token_config_v0.freeze_rules = freeze_rules;
        token_config_v0.unfreeze_rules = unfreeze_rules;
        token_config_v0.destroy_frozen_funds_rules = destroy_frozen_funds_rules;
        token_config_v0.emergency_action_rules = emergency_action_rules;
        token_config_v0.max_supply_change_rules = max_supply_change_rules;
        token_config_v0.conventions_change_rules = conventions_change_rules;
        token_config_v0.main_control_group_can_be_modified = main_control_group_change_authorized;
        token_config_v0.distribution_rules = distribution_rules;
        token_config_v0.description = token_description;

        // All tokens are created NotTradeable; a future SDK will add more modes.
        token_config_v0.marketplace_rules = TokenMarketplaceRules::V0(TokenMarketplaceRulesV0 {
            trade_mode: TokenTradeMode::NotTradeable,
            trade_mode_change_rules: marketplace_rules,
        });

        let token_config = TokenConfiguration::V0(token_config_v0);

        // 7) Insert this token config at position 0
        contract_v1
            .tokens
            .insert(TokenContractPosition::from(0u16), token_config);

        // 8) Wrap the whole struct in DataContract::V1
        Ok(DataContract::V1(contract_v1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_lookup_timeout_is_typed_and_actionable() {
        let error = crate::backend_task::await_network_request_with_timeout(
            std::time::Duration::from_millis(1),
            std::future::pending::<()>(),
            |source| TaskError::TokenLookupTimeout { source },
        )
        .await
        .expect_err("a pending token lookup must time out");

        assert!(matches!(error, TaskError::TokenLookupTimeout { .. }));
        assert!(error.to_string().contains("Check your connection"));

        let error = crate::backend_task::await_network_request_with_timeout(
            std::time::Duration::from_millis(1),
            std::future::pending::<()>(),
            |source| TaskError::TokenBalanceRefreshTimeout { source },
        )
        .await
        .expect_err("a pending token balance refresh must time out");

        assert!(matches!(
            error,
            TaskError::TokenBalanceRefreshTimeout { .. }
        ));
        assert!(error.to_string().contains("refresh the Tokens screen"));
    }

    #[tokio::test]
    async fn timed_out_managed_request_is_reaped_after_completion() {
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let task_manager = std::sync::Arc::new(crate::utils::tasks::TaskManager::new());
        let error = crate::backend_task::await_managed_network_request_with_timeout(
            task_manager,
            "test_request_reaper",
            std::time::Duration::from_millis(1),
            async move {
                release_rx
                    .await
                    .expect("the test must release the timed-out request");
                let _ = completed_tx.send(());
            },
            |source| TaskError::TokenBalanceRefreshTimeout { source },
        )
        .await
        .expect_err("the UI wait must time out before the request completes");

        assert!(matches!(
            error,
            TaskError::TokenBalanceRefreshTimeout { .. }
        ));
        release_tx
            .send(())
            .expect("the timed-out request must still be running");
        tokio::time::timeout(std::time::Duration::from_secs(1), completed_rx)
            .await
            .expect("the managed request must keep running")
            .expect("the managed request must report completion");
    }
}
