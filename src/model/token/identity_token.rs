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
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
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
    /// `value` in the token's decimals, with the alias as unit.
    pub fn amount_of(&self, value: TokenAmount) -> Amount {
        token_amount(&self.token_config, &self.token_alias, value)
    }

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
    /// result when it is set — compute with `false` and ask
    /// [`IdentityTokenAvailableActions::has_admin_permission`].
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
    /// available. Answers the real permission question only for actions
    /// computed with `in_dev_mode = false`, because the developer display
    /// override turns every flag on.
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

/// Whether the identity can transfer the token: it holds some, and the token
/// lets holders move it to another identity (Platform refuses every transfer
/// of a non-transferable token).
fn transfer_available(
    token_configuration: &TokenConfiguration,
    known_balance: Option<TokenAmount>,
) -> bool {
    token_configuration.is_transferable() && known_balance.is_some_and(|b| b > 0)
}

/// Whether `identity` is who the perpetual distribution pays: the contract
/// owner, a named identity, or any evonode.
pub fn is_perpetual_distribution_recipient(
    contract_owner_id: Identifier,
    recipient: TokenDistributionRecipient,
    identity: &QualifiedIdentity,
) -> bool {
    match recipient {
        TokenDistributionRecipient::ContractOwner => contract_owner_id == identity.identity.id(),
        TokenDistributionRecipient::Identity(identifier) => identifier == identity.identity.id(),
        TokenDistributionRecipient::EvonodesByParticipation => {
            identity.identity_type == IdentityType::Evonode
        }
    }
}

/// The registry name of a token as shown to the user: control and bidi
/// characters removed, so a look-alike name cannot hide its difference or
/// reorder the text around it. A name with nothing left falls back to the
/// shortened token ID.
pub fn token_display_name(name: &str, token_id: &Identifier) -> String {
    let cleaned = crate::model::identity_name::clean_display_text(name);
    if cleaned.is_empty() {
        crate::model::identity_name::shorten_id(&token_id.to_string(Encoding::Base58))
    } else {
        cleaned
    }
}

/// `value` in the token's decimals with `alias` as the unit — the one place
/// a token amount is formatted from its configuration.
pub fn token_amount(
    token_configuration: &TokenConfiguration,
    alias: &str,
    value: TokenAmount,
) -> Amount {
    Amount::new(value, token_configuration.conventions().decimals()).with_unit_name(alias)
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
                is_perpetual_distribution_recipient(
                    contract_owner_id,
                    dist.distribution_recipient(),
                    identity,
                )
            });
        perpetual_recipient || is_pre_programmed_recipient(token_configuration, &identity_id)
    };

    let can_estimate = {
        if let Some(dist) = token_configuration
            .distribution_rules()
            .perpetual_distribution()
        {
            in_dev_mode
                || is_perpetual_distribution_recipient(
                    contract_owner_id,
                    dist.distribution_recipient(),
                    identity,
                )
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
        token_amount(
            &token_balance.token_config,
            &token_balance.token_alias,
            token_balance.balance,
        )
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
        token_amount(
            &token_balance.token_config,
            &token_balance.token_alias,
            token_balance.balance,
        )
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

        let real = |who: &QualifiedIdentity| {
            get_available_token_actions_for_identity(None, who, &config, &contract, false, None)
                .has_admin_permission()
        };
        assert!(real(&owner));
        assert!(!real(&stranger));

        // The display override lights every admin button up for the stranger…
        let displayed = get_available_token_actions_for_identity(
            None, &stranger, &config, &contract, true, None,
        );
        assert!(displayed.can_mint && displayed.has_admin_permission());
        // …which is why the hub computes permissions with the override off.
        assert!(!real(&stranger));
    }

    #[test]
    fn a_token_name_is_shown_without_control_or_bidi_characters() {
        let token_id = Identifier::from([7; 32]);
        assert_eq!(
            token_display_name("Dash\u{200B}Coffee", &token_id),
            "DashCoffee"
        );
        assert_eq!(
            token_display_name("\u{202E}eeffoChsaD", &token_id),
            "eeffoChsaD",
            "a direction override is dropped, not obeyed"
        );
        assert_eq!(token_display_name("  Grit ", &token_id), "Grit");
        assert_eq!(
            token_display_name("\u{202E}\u{200F}", &token_id),
            crate::model::identity_name::shorten_id(&token_id.to_string(Encoding::Base58)),
            "nothing visible left: the ID stands in"
        );
    }

    #[test]
    fn a_token_name_loses_every_invisible_format_character() {
        let token_id = Identifier::from([7; 32]);
        for hidden in [
            "\u{00ad}",
            "\u{180e}",
            "\u{2061}",
            "\u{2063}",
            "\u{2064}",
            "\u{e0041}",
            "\u{e007f}",
        ] {
            assert_eq!(
                token_display_name(&format!("Dash{hidden}Coffee"), &token_id),
                "DashCoffee",
                "{hidden:?} survived"
            );
        }
        assert_eq!(
            token_display_name("\u{1F468}\u{200d}\u{1F4BB} Coin", &token_id),
            "\u{1F468}\u{200d}\u{1F4BB} Coin",
            "an emoji sequence keeps its joiner"
        );
    }

    #[test]
    fn the_perpetual_recipient_rule_names_owner_identity_or_evonode() {
        let owner = identity(1);
        let other = identity(2);
        let owner_id = owner.identity.id();
        assert!(is_perpetual_distribution_recipient(
            owner_id,
            TokenDistributionRecipient::ContractOwner,
            &owner
        ));
        assert!(!is_perpetual_distribution_recipient(
            owner_id,
            TokenDistributionRecipient::ContractOwner,
            &other
        ));
        assert!(is_perpetual_distribution_recipient(
            owner_id,
            TokenDistributionRecipient::Identity(other.identity.id()),
            &other
        ));
        assert!(!is_perpetual_distribution_recipient(
            owner_id,
            TokenDistributionRecipient::EvonodesByParticipation,
            &other
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
        assert!(
            !get_available_token_actions_for_identity(
                None,
                &owner,
                &transferable(),
                &contract,
                false,
                None
            )
            .has_admin_permission()
        );
    }
}
