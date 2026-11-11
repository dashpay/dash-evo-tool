//! Canonical token drafts and pure validation shared by the form and backend.

pub mod amount;
pub mod contract;
pub mod distributions;
mod document_json;
pub mod groups;
mod parse;
pub mod registration;
pub mod rules;

use amount::{AmountError, TokenAmountInput};
use dash_sdk::dpp::data_contract::associated_token::{
    token_configuration::v0::TokenConfigurationPresetFeatures,
    token_distribution_rules::TokenDistributionRules,
    token_keeps_history_rules::TokenKeepsHistoryRules,
};
use dash_sdk::dpp::data_contract::change_control_rules::{
    ChangeControlRules, authorized_action_takers::AuthorizedActionTakers,
};
use dash_sdk::dpp::data_contract::{GroupContractPosition, group::Group};
use dash_sdk::dpp::{ProtocolError, balances::credits::TokenAmount, consensus::ConsensusError};
use distributions::{FunctionDraft, FunctionKind};
use groups::GroupDraft;
use rules::{ActorDraft, RuleBundle};
use std::collections::BTreeMap;

/// All token-configuration inputs for
/// the single-token contract builder, grouped to keep the
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
    /// Whether holders may move the token to another identity. `false` needs
    /// a version 1 token configuration (protocol version 14).
    pub transferable: bool,
}

/// Ordered semantic sections, shared by validation and navigation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Basics,
    Supply,
    Distribution,
    Groups,
    History,
    Documents,
    Review,
}

impl Section {
    pub const ALL: [Self; 7] = [
        Self::Basics,
        Self::Supply,
        Self::Distribution,
        Self::Groups,
        Self::History,
        Self::Documents,
        Self::Review,
    ];
    /// The approved section label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Basics => "Basics",
            Self::Supply => "Supply & permissions",
            Self::Distribution => "Distribution",
            Self::Groups => "Groups",
            Self::History => "History",
            Self::Documents => "Search & documents",
            Self::Review => "Review",
        }
    }
}

/// A validation failure retains a typed cause and the field to revisit.
#[derive(Debug, thiserror::Error)]
#[error("{issue}")]
pub struct DraftError {
    pub section: Section,
    pub field: &'static str,
    #[source]
    pub issue: DraftIssue,
}

impl DraftError {
    /// Associates a failure with an editable field.
    pub fn new(section: Section, field: &'static str, issue: DraftIssue) -> Self {
        Self {
            section,
            field,
            issue,
        }
    }
    /// Preserves a protocol failure without displaying its technical text.
    pub fn protocol(
        section: Section,
        field: &'static str,
        source: impl Into<Box<ProtocolError>>,
    ) -> Self {
        Self::new(
            section,
            field,
            DraftIssue::Protocol {
                source: source.into(),
            },
        )
    }
    /// Reports the first typed protocol validation cause.
    pub fn consensus(section: Section, field: &'static str, errors: Vec<ConsensusError>) -> Self {
        Self::new(
            section,
            field,
            errors
                .into_iter()
                .next()
                .map_or(DraftIssue::InvalidConfiguration, |source| {
                    DraftIssue::Consensus {
                        source: Box::new(source),
                    }
                }),
        )
    }
}

/// Actionable input failures; technical sources remain in Debug and the error chain.
#[derive(Debug, thiserror::Error)]
pub enum DraftIssue {
    #[error(
        "Reduce document schemas, groups or allocations. The creation request is larger than this network allows."
    )]
    CreationSize,
    #[error("Choose a starting example, an existing template, or Custom rules.")]
    Policy,
    #[error("Enter both the singular and plural token names.")]
    Name,
    #[error("Use each language only once and include English (en).")]
    Language,
    #[error("Use names with 3 to 25 characters, without spaces or control characters.")]
    NameFormat,
    #[error("Leave the description empty or enter 3 to 100 UTF-8 bytes.")]
    Description,
    #[error(transparent)]
    Amount(#[from] AmountError),
    #[error("Choose a maximum supply at least as large as the initial supply.")]
    Supply,
    #[error(
        "Choose a default recipient, allow a recipient choice, or remove both authorities that can change recipient choice."
    )]
    MintDestination,
    #[error("Enter a valid Base58 identity ID.")]
    Identity,
    #[error("Choose an existing group, and select a main group before using its alias.")]
    MissingGroup,
    #[error("Enter a whole number within the supported range.")]
    Unsigned,
    #[error("Enter a signed whole number within the supported range.")]
    Signed,
    #[error("Enter a positive interval. Time intervals must align to whole minutes.")]
    Interval,
    #[error("Choose epoch-based intervals for EvoNode participation allocations.")]
    EvoNodeInterval,
    #[error("Use a different starting index for each step.")]
    DuplicateStep,
    #[error("Use each identity only once in a group.")]
    DuplicateMember,
    #[error("Add at least two different group members.")]
    GroupTooSmall,
    #[error("Enter approval power from 1 to 65,535.")]
    GroupPower,
    #[error(
        "This group needs {required} approval points but has only {available}. Lower the requirement or add member power."
    )]
    UnreachableGroup { required: u32, available: u64 },
    #[error("Keep each group's position unique. Add a new group with another position.")]
    DuplicateGroup,
    #[error("Choose a future local date and time for this allocation.")]
    ScheduleTime,
    #[error(
        "This recipient already has an allocation at this time. Change the time or remove the duplicate row."
    )]
    DuplicateSchedule,
    #[error("Add at least one scheduled allocation, or turn scheduled distribution off.")]
    EmptySchedule,
    #[error("Enter an allocation greater than zero and within the supported balance range.")]
    Allocation,
    #[error(
        "Remove moderator action fees. This token contract does not declare a moderation team."
    )]
    DocumentModeratorFees,
    #[error(
        "Use at most 50 distinct search keywords, each 3 to 50 UTF-8 bytes after lowercase conversion, without spaces or control characters."
    )]
    Keyword,
    #[error("Enter a JSON object mapping document names to object schemas, or clear the field.")]
    DocumentsShape,
    #[error("The document JSON could not be read. Correct its syntax and try again.")]
    DocumentsJson {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Keep document references inside the pasted definitions. Replace external references with local fragments beginning with #."
    )]
    ExternalDocumentReference,
    #[error("The document definitions are too large. Reduce them to at most 1 MiB.")]
    DocumentsSize,
    #[error(
        "These settings are not supported by the selected network. Change them or choose a supported network."
    )]
    Unsupported,
    #[error("These settings are not accepted. Check the indicated field and adjust its values.")]
    InvalidConfiguration,
    #[error("These settings could not be checked. Check the indicated field and try again.")]
    Protocol {
        #[source]
        source: Box<ProtocolError>,
    },
    #[error("These settings are not accepted. Check the indicated field and adjust its values.")]
    Consensus {
        #[source]
        source: Box<ConsensusError>,
    },
}

/// A localized name and whether its singular form is included in search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameDraft {
    pub language: String,
    pub singular: String,
    pub plural: String,
    pub searchable: bool,
}

/// The explicit starting policy; editing its authority bundle makes it Custom.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Policy {
    #[default]
    Unselected,
    Fixed,
    LimitedOwner,
    Custom,
    Preset(TokenConfigurationPresetFeatures),
}

/// Where newly minted or distributed tokens go. None is only a default-mint option.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Recipient {
    #[default]
    Owner,
    Identity(String),
    EvoNodes,
    None,
}

/// What counts one recurring period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IntervalKind {
    #[default]
    Time,
    Blocks,
    Epochs,
}

/// Fixed duration units, independent of calendar schedules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimeUnit {
    Seconds,
    Minutes,
    Hours,
    #[default]
    Days,
    Weeks,
    Years,
}

impl TimeUnit {
    pub const ALL: [Self; 6] = [
        Self::Seconds,
        Self::Minutes,
        Self::Hours,
        Self::Days,
        Self::Weeks,
        Self::Years,
    ];
    /// Unit label with its fixed-duration meaning.
    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
            Self::Weeks => "weeks",
            Self::Years => "years (365 days)",
        }
    }
    /// Converts to milliseconds with checked arithmetic.
    pub fn milliseconds(self, count: u64) -> Option<u64> {
        count.checked_mul(match self {
            Self::Seconds => 1_000,
            Self::Minutes => 60_000,
            Self::Hours => 3_600_000,
            Self::Days => 86_400_000,
            Self::Weeks => 604_800_000,
            Self::Years => 31_536_000_000,
        })
    }
}

/// Retains each function's inputs when the selected function changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringDraft {
    pub enabled: bool,
    pub interval_kind: IntervalKind,
    pub interval: String,
    pub time_unit: TimeUnit,
    pub recipient: Recipient,
    pub function: FunctionKind,
    pub functions: [FunctionDraft; 8],
}

impl Default for RecurringDraft {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_kind: IntervalKind::Time,
            interval: "1".into(),
            time_unit: TimeUnit::Days,
            recipient: Recipient::Owner,
            function: FunctionKind::Fixed,
            functions: FunctionKind::ALL.map(FunctionDraft::example),
        }
    }
}

/// An absolute allocation instant; rendering never recomputes it from now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleDraft {
    pub id: u64,
    pub timestamp: Option<u64>,
    pub recipient: Recipient,
    pub amount: TokenAmountInput,
}

/// The single editable token configuration, independent of identity readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenDraft {
    pub names: Vec<NameDraft>,
    pub description: String,
    pub decimals: u8,
    pub capitalize: bool,
    pub initial: TokenAmountInput,
    pub maximum: TokenAmountInput,
    pub no_maximum: bool,
    pub policy: Policy,
    pub permissions: RuleBundle,
    pub transferable: bool,
    pub start_paused: bool,
    pub allow_receive_frozen: bool,
    pub mint_recipient: Recipient,
    pub recurring: RecurringDraft,
    pub scheduled_enabled: bool,
    pub scheduled: Vec<ScheduleDraft>,
    pub once_enabled: bool,
    pub once_amount: TokenAmountInput,
    pub groups: Vec<GroupDraft>,
    pub main_group: Option<u16>,
    pub history: [bool; 6],
    pub keywords: String,
    pub documents: String,
}

impl Default for TokenDraft {
    fn default() -> Self {
        Self {
            names: vec![NameDraft {
                language: "en".into(),
                singular: String::new(),
                plural: String::new(),
                searchable: true,
            }],
            description: String::new(),
            decimals: 2,
            capitalize: false,
            initial: TokenAmountInput::default(),
            maximum: TokenAmountInput::default(),
            no_maximum: false,
            policy: Policy::Unselected,
            permissions: RuleBundle::default(),
            transferable: true,
            start_paused: false,
            allow_receive_frozen: false,
            mint_recipient: Recipient::Owner,
            recurring: RecurringDraft::default(),
            scheduled_enabled: false,
            scheduled: Vec::new(),
            once_enabled: false,
            once_amount: TokenAmountInput::default(),
            groups: Vec::new(),
            main_group: None,
            history: [true; 6],
            keywords: String::new(),
            documents: String::new(),
        }
    }
}

impl TokenDraft {
    /// Returns an example proposal without mutating the user's draft.
    pub fn example(limited_owner: bool) -> Self {
        let name = if limited_owner {
            "CloudPoint"
        } else {
            "CloudCredit"
        };
        Self {
            names: vec![NameDraft {
                language: "en".into(),
                singular: name.into(),
                plural: format!("{name}s"),
                searchable: true,
            }],
            initial: TokenAmountInput::tokens("1000"),
            maximum: TokenAmountInput::tokens(if limited_owner { "10000" } else { "1000" }),
            policy: if limited_owner {
                Policy::LimitedOwner
            } else {
                Policy::Fixed
            },
            permissions: if limited_owner {
                RuleBundle::limited_owner()
            } else {
                RuleBundle::default()
            },
            ..Default::default()
        }
    }

    /// Changes the approved example's policy fields while retaining authored content.
    pub fn example_proposal(&self, limited_owner: bool) -> Self {
        let example = Self::example(limited_owner);
        let mut proposal = self.clone();
        let _ = proposal.change_precision(example.decimals);
        proposal.initial = example.initial;
        proposal.maximum = example.maximum;
        proposal.no_maximum = false;
        proposal.policy = example.policy;
        proposal.permissions = example.permissions;
        proposal.transferable = true;
        proposal.start_paused = false;
        proposal.allow_receive_frozen = false;
        proposal.mint_recipient = Recipient::Owner;
        proposal.main_group = None;
        proposal.history = [true; 6];
        proposal.recurring.enabled = false;
        proposal.scheduled_enabled = false;
        proposal.once_enabled = false;
        proposal
    }

    /// Changes precision without rounding or discarding invalid or inactive inputs.
    /// An active invalid amount is reported after applying the precision so it can be edited.
    pub fn change_precision(&mut self, decimals: u8) -> Result<(), DraftError> {
        if decimals > 16 {
            return Err(DraftError::new(
                Section::Basics,
                "Decimal precision",
                AmountError::Decimals.into(),
            ));
        }
        let old = self.decimals;
        let mut error = None;
        let mut retarget = |amount: &mut TokenAmountInput, active: bool, section, field| {
            if let Err(source) = amount.change_precision(old, decimals)
                && active
                && error.is_none()
            {
                error = Some(DraftError::new(section, field, source.into()));
            }
        };
        retarget(&mut self.initial, true, Section::Supply, "Initial supply");
        retarget(
            &mut self.maximum,
            !self.no_maximum,
            Section::Supply,
            "Maximum supply",
        );
        retarget(
            &mut self.once_amount,
            self.once_enabled,
            Section::Distribution,
            "Amount per identity",
        );
        for row in &mut self.scheduled {
            retarget(
                &mut row.amount,
                self.scheduled_enabled,
                Section::Distribution,
                "Scheduled amount",
            );
        }
        for function in &mut self.recurring.functions {
            let active = self.recurring.enabled && function.kind == self.recurring.function;
            for (parameter, amount) in &mut function.amounts {
                retarget(amount, active, Section::Distribution, parameter.label());
            }
            for (_, amount) in &mut function.steps {
                retarget(amount, active, Section::Distribution, "Phase amount");
            }
        }
        self.decimals = decimals;
        error.map_or(Ok(()), Err)
    }

    /// A legacy template replaces permissions only; other configuration stays intact.
    pub fn preset_proposal(&self, features: TokenConfigurationPresetFeatures) -> Self {
        Self {
            policy: Policy::Preset(features),
            permissions: RuleBundle::from_preset(features),
            ..self.clone()
        }
    }

    /// Keeps example labels only while their applied policy settings still match.
    /// Permission-only SDK templates are classified by their authority bundle.
    pub fn reconcile_policy(&mut self) {
        let matches = match self.policy {
            Policy::Fixed | Policy::LimitedOwner => {
                let limited_owner = self.policy == Policy::LimitedOwner;
                let amount_matches = |input: &TokenAmountInput, tokens| {
                    matches!(
                        (input.parse(self.decimals), TokenAmountInput::tokens(tokens).parse(self.decimals)),
                        (Ok(actual), Ok(expected)) if actual == expected
                    )
                };
                self.permissions
                    == if limited_owner {
                        RuleBundle::limited_owner()
                    } else {
                        RuleBundle::default()
                    }
                    && self.decimals == 2
                    && amount_matches(&self.initial, "1000")
                    && !self.no_maximum
                    && amount_matches(&self.maximum, if limited_owner { "10000" } else { "1000" })
                    && self.transferable
                    && !self.start_paused
                    && !self.allow_receive_frozen
                    && self.mint_recipient == Recipient::Owner
                    && self.main_group.is_none()
                    && self.history == [true; 6]
                    && !self.recurring.enabled
                    && !self.scheduled_enabled
                    && !self.once_enabled
            }
            Policy::Preset(features) => self.permissions == RuleBundle::from_preset(features),
            _ => true,
        };
        if !matches {
            self.policy = Policy::Custom;
        }
    }

    /// Reports every reference that must be changed before deleting a group.
    pub fn group_references(&self, position: u16) -> Vec<&'static str> {
        let mut references = self.permissions.references(position, self.main_group);
        if self.main_group == Some(position) {
            references.push("Main group assignment");
        }
        references
    }

    /// Deletes only unreferenced groups, preserving all remaining local group IDs.
    pub fn remove_group(&mut self, position: u16) -> bool {
        if !self.group_references(position).is_empty() {
            return false;
        }
        let length = self.groups.len();
        self.groups.retain(|group| group.position != position);
        length != self.groups.len()
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    #[test]
    fn unused_examples_do_not_block_whole_token_precision() {
        let mut draft = TokenDraft::default();
        let latent = draft.recurring.clone();
        draft.change_precision(0).unwrap();
        assert_eq!(draft.decimals, 0);
        assert_eq!(draft.recurring, latent);
        draft.change_precision(1).unwrap();
        assert_eq!(draft.decimals, 1);
    }

    #[test]
    fn active_fraction_survives_precision_change_as_an_editable_error() {
        let mut draft = TokenDraft::example(false);
        draft.initial = TokenAmountInput::tokens("1.23");
        assert!(draft.change_precision(0).is_err());
        assert_eq!(draft.decimals, 0);
        assert_eq!(draft.initial.text, "1.23");
        assert!(draft.initial.parse(draft.decimals).is_err());
    }
    #[test]
    fn checkpoint_review_example_preserves_latent_raw_allocation_value() {
        let mut draft = TokenDraft {
            decimals: 3,
            ..Default::default()
        };
        draft.scheduled.push(ScheduleDraft {
            id: 0,
            timestamp: Some(100_000),
            recipient: Recipient::Owner,
            amount: TokenAmountInput {
                text: "1000".into(),
                base_units: true,
            },
        });
        let proposal = draft.example_proposal(false);
        assert_eq!(
            proposal.scheduled[0]
                .amount
                .parse(proposal.decimals)
                .unwrap(),
            100
        );
        assert_eq!(
            amount::format_amount(
                proposal.scheduled[0]
                    .amount
                    .parse(proposal.decimals)
                    .unwrap(),
                proposal.decimals
            ),
            "1"
        );
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    fn assert_policy_edits_become_custom(limited_owner: bool) {
        let edits: &[fn(&mut TokenDraft)] = &[
            |draft| draft.no_maximum = true,
            |draft| draft.initial.text = "500".into(),
            |draft| draft.maximum.text = "15000".into(),
            |draft| draft.transferable = false,
            |draft| draft.start_paused = true,
            |draft| draft.allow_receive_frozen = true,
            |draft| draft.recurring.enabled = true,
            |draft| draft.scheduled_enabled = true,
            |draft| draft.once_enabled = true,
            |draft| draft.mint_recipient = Recipient::Identity("unfinished".into()),
            |draft| draft.main_group = Some(0),
            |draft| draft.history[0] = false,
        ];
        for (index, edit) in edits.iter().enumerate() {
            let mut draft = TokenDraft::example(limited_owner);
            edit(&mut draft);
            let before = draft.clone();
            draft.reconcile_policy();
            assert_eq!(draft.policy, Policy::Custom, "policy edit {index}");
            draft.policy = before.policy;
            assert_eq!(draft, before, "classification must not modify settings");
        }
    }

    #[test]
    fn creator_policy_label_tracks_fixed_example_settings() {
        assert_policy_edits_become_custom(false);
    }

    #[test]
    fn creator_policy_label_tracks_owner_example_settings() {
        assert_policy_edits_become_custom(true);
    }

    #[test]
    fn creator_policy_label_ignores_names_units_and_inactive_recipes() {
        for limited_owner in [false, true] {
            let mut draft = TokenDraft::example(limited_owner);
            let policy = draft.policy;
            draft.names[0].singular = "Renamed".into();
            draft.initial = TokenAmountInput {
                text: "100000".into(),
                base_units: true,
            };
            draft.recurring.interval = "unfinished".into();
            draft.once_amount.text = "unfinished".into();
            draft.reconcile_policy();
            assert_eq!(draft.policy, policy);
        }
        let mut draft = TokenDraft::default()
            .preset_proposal(TokenConfigurationPresetFeatures::WithOnlyEmergencyAction);
        let policy = draft.policy;
        draft.no_maximum = true;
        draft.reconcile_policy();
        assert_eq!(
            draft.policy, policy,
            "SDK templates describe permissions only"
        );
    }
}
