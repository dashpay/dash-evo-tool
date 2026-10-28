//! Tokens as held by one identity: identifiers, balances, and the actions the
//! token's configuration grants that identity.
//!
//! Pure data and rule evaluation shared by `backend_task/`, `context/` and
//! every token UI; nothing here touches `AppContext`, the database or the SDK
//! client.

use crate::model::amount::Amount;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::accessors::v0::TokenPreProgrammedDistributionV0Methods;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Accessors;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::{TokenConfiguration, TokenContractPosition};
use dash_sdk::dpp::group::action_taker::{ActionGoal, ActionTaker};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::{DataContract, Identifier};

/// One `(identity, token)` pair.
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

/// The handles a token screen needs to address one identity's token.
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

impl From<&IdentityTokenBalance> for IdentityTokenBasicInfo {
    fn from(value: &IdentityTokenBalance) -> Self {
        Self {
            token_id: value.token_id,
            token_alias: value.token_alias.clone(),
            identity_id: value.identity_id,
            contract_id: value.data_contract_id,
            token_position: value.token_position,
        }
    }
}

/// A token tracked for an identity whose balance may not be known yet, with
/// the actions the identity may take on it.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenMaybeBalanceWithActions {
    pub token_id: Identifier,
    pub token_alias: String,
    pub token_config: TokenConfiguration,
    pub identity_id: Identifier,
    pub identity_name: Option<String>,
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
    /// The same token with a now-known `balance`.
    pub fn to_token_balance(&self, balance: TokenAmount) -> IdentityTokenBalance {
        IdentityTokenBalance {
            token_id: self.token_id,
            token_alias: self.token_alias.clone(),
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
    /// This balance plus the actions `identity` may take on the token.
    ///
    /// `in_dev_mode` is a *display* override that lights up every button
    /// regardless of the token's rules; never derive authorisation from the
    /// result when it is set — use [`identity_has_token_admin_permission`].
    pub fn into_with_actions(
        self,
        identity: &QualifiedIdentity,
        contract: &DataContract,
        in_dev_mode: bool,
        token_pricing: Option<&TokenPricingSchedule>,
    ) -> IdentityTokenBalanceWithActions {
        let available_actions = get_available_token_actions_for_identity(
            Some(self.balance),
            identity,
            &self.token_config,
            contract,
            in_dev_mode,
            token_pricing,
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

/// A token owned by an identity, with the actions the identity may take on it.
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

/// The actions one identity may take on one token.
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
    pub can_transfer: bool,
    pub can_update_config: bool,
}

impl IdentityTokenAvailableActions {
    /// How many action buttons My Tokens renders for these actions.
    pub fn shown_buttons(&self) -> usize {
        let mut count = 0;

        if self.can_claim {
            count += 1;
        }
        if self.can_estimate {
            count += 1;
        }
        if self.can_mint {
            count += 1;
        }
        if self.can_burn {
            count += 1;
        }
        if self.can_freeze {
            count += 1;
        }
        if self.can_unfreeze {
            count += 1;
        }
        if self.can_destroy {
            count += 1;
        }
        if self.can_do_emergency_action {
            count += 2;
        } // counts twice
        if self.can_maybe_purchase {
            count += 1;
        }
        if self.can_set_price {
            count += 1;
        }
        // self.can_transfer is intentionally excluded
        if self.can_update_config {
            count += 1;
        }

        count
    }

    /// Whether any administrative action (mint, burn, freeze, unfreeze,
    /// destroy frozen funds, emergency action, set price, update config) is
    /// available. Only meaningful for actions computed without the dev-mode
    /// display override; see [`identity_has_token_admin_permission`].
    pub fn has_admin_permission(&self) -> bool {
        self.can_mint
            || self.can_burn
            || self.can_freeze
            || self.can_unfreeze
            || self.can_destroy
            || self.can_do_emergency_action
            || self.can_set_price
            || self.can_update_config
    }
}

/// Whether `identity` holds at least one administrative permission on the
/// token, as authorised by the token configuration's change-control rules.
/// Independent of the user's role and of the dev-mode display override.
pub fn identity_has_token_admin_permission(
    identity: &QualifiedIdentity,
    token_configuration: &TokenConfiguration,
    contract: &DataContract,
) -> bool {
    get_available_token_actions_for_identity(
        None,
        identity,
        token_configuration,
        contract,
        false,
        None,
    )
    .has_admin_permission()
}

/// Whether the identity can transfer the token: it holds some, and the token
/// lets holders move it to another identity (Platform refuses every transfer
/// of a non-transferable token).
fn transfer_available(
    token_configuration: &TokenConfiguration,
    known_balance: Option<TokenAmount>,
) -> bool {
    token_configuration.is_transferable() && known_balance.is_some_and(|b| b > 0)
}

/// Whether `identity` may receive the perpetual distribution paid to `recipient`.
pub fn validate_perpetual_distribution_recipient(
    contract_owner_id: Identifier,
    recipient: TokenDistributionRecipient,
    identity: &QualifiedIdentity,
) -> Result<(), String> {
    match recipient {
        TokenDistributionRecipient::ContractOwner => {
            if contract_owner_id != identity.identity.id() {
                Err("This token's distribution recipient is the contract owner, and this identity is not the contract owner".to_string())
            } else {
                Ok(())
            }
        }
        TokenDistributionRecipient::Identity(identifier) => {
            if identifier != identity.identity.id() {
                Err(
                    "This identity is not a valid distribution recipient for this token"
                        .to_string(),
                )
            } else {
                Ok(())
            }
        }
        TokenDistributionRecipient::EvonodesByParticipation => {
            if identity.identity_type != IdentityType::Evonode {
                Err("This token's distribution recipient is EvonodesByParticipation, and this identity is not an evonode".to_string())
            } else {
                Ok(())
            }
        }
    }
}

/// Whether `identity_id` is owed anything by the token's pre-programmed
/// distribution. A schedule that names only other identities offers this one
/// nothing to claim.
fn is_pre_programmed_recipient(
    token_configuration: &TokenConfiguration,
    identity_id: &Identifier,
) -> bool {
    token_configuration
        .distribution_rules()
        .pre_programmed_distribution()
        .is_some_and(|dist| {
            dist.distributions()
                .values()
                .any(|recipients| recipients.contains_key(identity_id))
        })
}

/// The actions `identity` may take on a token, from its configuration rules,
/// the contract's groups and ownership, and the identity's known balance.
///
/// `in_dev_mode` turns every rule-gated action on for display. It never
/// grants anything: Platform still refuses the action when the rules do.
pub fn get_available_token_actions_for_identity(
    known_balance: Option<TokenAmount>,
    identity: &QualifiedIdentity,
    token_configuration: &TokenConfiguration,
    contract: &DataContract,
    in_dev_mode: bool,
    token_pricing: Option<&TokenPricingSchedule>,
) -> IdentityTokenAvailableActions {
    let main_group = token_configuration.main_control_group();
    let groups = contract.groups();
    let contract_owner_id = contract.owner_id();
    let identity_id = identity.identity.id();
    let solo_action_taker = ActionTaker::SingleIdentity(identity_id);

    let can_transfer = transfer_available(token_configuration, known_balance);

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

    // Every identity may take a once-per-identity claim, so its presence alone
    // offers the claim (Platform refuses a second one).
    let can_claim_once_per_identity =
        super::once_per_identity_amount(token_configuration).is_some();
    let can_claim = can_claim_once_per_identity || in_dev_mode || {
        let perpetual_recipient = token_configuration
            .distribution_rules()
            .perpetual_distribution()
            .is_some_and(|dist| {
                validate_perpetual_distribution_recipient(
                    contract_owner_id,
                    dist.distribution_recipient(),
                    identity,
                )
                .is_ok()
            });
        perpetual_recipient || is_pre_programmed_recipient(token_configuration, &identity_id)
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

    let can_maybe_purchase = token_pricing.is_some();

    let can_update_config = in_dev_mode
        || token_configuration
            .all_change_control_rules()
            .iter()
            .any(|(_, rule)| is_authorized(rule.admin_action_takers()));

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
        can_transfer,
        can_update_config,
    }
}

impl From<&IdentityTokenBalance> for Amount {
    /// Uses the token configuration's decimals and the token alias as the unit name.
    fn from(token_balance: &IdentityTokenBalance) -> Self {
        let decimal_places = token_balance.token_config.conventions().decimals();
        Self::new(token_balance.balance, decimal_places).with_unit_name(&token_balance.token_alias)
    }
}

impl From<IdentityTokenBalance> for Amount {
    fn from(token_balance: IdentityTokenBalance) -> Self {
        Self::from(&token_balance)
    }
}

impl From<&IdentityTokenBalanceWithActions> for Amount {
    /// Uses the token configuration's decimals and the token alias as the unit name.
    fn from(token_balance: &IdentityTokenBalanceWithActions) -> Self {
        let decimal_places = token_balance.token_config.conventions().decimals();
        Self::new(token_balance.balance, decimal_places).with_unit_name(&token_balance.token_alias)
    }
}

impl From<IdentityTokenBalanceWithActions> for Amount {
    fn from(token_balance: IdentityTokenBalanceWithActions) -> Self {
        Self::from(&token_balance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Setters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Setters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{
        TokenConfigurationPreset, TokenConfigurationPresetFeatures, TokenConfigurationV0,
    };
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
    use dash_sdk::dpp::data_contract::config::DataContractConfig;
    use dash_sdk::dpp::data_contract::v1::DataContractV1;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::collections::BTreeMap;

    fn transferable() -> TokenConfiguration {
        TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive())
    }

    #[test]
    fn transfer_needs_a_transferable_token_and_a_balance() {
        let mut non_transferable = transferable();
        non_transferable.set_transferable(false);

        assert!(transfer_available(&transferable(), Some(5)));
        assert!(!transfer_available(&transferable(), Some(0)));
        assert!(!transfer_available(&transferable(), None));
        assert!(
            !transfer_available(&non_transferable, Some(5)),
            "Platform refuses every transfer of a non-transferable token"
        );
    }

    /// A user identity with the given ID byte and no keys.
    fn identity(byte: u8) -> QualifiedIdentity {
        crate::context::test_support::bare_user_identity(
            Identifier::from([byte; 32]),
            dash_sdk::dpp::dashcore::Network::Testnet,
        )
    }

    /// A contract owned by `owner` carrying one token whose every admin action
    /// is reserved to the contract owner.
    fn owner_administered_contract(owner: Identifier) -> (DataContract, TokenConfiguration) {
        let platform_version = PlatformVersion::latest();
        let config = TokenConfiguration::V0(
            TokenConfigurationPreset {
                features: TokenConfigurationPresetFeatures::WithExtremeActions,
                action_taker: AuthorizedActionTakers::ContractOwner,
            }
            .token_configuration_v0(
                TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
                    localizations: Default::default(),
                    decimals: 2,
                }),
                1_000,
                None,
                true,
                false,
            ),
        );
        let contract = DataContract::V1(DataContractV1 {
            id: Identifier::from([0xC0; 32]),
            version: 1,
            owner_id: owner,
            document_types: BTreeMap::new(),
            config: DataContractConfig::default_for_version(platform_version)
                .expect("contract config"),
            schema_defs: None,
            groups: BTreeMap::new(),
            tokens: BTreeMap::from([(0, config.clone())]),
            keywords: vec![],
            created_at: None,
            updated_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            created_at_epoch: None,
            updated_at_epoch: None,
            description: None,
        });
        (contract, config)
    }

    #[test]
    fn admin_permission_follows_the_token_rules_not_the_dev_mode_override() {
        let owner = identity(1);
        let stranger = identity(2);
        let (contract, config) = owner_administered_contract(owner.identity.id());

        assert!(identity_has_token_admin_permission(
            &owner, &config, &contract
        ));
        assert!(!identity_has_token_admin_permission(
            &stranger, &config, &contract
        ));

        // The display override lights every admin button up for the stranger…
        let displayed = get_available_token_actions_for_identity(
            None, &stranger, &config, &contract, true, None,
        );
        assert!(displayed.can_mint && displayed.has_admin_permission());
        // …which is exactly what the permission check must not be fooled by.
        assert!(!identity_has_token_admin_permission(
            &stranger, &config, &contract
        ));
    }

    #[test]
    fn a_pre_programmed_distribution_is_claimable_only_by_its_recipients() {
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
        use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::TokenPreProgrammedDistribution;
        use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::v0::TokenPreProgrammedDistributionV0;
        let recipient = identity(2);
        let bystander = identity(3);
        let (contract, _) = owner_administered_contract(identity(1).identity.id());
        let mut config = transferable();
        let TokenDistributionRules::V0(mut rules) = config.distribution_rules().clone() else {
            panic!("the most restrictive preset is version 0");
        };
        rules.pre_programmed_distribution = Some(TokenPreProgrammedDistribution::V0(
            TokenPreProgrammedDistributionV0 {
                distributions: BTreeMap::from([(
                    1_000u64,
                    BTreeMap::from([(recipient.identity.id(), 500u64)]),
                )]),
            },
        ));
        config.set_distribution_rules(TokenDistributionRules::V0(rules));

        let actions = |who: &QualifiedIdentity, dev| {
            get_available_token_actions_for_identity(Some(0), who, &config, &contract, dev, None)
        };
        assert!(actions(&recipient, false).can_claim);
        assert!(
            !actions(&bystander, false).can_claim,
            "a schedule naming only others offers nothing to claim"
        );
        assert!(
            actions(&bystander, true).can_claim,
            "the developer display override still lights Claim"
        );
    }

    #[test]
    fn a_most_restrictive_token_grants_nobody_admin_permission() {
        let owner = identity(1);
        let (contract, _) = owner_administered_contract(owner.identity.id());
        assert!(!identity_has_token_admin_permission(
            &owner,
            &transferable(),
            &contract
        ));
    }
}
