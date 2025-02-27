use super::BackendTaskSuccessResult;
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};
use dash_sdk::{
    dpp::{
        data_contract::{
            associated_token::{
                token_configuration::v0::TokenConfigurationV0,
                token_configuration_convention::{
                    v0::TokenConfigurationLocalizationsV0, TokenConfigurationConvention,
                },
            },
            change_control_rules::{
                authorized_action_takers::AuthorizedActionTakers, v0::ChangeControlRulesV0,
                ChangeControlRules,
            },
            config::DataContractConfig,
            v1::DataContractV1,
            TokenConfiguration, TokenContractPosition,
        },
        identity::accessors::IdentityGettersV0,
        ProtocolError,
    },
    platform::{DataContract, Identifier, IdentityPublicKey},
    Sdk,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::mpsc;

mod burn_tokens;
mod destroy_frozen_funds;
mod freeze_tokens;
mod mint_tokens;
mod pause_tokens;
mod query_my_token_balances;
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
        should_capitalize: bool,
        decimals: u8,
        base_supply: u64,
        max_supply: u64,
        start_paused: bool,
        keeps_history: bool,
        manual_mint_authorized: AuthorizedActionTakers,
        manual_burn_authorized: AuthorizedActionTakers,
        freeze_authorized: AuthorizedActionTakers,
        unfreeze_authorized: AuthorizedActionTakers,
        destroy_frozen_funds_authorized: AuthorizedActionTakers,
        pause_and_resume_authorized: AuthorizedActionTakers,
    },
    QueryMyTokenBalances,
    QueryTokensByKeyword(String),
    QueryTokensByKeywordPage(String, Option<Identifier>),
    MintTokens {
        sending_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        amount: u64,
        recipient_id: Option<Identifier>,
    },
    TransferTokens {
        sending_identity: QualifiedIdentity,
        recipient_id: Identifier,
        amount: u64,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
    },
    BurnTokens {
        owner_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        amount: u64,
    },
    DestroyFrozenFunds {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        frozen_identity: Identifier,
    },
    FreezeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        freeze_identity: Identifier,
    },
    UnfreezeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        unfreeze_identity: Identifier,
    },
    PauseTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
    },
    ResumeTokens {
        actor_identity: QualifiedIdentity,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
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
                should_capitalize,
                decimals,
                base_supply,
                max_supply,
                start_paused,
                keeps_history,
                manual_mint_authorized,
                manual_burn_authorized,
                freeze_authorized,
                unfreeze_authorized,
                destroy_frozen_funds_authorized,
                pause_and_resume_authorized,
            } => {
                let data_contract = self
                    .build_data_contract_v1_with_one_token(
                        identity.identity.id().clone(),
                        token_name,
                        *should_capitalize,
                        *decimals,
                        *base_supply,
                        Some(*max_supply),
                        *start_paused,
                        *keeps_history,
                        manual_mint_authorized.clone(),
                        manual_burn_authorized.clone(),
                        freeze_authorized.clone(),
                        unfreeze_authorized.clone(),
                        destroy_frozen_funds_authorized.clone(),
                        pause_and_resume_authorized.clone(),
                    )
                    .map_err(|e| format!("Error building contract V1: {e}"))?;

                // 2) Call your existing function that registers the contract
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
            TokenTask::QueryTokensByKeyword(_query) => {
                // Placeholder
                Ok(BackendTaskSuccessResult::Message("QueryTokens".to_string()))

                // Actually do this
                // self.query_tokens(query, sdk, sender).await
            }
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
            TokenTask::QueryTokensByKeywordPage(_query, _cursor) => {
                // Placeholder
                Ok(BackendTaskSuccessResult::Message(
                    "QueryTokensByKeywordPage".to_string(),
                ))

                // Actually do this
                // self.query_tokens_page(query, cursor, sdk, sender).await
            }
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
        token_name: &str,
        should_capitalize: bool,
        decimals: u8,
        base_supply: u64,
        max_supply: Option<u64>,
        start_as_paused: bool,
        keeps_history: bool,
        manual_mint_authorized: AuthorizedActionTakers,
        manual_burn_authorized: AuthorizedActionTakers,
        freeze_authorized: AuthorizedActionTakers,
        unfreeze_authorized: AuthorizedActionTakers,
        destroy_frozen_funds_authorized: AuthorizedActionTakers,
        pause_and_resume_authorized: AuthorizedActionTakers,
    ) -> Result<DataContract, ProtocolError> {
        // 1) Create the V1 struct
        let mut contract_v1 = DataContractV1 {
            id: Identifier::random(),
            version: 1,
            owner_id,
            document_types: BTreeMap::new(),
            metadata: None,
            config: DataContractConfig::default_for_version(self.platform_version)?,
            schema_defs: None,
            groups: BTreeMap::new(),
            tokens: BTreeMap::new(),
        };

        // 2) Build a single TokenConfiguration in V0 format
        let mut token_config_v0 = TokenConfigurationV0::default_most_restrictive();
        let TokenConfigurationConvention::V0(ref mut conv_v0) = token_config_v0.conventions;
        conv_v0.decimals = decimals as u16;
        conv_v0.localizations.insert(
            "en".to_string(),
            TokenConfigurationLocalizationsV0 {
                should_capitalize,
                singular_form: token_name.to_string(),
                plural_form: format!("{}s", token_name),
            },
        );
        token_config_v0.base_supply = base_supply;
        token_config_v0.max_supply = max_supply;
        token_config_v0.start_as_paused = start_as_paused;
        token_config_v0.keeps_history = keeps_history;

        // 3) Manual Minting
        // Set manualMintingRules to "ContractOwner"
        token_config_v0.manual_minting_rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: manual_mint_authorized,
            admin_action_takers: AuthorizedActionTakers::NoOne,
            changing_authorized_action_takers_to_no_one_allowed: false,
            changing_admin_action_takers_to_no_one_allowed: false,
            self_changing_admin_action_takers_allowed: false,
        });

        // 4) Manual Burning
        token_config_v0.manual_burning_rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: manual_burn_authorized,
            admin_action_takers: AuthorizedActionTakers::NoOne,
            changing_authorized_action_takers_to_no_one_allowed: false,
            changing_admin_action_takers_to_no_one_allowed: false,
            self_changing_admin_action_takers_allowed: false,
        });

        // 5) Freeze/Unfreeze
        token_config_v0.freeze_rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: freeze_authorized,
            admin_action_takers: AuthorizedActionTakers::NoOne,
            changing_authorized_action_takers_to_no_one_allowed: false,
            changing_admin_action_takers_to_no_one_allowed: false,
            self_changing_admin_action_takers_allowed: false,
        });

        token_config_v0.unfreeze_rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: unfreeze_authorized,
            admin_action_takers: AuthorizedActionTakers::NoOne,
            changing_authorized_action_takers_to_no_one_allowed: false,
            changing_admin_action_takers_to_no_one_allowed: false,
            self_changing_admin_action_takers_allowed: false,
        });

        // 6) DestroyFrozenFunds
        token_config_v0.destroy_frozen_funds_rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: destroy_frozen_funds_authorized,
            admin_action_takers: AuthorizedActionTakers::NoOne,
            changing_authorized_action_takers_to_no_one_allowed: false,
            changing_admin_action_takers_to_no_one_allowed: false,
            self_changing_admin_action_takers_allowed: false,
        });

        token_config_v0.emergency_action_rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: pause_and_resume_authorized,
            admin_action_takers: AuthorizedActionTakers::NoOne,
            changing_authorized_action_takers_to_no_one_allowed: false,
            changing_admin_action_takers_to_no_one_allowed: false,
            self_changing_admin_action_takers_allowed: false,
        });

        // Wrap in the enum
        let token_config = TokenConfiguration::V0(token_config_v0);

        // 7) Insert this token config at position 0
        contract_v1
            .tokens
            .insert(TokenContractPosition::from(0u16), token_config);

        // 8) Wrap the whole struct in DataContract::V1
        Ok(DataContract::V1(contract_v1))
    }
}
