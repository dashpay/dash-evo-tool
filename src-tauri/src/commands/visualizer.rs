//! Visualizer Tauri IPC commands.
//!
//! Synchronous commands for parsing/deserializing data structures
//! (contracts, documents, proofs, state transitions) from raw bytes.

use crate::dto::common::IdentifierDto;
use crate::state::AppState;

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::serialization_traits::DocumentPlatformConversionMethodsV0;
use dash_sdk::dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructure;
use dash_sdk::platform::{DataContract, Document, Identifier};

use serde::{Deserialize, Serialize};
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
}
