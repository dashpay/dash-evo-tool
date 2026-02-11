//! Visualizer Tauri IPC commands.
//!
//! Synchronous commands for parsing/deserializing data structures
//! (contracts, documents, proofs, state transitions) from raw bytes.

use crate::dto::common::IdentifierDto;
use crate::state::AppState;

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::serialization_traits::DocumentPlatformConversionMethodsV0;
use dash_sdk::dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructure;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::drive::grovedb::operations::proof::GroveDBProof;
use dash_sdk::drive::query::PathQuery;
use dash_sdk::platform::{DataContract, Document, Identifier};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Input DTOs
// ---------------------------------------------------------------------------

/// Input for parsing a serialized data contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseDataContractInput {
    /// Hex-encoded contract bytes.
    pub hex_data: String,
}

/// Output from successfully parsing a data contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseDataContractOutput {
    /// Pretty-printed JSON representation of the contract.
    pub json: String,
}

/// Input for parsing a serialized document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseDocumentInput {
    /// Hex-encoded document bytes.
    pub hex_data: String,
    /// Contract ID (hex) the document belongs to.
    pub contract_id: IdentifierDto,
    /// Document type name within the contract.
    pub document_type_name: String,
}

/// Output from successfully parsing a document.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseDocumentOutput {
    /// Pretty-printed JSON representation of the document.
    pub json: String,
}

/// Input for parsing a serialized GroveDB proof.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseGrovedbProofInput {
    /// Hex-encoded proof bytes.
    pub hex_data: String,
}

/// Output from successfully parsing a GroveDB proof.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseGrovedbProofOutput {
    /// Human-readable string representation of the proof structure.
    pub text: String,
}

/// Input for parsing a serialized PathQuery (verification path query).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParsePathQueryInput {
    /// Hex-encoded PathQuery bytes (bincode-encoded).
    pub hex_data: String,
}

/// Output from successfully parsing a PathQuery.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParsePathQueryOutput {
    /// Human-readable string representation of the path query.
    pub text: String,
}

/// Input for parsing a serialized state transition.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseStateTransitionInput {
    /// Hex-encoded state transition bytes.
    pub hex_data: String,
}

/// Output from successfully parsing a state transition.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseStateTransitionOutput {
    /// Pretty-printed JSON representation of the state transition.
    pub json: String,
    /// Contract IDs detected in the state transition (Base58-encoded strings).
    pub detected_contract_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Parse hex-encoded bytes into a DataContract and return pretty-printed JSON.
///
/// This is a synchronous command — the deserialization happens inline.
/// Supports hex-encoded input (the frontend decodes base64/CSV → hex before calling).
#[tauri::command]
#[specta::specta]
pub fn parse_data_contract(
    state: tauri::State<'_, Arc<AppState>>,
    input: ParseDataContractInput,
) -> Result<ParseDataContractOutput, String> {
    let bytes = hex::decode(&input.hex_data).map_err(|e| format!("Invalid hex data: {e}"))?;

    if bytes.is_empty() {
        return Err("No data provided".into());
    }

    let ctx = state.context_for_network(state.active_network());
    let platform_version = ctx.platform_version();

    let data_contract = DataContract::versioned_deserialize(&bytes, false, platform_version)
        .map_err(|e| format!("Deserialization error: {e}"))?;

    let json = serde_json::to_string_pretty(&data_contract)
        .map_err(|e| format!("JSON serialization error: {e}"))?;

    Ok(ParseDataContractOutput { json })
}

/// Parse hex-encoded bytes into a Document and return pretty-printed JSON.
///
/// Requires a contract ID and document type name so the backend can look up
/// the document type schema needed for deserialization.
/// The frontend decodes base64/CSV → hex before calling.
#[tauri::command]
#[specta::specta]
pub fn parse_document(
    state: tauri::State<'_, Arc<AppState>>,
    input: ParseDocumentInput,
) -> Result<ParseDocumentOutput, String> {
    let bytes = hex::decode(&input.hex_data).map_err(|e| format!("Invalid hex data: {e}"))?;

    if bytes.is_empty() {
        return Err("No data provided".into());
    }

    // Look up the contract from the local database
    let contract_id_bytes =
        hex::decode(&input.contract_id).map_err(|e| format!("Invalid contract ID hex: {e}"))?;
    let contract_identifier = Identifier::from_bytes(&contract_id_bytes)
        .map_err(|e| format!("Invalid contract identifier: {e}"))?;

    let ctx = state.context_for_network(state.active_network());
    let qc = ctx
        .get_contract_by_id(&contract_identifier)
        .map_err(|e| format!("Database error loading contract: {e}"))?
        .ok_or_else(|| format!("Contract {} not found in local database", input.contract_id))?;

    // Get the document type from the contract
    let doc_type = qc
        .contract
        .document_types()
        .get(&input.document_type_name)
        .ok_or_else(|| {
            format!(
                "Document type '{}' not found in contract",
                input.document_type_name
            )
        })?;

    let platform_version = ctx.platform_version();

    let document = Document::from_bytes(&bytes, doc_type.as_ref(), platform_version)
        .map_err(|e| format!("Deserialization error: {e}"))?;

    let json = serde_json::to_string_pretty(&document)
        .map_err(|e| format!("JSON serialization error: {e}"))?;

    Ok(ParseDocumentOutput { json })
}

/// Parse hex-encoded bytes into a GroveDB proof and return its string representation.
///
/// Uses bincode deserialization with big-endian, no-limit config (matching the
/// egui implementation). The frontend decodes base64/CSV → hex before calling.
#[tauri::command]
#[specta::specta]
pub fn parse_grovedb_proof(
    input: ParseGrovedbProofInput,
) -> Result<ParseGrovedbProofOutput, String> {
    let bytes = hex::decode(&input.hex_data).map_err(|e| format!("Invalid hex data: {e}"))?;

    if bytes.is_empty() {
        return Err("No data provided".into());
    }

    let config = bincode::config::standard()
        .with_big_endian()
        .with_no_limit();

    let (proof, _): (GroveDBProof, _) = bincode::decode_from_slice(&bytes, config)
        .map_err(|e| format!("Deserialization error: {e}"))?;

    Ok(ParseGrovedbProofOutput {
        text: proof.to_string(),
    })
}

/// Parse hex-encoded bytes into a PathQuery and return its string representation.
///
/// Uses bincode deserialization with standard config (matching how path queries
/// are encoded in the backend).
#[tauri::command]
#[specta::specta]
pub fn parse_path_query(input: ParsePathQueryInput) -> Result<ParsePathQueryOutput, String> {
    let bytes = hex::decode(&input.hex_data).map_err(|e| format!("Invalid hex data: {e}"))?;

    if bytes.is_empty() {
        return Err("No data provided".into());
    }

    let config = bincode::config::standard();

    let (path_query, _): (PathQuery, _) = bincode::decode_from_slice(&bytes, config)
        .map_err(|e| format!("Deserialization error: {e}"))?;

    Ok(ParsePathQueryOutput {
        text: format!("{}", path_query),
    })
}

/// Parse hex-encoded bytes into a StateTransition and return pretty-printed JSON
/// plus any detected contract IDs.
///
/// Uses the same bincode deserialization as `broadcast_state_transition`.
/// After deserializing, recursively scans the JSON for objects matching
/// `{ "type": "singleContract", "id": "<base58>" }` to extract contract references.
#[tauri::command]
#[specta::specta]
pub fn parse_state_transition(
    input: ParseStateTransitionInput,
) -> Result<ParseStateTransitionOutput, String> {
    let bytes = hex::decode(&input.hex_data).map_err(|e| format!("Invalid hex data: {e}"))?;

    if bytes.is_empty() {
        return Err("No data provided".into());
    }

    let config = dash_sdk::dpp::bincode::config::standard()
        .with_big_endian()
        .with_no_limit();

    let (st, _): (StateTransition, _) =
        dash_sdk::dpp::bincode::decode_from_slice(&bytes, config)
            .map_err(|e| format!("Failed to parse state transition: {e}"))?;

    let json = serde_json::to_string_pretty(&st)
        .map_err(|e| format!("Failed to serialize to JSON: {e}"))?;

    // Extract contract IDs by scanning JSON for { "type": "singleContract", "id": "..." }
    let detected_contract_ids = match serde_json::from_str::<Value>(&json) {
        Ok(value) => {
            let mut ids = Vec::new();
            extract_contract_ids(&value, &mut ids);
            ids.dedup();
            ids
        }
        Err(_) => Vec::new(),
    };

    Ok(ParseStateTransitionOutput {
        json,
        detected_contract_ids,
    })
}

/// Recursively search a JSON value for contract ID references.
///
/// Looks for objects with shape `{ "type": "singleContract", "id": "<base58>" }`.
fn extract_contract_ids(value: &Value, ids: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let (Some(Value::String(type_str)), Some(Value::String(id))) =
                (map.get("type"), map.get("id"))
            {
                if type_str == "singleContract" {
                    ids.push(id.clone());
                }
            }
            for val in map.values() {
                extract_contract_ids(val, ids);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                extract_contract_ids(val, ids);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_data_contract_input_serializes() {
        let input = ParseDataContractInput {
            hex_data: "deadbeef".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"hexData\":\"deadbeef\""));
    }

    #[test]
    fn parse_data_contract_output_serializes() {
        let output = ParseDataContractOutput {
            json: "{\"test\": true}".into(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"json\""));
    }

    #[test]
    fn parse_data_contract_output_deserializes() {
        let json = r#"{"json":"{\"test\": true}"}"#;
        let output: ParseDataContractOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.json, "{\"test\": true}");
    }

    #[test]
    fn parse_document_input_serializes() {
        let input = ParseDocumentInput {
            hex_data: "deadbeef".into(),
            contract_id: "abc123".into(),
            document_type_name: "profile".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"hexData\":\"deadbeef\""));
        assert!(json.contains("\"contractId\":\"abc123\""));
        assert!(json.contains("\"documentTypeName\":\"profile\""));
    }

    #[test]
    fn parse_document_input_roundtrip() {
        let json = r#"{"hexData":"aabb","contractId":"def","documentTypeName":"domain"}"#;
        let input: ParseDocumentInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.hex_data, "aabb");
        assert_eq!(input.contract_id, "def");
        assert_eq!(input.document_type_name, "domain");
    }

    #[test]
    fn parse_document_output_serializes() {
        let output = ParseDocumentOutput {
            json: "{\"$id\": \"abc\"}".into(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"json\":\"{\\\"$id\\\": \\\"abc\\\"}\""));
    }

    #[test]
    fn parse_document_output_deserializes() {
        let json = r#"{"json":"{\"$id\": \"abc\"}"}"#;
        let output: ParseDocumentOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.json, "{\"$id\": \"abc\"}");
    }

    #[test]
    fn parse_grovedb_proof_input_serializes() {
        let input = ParseGrovedbProofInput {
            hex_data: "deadbeef".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"hexData\":\"deadbeef\""));
    }

    #[test]
    fn parse_grovedb_proof_input_deserializes() {
        let json = r#"{"hexData":"aabb"}"#;
        let input: ParseGrovedbProofInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.hex_data, "aabb");
    }

    #[test]
    fn parse_grovedb_proof_output_serializes() {
        let output = ParseGrovedbProofOutput {
            text: "GroveDBProof { ... }".into(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"text\":\"GroveDBProof { ... }\""));
    }

    #[test]
    fn parse_grovedb_proof_output_deserializes() {
        let json = r#"{"text":"some proof text"}"#;
        let output: ParseGrovedbProofOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.text, "some proof text");
    }

    #[test]
    fn parse_grovedb_proof_rejects_empty() {
        let result = parse_grovedb_proof(ParseGrovedbProofInput {
            hex_data: String::new(),
        });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No data provided");
    }

    #[test]
    fn parse_grovedb_proof_rejects_invalid_hex() {
        let result = parse_grovedb_proof(ParseGrovedbProofInput {
            hex_data: "xyz".into(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid hex data"));
    }

    #[test]
    fn parse_grovedb_proof_rejects_invalid_bincode() {
        let result = parse_grovedb_proof(ParseGrovedbProofInput {
            hex_data: "deadbeef".into(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Deserialization error"));
    }

    // --- State transition ---

    #[test]
    fn parse_state_transition_input_serializes() {
        let input = ParseStateTransitionInput {
            hex_data: "cafebabe".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"hexData\":\"cafebabe\""));
    }

    #[test]
    fn parse_state_transition_input_deserializes() {
        let json = r#"{"hexData":"aabb"}"#;
        let input: ParseStateTransitionInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.hex_data, "aabb");
    }

    #[test]
    fn parse_state_transition_output_serializes() {
        let output = ParseStateTransitionOutput {
            json: "{\"test\": true}".into(),
            detected_contract_ids: vec!["abc123".into(), "def456".into()],
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"json\""));
        assert!(json.contains("\"detectedContractIds\""));
        assert!(json.contains("\"abc123\""));
        assert!(json.contains("\"def456\""));
    }

    #[test]
    fn parse_state_transition_output_deserializes() {
        let json = r#"{"json":"{\"x\":1}","detectedContractIds":["id1"]}"#;
        let output: ParseStateTransitionOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.json, "{\"x\":1}");
        assert_eq!(output.detected_contract_ids, vec!["id1"]);
    }

    #[test]
    fn parse_state_transition_rejects_empty() {
        let result = parse_state_transition(ParseStateTransitionInput {
            hex_data: String::new(),
        });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No data provided");
    }

    #[test]
    fn parse_state_transition_rejects_invalid_hex() {
        let result = parse_state_transition(ParseStateTransitionInput {
            hex_data: "xyz".into(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid hex data"));
    }

    #[test]
    fn parse_state_transition_rejects_invalid_bincode() {
        let result = parse_state_transition(ParseStateTransitionInput {
            hex_data: "deadbeef".into(),
        });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Failed to parse state transition"));
    }

    // --- extract_contract_ids ---

    #[test]
    fn extract_contract_ids_finds_single_contract() {
        let json: Value = serde_json::from_str(
            r#"{"dataContract": {"contractBounds": {"type": "singleContract", "id": "ABC123"}}}"#,
        )
        .unwrap();
        let mut ids = Vec::new();
        extract_contract_ids(&json, &mut ids);
        assert_eq!(ids, vec!["ABC123"]);
    }

    #[test]
    fn extract_contract_ids_finds_multiple_contracts() {
        let json: Value = serde_json::from_str(
            r#"{"a": {"type": "singleContract", "id": "ID1"}, "b": [{"type": "singleContract", "id": "ID2"}]}"#,
        )
        .unwrap();
        let mut ids = Vec::new();
        extract_contract_ids(&json, &mut ids);
        ids.sort();
        assert_eq!(ids, vec!["ID1", "ID2"]);
    }

    #[test]
    fn extract_contract_ids_ignores_non_single_contract() {
        let json: Value =
            serde_json::from_str(r#"{"type": "multiContract", "id": "SHOULD_NOT_MATCH"}"#).unwrap();
        let mut ids = Vec::new();
        extract_contract_ids(&json, &mut ids);
        assert!(ids.is_empty());
    }

    #[test]
    fn extract_contract_ids_empty_json() {
        let json: Value = serde_json::from_str("{}").unwrap();
        let mut ids = Vec::new();
        extract_contract_ids(&json, &mut ids);
        assert!(ids.is_empty());
    }

    // --- PathQuery ---

    #[test]
    fn parse_path_query_input_serializes() {
        let input = ParsePathQueryInput {
            hex_data: "deadbeef".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"hexData\":\"deadbeef\""));
    }

    #[test]
    fn parse_path_query_input_deserializes() {
        let json = r#"{"hexData":"aabb"}"#;
        let input: ParsePathQueryInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.hex_data, "aabb");
    }

    #[test]
    fn parse_path_query_output_serializes() {
        let output = ParsePathQueryOutput {
            text: "PathQuery { ... }".into(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"text\":\"PathQuery { ... }\""));
    }

    #[test]
    fn parse_path_query_output_deserializes() {
        let json = r#"{"text":"some path query text"}"#;
        let output: ParsePathQueryOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.text, "some path query text");
    }

    #[test]
    fn parse_path_query_rejects_empty() {
        let result = parse_path_query(ParsePathQueryInput {
            hex_data: String::new(),
        });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No data provided");
    }

    #[test]
    fn parse_path_query_rejects_invalid_hex() {
        let result = parse_path_query(ParsePathQueryInput {
            hex_data: "xyz".into(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid hex data"));
    }

    #[test]
    fn parse_path_query_rejects_invalid_bincode() {
        let result = parse_path_query(ParsePathQueryInput {
            hex_data: "deadbeef".into(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Deserialization error"));
    }
}
