//! Serializable GroveSTARK proof data.
//!
//! Pure data types plus their (de)serialization. Proof *generation* and
//! *verification* — which drive the SDK and the `grovestark` prover — live in
//! `backend_task::grovestark`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofDataOutput {
    pub proof: Vec<u8>, // Serialized STARK proof
    pub public_inputs: PublicInputsData,
    pub metadata: ProofMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicInputsData {
    pub state_root: [u8; 32],
    pub contract_id: [u8; 32],
    pub message_hash: [u8; 32],
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofMetadata {
    pub created_at: u64,
    pub proof_size: usize,
    pub generation_time_ms: u64,
    pub security_level: u32,
}

/// Failure to (de)serialize a [`ProofDataOutput`].
#[derive(Debug, thiserror::Error)]
pub enum ProofSerializationError {
    #[error("Could not serialize the proof data.")]
    Serialize(#[source] serde_json::Error),
    #[error("Could not read the proof data.")]
    Deserialize(#[source] serde_json::Error),
    #[error("The proof data is not valid base64.")]
    Base64(#[source] base64::DecodeError),
}

impl ProofDataOutput {
    /// Serialize the proof to a JSON string.
    pub fn to_json_string(&self) -> Result<String, ProofSerializationError> {
        serde_json::to_string(self).map_err(ProofSerializationError::Serialize)
    }

    /// Serialize the proof to base64-encoded JSON.
    pub fn to_base64(&self) -> Result<String, ProofSerializationError> {
        use base64::{Engine as _, engine::general_purpose};
        let json_bytes = serde_json::to_vec(self).map_err(ProofSerializationError::Serialize)?;
        Ok(general_purpose::STANDARD.encode(json_bytes))
    }

    /// Deserialize from base64-encoded JSON.
    pub fn from_base64(base64_str: &str) -> Result<Self, ProofSerializationError> {
        use base64::{Engine as _, engine::general_purpose};
        let bytes = general_purpose::STANDARD
            .decode(base64_str)
            .map_err(ProofSerializationError::Base64)?;
        serde_json::from_slice(&bytes).map_err(ProofSerializationError::Deserialize)
    }

    /// Deserialize from a JSON string.
    pub fn from_json_string(json_str: &str) -> Result<Self, ProofSerializationError> {
        serde_json::from_str(json_str).map_err(ProofSerializationError::Deserialize)
    }
}
