//! Document-related Tauri IPC commands.
//!
//! Maps all 8 `DocumentTask` variants to Tauri commands. All document
//! operations are long-running and dispatched asynchronously via
//! `task_dispatcher::dispatch_task`. Results arrive as events.

use crate::commands::identity::parse_identifier;
use crate::dto::common::{CreditsDto, IdentifierDto};
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::document::DocumentTask;
use dash_evo_tool::backend_task::BackendTask;

use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV1Getters;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::tokens::gas_fees_paid_by::GasFeesPaidBy;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenEffect;
use dash_sdk::dpp::tokens::token_payment_info::v0::TokenPaymentInfoV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::platform::{DataContract, Document, DocumentQuery};

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs — serializable command parameters from the frontend
// ---------------------------------------------------------------------------

/// Token payment info for document operations that support token payments.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenPaymentInfoDto {
    /// Token ID (hex).
    pub token_id: IdentifierDto,
    /// Amount to pay in token credits.
    pub amount: CreditsDto,
}

/// Input for broadcasting (creating) a document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastDocumentInput {
    /// The document data as JSON.
    pub document_json: serde_json::Value,
    /// Optional token payment info.
    pub token_payment: Option<TokenPaymentInfoDto>,
    /// Document type name.
    pub document_type_name: String,
    /// Contract ID (hex) the document belongs to.
    pub contract_id: IdentifierDto,
    /// Identity ID (hex) that owns the document.
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
}

/// Input for deleting a document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDocumentInput {
    /// Document ID (hex) to delete.
    pub document_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Identity ID (hex) that owns the document.
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional token payment info.
    pub token_payment: Option<TokenPaymentInfoDto>,
}

/// Input for replacing a document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceDocumentInput {
    /// The updated document data as JSON.
    pub document_json: serde_json::Value,
    /// Document type name.
    pub document_type_name: String,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Identity ID (hex) that owns the document.
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional token payment info.
    pub token_payment: Option<TokenPaymentInfoDto>,
}

/// Input for transferring a document to a new owner.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferDocumentInput {
    /// Document ID (hex) to transfer.
    pub document_id: IdentifierDto,
    /// New owner identity ID (hex).
    pub new_owner_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Current owner identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional token payment info.
    pub token_payment: Option<TokenPaymentInfoDto>,
}

/// Input for purchasing a document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseDocumentInput {
    /// Price to pay in credits.
    pub price: CreditsDto,
    /// Document ID (hex) to purchase.
    pub document_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Buyer identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional token payment info.
    pub token_payment: Option<TokenPaymentInfoDto>,
}

/// Input for setting a price on a document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetDocumentPriceInput {
    /// Price in credits.
    pub price: CreditsDto,
    /// Document ID (hex).
    pub document_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Owner identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional token payment info.
    pub token_payment: Option<TokenPaymentInfoDto>,
}

/// A where clause for document queries.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WhereClauseDto {
    /// Field name.
    pub field: String,
    /// Operator (e.g., "==", "<", ">", ">=", "<=", "in", "startsWith").
    pub operator: String,
    /// Value as JSON.
    pub value: serde_json::Value,
}

/// An order-by clause for document queries.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OrderByClauseDto {
    /// Field name.
    pub field: String,
    /// Sort direction: "asc" or "desc".
    pub direction: String,
}

/// Input for fetching documents.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchDocumentsInput {
    /// Contract ID (hex) to query.
    pub contract_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Where clauses.
    pub where_clauses: Vec<WhereClauseDto>,
    /// Order-by clauses.
    pub order_by_clauses: Vec<OrderByClauseDto>,
    /// Max documents to return.
    pub limit: u32,
}

/// Input for fetching documents with pagination.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchDocumentsPageInput {
    /// Contract ID (hex) to query.
    pub contract_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Where clauses.
    pub where_clauses: Vec<WhereClauseDto>,
    /// Order-by clauses.
    pub order_by_clauses: Vec<OrderByClauseDto>,
    /// Optional cursor (hex bytes of the last document ID from previous page).
    pub start_after: Option<String>,
}

/// Input for fetching documents using a SQL-like query string.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchDocumentsSqlInput {
    /// Contract ID (hex) — used to look up the contract for query parsing.
    pub contract_id: IdentifierDto,
    /// SQL-like query text (e.g. `SELECT * FROM domain WHERE normalizedLabel = 'alice'`).
    pub sql_text: String,
    /// Optional cursor (hex bytes of the last document ID from previous page).
    pub start_after: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Look up a `QualifiedIdentity` from the local database by identifier.
fn lookup_identity(
    state: &AppState,
    identity_id: &str,
) -> Result<dash_evo_tool::model::qualified_identity::QualifiedIdentity, String> {
    let identifier = parse_identifier(identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_by_id(&identifier)
        .map_err(|e| format!("Database error loading identity: {e}"))?
        .ok_or_else(|| format!("Identity {} not found in local database", identity_id))
}

/// Look up a contract from the local database.
fn lookup_contract(state: &AppState, contract_id: &str) -> Result<Arc<DataContract>, String> {
    let identifier = parse_identifier(contract_id)?;
    let ctx = state.current_context();
    let qc = ctx
        .get_contract_by_id(&identifier)
        .map_err(|e| format!("Database error loading contract: {e}"))?
        .ok_or_else(|| format!("Contract {} not found in local database", contract_id))?;
    Ok(Arc::new(qc.contract.clone()))
}

/// Find a signing key from a QualifiedIdentity by key ID.
fn find_signing_key(
    qi: &dash_evo_tool::model::qualified_identity::QualifiedIdentity,
    key_id: u32,
) -> Result<dash_sdk::platform::IdentityPublicKey, String> {
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

    qi.identity
        .public_keys()
        .get(&(key_id as dash_sdk::dpp::identity::KeyID))
        .cloned()
        .ok_or_else(|| format!("Key ID {} not found on identity", key_id))
}

/// Find a document type by name from a data contract.
fn find_document_type(contract: &DataContract, type_name: &str) -> Result<DocumentType, String> {
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;

    contract
        .document_types()
        .get(type_name)
        .cloned()
        .ok_or_else(|| format!("Document type '{}' not found in contract", type_name))
}

/// Build `TokenPaymentInfo` from a document type's token cost, if any.
fn token_payment_from_cost(
    cost: Option<dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenCost>,
) -> Option<TokenPaymentInfo> {
    cost.map(|c| {
        TokenPaymentInfo::V0(TokenPaymentInfoV0 {
            payment_token_contract_id: c.contract_id,
            token_contract_position: c.token_contract_position,
            gas_fees_paid_by: c.gas_fees_paid_by,
            minimum_token_cost: None,
            maximum_token_cost: Some(c.token_amount),
        })
    })
}

/// Build a DocumentQuery from input parameters.
fn build_document_query(
    contract: Arc<DataContract>,
    document_type_name: &str,
    where_clauses: &[WhereClauseDto],
    order_by_clauses: &[OrderByClauseDto],
    limit: u32,
    start: Option<
        dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start,
    >,
) -> Result<DocumentQuery, String> {
    use dash_sdk::dpp::platform_value::Value;
    use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};

    let where_clauses_sdk: Vec<WhereClause> = where_clauses
        .iter()
        .map(|wc| {
            let operator = match wc.operator.as_str() {
                "==" | "=" => WhereOperator::Equal,
                "<" => WhereOperator::LessThan,
                "<=" => WhereOperator::LessThanOrEquals,
                ">" => WhereOperator::GreaterThan,
                ">=" => WhereOperator::GreaterThanOrEquals,
                "in" => WhereOperator::In,
                "startsWith" => WhereOperator::StartsWith,
                other => return Err(format!("Unknown where operator: {}", other)),
            };

            // Convert serde_json::Value to platform_value::Value
            let value: Value = wc.value.clone().into();

            Ok(WhereClause {
                field: wc.field.clone(),
                operator,
                value,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let order_by_sdk: Vec<OrderClause> = order_by_clauses
        .iter()
        .map(|ob| OrderClause {
            field: ob.field.clone(),
            ascending: ob.direction.to_lowercase() != "desc",
        })
        .collect();

    Ok(DocumentQuery {
        data_contract: contract,
        document_type_name: document_type_name.to_string(),
        where_clauses: where_clauses_sdk,
        order_by_clauses: order_by_sdk,
        limit,
        start,
    })
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Broadcast (create) a new document on Platform.
///
/// Dispatches `DocumentTask::BroadcastDocument`. Result arrives via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn document_broadcast(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: BroadcastDocumentInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;

    // Deserialize the document from JSON
    let document: Document = serde_json::from_value(input.document_json)
        .map_err(|e| format!("Invalid document JSON: {e}"))?;

    // Generate random entropy
    let entropy: [u8; 32] = rand::random();

    let token_payment = token_payment_from_cost(doc_type.document_creation_token_cost());

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::BroadcastDocument(
        document,
        token_payment,
        entropy,
        doc_type,
        contract,
        qi,
        signing_key,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Delete a document from Platform.
///
/// Dispatches `DocumentTask::DeleteDocument`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_delete(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: DeleteDocumentInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;
    let doc_id = parse_identifier(&input.document_id)?;
    let token_payment = token_payment_from_cost(doc_type.document_deletion_token_cost());

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::DeleteDocument(
        doc_id,
        doc_type,
        contract,
        qi,
        signing_key,
        token_payment,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Replace (update) a document on Platform.
///
/// Dispatches `DocumentTask::ReplaceDocument`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_replace(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: ReplaceDocumentInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;

    let document: Document = serde_json::from_value(input.document_json)
        .map_err(|e| format!("Invalid document JSON: {e}"))?;

    let token_payment = token_payment_from_cost(doc_type.document_replacement_token_cost());

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::ReplaceDocument(
        document,
        doc_type,
        contract,
        qi,
        signing_key,
        token_payment,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Transfer a document to a new owner.
///
/// Dispatches `DocumentTask::TransferDocument`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_transfer(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TransferDocumentInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;
    let doc_id = parse_identifier(&input.document_id)?;
    let new_owner_id = parse_identifier(&input.new_owner_id)?;
    let token_payment = token_payment_from_cost(doc_type.document_transfer_token_cost());

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::TransferDocument(
        doc_id,
        new_owner_id,
        doc_type,
        contract,
        qi,
        signing_key,
        token_payment,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Purchase a document.
///
/// Dispatches `DocumentTask::PurchaseDocument`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_purchase(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: PurchaseDocumentInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;
    let doc_id = parse_identifier(&input.document_id)?;
    let token_payment = token_payment_from_cost(doc_type.document_purchase_token_cost());

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::PurchaseDocument(
        input.price as Credits,
        doc_id,
        doc_type,
        contract,
        qi,
        signing_key,
        token_payment,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Set a price on a document.
///
/// Dispatches `DocumentTask::SetDocumentPrice`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_set_price(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SetDocumentPriceInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;
    let doc_id = parse_identifier(&input.document_id)?;
    let token_payment = token_payment_from_cost(doc_type.document_update_price_token_cost());

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::SetDocumentPrice(
        input.price as Credits,
        doc_id,
        doc_type,
        contract,
        qi,
        signing_key,
        token_payment,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch documents from Platform.
///
/// Dispatches `DocumentTask::FetchDocuments`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_fetch(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchDocumentsInput,
) -> Result<DispatchTaskResponse, String> {
    let contract = lookup_contract(&state, &input.contract_id)?;

    let query = build_document_query(
        contract,
        &input.document_type_name,
        &input.where_clauses,
        &input.order_by_clauses,
        input.limit,
        None,
    )?;

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocuments(query)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch documents from Platform with pagination.
///
/// Dispatches `DocumentTask::FetchDocumentsPage`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_fetch_page(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchDocumentsPageInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;

    let contract = lookup_contract(&state, &input.contract_id)?;

    let start = input
        .start_after
        .map(|id_str| {
            parse_identifier(&id_str).map(|id| Start::StartAfter(id.to_buffer().to_vec()))
        })
        .transpose()?;

    let query = build_document_query(
        contract,
        &input.document_type_name,
        &input.where_clauses,
        &input.order_by_clauses,
        100, // Page size
        start,
    )?;

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocumentsPage(query)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch documents from Platform using a SQL-like query string.
///
/// Parses the SQL text via `DriveDocumentQuery::from_sql_expr` and dispatches
/// `DocumentTask::FetchDocumentsPage`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn document_fetch_page_sql(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchDocumentsSqlInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
    use dash_sdk::platform::DriveDocumentQuery;

    let contract = lookup_contract(&state, &input.contract_id)?;

    let mut query: DocumentQuery = DriveDocumentQuery::from_sql_expr(
        &input.sql_text,
        &contract,
        None,
    )
    .map(Into::into)
    .map_err(|e| format!("Failed to parse query: {}", e))?;

    query.limit = 100;

    if let Some(id_str) = input.start_after {
        let id = parse_identifier(&id_str)?;
        query.start = Some(Start::StartAfter(id.to_buffer().to_vec()));
    }

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocumentsPage(query)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Token cost query (synchronous)
// ---------------------------------------------------------------------------

/// Output DTO for document type token cost information.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTypeTokenCostDto {
    /// The token amount required.
    pub token_amount: u64,
    /// Human-readable token name.
    pub token_name: String,
    /// What happens to the tokens: "transferred to the contract owner" or "burned".
    pub effect: String,
    /// Who pays gas fees: "you", "the contract owner", or the full PreferContractOwner string.
    pub gas_fees_paid_by: String,
    /// Token ID (hex) for the token used for payment.
    pub token_id: String,
}

/// Input DTO for querying document type token cost.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTypeTokenCostInput {
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Document type name.
    pub document_type_name: String,
    /// Action type: "create", "delete", "replace", "transfer", "purchase", or "setPrice".
    pub action_type: String,
}

/// Query token cost for a document type action (if any).
///
/// Returns `None` if the document type has no token cost for the given action.
#[tauri::command]
#[specta::specta]
pub fn document_type_token_cost(
    state: tauri::State<'_, Arc<AppState>>,
    input: DocumentTypeTokenCostInput,
) -> Result<Option<DocumentTypeTokenCostDto>, String> {
    let contract = lookup_contract(&state, &input.contract_id)?;
    let doc_type = find_document_type(&contract, &input.document_type_name)?;

    let cost = match input.action_type.as_str() {
        "create" => doc_type.document_creation_token_cost(),
        "delete" => doc_type.document_deletion_token_cost(),
        "replace" => doc_type.document_replacement_token_cost(),
        "transfer" => doc_type.document_transfer_token_cost(),
        "purchase" => doc_type.document_purchase_token_cost(),
        "setPrice" => doc_type.document_update_price_token_cost(),
        other => return Err(format!("Unknown action type: {}", other)),
    };

    let Some(token_cost) = cost else {
        return Ok(None);
    };

    // Resolve token name from the contract's token definitions
    let token_name = contract
        .tokens()
        .get(&token_cost.token_contract_position)
        .map(|t| {
            t.conventions()
                .singular_form_by_language_code_or_default("en")
                .to_string()
        })
        .unwrap_or_else(|| format!("Token {}", token_cost.token_contract_position));

    let effect = match token_cost.effect {
        DocumentActionTokenEffect::TransferTokenToContractOwner => {
            "transferred to the contract owner".to_string()
        }
        DocumentActionTokenEffect::BurnToken => "burned".to_string(),
    };

    let gas_fees_paid_by = match token_cost.gas_fees_paid_by {
        GasFeesPaidBy::DocumentOwner => "you".to_string(),
        GasFeesPaidBy::ContractOwner => "the contract owner".to_string(),
        GasFeesPaidBy::PreferContractOwner => {
            "the contract owner unless their balance is insufficient, in which case you pay"
                .to_string()
        }
    };

    // Use the token payment contract ID as the token ID
    let token_id = token_cost
        .contract_id
        .map(|id| hex::encode(id.to_buffer()))
        .unwrap_or_default();

    Ok(Some(DocumentTypeTokenCostDto {
        token_amount: token_cost.token_amount,
        token_name,
        effect,
        gas_fees_paid_by,
        token_id,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_document_input_serializes_with_camel_case() {
        let input = BroadcastDocumentInput {
            document_json: serde_json::json!({"label": "test"}),
            token_payment: None,
            document_type_name: "domain".into(),
            contract_id: "abc".into(),
            identity_id: "def".into(),
            key_id: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"documentJson\""));
        assert!(json.contains("\"documentTypeName\":\"domain\""));
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"identityId\":\"def\""));
        assert!(json.contains("\"keyId\":0"));
        assert!(json.contains("\"tokenPayment\":null"));
    }

    #[test]
    fn delete_document_input_serializes() {
        let input = DeleteDocumentInput {
            document_id: "doc1".into(),
            document_type_name: "profile".into(),
            contract_id: "abc".into(),
            identity_id: "def".into(),
            key_id: 1,
            token_payment: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"documentId\":\"doc1\""));
        assert!(json.contains("\"documentTypeName\":\"profile\""));
    }

    #[test]
    fn replace_document_input_serializes() {
        let input = ReplaceDocumentInput {
            document_json: serde_json::json!({"updated": true}),
            document_type_name: "domain".into(),
            contract_id: "abc".into(),
            identity_id: "def".into(),
            key_id: 0,
            token_payment: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"documentJson\""));
        assert!(json.contains("\"documentTypeName\":\"domain\""));
    }

    #[test]
    fn transfer_document_input_serializes() {
        let input = TransferDocumentInput {
            document_id: "doc1".into(),
            new_owner_id: "newowner".into(),
            document_type_name: "domain".into(),
            contract_id: "abc".into(),
            identity_id: "def".into(),
            key_id: 0,
            token_payment: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"documentId\":\"doc1\""));
        assert!(json.contains("\"newOwnerId\":\"newowner\""));
    }

    #[test]
    fn purchase_document_input_serializes() {
        let input = PurchaseDocumentInput {
            price: 100000,
            document_id: "doc1".into(),
            document_type_name: "domain".into(),
            contract_id: "abc".into(),
            identity_id: "def".into(),
            key_id: 0,
            token_payment: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"price\":100000"));
        assert!(json.contains("\"documentId\":\"doc1\""));
    }

    #[test]
    fn set_document_price_input_serializes() {
        let input = SetDocumentPriceInput {
            price: 50000,
            document_id: "doc1".into(),
            document_type_name: "domain".into(),
            contract_id: "abc".into(),
            identity_id: "def".into(),
            key_id: 0,
            token_payment: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"price\":50000"));
    }

    #[test]
    fn where_clause_dto_serializes() {
        let wc = WhereClauseDto {
            field: "normalizedLabel".into(),
            operator: "==".into(),
            value: serde_json::json!("alice"),
        };
        let json = serde_json::to_string(&wc).unwrap();
        assert!(json.contains("\"field\":\"normalizedLabel\""));
        assert!(json.contains("\"operator\":\"==\""));
    }

    #[test]
    fn order_by_clause_dto_serializes() {
        let ob = OrderByClauseDto {
            field: "createdAt".into(),
            direction: "desc".into(),
        };
        let json = serde_json::to_string(&ob).unwrap();
        assert!(json.contains("\"field\":\"createdAt\""));
        assert!(json.contains("\"direction\":\"desc\""));
    }

    #[test]
    fn fetch_documents_input_serializes() {
        let input = FetchDocumentsInput {
            contract_id: "abc".into(),
            document_type_name: "domain".into(),
            where_clauses: vec![],
            order_by_clauses: vec![],
            limit: 100,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"documentTypeName\":\"domain\""));
        assert!(json.contains("\"limit\":100"));
        assert!(json.contains("\"whereClauses\":[]"));
        assert!(json.contains("\"orderByClauses\":[]"));
    }

    #[test]
    fn fetch_documents_sql_input_serializes() {
        let input = FetchDocumentsSqlInput {
            contract_id: "abc".into(),
            sql_text: "SELECT * FROM domain WHERE normalizedLabel = 'alice'".into(),
            start_after: Some("deadbeef".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"sqlText\":"));
        assert!(json.contains("\"startAfter\":\"deadbeef\""));
    }

    #[test]
    fn fetch_documents_page_input_serializes() {
        let input = FetchDocumentsPageInput {
            contract_id: "abc".into(),
            document_type_name: "domain".into(),
            where_clauses: vec![],
            order_by_clauses: vec![],
            start_after: Some("deadbeef".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"startAfter\":\"deadbeef\""));
    }

    #[test]
    fn token_payment_info_dto_serializes() {
        let info = TokenPaymentInfoDto {
            token_id: "abc".into(),
            amount: 1000,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"tokenId\":\"abc\""));
        assert!(json.contains("\"amount\":1000"));
    }

    #[test]
    fn broadcast_document_input_roundtrip() {
        let json = r#"{"documentJson":{"label":"test"},"tokenPayment":null,"documentTypeName":"domain","contractId":"abc","identityId":"def","keyId":0}"#;
        let input: BroadcastDocumentInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.document_type_name, "domain");
        assert_eq!(input.contract_id, "abc");
        assert_eq!(input.identity_id, "def");
        assert_eq!(input.key_id, 0);
    }

    #[test]
    fn document_type_token_cost_dto_serializes() {
        let dto = DocumentTypeTokenCostDto {
            token_amount: 500,
            token_name: "DashToken".into(),
            effect: "burned".into(),
            gas_fees_paid_by: "you".into(),
            token_id: "abc123".into(),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"tokenAmount\":500"));
        assert!(json.contains("\"tokenName\":\"DashToken\""));
        assert!(json.contains("\"effect\":\"burned\""));
        assert!(json.contains("\"gasFeesPaidBy\":\"you\""));
        assert!(json.contains("\"tokenId\":\"abc123\""));
    }

    #[test]
    fn document_type_token_cost_input_serializes() {
        let input = DocumentTypeTokenCostInput {
            contract_id: "abc".into(),
            document_type_name: "domain".into(),
            action_type: "create".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"documentTypeName\":\"domain\""));
        assert!(json.contains("\"actionType\":\"create\""));
    }
}
