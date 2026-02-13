//! Token-related Tauri IPC commands.
//!
//! Maps all 22 `TokenTask` variants plus direct database methods to
//! Tauri commands. Long-running operations are dispatched asynchronously
//! via `task_dispatcher::dispatch_task` and results arrive as events.
//! Short reads (DB queries) return directly.

use crate::commands::identity::parse_identifier;
use crate::dto::common::{CreditsDto, IdentifierDto, TokenAmountDto};
use crate::dto::token::IdentityTokenIdentifierDto;
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::tokens::TokenTask;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::model::tokens::{
    get_available_token_actions_for_identity, IdentityTokenIdentifier,
};
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;

use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::platform::{DataContract, Identifier};

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs — serializable command parameters from the frontend
// ---------------------------------------------------------------------------

/// Input for querying a specific identity's token balance.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryIdentityTokenBalanceInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Token ID (hex).
    pub token_id: IdentifierDto,
}

/// Input for querying frozen identities.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryFrozenIdentitiesInput {
    /// Token ID (hex).
    pub token_id: IdentifierDto,
    /// Identity IDs (hex) to check.
    pub identity_ids: Vec<IdentifierDto>,
}

/// Input for querying token descriptions by keyword.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryDescriptionsByKeywordInput {
    /// Keyword to search for.
    pub keyword: String,
    /// Optional cursor for pagination (hex bytes).
    pub start_after: Option<String>,
}

/// Input for fetching a token by contract ID.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchTokenByContractIdInput {
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
}

/// Input for fetching a token by token ID.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchTokenByTokenIdInput {
    /// Token ID (hex).
    pub token_id: IdentifierDto,
}

/// Input for saving a token locally.
///
/// Instead of opaque JSON, accepts individual fields. The `TokenConfiguration`
/// is extracted server-side from the contract already in the local DB.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveTokenLocallyInput {
    /// Token ID (hex).
    pub token_id: IdentifierDto,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Token position within the contract.
    pub token_position: u16,
    /// Human-readable token name.
    pub token_name: String,
}

/// Input for querying token pricing.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryTokenPricingInput {
    /// Token ID (hex).
    pub token_id: IdentifierDto,
}

/// Shared input pattern for token operations that require identity, contract,
/// token position, and signing key.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenOperationInput {
    /// Identity ID (hex) performing the operation.
    pub identity_id: IdentifierDto,
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Token position within the contract.
    pub token_position: u16,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional public note.
    pub public_note: Option<String>,
}

/// Input for minting tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MintTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Amount to mint (string for u128).
    pub amount: TokenAmountDto,
    /// Optional recipient identity ID (hex). None = mint to self.
    pub recipient_id: Option<IdentifierDto>,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for transferring tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Recipient identity ID (hex).
    pub recipient_id: IdentifierDto,
    /// Amount to transfer (string for u128).
    pub amount: TokenAmountDto,
}

/// Input for burning tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BurnTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Amount to burn (string for u128).
    pub amount: TokenAmountDto,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for destroying frozen funds.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DestroyFrozenFundsInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Frozen identity ID (hex) whose funds to destroy.
    pub frozen_identity_id: IdentifierDto,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for freezing tokens (on a target identity).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FreezeTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Identity ID (hex) to freeze.
    pub freeze_identity_id: IdentifierDto,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for unfreezing tokens (on a target identity).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UnfreezeTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Identity ID (hex) to unfreeze.
    pub unfreeze_identity_id: IdentifierDto,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for pausing tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PauseTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for resuming tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResumeTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for claiming tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClaimTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Distribution type as a string (e.g., "Perpetual").
    pub distribution_type: String,
}

/// Input for estimating perpetual token rewards.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EstimatePerpetualRewardsInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Token ID (hex).
    pub token_id: IdentifierDto,
}

/// Input for updating token configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTokenConfigInput {
    /// Identity ID (hex) performing the update.
    pub identity_id: IdentifierDto,
    /// Contract ID (hex) containing the token.
    pub contract_id: IdentifierDto,
    /// Token ID (hex).
    pub token_id: IdentifierDto,
    /// Token alias / display name.
    pub token_alias: String,
    /// Token position within the contract.
    pub token_position: u16,
    /// The configuration change item as JSON.
    pub change_item_json: serde_json::Value,
    /// Key ID to use for signing.
    pub key_id: u32,
    /// Optional public note.
    pub public_note: Option<String>,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for purchasing tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseTokensInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Amount to purchase (string for u128).
    pub amount: TokenAmountDto,
    /// Total agreed price in credits.
    pub total_agreed_price: CreditsDto,
}

/// Input for setting direct purchase price.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetDirectPurchasePriceInput {
    /// Base operation fields.
    pub operation: TokenOperationInput,
    /// Pricing schedule as JSON (None to clear pricing).
    pub token_pricing_schedule: Option<serde_json::Value>,
    /// Optional group info as JSON.
    pub group_info: Option<serde_json::Value>,
}

/// Input for registering a token contract.
/// This is a complex operation with many parameters. The frontend sends
/// them as a structured JSON to avoid a function with 25+ parameters.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterTokenContractInput {
    /// Full token contract configuration as JSON.
    /// Must contain all fields needed by `TokenTask::RegisterTokenContract`.
    pub config_json: serde_json::Value,
    /// Identity ID (hex) that will own the contract.
    pub identity_id: IdentifierDto,
    /// Key ID for signing.
    pub key_id: u32,
}

/// Input for querying token claims from the token history contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryTokenClaimsInput {
    /// Token ID (hex).
    pub token_id: IdentifierDto,
    /// Recipient identity ID (hex).
    pub recipient_id: IdentifierDto,
}

/// Input for getting minting destination config for a token.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetMintingConfigInput {
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// Token position within the contract.
    pub token_position: u16,
}

/// Minting destination configuration for a token.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MintingConfigDto {
    /// Whether the minter can choose a custom recipient identity.
    pub allow_choosing_destination: bool,
    /// Default destination identity ID (hex), if one is configured.
    /// When set and allow_choosing_destination is true, this is the default recipient.
    /// When set and allow_choosing_destination is false, tokens always go to this identity.
    pub default_destination_identity_id: Option<IdentifierDto>,
}

/// Input for removing a token.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTokenInput {
    /// Token ID (hex) to remove.
    pub token_id: IdentifierDto,
}

/// Input for saving token ordering.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveTokenOrderInput {
    /// Ordered list of (identity_id, token_id) pairs.
    pub token_ids: Vec<IdentityTokenIdentifierDto>,
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn parse_token_amount(s: &str) -> Result<TokenAmount, String> {
    s.parse::<TokenAmount>()
        .map_err(|e| format!("Invalid token amount '{}': {}", s, e))
}

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

/// Parse optional GroupStateTransitionInfoStatus from JSON.
///
/// Expected JSON shapes:
///   Proposer: `{ "type": "proposer", "groupContractPosition": <u16> }`
///   Other signer: `{ "type": "otherSigner", "groupContractPosition": <u16>,
///                    "actionId": "<hex>", "actionIsProposer": <bool> }`
fn parse_group_info(
    json: Option<&serde_json::Value>,
) -> Result<Option<dash_sdk::dpp::group::GroupStateTransitionInfoStatus>, String> {
    use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};

    let json = match json {
        Some(v) if !v.is_null() => v,
        _ => return Ok(None),
    };

    let obj = json
        .as_object()
        .ok_or_else(|| "group_info must be a JSON object".to_string())?;

    let type_str = obj
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "group_info missing 'type' field".to_string())?;

    let group_contract_position = obj
        .get("groupContractPosition")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "group_info missing 'groupContractPosition' field".to_string())?
        as u16;

    match type_str {
        "proposer" => Ok(Some(
            GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(
                group_contract_position,
            ),
        )),
        "otherSigner" => {
            let action_id_hex = obj
                .get("actionId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "otherSigner missing 'actionId' field".to_string())?;
            let action_id = parse_identifier(action_id_hex)?;

            let action_is_proposer = obj
                .get("actionIsProposer")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            Ok(Some(
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                    GroupStateTransitionInfo {
                        group_contract_position,
                        action_id,
                        action_is_proposer,
                    },
                ),
            ))
        }
        other => Err(format!(
            "Unknown group_info type '{}'. Expected 'proposer' or 'otherSigner'.",
            other
        )),
    }
}

/// Parse a Start cursor from optional hex.
fn parse_start_cursor(
    hex: Option<&str>,
) -> Result<
    Option<dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start>,
    String,
> {
    use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
    match hex {
        Some(h) => {
            let bytes = hex::decode(h).map_err(|e| format!("Invalid start_after hex: {e}"))?;
            Ok(Some(Start::StartAfter(bytes)))
        }
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Query all token balances for loaded identities.
///
/// Dispatches `TokenTask::QueryMyTokenBalances`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_query_my_balances(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Query a specific identity's balance for a specific token.
///
/// Dispatches `TokenTask::QueryIdentityTokenBalance`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_query_identity_balance(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: QueryIdentityTokenBalanceInput,
) -> Result<DispatchTaskResponse, String> {
    let identity_id = parse_identifier(&input.identity_id)?;
    let token_id = parse_identifier(&input.token_id)?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(
        IdentityTokenIdentifier {
            identity_id,
            token_id,
        },
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Query which identities are frozen for a given token.
///
/// Dispatches `TokenTask::QueryFrozenIdentities`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_query_frozen_identities(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: QueryFrozenIdentitiesInput,
) -> Result<DispatchTaskResponse, String> {
    let token_id = parse_identifier(&input.token_id)?;
    let identity_ids: Vec<Identifier> = input
        .identity_ids
        .iter()
        .map(|s| parse_identifier(s))
        .collect::<Result<Vec<_>, _>>()?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryFrozenIdentities {
        token_id,
        identity_ids,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Query token descriptions by keyword.
///
/// Dispatches `TokenTask::QueryDescriptionsByKeyword`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_query_descriptions_by_keyword(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: QueryDescriptionsByKeywordInput,
) -> Result<DispatchTaskResponse, String> {
    let start = parse_start_cursor(input.start_after.as_deref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryDescriptionsByKeyword(
        input.keyword,
        start,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch a token by its contract ID.
///
/// Dispatches `TokenTask::FetchTokenByContractId`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_fetch_by_contract_id(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchTokenByContractIdInput,
) -> Result<DispatchTaskResponse, String> {
    let contract_id = parse_identifier(&input.contract_id)?;
    let task = BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByContractId(contract_id)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch a token by its token ID.
///
/// Dispatches `TokenTask::FetchTokenByTokenId`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_fetch_by_token_id(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchTokenByTokenIdInput,
) -> Result<DispatchTaskResponse, String> {
    let token_id = parse_identifier(&input.token_id)?;
    let task = BackendTask::TokenTask(Box::new(TokenTask::FetchTokenByTokenId(token_id)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Save a token to the local database.
///
/// Synchronous — reconstructs `TokenConfiguration` from the contract
/// already stored in the local DB, then inserts the token record.
#[tauri::command]
#[specta::specta]
pub fn token_save_locally(
    state: tauri::State<'_, Arc<AppState>>,
    input: SaveTokenLocallyInput,
) -> Result<(), String> {
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;

    let token_id = parse_identifier(&input.token_id)?;
    let contract_id = parse_identifier(&input.contract_id)?;
    let token_position = TokenContractPosition::from(input.token_position);

    let ctx = state.current_context();

    // Look up the contract — it must already be saved locally
    let qc = ctx
        .get_contract_by_id(&contract_id)
        .map_err(|e| format!("Database error loading contract: {e}"))?
        .ok_or_else(|| {
            format!(
                "Contract {} not found in local database. Save the contract first.",
                input.contract_id
            )
        })?;

    // Extract the token configuration at the specified position
    let token_config = qc
        .contract
        .tokens()
        .get(&token_position)
        .ok_or_else(|| {
            format!(
                "Token at position {} not found in contract {}",
                input.token_position, input.contract_id
            )
        })?
        .clone();

    // Insert into the local DB
    ctx.insert_token(
        &token_id,
        &input.token_name,
        token_config,
        &contract_id,
        input.token_position,
    )
    .map_err(|e| format!("Failed to save token: {e}"))
}

/// Query token pricing.
///
/// Dispatches `TokenTask::QueryTokenPricing`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_query_pricing(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: QueryTokenPricingInput,
) -> Result<DispatchTaskResponse, String> {
    let token_id = parse_identifier(&input.token_id)?;
    let task = BackendTask::TokenTask(Box::new(TokenTask::QueryTokenPricing(token_id)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Mint tokens.
///
/// Dispatches `TokenTask::MintTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_mint(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: MintTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let amount = parse_token_amount(&input.amount)?;
    let recipient_id = input
        .recipient_id
        .as_deref()
        .map(parse_identifier)
        .transpose()?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::MintTokens {
        sending_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        amount,
        recipient_id,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Transfer tokens to another identity.
///
/// Dispatches `TokenTask::TransferTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_transfer(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TransferTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let amount = parse_token_amount(&input.amount)?;
    let recipient_id = parse_identifier(&input.recipient_id)?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::TransferTokens {
        sending_identity: qi,
        recipient_id,
        amount,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Burn tokens.
///
/// Dispatches `TokenTask::BurnTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_burn(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: BurnTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let amount = parse_token_amount(&input.amount)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::BurnTokens {
        owner_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        amount,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Destroy frozen funds of a target identity.
///
/// Dispatches `TokenTask::DestroyFrozenFunds`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_destroy_frozen_funds(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: DestroyFrozenFundsInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let frozen_identity = parse_identifier(&input.frozen_identity_id)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::DestroyFrozenFunds {
        actor_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        frozen_identity,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Freeze tokens for a target identity.
///
/// Dispatches `TokenTask::FreezeTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_freeze(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FreezeTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let freeze_identity = parse_identifier(&input.freeze_identity_id)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::FreezeTokens {
        actor_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        freeze_identity,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Unfreeze tokens for a target identity.
///
/// Dispatches `TokenTask::UnfreezeTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_unfreeze(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: UnfreezeTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let unfreeze_identity = parse_identifier(&input.unfreeze_identity_id)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::UnfreezeTokens {
        actor_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        unfreeze_identity,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Pause a token.
///
/// Dispatches `TokenTask::PauseTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_pause(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: PauseTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::PauseTokens {
        actor_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Resume a paused token.
///
/// Dispatches `TokenTask::ResumeTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_resume(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: ResumeTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::ResumeTokens {
        actor_identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        public_note: input.operation.public_note,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Claim tokens from a distribution.
///
/// Dispatches `TokenTask::ClaimTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_claim(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: ClaimTokensInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;

    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;

    let distribution_type = match input.distribution_type.to_lowercase().as_str() {
        "perpetual" => TokenDistributionType::Perpetual,
        "pre_programmed" | "preprogrammed" => TokenDistributionType::PreProgrammed,
        other => return Err(format!("Unknown distribution type: {}", other)),
    };

    let task = BackendTask::TokenTask(Box::new(TokenTask::ClaimTokens {
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        actor_identity: qi,
        distribution_type,
        signing_key,
        public_note: input.operation.public_note,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Estimate perpetual token rewards.
///
/// Dispatches `TokenTask::EstimatePerpetualTokenRewardsWithExplanation`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_estimate_perpetual_rewards(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: EstimatePerpetualRewardsInput,
) -> Result<DispatchTaskResponse, String> {
    let identity_id = parse_identifier(&input.identity_id)?;
    let token_id = parse_identifier(&input.token_id)?;

    let task = BackendTask::TokenTask(Box::new(
        TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
            identity_id,
            token_id,
        },
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Query token claims from the token history contract.
///
/// Uses the system token history contract to fetch "claim" documents
/// filtered by token ID and recipient identity ID. Result via event
/// (arrives as a Document result type).
#[tauri::command]
#[specta::specta]
pub fn token_query_claims(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: QueryTokenClaimsInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_evo_tool::backend_task::document::DocumentTask;
    use dash_sdk::dpp::platform_value::Value;
    use dash_sdk::drive::query::{WhereClause, WhereOperator};

    let token_id = parse_identifier(&input.token_id)?;
    let recipient_id = parse_identifier(&input.recipient_id)?;

    let ctx = state.current_context();
    let token_history_contract = ctx.token_history_contract();

    let query = dash_sdk::platform::DocumentQuery {
        data_contract: token_history_contract,
        document_type_name: "claim".to_string(),
        where_clauses: vec![
            WhereClause {
                field: "tokenId".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(token_id.into()),
            },
            WhereClause {
                field: "recipientId".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(recipient_id.into()),
            },
        ],
        order_by_clauses: vec![],
        limit: 0,
        start: None,
    };

    let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocuments(query)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Update token configuration.
///
/// Dispatches `TokenTask::UpdateTokenConfig`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_update_config(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: UpdateTokenConfigInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;

    let token_id = parse_identifier(&input.token_id)?;
    let contract_id = parse_identifier(&input.contract_id)?;
    let token_position = TokenContractPosition::from(input.token_position);

    // Look up identity and contract from app state
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;
    let ctx = state.current_context();
    let qc = ctx
        .get_contract_by_id(&contract_id)
        .map_err(|e| format!("Database error loading contract: {e}"))?
        .ok_or_else(|| format!("Contract {} not found in local database", contract_id))?;

    // Get the token configuration from the contract
    let token_config = qc
        .contract
        .tokens()
        .get(&token_position)
        .ok_or_else(|| {
            format!(
                "Token at position {} not found in contract {}",
                input.token_position, contract_id
            )
        })?
        .clone();

    // Build IdentityTokenInfo
    let identity_token_info = dash_evo_tool::model::tokens::IdentityTokenInfo {
        token_id,
        token_alias: input.token_alias,
        identity: qi,
        data_contract: qc,
        token_config,
        token_position,
    };

    // Deserialize the change item from JSON
    let change_item: TokenConfigurationChangeItem = serde_json::from_value(input.change_item_json)
        .map_err(|e| format!("Invalid change_item_json: {e}"))?;

    let group_info = parse_group_info(input.group_info.as_ref())?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::UpdateTokenConfig {
        identity_token_info: Box::new(identity_token_info),
        change_item,
        signing_key,
        public_note: input.public_note,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Purchase tokens via direct purchase.
///
/// Dispatches `TokenTask::PurchaseTokens`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_purchase(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: PurchaseTokensInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let amount = parse_token_amount(&input.amount)?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::PurchaseTokens {
        identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        amount,
        total_agreed_price: input.total_agreed_price as Credits,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Set the direct purchase price for a token.
///
/// Dispatches `TokenTask::SetDirectPurchasePrice`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_set_direct_purchase_price(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SetDirectPurchasePriceInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.operation.identity_id)?;
    let signing_key = find_signing_key(&qi, input.operation.key_id)?;
    let contract = lookup_contract(&state, &input.operation.contract_id)?;
    let group_info = parse_group_info(input.group_info.as_ref())?;

    let pricing_schedule = input
        .token_pricing_schedule
        .map(|json| {
            serde_json::from_value(json).map_err(|e| format!("Invalid pricing schedule JSON: {e}"))
        })
        .transpose()?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::SetDirectPurchasePrice {
        identity: qi,
        data_contract: contract,
        token_position: TokenContractPosition::from(input.operation.token_position),
        signing_key,
        token_pricing_schedule: pricing_schedule,
        public_note: input.operation.public_note,
        group_info,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Register a new token contract.
///
/// This is a complex operation. The frontend sends the full configuration
/// as JSON, and this command deserializes and constructs the RegisterTokenContract task.
///
/// Dispatches `TokenTask::RegisterTokenContract`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn token_register_contract(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RegisterTokenContractInput,
) -> Result<DispatchTaskResponse, String> {
    // The config_json must be the full RegisterTokenContract struct in JSON form
    // except for identity and signing_key which we resolve here.
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;

    // Deserialize the config JSON into a serde_json::Value and extract fields
    let config = &input.config_json;

    // Extract basic fields
    let token_names: Vec<(String, String, String)> = serde_json::from_value(
        config
            .get("tokenNames")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![])),
    )
    .map_err(|e| format!("Invalid tokenNames: {e}"))?;

    let contract_keywords: Vec<String> = serde_json::from_value(
        config
            .get("contractKeywords")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![])),
    )
    .map_err(|e| format!("Invalid contractKeywords: {e}"))?;

    let token_description: Option<String> = config
        .get("tokenDescription")
        .and_then(|v| v.as_str())
        .map(String::from);

    let should_capitalize = config
        .get("shouldCapitalize")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let decimals = config.get("decimals").and_then(|v| v.as_u64()).unwrap_or(8) as u8;

    let base_supply: TokenAmount = config
        .get("baseSupply")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .parse()
        .map_err(|e| format!("Invalid baseSupply: {e}"))?;

    let max_supply: Option<TokenAmount> = config
        .get("maxSupply")
        .and_then(|v| v.as_str())
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| format!("Invalid maxSupply: {e}"))?;

    let start_paused = config
        .get("startPaused")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let allow_transfers_to_frozen_identities = config
        .get("allowTransfersToFrozenIdentities")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Complex fields deserialized from JSON
    let keeps_history: dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::TokenKeepsHistoryRules =
        serde_json::from_value(
            config.get("keepsHistory").cloned().unwrap_or(serde_json::json!("Never")),
        ).map_err(|e| format!("Invalid keepsHistory: {e}"))?;

    let main_control_group: Option<dash_sdk::dpp::data_contract::GroupContractPosition> = config
        .get("mainControlGroup")
        .and_then(|v| v.as_u64())
        .map(|v| v as u16);

    let manual_minting_rules = serde_json::from_value(
        config
            .get("manualMintingRules")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| format!("Invalid manualMintingRules: {e}"))?;

    let manual_burning_rules = serde_json::from_value(
        config
            .get("manualBurningRules")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| format!("Invalid manualBurningRules: {e}"))?;

    let freeze_rules =
        serde_json::from_value(config.get("freezeRules").cloned().unwrap_or_default())
            .map_err(|e| format!("Invalid freezeRules: {e}"))?;

    let unfreeze_rules =
        serde_json::from_value(config.get("unfreezeRules").cloned().unwrap_or_default())
            .map_err(|e| format!("Invalid unfreezeRules: {e}"))?;

    let destroy_frozen_funds_rules = serde_json::from_value(
        config
            .get("destroyFrozenFundsRules")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| format!("Invalid destroyFrozenFundsRules: {e}"))?;

    let emergency_action_rules = serde_json::from_value(
        config
            .get("emergencyActionRules")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| format!("Invalid emergencyActionRules: {e}"))?;

    let max_supply_change_rules = serde_json::from_value(
        config
            .get("maxSupplyChangeRules")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| format!("Invalid maxSupplyChangeRules: {e}"))?;

    let conventions_change_rules = serde_json::from_value(
        config
            .get("conventionsChangeRules")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| format!("Invalid conventionsChangeRules: {e}"))?;

    let main_control_group_change_authorized = serde_json::from_value(
        config
            .get("mainControlGroupChangeAuthorized")
            .cloned()
            .unwrap_or(serde_json::json!("NoOne")),
    )
    .map_err(|e| format!("Invalid mainControlGroupChangeAuthorized: {e}"))?;

    let distribution_rules =
        serde_json::from_value(config.get("distributionRules").cloned().unwrap_or_default())
            .map_err(|e| format!("Invalid distributionRules: {e}"))?;

    let groups = serde_json::from_value(
        config
            .get("groups")
            .cloned()
            .unwrap_or(serde_json::json!({})),
    )
    .map_err(|e| format!("Invalid groups: {e}"))?;

    let document_schemas: Option<std::collections::BTreeMap<String, serde_json::Value>> = config
        .get("documentSchemas")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| format!("Invalid documentSchemas: {e}"))?;

    let marketplace_trade_mode = config
        .get("marketplaceTradeMode")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;

    let marketplace_rules =
        serde_json::from_value(config.get("marketplaceRules").cloned().unwrap_or_default())
            .map_err(|e| format!("Invalid marketplaceRules: {e}"))?;

    let task = BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract {
        identity: qi,
        signing_key: Box::new(signing_key),
        token_names,
        contract_keywords,
        token_description,
        should_capitalize,
        decimals,
        base_supply,
        max_supply,
        start_paused,
        allow_transfers_to_frozen_identities,
        keeps_history,
        main_control_group,
        manual_minting_rules,
        manual_burning_rules,
        freeze_rules,
        unfreeze_rules: Box::new(unfreeze_rules),
        destroy_frozen_funds_rules: Box::new(destroy_frozen_funds_rules),
        emergency_action_rules: Box::new(emergency_action_rules),
        max_supply_change_rules: Box::new(max_supply_change_rules),
        conventions_change_rules: Box::new(conventions_change_rules),
        main_control_group_change_authorized,
        distribution_rules,
        groups,
        document_schemas,
        marketplace_trade_mode,
        marketplace_rules,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Direct database commands (synchronous, return immediately)
// ---------------------------------------------------------------------------

/// Read all "my token balances" from the local database.
///
/// This is a synchronous read — no network calls. The backend task
/// `QueryMyTokenBalances` writes balances to the DB; this command reads them back.
#[tauri::command]
#[specta::specta]
pub fn token_get_my_balances(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<crate::dto::token::IdentityTokenBalanceDto>, String> {
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;

    let ctx = state.current_context();
    let balances = ctx
        .identity_token_balances()
        .map_err(|e| format!("Failed to read token balances: {e}"))?;

    let in_dev_mode = ctx.is_developer_mode();

    Ok(balances
        .into_values()
        .map(|b| {
            let decimals = b.token_config.conventions().decimals();

            // Compute real permissions by looking up the identity and contract.
            // If either lookup fails, gracefully degrade to all-false.
            let actions = (|| {
                let identity = ctx.get_identity_by_id(&b.identity_id).ok()??;
                let qualified_contract = ctx.get_contract_by_id(&b.data_contract_id).ok()??;
                Some(get_available_token_actions_for_identity(
                    Some(b.balance),
                    &identity,
                    &b.token_config,
                    &qualified_contract.contract,
                    in_dev_mode,
                    None,
                ))
            })();

            let available_actions = match actions {
                Some(a) => crate::dto::token::IdentityTokenAvailableActionsDto {
                    can_claim: a.can_claim,
                    can_estimate: a.can_estimate,
                    can_mint: a.can_mint,
                    can_burn: a.can_burn,
                    can_freeze: a.can_freeze,
                    can_unfreeze: a.can_unfreeze,
                    can_destroy: a.can_destroy,
                    can_do_emergency_action: a.can_do_emergency_action,
                    can_maybe_purchase: a.can_maybe_purchase,
                    can_set_price: a.can_set_price,
                    can_transfer: a.can_transfer,
                    can_update_config: a.can_update_config,
                },
                None => crate::dto::token::IdentityTokenAvailableActionsDto {
                    can_claim: false,
                    can_estimate: false,
                    can_mint: false,
                    can_burn: false,
                    can_freeze: false,
                    can_unfreeze: false,
                    can_destroy: false,
                    can_do_emergency_action: false,
                    can_maybe_purchase: false,
                    can_set_price: false,
                    can_transfer: false,
                    can_update_config: false,
                },
            };

            crate::dto::token::IdentityTokenBalanceDto {
                token_id: hex::encode(b.token_id.to_buffer()),
                token_alias: b.token_alias,
                identity_id: hex::encode(b.identity_id.to_buffer()),
                balance: b.balance.to_string(),
                estimated_unclaimed_rewards: b.estimated_unclaimed_rewards.map(|r| r.to_string()),
                data_contract_id: hex::encode(b.data_contract_id.to_buffer()),
                token_position: b.token_position,
                decimals,
                available_actions,
            }
        })
        .collect())
}

/// Remove a token from the local database.
#[tauri::command]
#[specta::specta]
pub fn token_remove(
    state: tauri::State<'_, Arc<AppState>>,
    input: RemoveTokenInput,
) -> Result<(), String> {
    let token_id = parse_identifier(&input.token_id)?;
    let ctx = state.current_context();
    ctx.remove_token(&token_id)
        .map_err(|e| format!("Failed to remove token: {e}"))
}

/// Load the custom token ordering from the database.
#[tauri::command]
#[specta::specta]
pub fn token_load_order(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<IdentityTokenIdentifierDto>, String> {
    let db = state.db();
    let order = db
        .load_token_order()
        .map_err(|e| format!("Failed to load token order: {e}"))?;

    Ok(order
        .iter()
        .map(|(identity_id, token_id)| IdentityTokenIdentifierDto {
            identity_id: hex::encode(identity_id.to_vec()),
            token_id: hex::encode(token_id.to_vec()),
        })
        .collect())
}

/// Save the custom token ordering to the database.
#[tauri::command]
#[specta::specta]
pub fn token_save_order(
    state: tauri::State<'_, Arc<AppState>>,
    input: SaveTokenOrderInput,
) -> Result<(), String> {
    let ids: Vec<(Identifier, Identifier)> = input
        .token_ids
        .iter()
        .map(|pair| {
            let identity_id = parse_identifier(&pair.identity_id)?;
            let token_id = parse_identifier(&pair.token_id)?;
            Ok((identity_id, token_id))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let db = state.db();
    db.save_token_order(ids)
        .map_err(|e| format!("Failed to save token order: {e}"))
}

/// Get the minting destination configuration for a token.
///
/// Returns whether the minter can choose a custom recipient and the default
/// destination identity (if configured). This is used by the Mint screen to
/// show/hide the recipient input and auto-populate it.
#[tauri::command]
#[specta::specta]
pub fn token_get_minting_config(
    state: tauri::State<'_, Arc<AppState>>,
    input: GetMintingConfigInput,
) -> Result<MintingConfigDto, String> {
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;

    let contract_id = parse_identifier(&input.contract_id)?;
    let token_position = TokenContractPosition::from(input.token_position);

    let ctx = state.current_context();
    let qc = ctx
        .get_contract_by_id(&contract_id)
        .map_err(|e| format!("Database error loading contract: {e}"))?
        .ok_or_else(|| format!("Contract {} not found in local database", input.contract_id))?;

    let token_config = qc.contract.tokens().get(&token_position).ok_or_else(|| {
        format!(
            "Token at position {} not found in contract {}",
            input.token_position, input.contract_id
        )
    })?;

    let dist_rules = token_config.distribution_rules();
    let allow_choosing = dist_rules.minting_allow_choosing_destination();
    let default_dest = dist_rules
        .new_tokens_destination_identity()
        .map(|id| hex::encode(id.to_buffer()));

    Ok(MintingConfigDto {
        allow_choosing_destination: allow_choosing,
        default_destination_identity_id: default_dest,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_identity_token_balance_input_serializes() {
        let input = QueryIdentityTokenBalanceInput {
            identity_id: "abc".into(),
            token_id: "def".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"tokenId\":\"def\""));
    }

    #[test]
    fn query_frozen_identities_input_serializes() {
        let input = QueryFrozenIdentitiesInput {
            token_id: "abc".into(),
            identity_ids: vec!["id1".into(), "id2".into()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"tokenId\":\"abc\""));
        assert!(json.contains("\"identityIds\":[\"id1\",\"id2\"]"));
    }

    #[test]
    fn query_descriptions_by_keyword_input_serializes() {
        let input = QueryDescriptionsByKeywordInput {
            keyword: "dash".into(),
            start_after: Some("aabb".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyword\":\"dash\""));
        assert!(json.contains("\"startAfter\":\"aabb\""));
    }

    #[test]
    fn token_operation_input_serializes() {
        let input = TokenOperationInput {
            identity_id: "abc".into(),
            contract_id: "def".into(),
            token_position: 0,
            key_id: 2,
            public_note: Some("test note".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"contractId\":\"def\""));
        assert!(json.contains("\"tokenPosition\":0"));
        assert!(json.contains("\"keyId\":2"));
        assert!(json.contains("\"publicNote\":\"test note\""));
    }

    #[test]
    fn mint_tokens_input_serializes() {
        let input = MintTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            amount: "1000000".into(),
            recipient_id: Some("recipient".into()),
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"amount\":\"1000000\""));
        assert!(json.contains("\"recipientId\":\"recipient\""));
    }

    #[test]
    fn transfer_tokens_input_serializes() {
        let input = TransferTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            recipient_id: "recipient".into(),
            amount: "500".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"recipientId\":\"recipient\""));
        assert!(json.contains("\"amount\":\"500\""));
    }

    #[test]
    fn burn_tokens_input_serializes() {
        let input = BurnTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            amount: "100".into(),
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"amount\":\"100\""));
        assert!(json.contains("\"groupInfo\":null"));
    }

    #[test]
    fn destroy_frozen_funds_input_serializes() {
        let input = DestroyFrozenFundsInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            frozen_identity_id: "frozen".into(),
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"frozenIdentityId\":\"frozen\""));
    }

    #[test]
    fn freeze_tokens_input_serializes() {
        let input = FreezeTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            freeze_identity_id: "target".into(),
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"freezeIdentityId\":\"target\""));
    }

    #[test]
    fn unfreeze_tokens_input_serializes() {
        let input = UnfreezeTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            unfreeze_identity_id: "target".into(),
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"unfreezeIdentityId\":\"target\""));
    }

    #[test]
    fn pause_tokens_input_serializes() {
        let input = PauseTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"operation\""));
        assert!(json.contains("\"groupInfo\":null"));
    }

    #[test]
    fn resume_tokens_input_serializes() {
        let input = ResumeTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"operation\""));
    }

    #[test]
    fn claim_tokens_input_serializes() {
        let input = ClaimTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            distribution_type: "perpetual".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"distributionType\":\"perpetual\""));
    }

    #[test]
    fn estimate_perpetual_rewards_input_serializes() {
        let input = EstimatePerpetualRewardsInput {
            identity_id: "abc".into(),
            token_id: "def".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"tokenId\":\"def\""));
    }

    #[test]
    fn purchase_tokens_input_serializes() {
        let input = PurchaseTokensInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            amount: "10000".into(),
            total_agreed_price: 50000,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"amount\":\"10000\""));
        assert!(json.contains("\"totalAgreedPrice\":50000"));
    }

    #[test]
    fn set_direct_purchase_price_input_serializes() {
        let input = SetDirectPurchasePriceInput {
            operation: TokenOperationInput {
                identity_id: "abc".into(),
                contract_id: "def".into(),
                token_position: 0,
                key_id: 0,
                public_note: None,
            },
            token_pricing_schedule: Some(serde_json::json!({"price": 100})),
            group_info: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"tokenPricingSchedule\""));
    }

    #[test]
    fn register_token_contract_input_serializes() {
        let input = RegisterTokenContractInput {
            config_json: serde_json::json!({"decimals": 8}),
            identity_id: "abc".into(),
            key_id: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"configJson\""));
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"keyId\":0"));
    }

    #[test]
    fn remove_token_input_serializes() {
        let input = RemoveTokenInput {
            token_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"tokenId\":\"abc\""));
    }

    #[test]
    fn save_token_order_input_serializes() {
        let input = SaveTokenOrderInput {
            token_ids: vec![IdentityTokenIdentifierDto {
                identity_id: "abc".into(),
                token_id: "def".into(),
            }],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"tokenIds\""));
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"tokenId\":\"def\""));
    }

    #[test]
    fn parse_token_amount_valid() {
        assert_eq!(parse_token_amount("1000").unwrap(), 1000u64);
        assert_eq!(parse_token_amount("0").unwrap(), 0u64);
        assert_eq!(
            parse_token_amount("18446744073709551615").unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn parse_token_amount_invalid() {
        assert!(parse_token_amount("not_a_number").is_err());
        assert!(parse_token_amount("-1").is_err());
    }

    #[test]
    fn mint_tokens_roundtrip() {
        let json = r#"{
            "operation": {
                "identityId": "abc",
                "contractId": "def",
                "tokenPosition": 0,
                "keyId": 0,
                "publicNote": null
            },
            "amount": "1000",
            "recipientId": null,
            "groupInfo": null
        }"#;
        let input: MintTokensInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.amount, "1000");
        assert!(input.recipient_id.is_none());
    }

    #[test]
    fn fetch_token_by_contract_id_serializes() {
        let input = FetchTokenByContractIdInput {
            contract_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
    }

    #[test]
    fn fetch_token_by_token_id_serializes() {
        let input = FetchTokenByTokenIdInput {
            token_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"tokenId\":\"abc\""));
    }

    #[test]
    fn query_token_pricing_input_serializes() {
        let input = QueryTokenPricingInput {
            token_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"tokenId\":\"abc\""));
    }

    #[test]
    fn get_minting_config_input_serializes() {
        let input = GetMintingConfigInput {
            contract_id: "abc".into(),
            token_position: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"tokenPosition\":0"));
    }

    #[test]
    fn minting_config_dto_serializes() {
        let dto = MintingConfigDto {
            allow_choosing_destination: true,
            default_destination_identity_id: Some("deadbeef".into()),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"allowChoosingDestination\":true"));
        assert!(json.contains("\"defaultDestinationIdentityId\":\"deadbeef\""));
    }

    #[test]
    fn minting_config_dto_serializes_no_default() {
        let dto = MintingConfigDto {
            allow_choosing_destination: false,
            default_destination_identity_id: None,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"allowChoosingDestination\":false"));
        assert!(json.contains("\"defaultDestinationIdentityId\":null"));
    }
}
