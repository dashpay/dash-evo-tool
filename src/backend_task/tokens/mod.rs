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
        decimals: u8,
        base_supply: u64,
        max_supply: u64,
        start_paused: bool,
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
                decimals,
                base_supply,
                max_supply,
                start_paused,
            } => {
                // 1) Build the DataContract::V1 manually
                let data_contract = self
                    .build_data_contract_v1_with_one_token(
                        identity.identity.id().clone(),
                        token_name,
                        *decimals,
                        *base_supply,
                        Some(*max_supply), // or None if you want no max
                        *start_paused,
                    )
                    .map_err(|e| format!("Error building contract V1: {e}"))?;

                // 2) Call your existing function that registers the contract
                self.register_data_contract(
                    data_contract,
                    token_name.clone(), // alias for your DB
                    identity.clone(),
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
        decimals: u8,
        base_supply: u64,
        max_supply: Option<u64>,
        start_as_paused: bool,
    ) -> Result<DataContract, ProtocolError> {
        // 1) Create the V1 struct manually, filling in all fields:
        let mut contract_v1 = DataContractV1 {
            // Unique ID for the contract (you can replace with your own logic)
            id: Identifier::random(),

            // The version of this data contract
            version: 1,

            // The identity that owns the contract
            owner_id,

            // No documents in this example
            document_types: BTreeMap::new(),

            // Optional metadata, e.g. None
            metadata: None,

            // The overall contract config
            config: DataContractConfig::default_for_version(self.platform_version)?,

            // If you need definitions in $defs, put them here. Otherwise None
            schema_defs: None,

            // Groups are advanced multiparty features; we omit them here
            groups: BTreeMap::new(),

            // Finally, the tokens map (we'll fill it below)
            tokens: BTreeMap::new(),
        };

        // 2) Build a single TokenConfiguration in V0 format
        let mut token_config_v0 = TokenConfigurationV0::default_most_restrictive();

        // Adjust decimals
        if let TokenConfigurationConvention::V0(ref mut conv_v0) = token_config_v0.conventions {
            conv_v0.decimals = decimals as u16;

            // Add localizations or token name references:
            conv_v0.localizations.insert(
                "en".to_string(),
                TokenConfigurationLocalizationsV0 {
                    should_capitalize: true,
                    singular_form: token_name.to_string(),
                    plural_form: format!("{}s", token_name),
                },
            );
        }

        // Set base supply
        token_config_v0.base_supply = base_supply;

        // Set max supply
        token_config_v0.max_supply = max_supply;

        // Start paused or not
        token_config_v0.start_as_paused = start_as_paused;

        // Wrap in the enum
        let token_config = TokenConfiguration::V0(token_config_v0);

        // 3) Insert this token config at position 0
        contract_v1
            .tokens
            .insert(TokenContractPosition::from(0u16), token_config);

        // 4) Wrap the whole struct in DataContract::V1
        Ok(DataContract::V1(contract_v1))
    }
}
