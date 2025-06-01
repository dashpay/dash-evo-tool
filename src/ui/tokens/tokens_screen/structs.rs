use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::balances::credits::TokenAmount;
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

impl TokenInfoWithDataContract {
    /// Constructs a `TokenInfoWithDataContract` from a `TokenInfo` and a `DataContract`.
    pub fn from_with_data_contract(token_info: TokenInfo, data_contract: DataContract) -> Self {
        TokenInfoWithDataContract {
            token_id: token_info.token_id,
            token_name: token_info.token_name,
            data_contract,
            token_position: token_info.token_position,
            token_configuration: token_info.token_configuration,
            description: token_info.description,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct IdentityTokenIdentifier {
    pub identity_id: Identifier,
    pub token_id: Identifier,
}

impl From<IdentityTokenBalance> for IdentityTokenIdentifier {
    fn from(value: IdentityTokenBalance) -> Self {
        let IdentityTokenBalance {
            token_id,
            identity_id,
            ..
        } = value;

        IdentityTokenIdentifier {
            identity_id,
            token_id,
        }
    }
}

impl From<IdentityTokenMaybeBalance> for IdentityTokenIdentifier {
    fn from(value: IdentityTokenMaybeBalance) -> Self {
        let IdentityTokenMaybeBalance {
            token_id,
            identity_id,
            ..
        } = value;

        IdentityTokenIdentifier {
            identity_id,
            token_id,
        }
    }
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
    ) -> Result<Self, String> {
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
            .get_identity_by_id(identity_id)
            .map_err(|err| err.to_string())?
            .ok_or("Identity not found".to_string())?;
        let data_contract = app_context
            .get_contract_by_id(data_contract_id)
            .map_err(|err| err.to_string())?
            .ok_or("Contract not found".to_string())?;
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

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenMaybeBalance {
    pub token_id: Identifier,
    pub token_name: String,
    pub identity_id: Identifier,
    pub identity_alias: Option<String>,
    pub balance: Option<IdentityTokenBalance>,
}

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenBalance {
    pub token_id: Identifier,
    pub token_alias: String,
    pub token_config: TokenConfiguration,
    pub identity_id: Identifier,
    pub balance: TokenAmount,
    pub estimated_unclaimed_rewards: Option<TokenAmount>,
    pub data_contract_id: Identifier,
    pub token_position: u16,
}
