//! Contract assembly with an explicit registration ID and protocol version.

use super::{DraftError, Section, TokenContractParams};
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::{
    token_configuration::{accessors::v1::TokenConfigurationV1Setters, v0::TokenConfigurationV0},
    token_configuration_convention::TokenConfigurationConvention,
    token_configuration_localization::{
        TokenConfigurationLocalization, v0::TokenConfigurationLocalizationV0,
    },
    token_marketplace_rules::{
        TokenMarketplaceRules,
        v0::{TokenMarketplaceRulesV0, TokenTradeMode},
    },
};
use dash_sdk::dpp::data_contract::serialized_version::DataContractInSerializationFormat;
use dash_sdk::dpp::data_contract::{
    DataContract, TokenConfiguration, TokenContractPosition, config::DataContractConfig,
    v1::DataContractV1,
};
use dash_sdk::dpp::version::TryFromPlatformVersioned;
use dash_sdk::dpp::{ProtocolError, version::PlatformVersion};
use dash_sdk::platform::Identifier;
use std::collections::BTreeMap;

/// Assembles the already parsed settings at the ID that will be signed.
pub fn assemble_contract(
    owner_id: Identifier,
    contract_id: Identifier,
    params: TokenContractParams,
    version: &PlatformVersion,
) -> Result<DataContract, ProtocolError> {
    let TokenContractParams {
        token_names,
        contract_keywords,
        token_description,
        should_capitalize,
        decimals,
        base_supply,
        max_supply,
        start_paused: start_as_paused,
        allow_transfers_to_frozen_identities: allow_transfer_to_frozen_balance,
        keeps_history,
        main_control_group,
        manual_minting_rules,
        manual_burning_rules,
        freeze_rules,
        unfreeze_rules,
        destroy_frozen_funds_rules,
        emergency_action_rules,
        max_supply_change_rules,
        conventions_change_rules,
        main_control_group_change_authorized,
        distribution_rules,
        groups,
        document_schemas,
        marketplace_rules,
        transferable,
    } = params;

    let mut contract_v1 = DataContractV1 {
        id: contract_id,
        version: 1,
        owner_id,
        document_types: BTreeMap::new(), // Initialize empty, will populate below
        config: DataContractConfig::default_for_version(version)?,
        schema_defs: None,
        groups,
        tokens: BTreeMap::new(),
        keywords: contract_keywords,
        created_at: None,
        updated_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        created_at_epoch: None,
        updated_at_epoch: None,
        description: token_description.clone(),
    };

    let mut token_config_v0 = TokenConfigurationV0::default_most_restrictive();

    let TokenConfigurationConvention::V0(ref mut conv_v0) = token_config_v0.conventions;
    conv_v0.decimals = decimals;
    for (token_name, token_plural, language) in token_names {
        conv_v0.localizations.insert(
            language,
            TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                should_capitalize,
                singular_form: token_name,
                plural_form: token_plural,
            }),
        );
    }

    token_config_v0.base_supply = base_supply;
    token_config_v0.max_supply = max_supply;
    token_config_v0.start_as_paused = start_as_paused;
    token_config_v0.allow_transfer_to_frozen_balance = allow_transfer_to_frozen_balance;
    token_config_v0.keeps_history = keeps_history;
    token_config_v0.main_control_group = main_control_group;
    token_config_v0.manual_minting_rules = manual_minting_rules;
    token_config_v0.manual_burning_rules = manual_burning_rules;
    token_config_v0.freeze_rules = freeze_rules;
    token_config_v0.unfreeze_rules = unfreeze_rules;
    token_config_v0.destroy_frozen_funds_rules = destroy_frozen_funds_rules;
    token_config_v0.emergency_action_rules = emergency_action_rules;
    token_config_v0.max_supply_change_rules = max_supply_change_rules;
    token_config_v0.conventions_change_rules = conventions_change_rules;
    token_config_v0.main_control_group_can_be_modified = main_control_group_change_authorized;
    token_config_v0.distribution_rules = distribution_rules;
    token_config_v0.description = token_description;

    // All tokens are created NotTradeable; a future SDK will add more modes.
    token_config_v0.marketplace_rules = TokenMarketplaceRules::V0(TokenMarketplaceRulesV0 {
        trade_mode: TokenTradeMode::NotTradeable,
        trade_mode_change_rules: marketplace_rules,
    });

    let mut token_config = TokenConfiguration::V0(token_config_v0);
    // Upgrades to a version 1 configuration only for a token that cannot
    // be transferred, so a transferable token keeps the format every
    // network accepts.
    token_config.set_transferable(transferable);

    contract_v1
        .tokens
        .insert(TokenContractPosition::from(0u16), token_config);

    if let Some(schemas) = document_schemas {
        for (name, schema_json) in schemas {
            // Convert serde_json::Value to platform_value::Value
            let platform_value = schema_json.into();

            // Convert JSON schema to DocumentType using the proper parameters
            let mut validation_operations = Vec::new();
            let document_type =
                dash_sdk::dpp::data_contract::document_type::DocumentType::try_from_schema(
                    contract_id,
                    1,
                    contract_v1.config.version(),
                    &name,
                    platform_value,
                    None, // schema_defs
                    &contract_v1.tokens,
                    &contract_v1.config,
                    true, // validate_required
                    &mut validation_operations,
                    version,
                )?;
            contract_v1.document_types.insert(name, document_type);
        }
    }

    Ok(DataContract::V1(contract_v1))
}

/// Runs complete DPP document parsing before the immutable review is displayed.
pub fn canonical_contract(
    owner: Identifier,
    nonce: u64,
    params: TokenContractParams,
    version: &PlatformVersion,
) -> Result<DataContract, DraftError> {
    let contract_id = DataContract::generate_data_contract_id_v0(owner, nonce);
    let contract = assemble_contract(owner, contract_id, params, version)
        .map_err(|source| DraftError::protocol(Section::Documents, "Document types", source))?;
    let group_validation = DataContract::validate_groups(contract.groups(), version)
        .map_err(|source| DraftError::protocol(Section::Groups, "Groups", source))?;
    if !group_validation.is_valid() {
        return Err(DraftError::consensus(
            Section::Groups,
            "Groups",
            group_validation.errors,
        ));
    }
    for token in contract.tokens().values() {
        let references = token
            .validate_token_config_groups_exist(contract.groups(), version)
            .map_err(|source| DraftError::protocol(Section::Groups, "Group references", source))?;
        if !references.is_valid() {
            return Err(DraftError::consensus(
                Section::Groups,
                "Group references",
                references.errors,
            ));
        }
    }
    let validation=dash_sdk::dpp::data_contract::associated_token::token_configuration::validate_token_configurations(contract.tokens(),version);
    if !validation.is_valid() {
        return Err(DraftError::consensus(
            Section::Supply,
            "Token settings",
            validation.errors,
        ));
    }
    let value = DataContractInSerializationFormat::try_from_platform_versioned(&contract, version)
        .map_err(|source| DraftError::protocol(Section::Review, "Contract", source))?;
    DataContract::try_from_platform_versioned(value, true, &mut vec![], version)
        .map_err(|source| DraftError::protocol(Section::Documents, "Document types", source))
}

/// A prepared absolute allocation must still be in the future before signing.
pub fn validate_submission_time(contract: &DataContract, now_ms: u64) -> Result<(), DraftError> {
    use dash_sdk::dpp::data_contract::associated_token::{
        token_configuration::accessors::v0::TokenConfigurationV0Getters,
        token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters,
        token_pre_programmed_distribution::TokenPreProgrammedDistribution,
    };
    for token in contract.tokens().values() {
        if let Some(TokenPreProgrammedDistribution::V0(schedule)) =
            token.distribution_rules().pre_programmed_distribution()
        {
            if schedule.distributions.keys().any(|time| *time <= now_ms) {
                return Err(DraftError::new(
                    Section::Distribution,
                    "Scheduled allocation",
                    super::DraftIssue::ScheduleTime,
                ));
            }
        }
    }
    Ok(())
}
