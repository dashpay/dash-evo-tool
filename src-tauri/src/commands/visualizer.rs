//! Visualizer Tauri IPC commands.
//!
//! Synchronous commands for parsing/deserializing data structures
//! (contracts, documents, proofs, state transitions) from raw bytes.

use crate::state::AppState;

use dash_sdk::dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructure;
use dash_sdk::platform::DataContract;

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
}
