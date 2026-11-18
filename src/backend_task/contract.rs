use std::collections::BTreeMap;

use super::BackendTaskSuccessResult;
use crate::backend_task::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_contract::InsertTokensToo;
use crate::model::qualified_contract::InsertTokensToo::NoTokensShouldBeAdded;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::tokens::tokens_screen::{ContractDescriptionInfo, TokenInfo};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::group_action::GroupAction;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::group_actions::GroupActionsQuery;
use dash_sdk::platform::{
    DataContract, Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use dash_sdk::query_types::IndexMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ContractTask {
    FetchContracts(Vec<Identifier>),
    FetchContractsWithDescriptions(Vec<Identifier>),
    FetchActiveGroupActions(QualifiedContract, QualifiedIdentity),
    RemoveContract(Identifier),
    RegisterDataContract(DataContract, String, QualifiedIdentity, IdentityPublicKey), // contract, alias, identity, signing_key
    UpdateDataContract(DataContract, QualifiedIdentity, IdentityPublicKey), // contract, identity, signing_key
    SaveDataContract(DataContract, Option<String>, InsertTokensToo),
}

impl AppContext {
    pub async fn run_contract_task(
        &self,
        task: ContractTask,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        match task {
            ContractTask::FetchContracts(identifiers) => {
                match DataContract::fetch_many(sdk, identifiers).await {
                    Ok(data_contracts) => {
                        let mut results = vec![];
                        for data_contract in data_contracts {
                            if let Some(contract) = &data_contract.1 {
                                self.insert_contract_if_not_exists(
                                    contract,
                                    None,
                                    NoTokensShouldBeAdded,
                                )?;
                                results.push(Some(contract.clone()));
                            } else {
                                results.push(None);
                            }
                        }
                        Ok(BackendTaskSuccessResult::FetchedContracts(results))
                    }
                    Err(e) => Err(crate::backend_task::error::TaskError::from(e)),
                }
            }
            ContractTask::FetchContractsWithDescriptions(identifiers) => {
                // For each identifier, fetch the contract as in FetchContracts
                // and then if successful, fetch the contract description from the Search Contract's "fullDescription" document type
                match DataContract::fetch_many(sdk, identifiers).await {
                    Ok(data_contracts) => {
                        let mut results = BTreeMap::new();
                        for data_contract in data_contracts {
                            if let Some(contract) = &data_contract.1 {
                                // Fetch the contract description from the Search Contract
                                let search_contract = self.keyword_search_contract();
                                let document_query = DocumentQuery {
                                    integer_range_clauses: vec![],
                                    sub_queries: Vec::new(),
                                    select: SelectProjection::documents(),
                                    data_contract: search_contract.clone(),
                                    document_type_name: "fullDescription".to_string(),
                                    limit: 1,
                                    offset: None,
                                    start: None,
                                    where_clauses: vec![WhereClause {
                                        field: "contractId".to_string(),
                                        operator: WhereOperator::Equal,
                                        value: Value::Identifier(contract.id().into()),
                                    }],
                                    time_range_clauses: Vec::new(),
                                    group_by: Vec::new(),
                                    having: Vec::new(),
                                    order_by_clauses: vec![],
                                };
                                let document_option = Document::fetch(sdk, document_query)
                                    .await
                                    .map_err(crate::backend_task::error::TaskError::from)?;

                                let mut token_infos = vec![];
                                for token in contract.tokens() {
                                    let token_configuration = match contract
                                        .expected_token_configuration(*token.0)
                                    {
                                        Ok(config) => config,
                                        Err(e) => {
                                            tracing::warn!(
                                                "Skipping token at position {} in contract {}: {}",
                                                token.0,
                                                contract.id(),
                                                e
                                            );
                                            continue;
                                        }
                                    };
                                    let token_name = {
                                        let TokenConfigurationConvention::V0(conventions) =
                                            &token_configuration.conventions();
                                        conventions
                                            .singular_form_by_language_code_or_default("en")
                                            .to_string()
                                    };

                                    let token_info = TokenInfo {
                                        token_id: contract.token_id(*token.0).unwrap_or_default(),
                                        token_name,
                                        data_contract_id: contract.id(),
                                        token_position: *token.0,
                                        token_configuration: token_configuration.clone(),
                                        description: token.1.description().clone(),
                                    };

                                    token_infos.push(token_info);
                                }

                                let contract_description_info =
                                    document_option.map(|document| ContractDescriptionInfo {
                                        data_contract_id: contract.id(),
                                        description: document
                                            .get("description")
                                            .and_then(|v| v.as_text())
                                            .unwrap_or_default()
                                            .to_string(),
                                    });

                                results.insert(
                                    contract.id(),
                                    (contract_description_info, token_infos),
                                );
                            }
                        }
                        Ok(BackendTaskSuccessResult::ContractsWithDescriptions(results))
                    }
                    Err(e) => Err(crate::backend_task::error::TaskError::from(e)),
                }
            }
            ContractTask::FetchActiveGroupActions(contract, identity) => {
                let mut actions = IndexMap::new();

                let mut group_positions = vec![];
                for group in contract.contract.groups() {
                    if group.1.members().contains_key(&identity.identity.id()) {
                        group_positions.push(group.0);
                    }
                }

                for group_position in group_positions {
                    let query = GroupActionsQuery {
                        contract_id: contract.contract.id(),
                        group_contract_position: *group_position,
                        status: GroupActionStatus::ActionActive,
                        start_at_action_id: None,
                        limit: None,
                    };

                    let group_actions = GroupAction::fetch_many(sdk, query)
                        .await
                        .map_err(crate::backend_task::error::TaskError::from)?;

                    for group_action in group_actions {
                        if let Some(action) = &group_action.1 {
                            actions.insert(group_action.0, action.clone());
                        }
                    }
                }

                Ok(BackendTaskSuccessResult::ActiveGroupActions(actions))
            }
            ContractTask::RegisterDataContract(data_contract, alias, identity, signing_key) => {
                AppContext::register_data_contract(
                    self,
                    data_contract,
                    alias,
                    identity,
                    signing_key,
                    sdk,
                    sender,
                )
                .await
            }
            ContractTask::UpdateDataContract(mut data_contract, identity, signing_key) => {
                AppContext::update_data_contract(
                    self,
                    &mut data_contract,
                    identity,
                    signing_key,
                    sdk,
                    sender,
                )
                .await
            }
            ContractTask::RemoveContract(identifier) => self
                .remove_contract(&identifier)
                .map(|_| BackendTaskSuccessResult::RemovedContract),
            ContractTask::SaveDataContract(data_contract, alias, insert_tokens_too) => {
                self.insert_contract_if_not_exists(
                    &data_contract,
                    alias.as_deref(),
                    insert_tokens_too,
                )?;
                Ok(BackendTaskSuccessResult::SavedContract)
            }
        }
    }
}

/// Confirm the network rules before using a limited contract key. Nonce reads
/// may hit a cache; this read verifies fresh quorum-signed response metadata.
pub(super) async fn prepare_contract_signing_key(
    sdk: &Sdk,
    identity: &QualifiedIdentity,
    key: &IdentityPublicKey,
) -> Result<u64, super::error::TaskError> {
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;
    use dash_sdk::query_types::IdentityBalance;
    let limited = key.has_limits()
        || identity
            .identity
            .public_keys()
            .get(&key.id())
            .is_some_and(|key| key.has_limits());
    let block_time_ms = if limited
        && sdk
            .protocol_version_info()
            .verified_version
            .is_none_or(|version| version < 15)
    {
        let (_, metadata) =
            IdentityBalance::fetch_with_metadata(sdk, identity.identity.id(), None).await?;
        // The monotone cache alone does not prove which version this read saw.
        if metadata.protocol_version != 14 {
            return Err(
                if sdk
                    .protocol_version_info()
                    .verified_version
                    .is_some_and(|version| version >= 15)
                {
                    super::error::TaskError::ContractSigningKeyHasLimits
                } else {
                    super::error::TaskError::ContractSigningProtocolUnconfirmed
                },
            );
        }
        metadata.time_ms
    } else {
        // Only limited keys have an expiry; unlimited keys need no extra read.
        0
    };
    validate_contract_signing_key(
        identity,
        key,
        sdk.protocol_version_info().verified_version,
        block_time_ms,
    )?;
    Ok(block_time_ms)
}

/// Enforce the active network's contract signing rule, never an ingestion rule.
pub(super) fn validate_contract_signing_key(
    identity: &QualifiedIdentity,
    key: &IdentityPublicKey,
    protocol_version: Option<u32>,
    block_time_ms: u64,
) -> Result<(), super::error::TaskError> {
    use crate::model::identity_key_usability::{SigningScope, limits_permit};
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    let current_key = identity
        .identity
        .public_keys()
        .get(&key.id())
        .unwrap_or(key);
    let scope = SigningScope::DataContract { protocol_version };
    if !limits_permit(key, scope) || !limits_permit(current_key, scope) {
        if protocol_version.is_none() {
            return Err(super::error::TaskError::ContractSigningProtocolUnconfirmed);
        }
        return Err(super::error::TaskError::ContractSigningKeyHasLimits);
    }
    if [key, current_key].into_iter().any(|key| {
        crate::model::identity_key_usability::key_caveats(key, scope, block_time_ms)
            .iter()
            .any(crate::model::identity_key_usability::KeyCaveat::blocks_signing)
    }) {
        return Err(super::error::TaskError::ContractSigningKeyUnavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{auth_key, identity_holding};
    use dash_sdk::dpp::identity::SecurityLevel;

    #[tokio::test]
    async fn interop_unverified_balance_read_does_not_authorize_limited_contract_signing() {
        use dash_sdk::query_types::IdentityBalance;
        let key = auth_key(1, SecurityLevel::CRITICAL).with_limits(Some(200_000_000_000), None);
        let identity = identity_holding(std::slice::from_ref(&key));
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch::<IdentityBalance, _>(identity.identity.id(), Some(200_000_000_000))
            .await
            .unwrap();
        assert!(matches!(
            prepare_contract_signing_key(&sdk, &identity, &key).await,
            Err(super::super::error::TaskError::ContractSigningProtocolUnconfirmed)
        ));
    }

    #[test]
    fn interop_contract_expiry_uses_proved_block_time_including_the_boundary() {
        let key = auth_key(1, SecurityLevel::CRITICAL).with_limits(None, Some(1_000));
        let identity = identity_holding(std::slice::from_ref(&key));
        assert!(validate_contract_signing_key(&identity, &key, Some(14), 999).is_ok());
        assert!(matches!(
            validate_contract_signing_key(&identity, &key, Some(14), 1_000),
            Err(super::super::error::TaskError::ContractSigningKeyUnavailable)
        ));
        let old = auth_key(1, SecurityLevel::CRITICAL);
        assert!(matches!(
            validate_contract_signing_key(&identity, &old, Some(14), 1_000),
            Err(super::super::error::TaskError::ContractSigningKeyUnavailable)
        ));
    }

    #[test]
    fn interop_contract_preflight_uses_the_verified_protocol() {
        for (budget, expiry) in [(Some(200_000_000_000), None), (None, Some(u64::MAX))] {
            let key = auth_key(1, SecurityLevel::CRITICAL).with_limits(budget, expiry);
            let identity = identity_holding(std::slice::from_ref(&key));
            assert!(validate_contract_signing_key(&identity, &key, Some(14), 1).is_ok());
            assert!(matches!(
                validate_contract_signing_key(&identity, &key, Some(15), 1),
                Err(super::super::error::TaskError::ContractSigningKeyHasLimits)
            ));
            assert!(matches!(
                validate_contract_signing_key(&identity, &key, None, 1),
                Err(super::super::error::TaskError::ContractSigningProtocolUnconfirmed)
            ));
        }
    }

    #[test]
    fn pv15_contract_preflight_checks_current_key_limits() {
        let original = auth_key(1, SecurityLevel::CRITICAL);
        assert!(
            validate_contract_signing_key(
                &identity_holding(std::slice::from_ref(&original)),
                &original,
                Some(15),
                1
            )
            .is_ok()
        );
        for (budget, expiry) in [(Some(0), None), (Some(1_000), None), (None, Some(u64::MAX))] {
            let limited = original.clone().with_limits(budget, expiry);
            assert!(matches!(
                validate_contract_signing_key(
                    &identity_holding(std::slice::from_ref(&original)),
                    &limited,
                    Some(15),
                    1
                ),
                Err(super::super::error::TaskError::ContractSigningKeyHasLimits)
            ));
            let identity = identity_holding(&[limited]);
            assert!(matches!(
                validate_contract_signing_key(&identity, &original, Some(15), 1),
                Err(super::super::error::TaskError::ContractSigningKeyHasLimits)
            ));
        }
    }
}
