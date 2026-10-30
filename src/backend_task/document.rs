use crate::backend_task::error::TaskError;
use crate::backend_task::{
    BackendTaskSuccessResult, FeeResult, NETWORK_REQUEST_TIMEOUT,
    await_network_request_with_timeout,
};
use crate::context::AppContext;
use crate::model::fee_estimation::checked_action_fee_agreement;
use crate::model::index_only_document::{
    IndexOnlyDeleteValues, index_only_delete_supported, is_index_only, validate_index_only_delete,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::request_type::RequestType;
use dash_sdk::Error as SdkError;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::action_fees::agreement::DocumentActionFeeAgreement;
use dash_sdk::dpp::document::{DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::{
    data_contract::document_type::DocumentType,
    state_transition::batch_transition::batched_transition::document_transition_action_type::DocumentTransitionActionType,
};
use dash_sdk::drive::query::SelectProjection;
use dash_sdk::platform::documents::transitions::DocumentCreateResult;
use dash_sdk::platform::documents::transitions::DocumentCreateTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentDeleteResult;
use dash_sdk::platform::documents::transitions::DocumentDeleteTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentPurchaseResult;
use dash_sdk::platform::documents::transitions::DocumentPurchaseTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentReplaceResult;
use dash_sdk::platform::documents::transitions::DocumentReplaceTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentSetPriceResult;
use dash_sdk::platform::documents::transitions::DocumentSetPriceTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentTransferResult;
use dash_sdk::platform::documents::transitions::DocumentTransferTransitionBuilder;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{
    DataContract, Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use dash_sdk::query_types::IndexMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum DocumentTask {
    BroadcastDocument {
        document: Document,
        token_payment_info: Option<TokenPaymentInfo>,
        entropy: [u8; 32],
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    DeleteDocument {
        document_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    /// Deletes an entry of an indexOnly document type (protocol version 14),
    /// named by the values it was created with: such a type keeps no stored
    /// document to delete by id.
    DeleteIndexOnlyDocument {
        /// The id the fetched entry carries. Platform matches the entry by
        /// its values, not by this id.
        document_id: Identifier,
        values: IndexOnlyDeleteValues,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    ReplaceDocument {
        document: Document,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    TransferDocument {
        document_id: Identifier,
        new_owner_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    PurchaseDocument {
        price: Credits,
        document_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    SetDocumentPrice {
        price: Credits,
        document_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        /// The document action fee the user was shown and agreed to (protocol
        /// version 14). Required when the document type charges a fee for
        /// the action: the backend never agrees on the user's behalf.
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    },
    FetchDocuments(DocumentQuery),
    FetchDocumentsPage(DocumentQuery),
}

impl AppContext {
    /// Fetches a single document by id and bumps its revision, preparing it for
    /// a replace/transfer/purchase/set-price mutation.
    async fn fetch_document_for_mutation(
        &self,
        sdk: &Sdk,
        data_contract: Arc<DataContract>,
        document_type: &DocumentType,
        document_id: Identifier,
    ) -> Result<Document, TaskError> {
        let document_query = DocumentQuery {
            integer_range_clauses: vec![],
            sub_queries: Vec::new(),
            select: SelectProjection::documents(),
            data_contract,
            document_type_name: document_type.name().to_string(),
            where_clauses: vec![],
            time_range_clauses: Vec::new(),
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 1,
            offset: None,
            start: None,
        };
        let query_with_id = DocumentQuery::with_document_id(document_query, &document_id);
        let mut document = await_network_request_with_timeout(
            NETWORK_REQUEST_TIMEOUT,
            Document::fetch(sdk, query_with_id),
            |source| TaskError::DocumentFetchTimeout { source },
        )
        .await?
        .map_err(TaskError::from)?
        .ok_or(TaskError::DocumentNotFound)?;
        document.bump_revision();
        Ok(document)
    }

    /// Completes a delete `builder` (by id or by values) with the token
    /// payment, creation options and agreed action fee, then broadcasts it.
    #[allow(clippy::too_many_arguments)]
    async fn send_document_delete(
        &self,
        sdk: &Sdk,
        mut builder: DocumentDeleteTransitionBuilder,
        document_type: &DocumentType,
        qualified_identity: &QualifiedIdentity,
        identity_key: &IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
        action_fee_agreement: Option<DocumentActionFeeAgreement>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if let Some(token_payment) = token_payment_info {
            builder = builder.with_token_payment_info(token_payment);
        }

        let maybe_options = self.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }
        // After the options: setting them replaces the struct the
        // agreement is kept in.
        if let Some(agreement) = checked_action_fee_agreement(
            document_type.as_ref(),
            DocumentTransitionActionType::Delete,
            action_fee_agreement,
        )? {
            builder = builder.with_action_fee_agreement(agreement);
        }

        let result = sdk
            .document_delete(builder, identity_key, qualified_identity)
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

        // Handle the result - DocumentDeleteResult contains the deleted document ID
        let estimated_fee = self.fee_estimator().estimate_document_batch(1);
        let fee_result = FeeResult::estimated_only(estimated_fee);
        match result {
            DocumentDeleteResult::Deleted(deleted_id) => Ok(
                BackendTaskSuccessResult::DeletedDocument(deleted_id, fee_result),
            ),
        }
    }

    pub async fn run_document_task(
        &self,
        task: DocumentTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match self.run_document_task_inner(task, sdk).await {
            // The multiplier the refusal names is unproven node data: it only
            // triggers a proved epoch refresh, never feeds the cache itself.
            Err(refusal @ TaskError::DocumentActionFeeMultiplierRose { .. }) => {
                let refreshed = self.refresh_current_epoch(sdk).await.map(|_| ());
                Err(fee_refusal_after_refresh(refusal, refreshed))
            }
            result => result,
        }
    }

    async fn run_document_task_inner(
        &self,
        task: DocumentTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            DocumentTask::FetchDocuments(document_query) => await_network_request_with_timeout(
                NETWORK_REQUEST_TIMEOUT,
                Document::fetch_many(sdk, document_query),
                |source| TaskError::DocumentFetchTimeout { source },
            )
            .await?
            .map(BackendTaskSuccessResult::Documents)
            .map_err(TaskError::from),
            DocumentTask::FetchDocumentsPage(mut document_query) => {
                // Set the limit for each page
                document_query.limit = 100;

                // Initialize an empty IndexMap to accumulate documents for this page
                let mut page_docs: IndexMap<Identifier, Option<Document>> = IndexMap::new();

                // Fetch a single page
                let docs_batch_result = await_network_request_with_timeout(
                    NETWORK_REQUEST_TIMEOUT,
                    Document::fetch_many(sdk, document_query),
                    |source| TaskError::DocumentFetchTimeout { source },
                )
                .await?
                .map_err(TaskError::from)?;

                let batch_len = docs_batch_result.len();

                // Insert the batch into the page map
                for (id, doc_opt) in docs_batch_result {
                    page_docs.insert(id, doc_opt);
                }

                // Determine if there's a next page
                let has_next_page = batch_len == 100;

                // If there's a next page, set the 'start' parameter for the next cursor
                let next_cursor = if has_next_page {
                    page_docs.keys().last().cloned().map(|last_doc_id| {
                        let id_bytes = last_doc_id.to_buffer();
                        Start::StartAfter(id_bytes.to_vec())
                    })
                } else {
                    None
                };

                Ok(BackendTaskSuccessResult::PageDocuments(
                    page_docs,
                    next_cursor,
                ))
            }
            DocumentTask::BroadcastDocument {
                document,
                token_payment_info,
                entropy,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                action_fee_agreement,
            } => {
                let contest_fund = crate::model::fee_estimation::document_create_contest_fund_cap(
                    document_type.as_ref(),
                    &document,
                    sdk.version(),
                )
                .map_err(|error| dash_sdk::Error::Protocol(*error))?;
                let mut builder = DocumentCreateTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    entropy,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let mut maybe_options = self.state_transition_options();
                if let Some(cap) = contest_fund {
                    maybe_options
                        .get_or_insert_with(Default::default)
                        .contest_fund = Some(cap);
                }
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }
                // After the options: setting them replaces the struct the
                // agreement is kept in.
                if let Some(agreement) = checked_action_fee_agreement(
                    document_type.as_ref(),
                    DocumentTransitionActionType::Create,
                    action_fee_agreement,
                )? {
                    builder = builder.with_action_fee_agreement(agreement);
                }

                let result = sdk
                    .document_create(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentCreateResult contains the created document
                match result {
                    DocumentCreateResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::BroadcastedDocument(document))
                    }
                }
            }
            DocumentTask::DeleteDocument {
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
                action_fee_agreement,
            } => {
                refuse_index_only(&document_type)?;
                let builder = DocumentDeleteTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document_id,
                    qualified_identity.identity.id(),
                );
                self.send_document_delete(
                    sdk,
                    builder,
                    &document_type,
                    &qualified_identity,
                    &identity_key,
                    token_payment_info,
                    action_fee_agreement,
                )
                .await
            }
            DocumentTask::DeleteIndexOnlyDocument {
                document_id,
                values,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
                action_fee_agreement,
            } => {
                let builder = index_only_delete_builder(
                    data_contract,
                    &document_type,
                    document_id,
                    qualified_identity.identity.id(),
                    values,
                    sdk.version(),
                )?;
                self.send_document_delete(
                    sdk,
                    builder,
                    &document_type,
                    &qualified_identity,
                    &identity_key,
                    token_payment_info,
                    action_fee_agreement,
                )
                .await
            }
            DocumentTask::ReplaceDocument {
                document,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
                action_fee_agreement,
            } => {
                refuse_index_only(&document_type)?;
                let mut builder = DocumentReplaceTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }
                // After the options: setting them replaces the struct the
                // agreement is kept in.
                if let Some(agreement) = checked_action_fee_agreement(
                    document_type.as_ref(),
                    DocumentTransitionActionType::Replace,
                    action_fee_agreement,
                )? {
                    builder = builder.with_action_fee_agreement(agreement);
                }

                let result = sdk
                    .document_replace(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentReplaceResult contains the replaced document
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentReplaceResult::Document(document) => Ok(
                        BackendTaskSuccessResult::ReplacedDocument(document.id(), fee_result),
                    ),
                }
            }
            DocumentTask::TransferDocument {
                document_id,
                new_owner_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
                action_fee_agreement,
            } => {
                refuse_index_only(&document_type)?;
                let document = self
                    .fetch_document_for_mutation(
                        sdk,
                        data_contract.clone(),
                        &document_type,
                        document_id,
                    )
                    .await?;

                let mut builder = DocumentTransferTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    new_owner_id,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }
                // After the options: setting them replaces the struct the
                // agreement is kept in.
                if let Some(agreement) = checked_action_fee_agreement(
                    document_type.as_ref(),
                    DocumentTransitionActionType::Transfer,
                    action_fee_agreement,
                )? {
                    builder = builder.with_action_fee_agreement(agreement);
                }

                let result = sdk
                    .document_transfer(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentTransferResult contains the transferred document
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentTransferResult::Document(document) => Ok(
                        BackendTaskSuccessResult::TransferredDocument(document.id(), fee_result),
                    ),
                }
            }
            DocumentTask::PurchaseDocument {
                price,
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
                action_fee_agreement,
            } => {
                refuse_index_only(&document_type)?;
                let document = self
                    .fetch_document_for_mutation(
                        sdk,
                        data_contract.clone(),
                        &document_type,
                        document_id,
                    )
                    .await?;

                let mut builder = DocumentPurchaseTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    qualified_identity.identity.id(),
                    price,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }
                // After the options: setting them replaces the struct the
                // agreement is kept in.
                if let Some(agreement) = checked_action_fee_agreement(
                    document_type.as_ref(),
                    DocumentTransitionActionType::Purchase,
                    action_fee_agreement,
                )? {
                    builder = builder.with_action_fee_agreement(agreement);
                }

                let result = sdk
                    .document_purchase(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentPurchaseResult contains the purchased document
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentPurchaseResult::Document(document) => Ok(
                        BackendTaskSuccessResult::PurchasedDocument(document.id(), fee_result),
                    ),
                }
            }
            DocumentTask::SetDocumentPrice {
                price,
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
                action_fee_agreement,
            } => {
                refuse_index_only(&document_type)?;
                let document = self
                    .fetch_document_for_mutation(
                        sdk,
                        data_contract.clone(),
                        &document_type,
                        document_id,
                    )
                    .await?;

                let mut builder = DocumentSetPriceTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    price,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }
                // After the options: setting them replaces the struct the
                // agreement is kept in.
                if let Some(agreement) = checked_action_fee_agreement(
                    document_type.as_ref(),
                    DocumentTransitionActionType::UpdatePrice,
                    action_fee_agreement,
                )? {
                    builder = builder.with_action_fee_agreement(agreement);
                }

                let result = sdk
                    .document_set_price(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentSetPriceResult contains the document with updated price
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentSetPriceResult::Document(document) => Ok(
                        BackendTaskSuccessResult::SetDocumentPrice(document.id(), fee_result),
                    ),
                }
            }
        }
    }
}

/// Refuses an id-based action (delete by id, replace, transfer, purchase,
/// set price) on an indexOnly document type before anything is fetched or
/// signed: such a type keeps no stored document, and Platform would only
/// refuse the action after the fee is paid.
fn refuse_index_only(document_type: &DocumentType) -> Result<(), TaskError> {
    if is_index_only(document_type.as_ref()) {
        return Err(TaskError::IndexOnlyDocumentReadOnly);
    }
    Ok(())
}

/// The delete of an indexOnly entry, built from its values for
/// `platform_version`. Refused before anything is signed when the network
/// has no such transition yet or when the values would be refused.
fn index_only_delete_builder(
    data_contract: Arc<DataContract>,
    document_type: &DocumentType,
    document_id: Identifier,
    owner_id: Identifier,
    delete: IndexOnlyDeleteValues,
    platform_version: &PlatformVersion,
) -> Result<DocumentDeleteTransitionBuilder, TaskError> {
    if !index_only_delete_supported(platform_version) {
        return Err(TaskError::IndexOnlyDeleteNotSupported);
    }
    validate_index_only_delete(document_type.as_ref(), &delete, platform_version)?;
    DocumentDeleteTransitionBuilder::from_values(
        data_contract,
        document_type.name().to_string(),
        document_id,
        owner_id,
        delete.values,
        delete.created_at,
        platform_version,
    )
    .map_err(TaskError::from)
}

/// The error to report for a fee multiplier refusal once the proved epoch
/// refresh it triggered has finished: the refusal itself when the cache now
/// holds the network's multiplier, otherwise a variant saying the contract
/// fee could not be updated (the cache keeps its previous value).
fn fee_refusal_after_refresh(
    refusal: TaskError,
    refreshed: Result<(), Box<SdkError>>,
) -> TaskError {
    match (refusal, refreshed) {
        (refusal, Ok(())) => refusal,
        (
            TaskError::DocumentActionFeeMultiplierRose {
                increase_tolerance_percent,
                source_error,
                ..
            },
            Err(refresh_error),
        ) => {
            tracing::warn!(
                refusal = ?source_error,
                "Network refused a document action for a risen fee multiplier"
            );
            TaskError::DocumentActionFeeMultiplierNotRefreshed {
                increase_tolerance_percent,
                source_error: refresh_error,
            }
        }
        (other, Err(_)) => other,
    }
}

#[cfg(test)]
mod tests {
    fn fee_refusal() -> TaskError {
        TaskError::DocumentActionFeeMultiplierRose {
            known_fee_multiplier_permille: 1000,
            current_fee_multiplier_permille: 1500,
            increase_tolerance_percent: 20,
            source_error: Box::new(dash_sdk::Error::Generic("rose".to_string())),
        }
    }

    /// After a proved refresh the refusal is reported as is: the cache holds
    /// the network's multiplier, so trying again quotes the current fee.
    #[test]
    fn a_fee_refusal_stands_once_the_epoch_is_refreshed() {
        let error = fee_refusal_after_refresh(fee_refusal(), Ok(()));
        assert!(matches!(
            error,
            TaskError::DocumentActionFeeMultiplierRose { .. }
        ));
        assert!(error.to_string().contains("has been updated"));
    }

    /// When the proved refresh fails, the user is told the contract fee
    /// could not be updated rather than that it was.
    #[test]
    fn a_failed_epoch_refresh_says_the_fee_was_not_updated() {
        let error = fee_refusal_after_refresh(
            fee_refusal(),
            Err(Box::new(dash_sdk::Error::Generic("offline".to_string()))),
        );
        assert!(matches!(
            error,
            TaskError::DocumentActionFeeMultiplierNotRefreshed {
                increase_tolerance_percent: 20,
                ..
            }
        ));
        assert!(error.to_string().contains("could not be updated"));
    }

    use super::*;
    use crate::model::index_only_document::IndexOnlyDeleteError;
    use crate::model::index_only_document::test_fixtures::index_only_contract;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::platform_value::Value;
    use std::collections::BTreeMap;

    fn owned_type(contract: &DataContract, name: &str) -> DocumentType {
        contract
            .document_type_for_name(name)
            .expect("document type")
            .to_owned_document_type()
    }

    fn like_delete(post: [u8; 32]) -> IndexOnlyDeleteValues {
        IndexOnlyDeleteValues {
            values: BTreeMap::from([("postId".to_string(), Value::Identifier(post))]),
            created_at: None,
        }
    }

    fn pv(version: u32) -> &'static PlatformVersion {
        PlatformVersion::get(version).expect("a known protocol version")
    }

    /// Id-based actions are refused for an indexOnly type and allowed for a
    /// stored one.
    #[test]
    fn id_based_actions_are_refused_for_index_only_types() {
        let contract = index_only_contract();
        assert!(matches!(
            refuse_index_only(&owned_type(&contract, "like")),
            Err(TaskError::IndexOnlyDocumentReadOnly)
        ));
        refuse_index_only(&owned_type(&contract, "note")).expect("a stored type");
    }

    #[test]
    fn an_index_only_delete_is_refused_below_protocol_14() {
        let contract = index_only_contract();
        let error = index_only_delete_builder(
            Arc::new(contract.clone()),
            &owned_type(&contract, "like"),
            Identifier::new([1; 32]),
            Identifier::new([2; 32]),
            like_delete([7; 32]),
            pv(13),
        )
        .err()
        .expect("refused");
        assert!(matches!(error, TaskError::IndexOnlyDeleteNotSupported));
    }

    #[test]
    fn an_index_only_delete_with_a_missing_value_is_refused_before_signing() {
        let contract = index_only_contract();
        let error = index_only_delete_builder(
            Arc::new(contract.clone()),
            &owned_type(&contract, "like"),
            Identifier::new([1; 32]),
            Identifier::new([2; 32]),
            IndexOnlyDeleteValues::default(),
            pv(14),
        )
        .err()
        .expect("refused");
        assert!(matches!(
            error,
            TaskError::IndexOnlyDeleteRefused {
                source: IndexOnlyDeleteError::MissingValue { ref path }
            } if path == "postId"
        ));
    }

    /// The builder carries the entry's values, so the delete names it by
    /// them; the id and owner are the ones given.
    #[test]
    fn an_index_only_delete_is_built_from_the_values() {
        let contract = index_only_contract();
        let builder = index_only_delete_builder(
            Arc::new(contract.clone()),
            &owned_type(&contract, "like"),
            Identifier::new([1; 32]),
            Identifier::new([2; 32]),
            like_delete([7; 32]),
            pv(14),
        )
        .expect("a valid delete");
        let document = builder.document.expect("the delete carries the values");
        assert_eq!(document.id(), Identifier::new([1; 32]));
        assert_eq!(document.owner_id(), Identifier::new([2; 32]));
        assert_eq!(
            document
                .properties()
                .get("postId")
                .and_then(|value| value.to_identifier().ok()),
            Some(Identifier::new([7; 32]))
        );
    }

    /// A delete by id of an indexOnly entry is refused by the task itself,
    /// before any network request.
    #[tokio::test]
    async fn a_delete_by_id_of_an_index_only_entry_is_refused_by_the_task() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(tmp.path());
        let contract = index_only_contract();
        let identity =
            crate::context::test_support::bare_user_identity(Identifier::new([2; 32]), ctx.network);
        let task = DocumentTask::DeleteDocument {
            document_id: Identifier::new([1; 32]),
            document_type: owned_type(&contract, "like"),
            data_contract: Arc::new(contract),
            qualified_identity: identity,
            identity_key: IdentityPublicKey::random_ecdsa_high_level_authentication_key(
                1,
                Some(1),
                pv(14),
            )
            .expect("a key")
            .0,
            token_payment_info: None,
            action_fee_agreement: None,
        };
        let error = ctx
            .run_document_task(task, &ctx.sdk())
            .await
            .expect_err("refused");
        assert!(matches!(error, TaskError::IndexOnlyDocumentReadOnly));
    }

    #[tokio::test]
    async fn document_network_timeout_is_typed_and_actionable() {
        let error = crate::backend_task::await_network_request_with_timeout(
            std::time::Duration::from_millis(1),
            std::future::pending::<()>(),
            |source| TaskError::DocumentFetchTimeout { source },
        )
        .await
        .expect_err("a pending document request must time out");

        assert!(matches!(error, TaskError::DocumentFetchTimeout { .. }));
        assert!(error.to_string().contains("Check your connection"));
    }
}
