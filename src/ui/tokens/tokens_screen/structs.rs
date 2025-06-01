use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::tokens::tokens_screen::validate_perpetual_distribution_recipient;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Accessors;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::{TokenConfiguration, TokenContractPosition};
use dash_sdk::dpp::group::action_taker::{ActionGoal, ActionTaker};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
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

impl From<IdentityTokenMaybeBalanceWithActions> for IdentityTokenIdentifier {
    fn from(value: IdentityTokenMaybeBalanceWithActions) -> Self {
        let IdentityTokenMaybeBalanceWithActions {
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
pub struct IdentityTokenBasicInfo {
    pub token_id: Identifier,
    pub token_alias: String,
    pub identity_id: Identifier,
    pub contract_id: Identifier,
    pub token_position: TokenContractPosition,
}

impl From<IdentityTokenBalanceWithActions> for IdentityTokenBasicInfo {
    fn from(value: IdentityTokenBalanceWithActions) -> Self {
        Self {
            token_id: value.token_id,
            token_alias: value.token_alias,
            identity_id: value.identity_id,

            contract_id: value.data_contract_id,
            token_position: value.token_position,
        }
    }
}

impl From<IdentityTokenMaybeBalanceWithActions> for IdentityTokenBasicInfo {
    fn from(value: IdentityTokenMaybeBalanceWithActions) -> Self {
        Self {
            token_id: value.token_id,
            token_alias: value.token_alias,
            identity_id: value.identity_id,
            contract_id: value.data_contract_id,
            token_position: value.token_position,
        }
    }
}

impl From<&IdentityTokenBalanceWithActions> for IdentityTokenBasicInfo {
    fn from(value: &IdentityTokenBalanceWithActions) -> Self {
        Self {
            token_id: value.token_id,
            token_alias: value.token_alias.clone(),
            identity_id: value.identity_id,
            contract_id: value.data_contract_id,
            token_position: value.token_position,
        }
    }
}

impl From<&IdentityTokenMaybeBalanceWithActions> for IdentityTokenBasicInfo {
    fn from(value: &IdentityTokenMaybeBalanceWithActions) -> Self {
        Self {
            token_id: value.token_id,
            token_alias: value.token_alias.clone(),
            identity_id: value.identity_id,
            contract_id: value.data_contract_id,
            token_position: value.token_position,
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
    pub fn try_from_identity_token_balance_with_actions_with_lookup(
        identity_token_balance: &IdentityTokenBalanceWithActions,
        app_context: &AppContext,
    ) -> Result<Self, String> {
        let IdentityTokenBalanceWithActions {
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

    pub fn try_from_identity_token_maybe_balance_with_actions_with_lookup(
        identity_token_balance: &IdentityTokenMaybeBalanceWithActions,
        app_context: &AppContext,
    ) -> Result<Self, String> {
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
pub struct IdentityTokenMaybeBalanceWithActions {
    pub token_id: Identifier,
    pub token_alias: String,
    pub token_name: String,
    pub token_config: TokenConfiguration,
    pub identity_id: Identifier,
    pub identity_alias: Option<String>,
    pub balance: Option<TokenAmount>,
    pub estimated_unclaimed_rewards: Option<TokenAmount>,
    pub data_contract_id: Identifier,
    pub token_position: u16,
    pub available_actions: IdentityTokenAvailableActions,
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

impl IdentityTokenMaybeBalanceWithActions {
    /// Converts this `IdentityTokenMaybeBalanceWithActions` into an `IdentityTokenBalance`
    /// by providing a concrete `TokenAmount` balance.
    ///
    /// # Arguments
    ///
    /// * `balance` - The known token balance to be set on the resulting `IdentityTokenBalance`.
    ///
    /// # Returns
    ///
    /// A fully-formed `IdentityTokenBalance` that includes the provided balance and fields from `self`.
    pub fn to_token_balance(&self, balance: TokenAmount) -> IdentityTokenBalance {
        IdentityTokenBalance {
            token_id: self.token_id,
            token_alias: self.token_name.clone(),
            token_config: self.token_config.clone(),
            identity_id: self.identity_id,
            balance,
            estimated_unclaimed_rewards: self.estimated_unclaimed_rewards,
            data_contract_id: self.data_contract_id,
            token_position: self.token_position,
        }
    }
}

impl From<IdentityTokenBalanceWithActions> for IdentityTokenBalance {
    fn from(with_actions: IdentityTokenBalanceWithActions) -> Self {
        IdentityTokenBalance {
            token_id: with_actions.token_id,
            token_alias: with_actions.token_alias,
            token_config: with_actions.token_config,
            identity_id: with_actions.identity_id,
            balance: with_actions.balance,
            estimated_unclaimed_rewards: with_actions.estimated_unclaimed_rewards,
            data_contract_id: with_actions.data_contract_id,
            token_position: with_actions.token_position,
        }
    }
}

impl From<&IdentityTokenBalanceWithActions> for IdentityTokenBalance {
    fn from(with_actions: &IdentityTokenBalanceWithActions) -> Self {
        IdentityTokenBalance {
            token_id: with_actions.token_id,
            token_alias: with_actions.token_alias.clone(),
            token_config: with_actions.token_config.clone(),
            identity_id: with_actions.identity_id,
            balance: with_actions.balance,
            estimated_unclaimed_rewards: with_actions.estimated_unclaimed_rewards,
            data_contract_id: with_actions.data_contract_id,
            token_position: with_actions.token_position,
        }
    }
}

impl IdentityTokenBalance {
    /// Converts this `IdentityTokenBalance` into a `IdentityTokenBalanceWithActions`
    /// by computing available actions for the given identity and token configuration.
    pub fn into_with_actions(
        self,
        identity: &QualifiedIdentity,
        contract: &DataContract,
        in_dev_mode: bool,
    ) -> IdentityTokenBalanceWithActions {
        let available_actions = get_available_token_actions_for_identity(
            identity,
            &self.token_config,
            contract,
            in_dev_mode,
        );

        IdentityTokenBalanceWithActions {
            token_id: self.token_id,
            token_alias: self.token_alias,
            token_config: self.token_config,
            identity_id: self.identity_id,
            balance: self.balance,
            estimated_unclaimed_rewards: self.estimated_unclaimed_rewards,
            data_contract_id: self.data_contract_id,
            token_position: self.token_position,
            available_actions,
        }
    }
}

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenBalanceWithActions {
    pub token_id: Identifier,
    pub token_alias: String,
    pub token_config: TokenConfiguration,
    pub identity_id: Identifier,
    pub balance: TokenAmount,
    pub estimated_unclaimed_rewards: Option<TokenAmount>,
    pub data_contract_id: Identifier,
    pub token_position: u16,
    pub available_actions: IdentityTokenAvailableActions,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdentityTokenAvailableActions {
    pub can_claim: bool,
    pub can_estimate: bool,
    pub can_mint: bool,
    pub can_burn: bool,
    pub can_freeze: bool,
    pub can_unfreeze: bool,
    pub can_destroy: bool,
    pub can_do_emergency_action: bool,
    pub can_maybe_purchase: bool,
    pub can_set_price: bool,
}

pub fn get_available_token_actions_for_identity(
    identity: &QualifiedIdentity,
    token_configuration: &TokenConfiguration,
    contract: &DataContract,
    in_dev_mode: bool,
) -> IdentityTokenAvailableActions {
    let main_group = token_configuration.main_control_group();
    let groups = contract.groups();
    let contract_owner_id = contract.owner_id();
    let identity_id = identity.identity.id();
    let solo_action_taker = ActionTaker::SingleIdentity(identity_id);

    let is_authorized = |takers: &AuthorizedActionTakers| {
        takers.allowed_for_action_taker(
            &contract_owner_id,
            main_group,
            groups,
            &solo_action_taker,
            ActionGoal::ActionCompletion,
        ) || takers.allowed_for_action_taker(
            &contract_owner_id,
            main_group,
            groups,
            &solo_action_taker,
            ActionGoal::ActionParticipation,
        )
    };

    let can_claim = {
        if let Some(dist) = token_configuration
            .distribution_rules()
            .perpetual_distribution()
        {
            in_dev_mode
                || validate_perpetual_distribution_recipient(
                    contract_owner_id,
                    dist.distribution_recipient(),
                    identity,
                )
                .is_ok()
                || token_configuration
                    .distribution_rules()
                    .pre_programmed_distribution()
                    .is_some()
        } else {
            in_dev_mode
                || token_configuration
                    .distribution_rules()
                    .pre_programmed_distribution()
                    .is_some()
        }
    };

    let can_estimate = {
        if let Some(dist) = token_configuration
            .distribution_rules()
            .perpetual_distribution()
        {
            in_dev_mode
                || validate_perpetual_distribution_recipient(
                    contract_owner_id,
                    dist.distribution_recipient(),
                    identity,
                )
                .is_ok()
        } else {
            in_dev_mode
        }
    };

    let can_maybe_purchase = in_dev_mode
        || token_configuration
            .distribution_rules()
            .change_direct_purchase_pricing_rules()
            .authorized_to_make_change_action_takers()
            != &AuthorizedActionTakers::NoOne;

    IdentityTokenAvailableActions {
        can_claim,
        can_estimate,
        can_mint: in_dev_mode
            || is_authorized(
                token_configuration
                    .manual_minting_rules()
                    .authorized_to_make_change_action_takers(),
            ),
        can_burn: in_dev_mode
            || is_authorized(
                token_configuration
                    .manual_burning_rules()
                    .authorized_to_make_change_action_takers(),
            ),
        can_freeze: in_dev_mode
            || is_authorized(
                token_configuration
                    .freeze_rules()
                    .authorized_to_make_change_action_takers(),
            ),
        can_unfreeze: in_dev_mode
            || is_authorized(
                token_configuration
                    .unfreeze_rules()
                    .authorized_to_make_change_action_takers(),
            ),
        can_destroy: in_dev_mode
            || is_authorized(
                token_configuration
                    .destroy_frozen_funds_rules()
                    .authorized_to_make_change_action_takers(),
            ),
        can_do_emergency_action: in_dev_mode
            || is_authorized(
                token_configuration
                    .emergency_action_rules()
                    .authorized_to_make_change_action_takers(),
            ),
        can_maybe_purchase,
        can_set_price: in_dev_mode
            || is_authorized(
                token_configuration
                    .distribution_rules()
                    .change_direct_purchase_pricing_rules()
                    .authorized_to_make_change_action_takers(),
            ),
    }
}
