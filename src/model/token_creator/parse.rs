use super::rules::{RuleKind, parse_identity};
use super::*;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::associated_token::{
    token_configuration_convention::{
        TokenConfigurationConvention, v0::TokenConfigurationConventionV0,
    },
    token_configuration_localization::{
        TokenConfigurationLocalization, v0::TokenConfigurationLocalizationV0,
    },
    token_distribution_rules::v0::TokenDistributionRulesV0,
    token_keeps_history_rules::v0::TokenKeepsHistoryRulesV0,
    token_perpetual_distribution::{
        TokenPerpetualDistribution, distribution_recipient::TokenDistributionRecipient,
        reward_distribution_type::RewardDistributionType, v0::TokenPerpetualDistributionV0,
    },
    token_pre_programmed_distribution::{
        TokenPreProgrammedDistribution, v0::TokenPreProgrammedDistributionV0,
    },
};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use std::collections::{BTreeMap, BTreeSet};

impl Recipient {
    /// Resolves owner and specific identities for an initial or scheduled allocation.
    pub fn identity(&self, owner: Identifier) -> Result<Identifier, DraftIssue> {
        match self {
            Self::Owner | Self::None => Ok(owner),
            Self::Identity(text) => parse_identity(text),
            Self::EvoNodes => Err(DraftIssue::Identity),
        }
    }
}

impl RecurringDraft {
    /// Parses and validates an enabled recurring allocation against the selected network.
    pub fn parse(
        &self,
        decimals: u8,
        network: Network,
        version: &PlatformVersion,
    ) -> Result<Option<TokenPerpetualDistribution>, DraftError> {
        if !self.enabled {
            return Ok(None);
        }
        let error = |issue| DraftError::new(Section::Distribution, "Recurring interval", issue);
        let interval = self
            .interval
            .parse::<u64>()
            .map_err(|_| error(DraftIssue::Interval))?;
        if interval == 0 {
            return Err(error(DraftIssue::Interval));
        }
        let function = self.functions[self.function as usize].parse(decimals, 0, version)?;
        let distribution_type = match self.interval_kind {
            IntervalKind::Time => RewardDistributionType::TimeBasedDistribution {
                interval: self
                    .time_unit
                    .milliseconds(interval)
                    .ok_or_else(|| error(DraftIssue::Interval))?,
                function,
            },
            IntervalKind::Blocks => {
                RewardDistributionType::BlockBasedDistribution { interval, function }
            }
            IntervalKind::Epochs => RewardDistributionType::EpochBasedDistribution {
                interval: interval
                    .try_into()
                    .map_err(|_| error(DraftIssue::Interval))?,
                function,
            },
        };
        let validation = distribution_type
            .validate_structure_interval(network, version)
            .map_err(|source| {
                DraftError::protocol(Section::Distribution, "Recurring interval", source)
            })?;
        if !validation.is_valid() {
            return Err(DraftError::consensus(
                Section::Distribution,
                "Recurring interval",
                validation.errors,
            ));
        }
        let distribution_recipient = match &self.recipient {
            Recipient::Owner => TokenDistributionRecipient::ContractOwner,
            Recipient::Identity(text) => {
                TokenDistributionRecipient::Identity(parse_identity(text).map_err(error)?)
            }
            Recipient::EvoNodes if self.interval_kind == IntervalKind::Epochs => {
                TokenDistributionRecipient::EvonodesByParticipation
            }
            Recipient::EvoNodes => return Err(error(DraftIssue::EvoNodeInterval)),
            Recipient::None => return Err(error(DraftIssue::Identity)),
        };
        Ok(Some(TokenPerpetualDistribution::V0(
            TokenPerpetualDistributionV0 {
                distribution_type,
                distribution_recipient,
            },
        )))
    }
}

impl TokenDraft {
    /// Validates the draft into one immutable protocol configuration. No network or IO occurs.
    pub fn parse(
        &self,
        owner: Identifier,
        network: Network,
        version: &PlatformVersion,
        now_ms: u64,
    ) -> Result<TokenContractParams, DraftError> {
        if self.policy == Policy::Unselected {
            return Err(DraftError::new(
                Section::Basics,
                "Starting policy",
                DraftIssue::Policy,
            ));
        }
        if self.decimals > 16 {
            return Err(DraftError::new(
                Section::Supply,
                "Decimal places",
                DraftIssue::Amount(AmountError::Decimals),
            ));
        }
        let token_names = self.parse_names(version)?;
        let description = self.description.trim();
        if !description.is_empty() && !(3..=100).contains(&description.len()) {
            return Err(DraftError::new(
                Section::Documents,
                "Description",
                DraftIssue::Description,
            ));
        }
        let base_supply = self
            .initial
            .parse(self.decimals)
            .map_err(|source| DraftError::new(Section::Supply, "Initial supply", source.into()))?;
        if base_supply > i64::MAX as u64 {
            return Err(DraftError::new(
                Section::Supply,
                "Initial supply",
                DraftIssue::Allocation,
            ));
        }
        let max_supply = if self.no_maximum {
            None
        } else {
            Some(self.maximum.parse(self.decimals).map_err(|source| {
                DraftError::new(Section::Supply, "Maximum supply", source.into())
            })?)
        };
        if max_supply.is_some_and(|maximum| maximum < base_supply) {
            return Err(DraftError::new(
                Section::Supply,
                "Maximum supply",
                DraftIssue::Supply,
            ));
        }
        if !self.transferable && version.protocol_version < 14 {
            return Err(DraftError::new(
                Section::Supply,
                "Ordinary transfers",
                DraftIssue::Unsupported,
            ));
        }
        let mut groups = BTreeMap::new();
        let mut remap = BTreeMap::new();
        for (index, group) in self.groups.iter().enumerate() {
            let position = u16::try_from(index).map_err(|_| {
                DraftError::new(Section::Groups, "Group position", DraftIssue::Unsigned)
            })?;
            groups.insert(position, group.parse(version)?);
            if remap.insert(group.position, position).is_some() {
                return Err(DraftError::new(
                    Section::Groups,
                    "Group position",
                    DraftIssue::DuplicateGroup,
                ));
            }
        }
        let positions: BTreeSet<_> = remap.keys().copied().collect();
        if self
            .main_group
            .is_some_and(|position| !positions.contains(&position))
        {
            return Err(DraftError::new(
                Section::Groups,
                "Main group",
                DraftIssue::MissingGroup,
            ));
        }
        let mut rules = self.permissions.parse(&positions, self.main_group)?;
        let remap_actor = |actor: &mut AuthorizedActionTakers| {
            if let AuthorizedActionTakers::Group(position) = actor
                && let Some(mapped) = remap.get(position)
            {
                *position = *mapped;
            }
        };
        for rule in &mut rules {
            let ChangeControlRules::V0(rule) = rule;
            remap_actor(&mut rule.authorized_to_make_change);
            remap_actor(&mut rule.admin_action_takers);
        }
        let mut main_control_group_change_authorized = self
            .permissions
            .main_group_change
            .parse(&positions, self.main_group)
            .map_err(|issue| DraftError::new(Section::Groups, "Main group replacement", issue))?;
        remap_actor(&mut main_control_group_change_authorized);
        let destination = match &self.mint_recipient {
            Recipient::None => None,
            other => Some(other.identity(owner).map_err(|issue| {
                DraftError::new(Section::Supply, "Default mint recipient", issue)
            })?),
        };
        let override_rule = self.permissions.get(RuleKind::Override);
        if destination.is_none()
            && !self.permissions.allow_mint_recipient_choice
            && (override_rule.performer != ActorDraft::NoOne
                || override_rule.administrator != ActorDraft::NoOne)
        {
            return Err(DraftError::new(
                Section::Supply,
                "Default mint recipient",
                DraftIssue::MintDestination,
            ));
        }
        let perpetual_distribution = self.recurring.parse(self.decimals, network, version)?;
        let pre_programmed_distribution = self.parse_schedule(owner, now_ms)?;
        let once_amount = if self.once_enabled {
            if version.protocol_version < 14 {
                return Err(DraftError::new(
                    Section::Distribution,
                    "Once per identity",
                    DraftIssue::Unsupported,
                ));
            }
            let amount = self.once_amount.parse(self.decimals).map_err(|source| {
                DraftError::new(Section::Distribution, "Amount per identity", source.into())
            })?;
            if amount == 0 || amount > crate::model::token::MAX_ONCE_PER_IDENTITY_AMOUNT {
                return Err(DraftError::new(
                    Section::Distribution,
                    "Amount per identity",
                    DraftIssue::Allocation,
                ));
            }
            Some(amount)
        } else {
            None
        };
        let distribution_rules = crate::model::token::distribution_rules_with_once_per_identity(
            TokenDistributionRulesV0 {
                perpetual_distribution,
                pre_programmed_distribution,
                perpetual_distribution_rules: rules[RuleKind::Recurring as usize].clone(),
                new_tokens_destination_identity: destination,
                new_tokens_destination_identity_rules: rules[RuleKind::Destination as usize]
                    .clone(),
                minting_allow_choosing_destination: self.permissions.allow_mint_recipient_choice,
                minting_allow_choosing_destination_rules: rules[RuleKind::Override as usize]
                    .clone(),
                change_direct_purchase_pricing_rules: rules[RuleKind::Pricing as usize].clone(),
            },
            once_amount,
        );
        let history = self.history;
        Ok(TokenContractParams {
            token_names,
            contract_keywords: self.parse_keywords()?,
            token_description: (!description.is_empty()).then(|| description.to_owned()),
            should_capitalize: self.capitalize,
            decimals: self.decimals,
            base_supply,
            max_supply,
            start_paused: self.start_paused,
            allow_transfers_to_frozen_identities: self.allow_receive_frozen,
            keeps_history: TokenKeepsHistoryRulesV0 {
                keeps_transfer_history: history[0],
                keeps_freezing_history: history[1],
                keeps_minting_history: history[2],
                keeps_burning_history: history[3],
                keeps_direct_pricing_history: history[4],
                keeps_direct_purchase_history: history[5],
            }
            .into(),
            main_control_group: self
                .main_group
                .and_then(|position| remap.get(&position).copied()),
            manual_minting_rules: rules[RuleKind::Mint as usize].clone(),
            manual_burning_rules: rules[RuleKind::Burn as usize].clone(),
            freeze_rules: rules[RuleKind::Freeze as usize].clone(),
            unfreeze_rules: rules[RuleKind::Unfreeze as usize].clone(),
            destroy_frozen_funds_rules: rules[RuleKind::Destroy as usize].clone(),
            emergency_action_rules: rules[RuleKind::Pause as usize].clone(),
            max_supply_change_rules: rules[RuleKind::Cap as usize].clone(),
            conventions_change_rules: rules[RuleKind::Conventions as usize].clone(),
            main_control_group_change_authorized,
            distribution_rules,
            groups,
            document_schemas: self.parse_documents()?,
            marketplace_rules: rules[RuleKind::Marketplace as usize].clone(),
            transferable: self.transferable,
        })
    }

    fn parse_names(
        &self,
        version: &PlatformVersion,
    ) -> Result<Vec<(String, String, String)>, DraftError> {
        let error = |issue| DraftError::new(Section::Basics, "Token names and languages", issue);
        let mut languages = BTreeSet::new();
        let mut localizations = BTreeMap::new();
        for name in &self.names {
            if name.singular.is_empty() || name.plural.is_empty() {
                return Err(error(DraftIssue::Name));
            }
            for text in [&name.singular, &name.plural] {
                if !(3..=25).contains(&text.chars().count())
                    || text.chars().any(|c| c.is_whitespace() || c.is_control())
                {
                    return Err(error(DraftIssue::NameFormat));
                }
            }
            if !languages.insert(name.language.to_lowercase()) {
                return Err(error(DraftIssue::Language));
            }
            localizations.insert(
                name.language.clone(),
                TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                    singular_form: name.singular.clone(),
                    plural_form: name.plural.clone(),
                    should_capitalize: self.capitalize,
                }),
            );
        }
        if !languages.contains("en") {
            return Err(error(DraftIssue::Language));
        }
        let conventions = TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
            decimals: self.decimals,
            localizations,
        });
        let validation = conventions
            .validate_localizations(version)
            .map_err(|source| {
                DraftError::protocol(Section::Basics, "Token names and languages", source)
            })?;
        if !validation.is_valid() {
            return Err(DraftError::consensus(
                Section::Basics,
                "Token names and languages",
                validation.errors,
            ));
        }
        Ok(self
            .names
            .iter()
            .map(|name| {
                (
                    name.singular.clone(),
                    name.plural.clone(),
                    name.language.clone(),
                )
            })
            .collect())
    }

    /// The actual unique search keywords, including every name marked searchable.
    pub fn parse_keywords(&self) -> Result<Vec<String>, DraftError> {
        let mut keywords = BTreeSet::new();
        for text in self
            .keywords
            .split(',')
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .chain(
                self.names
                    .iter()
                    .filter(|name| name.searchable)
                    .map(|name| name.singular.as_str()),
            )
        {
            // DPP lowercases during contract reconstruction. Validate and deduplicate
            // that exact representation, including Unicode expansions such as İ → i̇.
            let canonical = text.to_lowercase();
            if !(3..=50).contains(&canonical.len())
                || canonical
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(DraftError::new(
                    Section::Documents,
                    "Search keywords",
                    DraftIssue::Keyword,
                ));
            }
            keywords.insert(canonical);
        }
        if keywords.len() > 50 {
            return Err(DraftError::new(
                Section::Documents,
                "Search keywords",
                DraftIssue::Keyword,
            ));
        }
        Ok(keywords.into_iter().collect())
    }

    /// Parses the document map afresh, so clearing it always removes old definitions.
    pub fn parse_documents(
        &self,
    ) -> Result<Option<BTreeMap<String, serde_json::Value>>, DraftError> {
        let error = |issue| DraftError::new(Section::Documents, "Document type JSON", issue);
        let text = self.documents.trim();
        if text.is_empty() {
            return Ok(None);
        }
        if text.len() > 1_048_576 {
            return Err(error(DraftIssue::DocumentsSize));
        }
        let super::document_json::UniqueJson(value) = serde_json::from_str(text)
            .map_err(|source| error(DraftIssue::DocumentsJson { source }))?;
        let map = value
            .as_object()
            .ok_or_else(|| error(DraftIssue::DocumentsShape))?;
        if map.values().any(|schema| !schema.is_object()) {
            return Err(error(DraftIssue::DocumentsShape));
        }
        if map.is_empty() {
            return Ok(None);
        }
        validate_document_references(&value).map_err(error)?;
        Ok(Some(
            map.iter()
                .map(|(name, schema)| (name.clone(), schema.clone()))
                .collect(),
        ))
    }

    fn parse_schedule(
        &self,
        owner: Identifier,
        now_ms: u64,
    ) -> Result<Option<TokenPreProgrammedDistribution>, DraftError> {
        if !self.scheduled_enabled {
            return Ok(None);
        }
        let error = |issue| DraftError::new(Section::Distribution, "Scheduled allocation", issue);
        if self.scheduled.is_empty() {
            return Err(error(DraftIssue::EmptySchedule));
        }
        let mut distributions: BTreeMap<u64, BTreeMap<Identifier, u64>> = BTreeMap::new();
        for allocation in &self.scheduled {
            let timestamp = allocation
                .timestamp
                .filter(|timestamp| *timestamp > now_ms)
                .ok_or_else(|| error(DraftIssue::ScheduleTime))?;
            let recipient = allocation.recipient.identity(owner).map_err(error)?;
            let amount = allocation
                .amount
                .parse(self.decimals)
                .map_err(|source| error(source.into()))?;
            if amount == 0 || amount > i64::MAX as u64 {
                return Err(error(DraftIssue::Allocation));
            }
            if distributions
                .entry(timestamp)
                .or_default()
                .insert(recipient, amount)
                .is_some()
            {
                return Err(error(DraftIssue::DuplicateSchedule));
            }
        }
        for allocations in distributions.values() {
            if allocations
                .values()
                .try_fold(0u64, |sum, amount| sum.checked_add(*amount))
                .is_none_or(|sum| sum > i64::MAX as u64)
            {
                return Err(error(DraftIssue::Allocation));
            }
        }
        Ok(Some(TokenPreProgrammedDistribution::V0(
            TokenPreProgrammedDistributionV0 { distributions },
        )))
    }
}

// A fragment is the only reference that cannot escape the supplied document.
fn validate_document_references(value: &serde_json::Value) -> Result<(), DraftIssue> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "$ref" | "$dynamicRef" | "$recursiveRef" | "$id"
                ) && child
                    .as_str()
                    .is_some_and(|reference| !reference.starts_with('#'))
                {
                    return Err(DraftIssue::ExternalDocumentReference);
                }
                validate_document_references(child)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                validate_document_references(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;

    fn parse(draft: &TokenDraft) -> Result<TokenContractParams, DraftError> {
        draft.parse(
            Identifier::from([7; 32]),
            Network::Testnet,
            PlatformVersion::latest(),
            1000,
        )
    }

    #[test]
    fn blank_maximum_zero_and_unlimited_are_three_distinct_inputs() {
        let mut draft = TokenDraft::example(false);
        draft.maximum.text.clear();
        assert!(parse(&draft).is_err());
        draft.maximum.text = "0".into();
        assert!(parse(&draft).is_err());
        draft.initial.text = "0".into();
        assert_eq!(parse(&draft).unwrap().max_supply, Some(0));
        draft.no_maximum = true;
        assert_eq!(parse(&draft).unwrap().max_supply, None);
    }

    #[test]
    fn scheduled_instants_stay_exact_and_duplicates_are_rejected() {
        let mut draft = TokenDraft::example(false);
        draft.scheduled_enabled = true;
        draft.scheduled.push(ScheduleDraft {
            id: 1,
            timestamp: Some(100_000),
            recipient: Recipient::Owner,
            amount: TokenAmountInput::tokens("2"),
        });
        let first = parse(&draft).unwrap();
        let later = draft
            .parse(
                Identifier::from([7; 32]),
                Network::Testnet,
                PlatformVersion::latest(),
                2000,
            )
            .unwrap();
        assert_eq!(
            first.distribution_rules.pre_programmed_distribution(),
            later.distribution_rules.pre_programmed_distribution()
        );
        draft.scheduled.push(draft.scheduled[0].clone());
        assert!(matches!(
            parse(&draft).unwrap_err().issue,
            DraftIssue::DuplicateSchedule
        ));
    }

    #[test]
    fn clearing_document_json_removes_previous_schemas() {
        let mut draft = TokenDraft::example(false);
        draft.documents = r#"{"note":{"type":"object","properties":{}}}"#.into();
        assert!(draft.parse_documents().unwrap().is_some());
        draft.documents.clear();
        assert!(draft.parse_documents().unwrap().is_none());
    }

    #[test]
    fn deleting_a_referenced_group_never_retargets_any_rule() {
        let mut draft = TokenDraft::example(false);
        draft.groups = vec![GroupDraft::new(0), GroupDraft::new(3)];
        draft.permissions.get_mut(RuleKind::Mint).performer = ActorDraft::Group(3);
        assert!(!draft.remove_group(3));
        assert!(draft.remove_group(0));
        assert_eq!(draft.groups[0].position, 3);
        assert_eq!(
            draft.permissions.get(RuleKind::Mint).performer,
            ActorDraft::Group(3)
        );
    }
    #[test]
    fn document_references_cannot_read_http_or_local_files() {
        for reference in [
            "https://example.invalid/private",
            "file:///etc/passwd",
            "../private.json",
        ] {
            let mut draft = TokenDraft::default();
            draft.documents = serde_json::json!({"note": {"type": "object", "properties": {"field": {"$ref": reference}}}}).to_string();
            assert!(draft.parse_documents().is_err(), "accepted {reference}");
        }
        let mut draft = TokenDraft::default();
        draft.documents = r##"{"note":{"type":"object","$defs":{"name":{"type":"string"}},"properties":{"name":{"$ref":"#/$defs/name"}}}}"##.into();
        assert!(draft.parse_documents().is_ok());
    }

    #[test]
    fn removing_unrelated_group_preserves_logical_authority_in_snapshot() {
        use super::super::groups::MemberDraft;
        use dash_sdk::dpp::data_contract::group::Group;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let mut draft = TokenDraft::example(false);
        draft.groups = (0..3)
            .map(|position| {
                let mut group = GroupDraft::new(position);
                group.members = [position as u8 + 1, position as u8 + 11]
                    .map(|value| MemberDraft {
                        identity: Identifier::from([value; 32]).to_string(Encoding::Base58),
                        weight: "1".into(),
                    })
                    .into();
                group
            })
            .collect();
        draft.permissions.get_mut(RuleKind::Mint).performer = ActorDraft::Group(2);
        let before = parse(&draft).unwrap();
        assert!(draft.remove_group(0));
        let after = parse(&draft).unwrap();
        let ChangeControlRules::V0(rule) = after.manual_minting_rules;
        assert_eq!(
            rule.authorized_to_make_change,
            AuthorizedActionTakers::Group(1)
        );
        let Group::V0(before_group) = &before.groups[&2];
        let Group::V0(after_group) = &after.groups[&1];
        assert_eq!(before_group.members, after_group.members);
    }

    #[test]
    fn duplicate_document_names_or_nested_keys_are_rejected() {
        for text in [
            r#"{"note":{},"note":{}}"#,
            r#"{"note":{"type":"object","type":"string"}}"#,
        ] {
            let draft = TokenDraft {
                documents: text.into(),
                ..Default::default()
            };
            assert!(draft.parse_documents().is_err());
        }
    }

    #[test]
    fn owner_alias_and_same_explicit_identity_are_duplicate_allocations() {
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let mut draft = TokenDraft::example(false);
        draft.scheduled_enabled = true;
        draft.scheduled = vec![
            ScheduleDraft {
                id: 0,
                timestamp: Some(100_000),
                recipient: Recipient::Owner,
                amount: TokenAmountInput::tokens("1"),
            },
            ScheduleDraft {
                id: 1,
                timestamp: Some(100_000),
                recipient: Recipient::Identity(
                    Identifier::from([7; 32]).to_string(Encoding::Base58),
                ),
                amount: TokenAmountInput::tokens("2"),
            },
        ];
        assert!(matches!(
            parse(&draft).unwrap_err().issue,
            DraftIssue::DuplicateSchedule
        ));
    }
    #[test]
    fn token_registration_protocol_limits_are_enforced_before_signing() {
        let owner = Identifier::from([9; 32]);
        let version = PlatformVersion::latest();
        let mut draft = TokenDraft::example(false);
        draft.initial = TokenAmountInput {
            text: (i64::MAX as u64 + 1).to_string(),
            base_units: true,
        };
        draft.no_maximum = true;
        assert!(draft.parse(owner, Network::Testnet, version, 0).is_err());
        let mut draft = TokenDraft::example(false);
        draft.keywords = "é".repeat(30);
        assert!(draft.parse(owner, Network::Testnet, version, 0).is_err());
        draft.keywords = (0..51)
            .map(|i| format!("key{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(draft.parse(owner, Network::Testnet, version, 0).is_err());
        let mut draft = TokenDraft::example(false);
        draft.scheduled_enabled = true;
        draft.scheduled =
            vec![
                ScheduleDraft {
                    id: 0,
                    timestamp: Some(100_000),
                    recipient: Recipient::Owner,
                    amount: TokenAmountInput {
                        text: i64::MAX.to_string(),
                        base_units: true,
                    },
                },
                ScheduleDraft {
                    id: 1,
                    timestamp: Some(100_000),
                    recipient: Recipient::Identity(Identifier::from([8; 32]).to_string(
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                    )),
                    amount: TokenAmountInput {
                        text: "1".into(),
                        base_units: true,
                    },
                },
            ];
        assert!(draft.parse(owner, Network::Testnet, version, 0).is_err());
    }
}
