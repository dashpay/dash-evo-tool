use std::collections::BTreeMap;

use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::{context::AppContext, model::qualified_identity::DPNSNameInfo};
use bip39::rand::{Rng, SeedableRng, rngs::StdRng};
use dash_sdk::{
    Sdk,
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters, document_type::accessors::DocumentTypeV0Getters,
        },
        document::{DocumentV0, DocumentV0Getters},
        identity::accessors::IdentityGettersV0,
        platform_value::{Bytes32, Value},
        util::{hash::hash_double, strings::convert_to_homograph_safe_chars},
    },
    drive::query::{SelectProjection, WhereClause, WhereOperator},
    platform::Fetch,
    platform::{Document, DocumentQuery, FetchMany, transition::put_document::PutDocument},
};

use super::{BackendTaskSuccessResult, RegisterDpnsNameInput};

fn rebrand_dpns_domain_conflict(error: TaskError) -> TaskError {
    match error {
        TaskError::PlatformEntryConflict { source_error } => {
            TaskError::DpnsUsernameAlreadyTaken { source_error }
        }
        other => other,
    }
}

impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        sdk: &Sdk,
        input: RegisterDpnsNameInput,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut rng = StdRng::from_entropy();
        let dpns_contract = self.dpns_contract.clone();

        let mut qualified_identity = input.qualified_identity;

        let entropy = Bytes32::random_with_rng(&mut rng);
        let preorder_document_type = dpns_contract
            .document_type_for_name("preorder")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let domain_document_type = dpns_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;

        let preorder_id = Document::generate_document_id_v0(
            &dpns_contract.id(),
            &qualified_identity.identity.id(),
            preorder_document_type.name().as_str(),
            entropy.as_slice(),
        );
        let domain_id = Document::generate_document_id_v0(
            &dpns_contract.id(),
            &qualified_identity.identity.id(),
            domain_document_type.name().as_str(),
            entropy.as_slice(),
        );

        let salt: [u8; 32] = rng.r#gen();
        let mut salted_domain_buffer: Vec<u8> = vec![];
        salted_domain_buffer.extend(salt);
        salted_domain_buffer
            .extend((convert_to_homograph_safe_chars(&input.name_input) + ".dash").as_bytes());
        let salted_domain_hash = hash_double(salted_domain_buffer);

        let preorder_document = Document::V0(DocumentV0 {
            id: preorder_id,
            owner_id: qualified_identity.identity.id(),
            creator_id: None,
            properties: BTreeMap::from([(
                "saltedDomainHash".to_string(),
                salted_domain_hash.into(),
            )]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        });
        let domain_document = Document::V0(DocumentV0 {
            id: domain_id,
            owner_id: qualified_identity.identity.id(),
            creator_id: None,
            properties: BTreeMap::from([
                ("parentDomainName".to_string(), "dash".into()),
                ("normalizedParentDomainName".to_string(), "dash".into()),
                ("label".to_string(), input.name_input.clone().into()),
                (
                    "normalizedLabel".to_string(),
                    convert_to_homograph_safe_chars(&input.name_input).into(),
                ),
                ("preorderSalt".to_string(), salt.into()),
                (
                    "records".to_string(),
                    BTreeMap::from([(
                        "identity".to_string(),
                        Into::<dash_sdk::dpp::platform_value::Value>::into(
                            qualified_identity.identity.id(),
                        ),
                    )])
                    .into(),
                ),
                (
                    "subdomainRules".to_string(),
                    BTreeMap::from([(
                        "allowSubdomains".to_string(),
                        Into::<dash_sdk::dpp::platform_value::Value>::into(false),
                    )])
                    .into(),
                ),
            ]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        });

        let public_key = qualified_identity
            .document_signing_key(&preorder_document_type)
            .ok_or(TaskError::NoDocumentSigningKey)?;

        let fee_estimator = self.fee_estimator();
        let estimated_fee = fee_estimator.estimate_document_batch(2);

        let balance_before = qualified_identity.identity.balance();

        let _ = preorder_document
            .put_to_platform_and_wait_for_response(
                sdk,
                preorder_document_type.to_owned_document_type(),
                Some(entropy.0),
                public_key.clone(),
                None,
                &qualified_identity,
                None,
            )
            .await?;

        let _ = domain_document
            .put_to_platform_and_wait_for_response(
                sdk,
                domain_document_type.to_owned_document_type(),
                Some(entropy.0),
                public_key.clone(),
                None,
                &qualified_identity,
                None,
            )
            .await
            .map_err(|error| rebrand_dpns_domain_conflict(TaskError::from(error)))?;

        let dpns_names_document_query = DocumentQuery {
            select: SelectProjection::documents(),
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(qualified_identity.identity.id().into()),
            }],
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 100,
            start: None,
        };

        let owned_dpns_names = Document::fetch_many(sdk, dpns_names_document_query)
            .await
            .map(|document_map| {
                document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc
                                .get("label")
                                .map(|label| label.to_str().unwrap_or_default());
                            let acquired_at = doc
                                .created_at()
                                .into_iter()
                                .chain(doc.transferred_at())
                                .max();

                            match (name, acquired_at) {
                                (Some(name), Some(acquired_at)) => Some(DPNSNameInfo {
                                    name: name.to_string(),
                                    acquired_at,
                                }),
                                _ => None,
                            }
                        })
                    })
                    .collect::<Vec<DPNSNameInfo>>()
            })
            .map_err(|e| TaskError::DpnsFetchError {
                source: Box::new(e),
            })?;

        qualified_identity.dpns_names = owned_dpns_names;

        if qualified_identity.alias.is_none() {
            qualified_identity.alias = Some(format!("{}.dash", input.name_input));
        }

        let refreshed_identity = dash_sdk::platform::Identity::fetch_by_identifier(
            sdk,
            qualified_identity.identity.id(),
        )
        .await?
        .ok_or(TaskError::IdentityNotFound)?;

        let balance_after = refreshed_identity.balance();
        let actual_fee = balance_before.saturating_sub(balance_after);

        tracing::info!(
            "DPNS registration complete: estimated fee {} credits, actual fee {} credits",
            estimated_fee,
            actual_fee
        );
        if actual_fee != estimated_fee {
            tracing::warn!(
                "Fee mismatch: estimated {} vs actual {} (diff: {})",
                estimated_fee,
                actual_fee,
                actual_fee as i64 - estimated_fee as i64
            );
        }

        qualified_identity.identity = refreshed_identity;

        self.update_local_qualified_identity(&qualified_identity)?;

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::RegisteredDpnsName(fee_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::consensus::state::document::duplicate_unique_index_error::DuplicateUniqueIndexError;
    use dash_sdk::dpp::consensus::state::state_error::StateError;
    use dash_sdk::dpp::consensus::{
        ConsensusError, ConsensusError::StateError as ConsensusStateError,
    };
    use dash_sdk::platform::Identifier;

    fn duplicate_unique_index_conflict(properties: Vec<&str>) -> TaskError {
        let consensus = ConsensusError::from(DuplicateUniqueIndexError::new(
            Identifier::random(),
            properties.into_iter().map(str::to_string).collect(),
        ));
        let source_error = Box::new(dash_sdk::Error::StateTransitionBroadcastError(
            dash_sdk::error::StateTransitionBroadcastError {
                code: 40105,
                message: "duplicate unique index".to_string(),
                cause: Some(consensus),
            },
        ));

        TaskError::PlatformEntryConflict { source_error }
    }

    #[test]
    fn rebrand_dpns_domain_conflict_maps_platform_entry_conflict() {
        let error =
            duplicate_unique_index_conflict(vec!["normalizedParentDomainName", "normalizedLabel"]);

        let rebranded = rebrand_dpns_domain_conflict(error);

        assert_eq!(
            rebranded.to_string(),
            "This username is already taken. Please choose a different username and try again."
        );
        let TaskError::DpnsUsernameAlreadyTaken { source_error } = rebranded else {
            panic!("expected DpnsUsernameAlreadyTaken");
        };
        let dash_sdk::Error::StateTransitionBroadcastError(broadcast_error) = source_error.as_ref()
        else {
            panic!("expected StateTransitionBroadcastError source");
        };
        let Some(ConsensusStateError(StateError::DuplicateUniqueIndexError(error))) =
            broadcast_error.cause.as_ref()
        else {
            panic!("expected DuplicateUniqueIndexError cause");
        };
        assert_eq!(error.duplicating_properties().len(), 2);
    }

    #[test]
    fn rebrand_dpns_domain_conflict_passes_through_other_errors() {
        let error = rebrand_dpns_domain_conflict(TaskError::DataContractNotFound);

        assert!(matches!(error, TaskError::DataContractNotFound));
    }
}
