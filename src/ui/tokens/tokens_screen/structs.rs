pub use crate::model::token::{
    IdentityTokenAvailableActions, IdentityTokenBalance, IdentityTokenBalanceWithActions,
    IdentityTokenBasicInfo, IdentityTokenIdentifier, IdentityTokenMaybeBalanceWithActions,
    get_available_token_actions_for_identity,
};

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::{TokenConfiguration, TokenContractPosition};
use dash_sdk::platform::{DataContract, Identifier};

/// Token info
#[derive(Clone, Debug, PartialEq)]
pub struct TokenInfo {
    pub token_id: Identifier,
    pub token_name: String,
    pub data_contract_id: Identifier,
    pub token_position: u16,
    pub token_configuration: TokenConfiguration,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenInfoWithDataContract {
    pub token_id: Identifier,
    pub token_name: String,
    pub data_contract: DataContract,
    pub token_position: u16,
    pub token_configuration: TokenConfiguration,
    pub description: Option<String>,
}

/// Identity Token Info
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenInfo {
    pub token_id: Identifier,
    pub token_alias: String,
    pub identity: QualifiedIdentity,
    pub data_contract: QualifiedContract,
    pub token_config: TokenConfiguration,
    pub token_position: TokenContractPosition,
}

impl IdentityTokenInfo {
    pub fn try_from_identity_token_balance_with_lookup(
        identity_token_balance: &IdentityTokenBalance,
        app_context: &AppContext,
    ) -> Result<Self, TaskError> {
        let IdentityTokenBalance {
            token_id,
            token_alias,
            token_config,
            identity_id,
            data_contract_id,
            token_position,
            ..
        } = identity_token_balance;
        let identity = app_context
            .get_identity_by_id(identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let data_contract = app_context
            .get_contract_by_id(data_contract_id)?
            .ok_or(TaskError::DataContractNotFound)?;
        Ok(Self {
            token_id: *token_id,
            token_alias: token_alias.clone(),
            identity,
            data_contract,
            token_config: token_config.clone(),
            token_position: *token_position,
        })
    }
    pub fn try_from_identity_token_maybe_balance_with_actions_with_lookup(
        identity_token_balance: &IdentityTokenMaybeBalanceWithActions,
        app_context: &AppContext,
    ) -> Result<Self, TaskError> {
        let IdentityTokenMaybeBalanceWithActions {
            token_id,
            token_alias,
            token_config,
            identity_id,
            data_contract_id,
            token_position,
            ..
        } = identity_token_balance;
        let identity = app_context
            .get_identity_by_id(identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let data_contract = app_context
            .get_contract_by_id(data_contract_id)?
            .ok_or(TaskError::DataContractNotFound)?;
        Ok(Self {
            token_id: *token_id,
            token_alias: token_alias.clone(),
            identity,
            data_contract,
            token_config: token_config.clone(),
            token_position: *token_position,
        })
    }
}

impl Amount {
    /// Creates an [`Amount`] for a token, taking decimals and unit name from
    /// the token configuration.
    pub fn from_token(token_info: &IdentityTokenInfo, value: TokenAmount) -> Self {
        let decimal_places = token_info.token_config.conventions().decimals();
        Self::new(value, decimal_places).with_unit_name(&token_info.token_alias)
    }
}
