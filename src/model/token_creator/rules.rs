//! Token authorities, independent of form visibility or template selection.

use super::{DraftError, DraftIssue, Section};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{
    TokenConfigurationPreset, TokenConfigurationPresetFeatures,
};
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::{
    TokenConfigurationConvention, v0::TokenConfigurationConventionV0,
};
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_marketplace_rules::TokenMarketplaceRules;
use dash_sdk::dpp::data_contract::change_control_rules::{
    ChangeControlRules, authorized_action_takers::AuthorizedActionTakers, v0::ChangeControlRulesV0,
};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::collections::BTreeSet;

/// Every independently configurable action in the creator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum RuleKind {
    Mint,
    Burn,
    Freeze,
    Unfreeze,
    Destroy,
    Pause,
    Cap,
    Conventions,
    Recurring,
    Destination,
    Override,
    Marketplace,
    Pricing,
}

impl RuleKind {
    pub const ALL: [Self; 13] = [
        Self::Mint,
        Self::Burn,
        Self::Freeze,
        Self::Unfreeze,
        Self::Destroy,
        Self::Pause,
        Self::Cap,
        Self::Conventions,
        Self::Recurring,
        Self::Destination,
        Self::Override,
        Self::Marketplace,
        Self::Pricing,
    ];

    /// Human-readable action scope.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mint => "Mint more tokens",
            Self::Burn => "Burn the actor's own tokens",
            Self::Freeze => "Freeze one holder",
            Self::Unfreeze => "Unfreeze one holder",
            Self::Destroy => "Destroy an entire frozen balance",
            Self::Pause => "Pause or resume transfers",
            Self::Cap => "Change maximum supply",
            Self::Conventions => "Change display conventions",
            Self::Recurring => "Change recurring distribution",
            Self::Destination => "Change the default mint recipient",
            Self::Override => "Change per-mint recipient choice",
            Self::Marketplace => "Change marketplace trade mode",
            Self::Pricing => "Change direct-purchase pricing",
        }
    }

    /// Stable contextual-help identifier.
    pub fn help_key(self) -> &'static str {
        match self {
            Self::Mint => "rule.mint",
            Self::Burn => "rule.burn",
            Self::Freeze => "rule.freeze",
            Self::Unfreeze => "rule.unfreeze",
            Self::Destroy => "rule.destroy",
            Self::Pause => "rule.pause",
            Self::Cap => "rule.cap",
            Self::Conventions => "rule.conventions",
            Self::Recurring => "rule.recurring",
            Self::Destination => "rule.destination",
            Self::Override => "rule.override",
            Self::Marketplace => "rule.marketplace",
            Self::Pricing => "rule.pricing",
        }
    }
}

/// An authority selection retaining incomplete identity input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ActorDraft {
    #[default]
    NoOne,
    Owner,
    Identity(String),
    MainGroup,
    Group(u16),
}

impl ActorDraft {
    /// Resolves a selection only when every referenced group or identity exists in the draft.
    pub fn parse(
        &self,
        groups: &BTreeSet<u16>,
        main: Option<u16>,
    ) -> Result<AuthorizedActionTakers, DraftIssue> {
        Ok(match self {
            Self::NoOne => AuthorizedActionTakers::NoOne,
            Self::Owner => AuthorizedActionTakers::ContractOwner,
            Self::Identity(text) => AuthorizedActionTakers::Identity(parse_identity(text)?),
            Self::MainGroup if main.is_some_and(|id| groups.contains(&id)) => {
                AuthorizedActionTakers::MainGroup
            }
            Self::Group(id) if groups.contains(id) => AuthorizedActionTakers::Group(*id),
            Self::MainGroup | Self::Group(_) => return Err(DraftIssue::MissingGroup),
        })
    }

    /// Whether the selection uses a particular numbered group, including its main-group alias.
    pub fn references(&self, group: u16, main: Option<u16>) -> bool {
        matches!(self, Self::Group(id) if *id == group)
            || (*self == Self::MainGroup && main == Some(group))
    }

    /// Complete public authority label; no claim about ownership of an entered key.
    pub fn label(&self) -> String {
        match self {
            Self::NoOne => "No one".into(),
            Self::Owner => "Contract owner".into(),
            Self::Identity(id) if id.is_empty() => "Specific identity (not entered)".into(),
            Self::Identity(id) => format!("Identity {id}"),
            Self::MainGroup => "Main group".into(),
            Self::Group(id) => format!("Group {id}"),
        }
    }
}

impl From<&AuthorizedActionTakers> for ActorDraft {
    fn from(value: &AuthorizedActionTakers) -> Self {
        match value {
            AuthorizedActionTakers::NoOne => Self::NoOne,
            AuthorizedActionTakers::ContractOwner => Self::Owner,
            AuthorizedActionTakers::Identity(id) => Self::Identity(id.to_string(Encoding::Base58)),
            AuthorizedActionTakers::MainGroup => Self::MainGroup,
            AuthorizedActionTakers::Group(position) => Self::Group(*position),
        }
    }
}

/// Parses an identity's public identifier, without asserting control or network existence.
pub fn parse_identity(text: &str) -> Result<Identifier, DraftIssue> {
    Identifier::from_string(text.trim(), Encoding::Base58).map_err(|_| DraftIssue::Identity)
}

/// Both authorities and all three protocol change allowances for one action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleDraft {
    pub performer: ActorDraft,
    pub administrator: ActorDraft,
    pub allow_disable_action: bool,
    pub allow_remove_administrator: bool,
    pub allow_replace_administrator: bool,
}

impl RuleDraft {
    /// A conservative permanent restriction with no actor, administrator or change allowance.
    pub fn permanently_disabled(&self) -> bool {
        self.performer == ActorDraft::NoOne
            && self.administrator == ActorDraft::NoOne
            && !self.allow_disable_action
            && !self.allow_remove_administrator
            && !self.allow_replace_administrator
    }

    /// Parses exactly what is displayed, including otherwise hidden administrator selections.
    pub fn parse(
        &self,
        groups: &BTreeSet<u16>,
        main: Option<u16>,
    ) -> Result<ChangeControlRules, DraftIssue> {
        Ok(ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: self.performer.parse(groups, main)?,
            admin_action_takers: self.administrator.parse(groups, main)?,
            changing_authorized_action_takers_to_no_one_allowed: self.allow_disable_action,
            changing_admin_action_takers_to_no_one_allowed: self.allow_remove_administrator,
            self_changing_admin_action_takers_allowed: self.allow_replace_administrator,
        }))
    }
}

impl From<&ChangeControlRules> for RuleDraft {
    fn from(value: &ChangeControlRules) -> Self {
        let ChangeControlRules::V0(rule) = value;
        Self {
            performer: (&rule.authorized_to_make_change).into(),
            administrator: (&rule.admin_action_takers).into(),
            allow_disable_action: rule.changing_authorized_action_takers_to_no_one_allowed,
            allow_remove_administrator: rule.changing_admin_action_takers_to_no_one_allowed,
            allow_replace_administrator: rule.self_changing_admin_action_takers_allowed,
        }
    }
}

/// A complete permission template, including the settings outside the 13 rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleBundle {
    pub rules: [RuleDraft; 13],
    pub allow_mint_recipient_choice: bool,
    pub main_group_change: ActorDraft,
}

impl Default for RuleBundle {
    fn default() -> Self {
        Self {
            rules: std::array::from_fn(|_| RuleDraft::default()),
            allow_mint_recipient_choice: false,
            main_group_change: ActorDraft::NoOne,
        }
    }
}

impl RuleBundle {
    /// Returns one rule by its fixed protocol role.
    pub fn get(&self, kind: RuleKind) -> &RuleDraft {
        &self.rules[kind as usize]
    }
    /// Edits one rule without changing other powers.
    pub fn get_mut(&mut self, kind: RuleKind) -> &mut RuleDraft {
        &mut self.rules[kind as usize]
    }

    /// The approved limited-owner example: mint, own-balance burn and pause only.
    pub fn limited_owner() -> Self {
        let mut bundle = Self::default();
        for kind in [RuleKind::Mint, RuleKind::Burn, RuleKind::Pause] {
            bundle.get_mut(kind).performer = ActorDraft::Owner;
        }
        bundle
    }

    /// Retains the exact behavior of all five supported SDK presets.
    pub fn from_preset(features: TokenConfigurationPresetFeatures) -> Self {
        let preset = TokenConfigurationPreset {
            features,
            action_taker: AuthorizedActionTakers::ContractOwner,
        };
        let config = preset.token_configuration_v0(
            TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
                localizations: Default::default(),
                decimals: 2,
            }),
            0,
            None,
            true,
            true,
        );
        let distribution = &config.distribution_rules;
        let TokenMarketplaceRules::V0(marketplace) = &config.marketplace_rules;
        Self {
            rules: [
                (&config.manual_minting_rules).into(),
                (&config.manual_burning_rules).into(),
                (&config.freeze_rules).into(),
                (&config.unfreeze_rules).into(),
                (&config.destroy_frozen_funds_rules).into(),
                (&config.emergency_action_rules).into(),
                (&config.max_supply_change_rules).into(),
                (&config.conventions_change_rules).into(),
                distribution.perpetual_distribution_rules().into(),
                distribution.new_tokens_destination_identity_rules().into(),
                distribution
                    .minting_allow_choosing_destination_rules()
                    .into(),
                (&marketplace.trade_mode_change_rules).into(),
                distribution.change_direct_purchase_pricing_rules().into(),
            ],
            allow_mint_recipient_choice: distribution.minting_allow_choosing_destination(),
            main_group_change: (&config.main_control_group_can_be_modified).into(),
        }
    }

    /// Validates every rule, even when its corresponding feature starts disabled.
    pub fn parse(
        &self,
        groups: &BTreeSet<u16>,
        main: Option<u16>,
    ) -> Result<[ChangeControlRules; 13], DraftError> {
        let mut result =
            std::array::from_fn(|_| ChangeControlRules::V0(ChangeControlRulesV0::default()));
        for kind in RuleKind::ALL {
            result[kind as usize] = self
                .get(kind)
                .parse(groups, main)
                .map_err(|issue| DraftError::new(Section::Supply, kind.label(), issue))?;
        }
        Ok(result)
    }

    /// Lists rules whose performer or administrator references a group.
    pub fn references(&self, group: u16, main: Option<u16>) -> Vec<&'static str> {
        let mut references: Vec<_> = RuleKind::ALL
            .into_iter()
            .filter(|kind| {
                let rule = self.get(*kind);
                rule.performer.references(group, main) || rule.administrator.references(group, main)
            })
            .map(RuleKind::label)
            .collect();
        if self.main_group_change.references(group, main) {
            references.push("Main group replacement");
        }
        references
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_owner_example_has_only_three_performers_and_no_administrators() {
        let bundle = RuleBundle::limited_owner();
        for kind in RuleKind::ALL {
            let rule = bundle.get(kind);
            assert_eq!(
                rule.performer == ActorDraft::Owner,
                [RuleKind::Mint, RuleKind::Burn, RuleKind::Pause].contains(&kind)
            );
            assert_eq!(rule.administrator, ActorDraft::NoOne);
            assert!(
                !rule.allow_disable_action
                    && !rule.allow_remove_administrator
                    && !rule.allow_replace_administrator
            );
        }
    }

    #[test]
    fn disabled_performer_is_not_a_permanent_restriction_when_an_admin_remains() {
        let rule = RuleDraft {
            administrator: ActorDraft::Owner,
            ..Default::default()
        };
        assert!(!rule.permanently_disabled());
    }

    #[test]
    fn missing_group_references_are_rejected_even_for_disabled_features() {
        let mut bundle = RuleBundle::default();
        bundle.get_mut(RuleKind::Recurring).administrator = ActorDraft::Group(7);
        assert!(bundle.parse(&BTreeSet::new(), None).is_err());
        assert_eq!(
            bundle.references(7, None),
            vec![RuleKind::Recurring.label()]
        );
    }
}
