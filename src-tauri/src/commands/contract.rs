//! Contract-related Tauri IPC commands.
//!
//! Maps all 7 `ContractTask` variants plus direct database methods to
//! Tauri commands. Long-running operations are dispatched asynchronously
//! via `task_dispatcher::dispatch_task` and results arrive as events.
//! Short reads (DB queries) return directly.

use crate::commands::identity::parse_identifier;
use crate::dto::common::IdentifierDto;
use crate::dto::contract::{ContractSummaryDto, DataContractDto};
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::contract::ContractTask;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::database::contracts::InsertTokensToo;

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::platform::{DataContract, Identifier};

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs — serializable command parameters from the frontend
// ---------------------------------------------------------------------------

/// Input for fetching contracts by IDs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchContractsInput {
    /// Contract IDs (hex) to fetch.
    pub contract_ids: Vec<IdentifierDto>,
}

/// Input for fetching contracts with descriptions.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchContractsWithDescriptionsInput {
    /// Contract IDs (hex) to fetch with descriptions.
    pub contract_ids: Vec<IdentifierDto>,
}

/// Input for fetching active group actions.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchActiveGroupActionsInput {
    /// The contract ID (hex) to query group actions for.
    pub contract_id: IdentifierDto,
    /// The identity ID (hex) to check group membership.
    pub identity_id: IdentifierDto,
}

/// Input for removing a contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoveContractInput {
    /// Contract ID (hex) to remove.
    pub contract_id: IdentifierDto,
}

/// Input for registering a new data contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDataContractInput {
    /// The contract schema as a JSON value.
    pub contract_json: serde_json::Value,
    /// Alias for the contract.
    pub alias: String,
    /// Identity ID (hex) that will own the contract.
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
}

/// Input for updating an existing data contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDataContractInput {
    /// The updated contract schema as a JSON value.
    pub contract_json: serde_json::Value,
    /// Identity ID (hex) that owns the contract.
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub key_id: u32,
}

/// Input for saving a contract locally.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveDataContractInput {
    /// The contract JSON to save.
    pub contract_json: serde_json::Value,
    /// Optional alias.
    pub alias: Option<String>,
    /// Whether to also insert tokens from this contract.
    pub insert_tokens: bool,
}

/// Input for setting a contract alias.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetContractAliasInput {
    /// Contract ID (hex).
    pub contract_id: IdentifierDto,
    /// New alias (None to clear).
    pub alias: Option<String>,
}

/// Response from `contract_fetch` — includes the task ID and hex-normalised
/// versions of every requested identifier so the frontend can match results
/// regardless of whether the user typed hex or base58.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchContractsResponse {
    /// Unique task ID. Listen for `TaskResultEvent` or `TaskErrorEvent` with
    /// this ID to receive the result.
    pub task_id: String,
    /// The contract IDs normalised to lowercase hex (same order as input).
    pub normalized_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn parse_identifiers(strs: &[String]) -> Result<Vec<Identifier>, String> {
    strs.iter().map(|s| parse_identifier(s)).collect()
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

/// Convert a DataContract to a DataContractDto.
fn data_contract_to_dto(contract: &DataContract, alias: Option<String>) -> DataContractDto {
    let id_hex = hex::encode(contract.id().to_vec());
    let owner_id_hex = hex::encode(contract.owner_id().to_vec());

    let document_type_names: Vec<String> = contract.document_types().keys().cloned().collect();

    let token_count = contract.tokens().len() as u16;

    // Serialize the full contract to JSON for display/editing.
    let schema_json = serde_json::to_value(contract).unwrap_or(serde_json::Value::Null);

    DataContractDto {
        id: id_hex,
        owner_id: owner_id_hex,
        alias,
        version: contract.version(),
        document_type_names,
        token_count,
        schema_json,
    }
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Fetch contracts from Platform by their IDs.
///
/// Dispatches `ContractTask::FetchContracts`. Result arrives via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn contract_fetch(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchContractsInput,
) -> Result<FetchContractsResponse, String> {
    let ids = parse_identifiers(&input.contract_ids)?;
    let normalized_ids = ids.iter().map(|id| hex::encode(id.to_buffer())).collect();
    let task = BackendTask::ContractTask(Box::new(ContractTask::FetchContracts(ids)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(FetchContractsResponse {
        task_id,
        normalized_ids,
    })
}

/// Fetch contracts from Platform with descriptions.
///
/// Dispatches `ContractTask::FetchContractsWithDescriptions`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn contract_fetch_with_descriptions(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchContractsWithDescriptionsInput,
) -> Result<DispatchTaskResponse, String> {
    let ids = parse_identifiers(&input.contract_ids)?;
    let task =
        BackendTask::ContractTask(Box::new(ContractTask::FetchContractsWithDescriptions(ids)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch active group actions for a contract and identity.
///
/// Dispatches `ContractTask::FetchActiveGroupActions`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn contract_fetch_active_group_actions(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchActiveGroupActionsInput,
) -> Result<DispatchTaskResponse, String> {
    let contract_id = parse_identifier(&input.contract_id)?;
    let ctx = state.current_context();
    let qualified_contract = ctx
        .get_contract_by_id(&contract_id)
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Contract {} not found", input.contract_id))?;

    let qi = lookup_identity(&state, &input.identity_id)?;

    let task = BackendTask::ContractTask(Box::new(ContractTask::FetchActiveGroupActions(
        qualified_contract,
        qi,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Register a new data contract on Platform.
///
/// Dispatches `ContractTask::RegisterDataContract`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn contract_register(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RegisterDataContractInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;

    // Deserialize the contract from JSON
    let data_contract: DataContract = serde_json::from_value(input.contract_json)
        .map_err(|e| format!("Invalid contract JSON: {e}"))?;

    let task = BackendTask::ContractTask(Box::new(ContractTask::RegisterDataContract(
        data_contract,
        input.alias,
        qi,
        signing_key,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Update an existing data contract on Platform.
///
/// Dispatches `ContractTask::UpdateDataContract`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn contract_update(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: UpdateDataContractInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = find_signing_key(&qi, input.key_id)?;

    let data_contract: DataContract = serde_json::from_value(input.contract_json)
        .map_err(|e| format!("Invalid contract JSON: {e}"))?;

    let task = BackendTask::ContractTask(Box::new(ContractTask::UpdateDataContract(
        data_contract,
        qi,
        signing_key,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Save a contract to the local database (without broadcasting).
///
/// Dispatches `ContractTask::SaveDataContract`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn contract_save(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SaveDataContractInput,
) -> Result<DispatchTaskResponse, String> {
    let data_contract: DataContract = serde_json::from_value(input.contract_json)
        .map_err(|e| format!("Invalid contract JSON: {e}"))?;

    let insert_tokens = if input.insert_tokens {
        InsertTokensToo::AllTokensShouldBeAdded
    } else {
        InsertTokensToo::NoTokensShouldBeAdded
    };

    let task = BackendTask::ContractTask(Box::new(ContractTask::SaveDataContract(
        data_contract,
        input.alias,
        insert_tokens,
    )));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Remove a contract from the local database.
///
/// Dispatches `ContractTask::RemoveContract`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn contract_remove(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RemoveContractInput,
) -> Result<DispatchTaskResponse, String> {
    let id = parse_identifier(&input.contract_id)?;
    let task = BackendTask::ContractTask(Box::new(ContractTask::RemoveContract(id)));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Direct database read commands (synchronous, return immediately)
// ---------------------------------------------------------------------------

/// List all locally stored contracts.
#[tauri::command]
#[specta::specta]
pub fn contract_list_local(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<ContractSummaryDto>, String> {
    let ctx = state.current_context();
    let contracts = ctx
        .get_contracts(None, None)
        .map_err(|e| format!("Failed to load contracts: {e}"))?;

    Ok(contracts
        .iter()
        .map(|qc| {
            let id_hex = hex::encode(qc.contract.id().to_vec());
            ContractSummaryDto {
                id: id_hex,
                alias: qc.alias.clone(),
                document_type_count: qc.contract.document_types().len(),
                token_count: qc.contract.tokens().len() as u16,
            }
        })
        .collect())
}

/// Get a contract by its ID from the local database.
#[tauri::command]
#[specta::specta]
pub fn contract_get_by_id(
    state: tauri::State<'_, Arc<AppState>>,
    contract_id: IdentifierDto,
) -> Result<Option<DataContractDto>, String> {
    let identifier = parse_identifier(&contract_id)?;
    let ctx = state.current_context();
    let contract = ctx
        .get_contract_by_id(&identifier)
        .map_err(|e| format!("Failed to get contract: {e}"))?;

    Ok(contract.map(|qc| data_contract_to_dto(&qc.contract, qc.alias.clone())))
}

/// Get a contract by its token ID from the local database.
#[tauri::command]
#[specta::specta]
pub fn contract_get_by_token_id(
    state: tauri::State<'_, Arc<AppState>>,
    token_id: IdentifierDto,
) -> Result<Option<DataContractDto>, String> {
    let identifier = parse_identifier(&token_id)?;
    let ctx = state.current_context();
    let contract = ctx
        .get_contract_by_token_id(&identifier)
        .map_err(|e| format!("Failed to get contract by token ID: {e}"))?;

    Ok(contract.map(|qc| data_contract_to_dto(&qc.contract, qc.alias.clone())))
}

/// Set or clear the alias for a contract.
#[tauri::command]
#[specta::specta]
pub fn contract_set_alias(
    state: tauri::State<'_, Arc<AppState>>,
    input: SetContractAliasInput,
) -> Result<(), String> {
    let identifier = parse_identifier(&input.contract_id)?;
    let db = state.db();
    db.set_contract_alias(&identifier, input.alias.as_deref())
        .map_err(|e| format!("Failed to set contract alias: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::contract::GroupActionDto;

    #[test]
    fn fetch_contracts_input_serializes_with_camel_case() {
        let input = FetchContractsInput {
            contract_ids: vec!["abc123".into(), "def456".into()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractIds\":[\"abc123\",\"def456\"]"));
    }

    #[test]
    fn fetch_with_descriptions_input_serializes() {
        let input = FetchContractsWithDescriptionsInput {
            contract_ids: vec!["abc".into()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractIds\":[\"abc\"]"));
    }

    #[test]
    fn fetch_active_group_actions_input_serializes() {
        let input = FetchActiveGroupActionsInput {
            contract_id: "abc".into(),
            identity_id: "def".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"identityId\":\"def\""));
    }

    #[test]
    fn remove_contract_input_serializes() {
        let input = RemoveContractInput {
            contract_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
    }

    #[test]
    fn register_contract_input_serializes() {
        let input = RegisterDataContractInput {
            contract_json: serde_json::json!({"test": true}),
            alias: "My Contract".into(),
            identity_id: "abc".into(),
            key_id: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":\"My Contract\""));
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"keyId\":0"));
        assert!(json.contains("\"contractJson\""));
    }

    #[test]
    fn update_contract_input_serializes() {
        let input = UpdateDataContractInput {
            contract_json: serde_json::json!({"version": 2}),
            identity_id: "abc".into(),
            key_id: 1,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"keyId\":1"));
    }

    #[test]
    fn save_contract_input_serializes() {
        let input = SaveDataContractInput {
            contract_json: serde_json::json!({}),
            alias: Some("test".into()),
            insert_tokens: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"insertTokens\":true"));
        assert!(json.contains("\"alias\":\"test\""));
    }

    #[test]
    fn set_contract_alias_input_serializes() {
        let input = SetContractAliasInput {
            contract_id: "abc".into(),
            alias: Some("New Alias".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractId\":\"abc\""));
        assert!(json.contains("\"alias\":\"New Alias\""));
    }

    #[test]
    fn set_contract_alias_input_clear_serializes() {
        let input = SetContractAliasInput {
            contract_id: "abc".into(),
            alias: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":null"));
    }

    #[test]
    fn contract_summary_dto_serializes() {
        let dto = ContractSummaryDto {
            id: "aabbcc".into(),
            alias: Some("Test".into()),
            document_type_count: 3,
            token_count: 1,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"documentTypeCount\":3"));
        assert!(json.contains("\"tokenCount\":1"));
    }

    #[test]
    fn group_action_dto_serializes() {
        let dto = GroupActionDto {
            group_position: 0,
            action_id: "action-1".into(),
            action_type: "TokenMint".into(),
            signers_count: 2,
            required_signatures: 3,
            details: serde_json::json!({"amount": 1000}),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"groupPosition\":0"));
        assert!(json.contains("\"actionType\":\"TokenMint\""));
        assert!(json.contains("\"signersCount\":2"));
        assert!(json.contains("\"requiredSignatures\":3"));
    }

    #[test]
    fn parse_identifier_valid_hex() {
        let hex_str = "a".repeat(64);
        assert!(parse_identifier(&hex_str).is_ok());
    }

    #[test]
    fn parse_identifier_invalid() {
        assert!(parse_identifier("not-hex").is_err());
    }

    #[test]
    fn parse_identifiers_batch() {
        let ids = vec!["a".repeat(64), "b".repeat(64)];
        assert!(parse_identifiers(&ids).is_ok());
        assert_eq!(parse_identifiers(&ids).unwrap().len(), 2);
    }

    #[test]
    fn fetch_contracts_input_roundtrip() {
        let json = r#"{"contractIds":["abc","def"]}"#;
        let input: FetchContractsInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.contract_ids.len(), 2);
        assert_eq!(input.contract_ids[0], "abc");
    }
}
