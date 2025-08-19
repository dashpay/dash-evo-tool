use dash_sdk::Sdk;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::documents::document_query::DocumentQuery;
use dash_sdk::platform::{Fetch, FetchWithProof};
use ed25519_dalek::{Signer, SigningKey};
use grovestark::{
    GroveSTARK, PublicInputs, STARKConfig, STARKProof, Verifier, create_witness_from_sdk_proofs,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

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

pub struct GroveStarkIntegration {
    prover: GroveSTARK,
    verifier: Verifier,
}

impl GroveStarkIntegration {
    pub fn new(security_level: u32, grinding_bits: u32) -> Self {
        let config = STARKConfig {
            field_bits: 64,
            expansion_factor: 8,
            num_queries: 20,
            folding_factor: 4,
            grinding_bits: grinding_bits as usize,
            trace_length: 65536,
            num_trace_columns: 32,
            max_remainder_degree: 255,
            security_level: security_level as usize,
        };

        Self {
            prover: GroveSTARK::with_config(config.clone()),
            verifier: Verifier::new(config),
        }
    }

    /// Generate a proof for document ownership
    pub async fn generate_proof(
        &self,
        sdk: &Sdk,
        identity_id: &str,
        contract_id: &str,
        document_type: &str,
        document_id: &str,
        private_key: &[u8; 32],
    ) -> Result<ProofDataOutput, ProofError> {
        let start_time = Instant::now();

        tracing::info!("Starting ZK proof generation");
        tracing::info!("Identity ID: {}", identity_id);
        tracing::info!("Contract ID: {}", contract_id);
        tracing::info!("Document Type: {}", document_type);
        tracing::info!("Document ID: {}", document_id);

        // Step 1: Parse identifiers
        tracing::debug!("Parsing identifiers...");
        let identity_identifier = Identifier::from_string(
            identity_id,
            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
        )
        .map_err(|e| {
            tracing::error!("Failed to parse identity ID: {}", e);
            ProofError::InvalidIdentityId(e.to_string())
        })?;
        let contract_identifier = Identifier::from_string(
            contract_id,
            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
        )
        .map_err(|e| ProofError::InvalidContractId(e.to_string()))?;

        // Step 2: Fetch identity with proof
        tracing::info!("Fetching identity with proof...");
        let (identity_opt, identity_proof_data) =
            Identity::fetch_with_proof(sdk, identity_identifier)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to fetch identity with proof: {}", e);
                    ProofError::Platform(e.to_string())
                })?;

        let identity = identity_opt.ok_or_else(|| ProofError::IdentityNotFound)?;

        tracing::info!(
            "Identity proof size: {} bytes",
            identity_proof_data.grovedb_proof.len()
        );

        // Step 3: Fetch contract and create DocumentQuery
        tracing::info!("Fetching contract...");
        let contract = dash_sdk::platform::DataContract::fetch(sdk, contract_identifier)
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch contract: {}", e);
                ProofError::Platform(e.to_string())
            })?
            .ok_or_else(|| {
                tracing::error!("Contract not found for ID: {}", contract_id);
                ProofError::InvalidContractId("Contract not found".to_string())
            })?;

        let document_id_identifier = Identifier::from_string(
            document_id,
            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
        )
        .map_err(|e| ProofError::Platform(e.to_string()))?;

        let query = DocumentQuery::new(contract, document_type)
            .map_err(|e| ProofError::Platform(e.to_string()))?
            .with_document_id(&document_id_identifier);

        tracing::info!("Fetching document with proof...");
        let (document_opt, document_proof_data) =
            dash_sdk::dpp::document::Document::fetch_with_proof(sdk, query)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to fetch document with proof: {}", e);
                    ProofError::Platform(e.to_string())
                })?;

        let document = document_opt.ok_or_else(|| {
            tracing::error!("Document not found for ID: {}", document_id);
            ProofError::DocumentNotFound
        })?;

        tracing::info!(
            "Document proof size: {} bytes",
            document_proof_data.grovedb_proof.len()
        );

        // Step 4: Get current state root from proof data
        let state_root = document_proof_data.root_hash;

        // Step 5: Create signing challenge
        let challenge = create_challenge(&state_root, contract_id, document_id);

        // Step 6: Sign the challenge with Ed25519
        let ed25519_sig = sign_challenge_ed25519(private_key, &challenge)?;

        // Step 7: Log proof information and save to files for testing
        tracing::info!(
            "Using separate proofs - identity: {} bytes, document: {} bytes",
            identity_proof_data.grovedb_proof.len(),
            document_proof_data.grovedb_proof.len()
        );

        // Save proofs to files for testing in grovestark
        let test_dir = std::path::Path::new("grovestark_test_proofs");
        if let Err(e) = std::fs::create_dir_all(&test_dir) {
            tracing::warn!("Failed to create test proof directory: {}", e);
        } else {
            // Save document proof
            let doc_proof_path = test_dir.join(format!("document_proof_{}.bin", document_id));
            if let Err(e) = std::fs::write(&doc_proof_path, &document_proof_data.grovedb_proof) {
                tracing::error!("Failed to save document proof to file: {}", e);
            } else {
                tracing::info!("Document proof saved to: {}", doc_proof_path.display());
            }

            // Save identity proof
            let id_proof_path = test_dir.join(format!("identity_proof_{}.bin", identity_id));
            if let Err(e) = std::fs::write(&id_proof_path, &identity_proof_data.grovedb_proof) {
                tracing::error!("Failed to save identity proof to file: {}", e);
            } else {
                tracing::info!("Identity proof saved to: {}", id_proof_path.display());
            }

            // Save proof metadata as JSON for reference
            let metadata = serde_json::json!({
                "identity_id": identity_id.to_string(),
                "contract_id": contract_id.to_string(),
                "document_id": document_id.to_string(),
                "document_type": document_type,
                "identity_proof_size": identity_proof_data.grovedb_proof.len(),
                "document_proof_size": document_proof_data.grovedb_proof.len(),
                "state_root": hex::encode(document_proof_data.root_hash),
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });

            let metadata_path = test_dir.join(format!("proof_metadata_{}.json", document_id));
            if let Err(e) = std::fs::write(
                &metadata_path,
                serde_json::to_string_pretty(&metadata).unwrap_or_default(),
            ) {
                tracing::error!("Failed to save proof metadata: {}", e);
            } else {
                tracing::info!("Proof metadata saved to: {}", metadata_path.display());
            }
        }

        // Debug: Log first few bytes of each proof to understand format
        if document_proof_data.grovedb_proof.len() >= 10 {
            let first_bytes: Vec<String> = document_proof_data.grovedb_proof[..10]
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            tracing::debug!("Document proof first 10 bytes: {}", first_bytes.join(" "));
        }

        if identity_proof_data.grovedb_proof.len() >= 10 {
            let first_bytes: Vec<String> = identity_proof_data.grovedb_proof[..10]
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            tracing::debug!("Identity proof first 10 bytes: {}", first_bytes.join(" "));
        }

        // Log full hex for easier debugging (only in debug mode)
        tracing::debug!(
            "Full document proof hex: {}",
            hex::encode(&document_proof_data.grovedb_proof)
        );
        tracing::debug!(
            "Full identity proof hex: {}",
            hex::encode(&identity_proof_data.grovedb_proof)
        );

        // Step 8: Use GroveSTARK's new SDK integration API
        // This accepts raw SDK proofs directly without needing special formatting
        tracing::info!("Creating witness with GroveSTARK SDK integration...");
        let witness = create_witness_from_sdk_proofs(
            &document_proof_data.grovedb_proof, // Raw document proof from SDK
            &identity_proof_data.grovedb_proof, // Raw identity proof from SDK
            serde_json::to_vec(&document)
                .map_err(|e| ProofError::SerializationError(e.to_string()))?,
            identity.id().to_buffer().to_vec(),
            &ed25519_sig.r,          // Signature R component
            &ed25519_sig.s,          // Signature s component
            &ed25519_sig.public_key, // Ed25519 public key
            &challenge,              // Message that was signed
            private_key,             // Private key
        )
        .map_err(|e| {
            tracing::error!("GroveSTARK witness creation failed: {:?}", e);
            ProofError::ProofGenerationFailed(format!(
                "GroveSTARK witness creation failed: {:?}",
                e
            ))
        })?;

        tracing::info!("Witness created successfully");

        // Step 8: Prepare public inputs
        let public_inputs = PublicInputs {
            state_root,
            contract_id: contract_identifier.to_buffer(),
            message_hash: challenge,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| ProofError::TimeError(e.to_string()))?
                .as_secs(),
        };

        // Step 9: Generate the STARK proof
        tracing::info!("Generating STARK proof (this may take 10-30 seconds)...");
        let proof = self
            .prover
            .prove(witness, public_inputs.clone())
            .map_err(|e| {
                tracing::error!("STARK proof generation failed: {}", e);
                ProofError::ProofGenerationFailed(e.to_string())
            })?;

        tracing::info!("STARK proof generated successfully");

        // Step 10: Serialize the proof
        let serialized_proof = serde_json::to_vec(&proof)
            .map_err(|e| ProofError::SerializationError(e.to_string()))?;

        let generation_time = start_time.elapsed();
        tracing::info!(
            "Total proof generation time: {:.2}s",
            generation_time.as_secs_f32()
        );

        Ok(ProofDataOutput {
            proof: serialized_proof.clone(),
            public_inputs: PublicInputsData {
                state_root: public_inputs.state_root,
                contract_id: public_inputs.contract_id,
                message_hash: public_inputs.message_hash,
                timestamp: public_inputs.timestamp,
            },
            metadata: ProofMetadata {
                created_at: public_inputs.timestamp,
                proof_size: serialized_proof.len(),
                generation_time_ms: generation_time.as_millis() as u64,
                security_level: 128, // Default security level
            },
        })
    }

    /// Verify a proof
    pub fn verify_proof(&self, proof_data: &ProofDataOutput) -> Result<bool, ProofError> {
        // Step 1: Deserialize the proof
        let stark_proof: STARKProof = serde_json::from_slice(&proof_data.proof)
            .map_err(|e| ProofError::DeserializationError(e.to_string()))?;

        // Step 2: Reconstruct public inputs
        let public_inputs = PublicInputs {
            state_root: proof_data.public_inputs.state_root,
            contract_id: proof_data.public_inputs.contract_id,
            message_hash: proof_data.public_inputs.message_hash,
            timestamp: proof_data.public_inputs.timestamp,
        };

        // Step 3: Verify the proof using GroveSTARK's verifier
        self.verifier
            .verify(&stark_proof, &public_inputs)
            .map_err(|e| ProofError::VerificationFailed(e.to_string()))
    }
}

impl ProofDataOutput {
    /// Serialize the proof to JSON string
    pub fn to_json_string(&self) -> Result<String, ProofError> {
        serde_json::to_string(self).map_err(|e| ProofError::SerializationError(e.to_string()))
    }

    /// Serialize the proof to base64-encoded JSON
    pub fn to_base64(&self) -> Result<String, ProofError> {
        let json_bytes =
            serde_json::to_vec(self).map_err(|e| ProofError::SerializationError(e.to_string()))?;
        Ok(base64::encode(json_bytes))
    }

    /// Deserialize from base64-encoded JSON
    pub fn from_base64(base64_str: &str) -> Result<Self, ProofError> {
        let bytes = base64::decode(base64_str)
            .map_err(|e| ProofError::DeserializationError(format!("Base64 decode error: {}", e)))?;
        serde_json::from_slice(&bytes).map_err(|e| ProofError::DeserializationError(e.to_string()))
    }

    /// Deserialize from JSON string
    pub fn from_json_string(json_str: &str) -> Result<Self, ProofError> {
        serde_json::from_str(json_str).map_err(|e| ProofError::DeserializationError(e.to_string()))
    }
}

/// Create a challenge message for signing
fn create_challenge(state_root: &[u8; 32], contract_id: &str, document_id: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(state_root);
    hasher.update(contract_id.as_bytes());
    hasher.update(document_id.as_bytes());

    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Sign a challenge with Ed25519 and extract signature components
fn sign_challenge_ed25519(
    private_key: &[u8; 32],
    message: &[u8; 32],
) -> Result<Ed25519SignatureData, ProofError> {
    // Create Ed25519 signing key from private key bytes
    let signing_key = SigningKey::from_bytes(private_key);
    let verifying_key = signing_key.verifying_key();

    // Sign the message
    let signature = signing_key.sign(message);

    // Extract R and s from the signature
    let sig_bytes = signature.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&sig_bytes[0..32]);
    s.copy_from_slice(&sig_bytes[32..64]);

    Ok(Ed25519SignatureData {
        r,
        s,
        public_key: *verifying_key.as_bytes(),
    })
}

#[derive(Debug)]
struct Ed25519SignatureData {
    r: [u8; 32],
    s: [u8; 32],
    public_key: [u8; 32],
}

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("Platform error: {0}")]
    Platform(String),

    #[error("Invalid identity ID: {0}")]
    InvalidIdentityId(String),

    #[error("Invalid contract ID: {0}")]
    InvalidContractId(String),

    #[error("Identity not found")]
    IdentityNotFound,

    #[error("Document not found")]
    DocumentNotFound,

    #[error("Private key not available")]
    PrivateKeyNotAvailable,

    #[error("Proof generation failed: {0}")]
    ProofGenerationFailed(String),

    #[error("Proof verification failed: {0}")]
    VerificationFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Signing failed: {0}")]
    SigningFailed(String),

    #[error("Invalid proof: {0}")]
    InvalidProof(String),

    #[error("Time error: {0}")]
    TimeError(String),
}
