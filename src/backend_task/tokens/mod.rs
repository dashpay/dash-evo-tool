use super::BackendTaskSuccessResult;
use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::platform::Fetch;
use dash_sdk::{
    dpp::{
        data_contract::{
            associated_token::{
                token_configuration::v0::TokenConfigurationV0,
                token_configuration_convention::TokenConfigurationConvention,
                token_configuration_localization::{
                    v0::TokenConfigurationLocalizationV0, TokenConfigurationLocalization,
                },
                token_distribution_key::TokenDistributionType,
                token_distribution_rules::TokenDistributionRules,
                token_keeps_history_rules::{v0::TokenKeepsHistoryRulesV0, TokenKeepsHistoryRules},
            },
            change_control_rules::{
                authorized_action_takers::AuthorizedActionTakers, ChangeControlRules,
            },
            config::DataContractConfig,
            group::Group,
            v1::DataContractV1,
            TokenConfiguration, TokenContractPosition,
        },
        identity::accessors::IdentityGettersV0,
        ProtocolError,
    },
    platform::{
        proto::get_documents_request::get_documents_request_v0::Start, DataContract, Identifier,
        IdentityPublicKey,
    },
    Sdk,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::mpsc;

mod burn_tokens;
mod claim_tokens;
mod destroy_frozen_funds;
mod freeze_tokens;
mod mint_tokens;
mod pause_tokens;
mod query_my_token_balances;
mod query_token_non_claimed_perpetual_distribution_rewards;
mod query_tokens;
mod resume_tokens;
mod transfer_tokens;
mod unfreeze_tokens;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenTask {
    RegisterTokenContract {
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        token_name: String,
        contract_keywords: Vec<String>,
        token_description: Option<String>,
        should_capitalize: bool,
        decimals: u16,
        base_supply: TokenAmount,
        max_supply: Option<TokenAmount>,
        start_paused: bool,
        keeps_history: bool,
        main_control_group: Option<GroupContractPosition>,

        // Manual Mint
        manual_minting_rules: ChangeControlRules,
        manual_burning_rules: ChangeControlRules,
        freeze_rules: ChangeControlRules,
        unfreeze_rules: ChangeControlRules,
        destroy_frozen_funds_rules: ChangeControlRules,
        emergency_action_rules: ChangeControlRules,
        max_supply_change_rules: ChangeControlRules,
        conventions_change_rules: ChangeControlRules,

        // Main Control Group Change
        main_control_group_change_authorized: AuthorizedActionTakers,

        distribution_rules: TokenDistributionRules,
        groups: BTreeMap<GroupContractPosition, Group>,
    },
    QueryMyTokenBalances,
    QueryIdentityTokenBalance(IdentityTokenIdentifier),
    QueryDescriptionsByKeyword(String, Option<Start>),
    FetchTokenByContractId(Identifier),
    MintTokens {
        sending_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        amount: TokenAmount,
        recipient_id: Option<Identifier>,
    },
    TransferTokens {
        sending_identity: QualifiedIdentity,
        recipient_id: Identifier,
        amount: TokenAmount,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
    },
    BurnTokens {
        owner_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        amount: TokenAmount,
    },
    DestroyFrozenFunds {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        frozen_identity: Identifier,
    },
    FreezeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        freeze_identity: Identifier,
    },
    UnfreezeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
        unfreeze_identity: Identifier,
    },
    PauseTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
    },
    ResumeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: TokenContractPosition,
        signing_key: IdentityPublicKey,
    },
    ClaimTokens {
        data_contract: DataContract,
        token_position: TokenContractPosition,
        actor_identity: QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
    },
    EstimatePerpetualTokenRewards {
        identity_id: Identifier,
        token_id: Identifier,
    },
}

impl AppContext {
    pub async fn run_token_task(
        self: &Arc<Self>,
        task: TokenTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            TokenTask::RegisterTokenContract {
                identity,
                signing_key,
                token_name,
                contract_keywords,
                token_description,
                should_capitalize,
                decimals,
                base_supply,
                max_supply,
                start_paused,
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
            } => {
                let data_contract = self
                    .build_data_contract_v1_with_one_token(
                        identity.identity.id().clone(),
                        token_name.clone(),
                        contract_keywords.to_vec(),
                        token_description.clone(),
                        *should_capitalize,
                        *decimals,
                        *base_supply,
                        *max_supply,
                        *start_paused,
                        *keeps_history,
                        *main_control_group,
                        manual_minting_rules.clone(),
                        manual_burning_rules.clone(),
                        freeze_rules.clone(),
                        unfreeze_rules.clone(),
                        destroy_frozen_funds_rules.clone(),
                        emergency_action_rules.clone(),
                        max_supply_change_rules.clone(),
                        conventions_change_rules.clone(),
                        main_control_group_change_authorized.clone(),
                        distribution_rules.clone(),
                        groups.clone(),
                    )
                    .map_err(|e| format!("Error building contract V1: {e}"))?;

                self.register_data_contract(
                    data_contract,
                    token_name.clone(),
                    identity.clone(),
                    signing_key.clone(),
                    sdk,
                )
                .await
                .map(|_| {
                    BackendTaskSuccessResult::Message(
                        "Successfully registered token contract".to_string(),
                    )
                })
                .map_err(|e| format!("Failed to register token contract: {e}"))
            }
            TokenTask::QueryMyTokenBalances => self
                .query_my_token_balances(sdk, sender)
                .await
                .map_err(|e| format!("Failed to fetch token balances: {e}")),
            TokenTask::MintTokens {
                sending_identity,
                data_contract,
                token_position,
                signing_key,
                amount,
                recipient_id,
            } => self
                .mint_tokens(
                    sending_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    *amount,
                    recipient_id.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to mint tokens: {e}")),
            TokenTask::QueryDescriptionsByKeyword(keyword, cursor) => self
                .query_descriptions_by_keyword(&keyword, cursor, sdk)
                .await
                .map_err(|e| format!("Failed to query tokens by keyword: {e}")),
            TokenTask::TransferTokens {
                sending_identity,
                recipient_id,
                amount,
                data_contract,
                token_position,
                signing_key,
            } => self
                .transfer_tokens(
                    &sending_identity,
                    *recipient_id,
                    *amount,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to transfer tokens: {e}")),
            TokenTask::BurnTokens {
                owner_identity,
                data_contract,
                token_position,
                signing_key,
                amount,
            } => self
                .burn_tokens(
                    owner_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    *amount,
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to burn tokens: {e}")),
            TokenTask::DestroyFrozenFunds {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                frozen_identity,
            } => self
                .destroy_frozen_funds(
                    actor_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    frozen_identity.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to destroy frozen funds: {e}")),
            TokenTask::FreezeTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                freeze_identity,
            } => self
                .freeze_tokens(
                    actor_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    freeze_identity.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to freeze tokens: {e}")),
            TokenTask::UnfreezeTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
                unfreeze_identity,
            } => self
                .unfreeze_tokens(
                    actor_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    unfreeze_identity.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to unfreeze tokens: {e}")),
            TokenTask::PauseTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
            } => self
                .pause_tokens(
                    actor_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to pause tokens: {e}")),
            TokenTask::ResumeTokens {
                actor_identity,
                data_contract,
                token_position,
                signing_key,
            } => self
                .resume_tokens(
                    actor_identity,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to resume tokens: {e}")),
            TokenTask::ClaimTokens {
                data_contract,
                token_position,
                actor_identity,
                distribution_type,
                signing_key,
            } => self
                .claim_tokens(
                    data_contract,
                    *token_position,
                    actor_identity,
                    *distribution_type,
                    signing_key.clone(),
                    sdk,
                )
                .await
                .map_err(|e| format!("Failed to claim tokens: {e}")),
            TokenTask::EstimatePerpetualTokenRewards {
                identity_id,
                token_id,
            } => self
                .query_token_non_claimed_perpetual_distribution_rewards(
                    *identity_id,
                    *token_id,
                    sdk,
                )
                .await
                .map_err(|e| format!("Failed to claim tokens: {e}")),
            TokenTask::QueryIdentityTokenBalance(identity_token_pair) => self
                .query_token_balance(
                    sdk,
                    identity_token_pair.identity_id,
                    identity_token_pair.token_id,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to fetch token balance: {e}")),
            TokenTask::FetchTokenByContractId(contract_id) => {
                match DataContract::fetch_by_identifier(sdk, *contract_id).await {
                    Ok(Some(data_contract)) => {
                        Ok(BackendTaskSuccessResult::FetchedContract(data_contract))
                    }
                    Ok(None) => Ok(BackendTaskSuccessResult::Message(
                        "Contract not found".to_string(),
                    )),
                    Err(e) => Err(format!("Error fetching contracts: {}", e.to_string())),
                }
            }
        }
    }

    /// Constructs a DataContract::V1 with:
    /// - contract_id (random)
    /// - version = 1
    /// - the specified owner_id
    /// - an empty set of documents, groups, schema_defs
    /// - a single token in tokens[0] with fields derived from your parameters.
    pub fn build_data_contract_v1_with_one_token(
        &self,
        owner_id: Identifier,
        token_name: String,
        contract_keywords: Vec<String>,
        token_description: Option<String>,
        should_capitalize: bool,
        decimals: u16,
        base_supply: u64,
        max_supply: Option<u64>,
        start_as_paused: bool,
        keeps_history: bool,
        main_control_group: Option<u16>,
        manual_minting_rules: ChangeControlRules,
        manual_burning_rules: ChangeControlRules,
        freeze_rules: ChangeControlRules,
        unfreeze_rules: ChangeControlRules,
        destroy_frozen_funds_rules: ChangeControlRules,
        emergency_action_rules: ChangeControlRules,
        max_supply_change_rules: ChangeControlRules,
        conventions_change_rules: ChangeControlRules,
        main_control_group_change_authorized: AuthorizedActionTakers,
        distribution_rules: TokenDistributionRules,
        groups: BTreeMap<u16, Group>,
    ) -> Result<DataContract, ProtocolError> {
        // 1) Create the V1 struct
        let mut contract_v1 = DataContractV1 {
            id: Identifier::random(),
            version: 1,
            owner_id,
            document_types: BTreeMap::new(),
            config: DataContractConfig::default_for_version(self.platform_version)?,
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
            description: None,
        };

        // 2) Build a single TokenConfiguration in V0 format
        let mut token_config_v0 = TokenConfigurationV0::default_most_restrictive();

        let TokenConfigurationConvention::V0(ref mut conv_v0) = token_config_v0.conventions;
        conv_v0.decimals = decimals as u16;
        conv_v0.localizations.insert(
            "en".to_string(),
            TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                should_capitalize,
                singular_form: token_name.to_string(),
                plural_form: format!("{}s", token_name),
            }),
        );

        let keeps_history_rules = TokenKeepsHistoryRules::V0(TokenKeepsHistoryRulesV0 {
            keeps_transfer_history: keeps_history,
            keeps_minting_history: keeps_history,
            keeps_burning_history: keeps_history,
            keeps_freezing_history: keeps_history,
            keeps_direct_pricing_history: keeps_history,
            keeps_direct_purchase_history: keeps_history,
        });

        token_config_v0.base_supply = base_supply;
        token_config_v0.max_supply = max_supply;
        token_config_v0.start_as_paused = start_as_paused;
        token_config_v0.keeps_history = keeps_history_rules;
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

        let token_config = TokenConfiguration::V0(token_config_v0);

        // 7) Insert this token config at position 0
        contract_v1
            .tokens
            .insert(TokenContractPosition::from(0u16), token_config);

        // 8) Wrap the whole struct in DataContract::V1
        Ok(DataContract::V1(contract_v1))
    }
}
