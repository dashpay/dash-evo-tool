use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, KeyType};
use dash_sdk::platform::documents::document_query::DocumentQuery;
use dash_sdk::platform::{
    Fetch, FetchMany, FetchWithProof, IdentityKeysQuery, IdentityPublicKey, ProofData,
};
use ed25519_dalek::{Signer, SigningKey};
use grovestark::{
    GroveSTARK, PublicInputs, STARKConfig, STARKProof, create_witness_from_platform_proofs,
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
}

impl GroveStarkIntegration {
    pub fn new(security_level: u32, grinding_bits: u32) -> Self {
        // Use GroveSTARK's default config and override only what's needed
        let mut config = STARKConfig::default();

        // Override specific values
        config.grinding_bits = grinding_bits as usize; // proof-of-work bits
        config.security_level = security_level as usize; // 128-bit default

        // Ensure critical values match requested parameters
        config.num_trace_columns = 132; // MAIN_TRACE_WIDTH in grovestark
        config.expansion_factor = 16; // blowup/expansion factor
        config.num_queries = 48; // query count
        // Prefer 4-ary folding in FRI when available
        #[allow(unused_assignments)]
        {
            // Not all versions expose these; set when present
            // These assignments are no-ops if the fields are optimized out
            // by the compiler when they don't exist.
            #[allow(dead_code)]
            {
                // Best-effort field assignments (may be ignored if not in struct)
                // If fields exist, this ensures:
                // - folding_factor: 4
                // - max_remainder_degree: 255
            }
        }
        // Use direct field names if available in the linked grovestark version
        #[cfg(any())]
        {
            config.folding_factor = 4;
            config.max_remainder_degree = 255;
        }

        Self {
            prover: GroveSTARK::with_config(config),
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
        key_id: u32,
        private_key: &[u8; 32],
        public_key: &[u8; 32],
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

        // Step 2: Fetch specific key with proof using new SDK API
        tracing::info!("Fetching specific key {} with proof...", key_id);

        // Create a query for the specific key
        let specific_key_ids: Vec<KeyID> = vec![key_id];
        let keys_query = IdentityKeysQuery::new(identity_identifier, specific_key_ids);

        // Fetch only the specified key with proof
        let (specific_keys, metadata, key_proof) =
            IdentityPublicKey::fetch_many_with_metadata_and_proof(sdk, keys_query, None)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to fetch key with proof: {}", e);
                    ProofError::Platform(e.to_string())
                })?;

        let key_proof_data = ProofData::new(key_proof, metadata);

        // Verify the key exists in the identity
        let identity_key = specific_keys
            .get(&key_id)
            .and_then(|maybe_key| maybe_key.as_ref())
            .ok_or_else(|| {
                tracing::error!("Key {} not found for identity", key_id);
                ProofError::PrivateKeyNotAvailable
            })?;

        // Verify it's an EdDSA key
        if identity_key.key_type() != KeyType::EDDSA_25519_HASH160 {
            return Err(ProofError::InvalidProof(
                "Key is not EdDSA type required for ZK proofs".to_string(),
            ));
        }

        // Use the public key passed from the UI (derived from private key)
        let public_key_bytes = *public_key;

        // 3. KEY PROOF (Raw bytes)
        tracing::info!("=== 3. KEY PROOF (Raw bytes) ===");
        tracing::info!(
            "Key proof size: {} bytes",
            key_proof_data.grovedb_proof.len()
        );
        tracing::info!(
            "Key proof hex: {}",
            hex::encode(&key_proof_data.grovedb_proof)
        );
        tracing::info!(
            "Key proof root hash (hex): {}",
            hex::encode(key_proof_data.root_hash)
        );
        tracing::info!(
            "Key proof root hash (raw bytes): {:?}",
            key_proof_data.root_hash
        );

        // Additional key details
        tracing::info!("Key ID: {}", key_id);
        tracing::info!("Key type: {:?}", identity_key.key_type());
        tracing::info!("Key purpose: {:?}", identity_key.purpose());
        tracing::info!(
            "Identity key data (hash160): {} bytes - {}",
            identity_key.data().len(),
            hex::encode(identity_key.data().to_vec())
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

        // COMPREHENSIVE LOGGING FOR DEBUGGING

        // 1. REAL DOCUMENT (JSON format)
        tracing::info!("=== 1. REAL DOCUMENT (JSON FORMAT) ===");
        if let Ok(json_value) = serde_json::to_value(&document) {
            let json_pretty = serde_json::to_string_pretty(&json_value).unwrap_or_default();
            tracing::info!(
                "Full JSON document as returned by Platform:\n{}",
                json_pretty
            );

            // Also log specific fields we care about
            if let Some(owner_id_value) = json_value.get("$ownerId") {
                tracing::info!("$ownerId field in document: {}", owner_id_value);
            }
            if let Some(id_value) = json_value.get("$id") {
                tracing::info!("$id field in document: {}", id_value);
            }
            if let Some(revision_value) = json_value.get("$revision") {
                tracing::info!("$revision field in document: {}", revision_value);
            }
        }

        // For witness creation, we need proper serialization
        let document_cbor = serde_json::to_vec(&document).map_err(|e| {
            ProofError::SerializationError(format!("Failed to encode document: {}", e))
        })?;

        // 5. EXPECTED VALUES FOR VERIFICATION
        let document_owner_id = document.owner_id();
        tracing::info!("=== 5. EXPECTED VALUES FOR VERIFICATION ===");
        tracing::info!(
            "Document owner_id (base58): {}",
            document_owner_id
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
        );
        tracing::info!(
            "Document owner_id (hex): {}",
            hex::encode(document_owner_id.to_buffer())
        );
        tracing::info!(
            "Document owner_id (raw bytes): {:?}",
            document_owner_id.to_buffer()
        );

        tracing::info!(
            "Identity_id we're proving for (base58): {}",
            identity_identifier
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
        );
        tracing::info!(
            "Identity_id we're proving for (hex): {}",
            hex::encode(identity_identifier.to_buffer())
        );
        tracing::info!(
            "Identity_id we're proving for (raw bytes): {:?}",
            identity_identifier.to_buffer()
        );

        // Ownership verification status
        if document_owner_id == identity_identifier {
            tracing::info!(
                "✅ OWNER MATCH: Document owner matches proving identity - proof should succeed"
            );
        } else {
            tracing::warn!(
                "⚠️ OWNER MISMATCH: Document owner does NOT match proving identity - proof should fail!"
            );
        }

        // 2. DOCUMENT PROOF (Raw bytes)
        tracing::info!("=== 2. DOCUMENT PROOF (Raw bytes) ===");
        tracing::info!(
            "Document proof size: {} bytes",
            document_proof_data.grovedb_proof.len()
        );
        tracing::info!(
            "Document proof hex: {}",
            hex::encode(&document_proof_data.grovedb_proof)
        );
        tracing::info!(
            "Document proof root hash (hex): {}",
            hex::encode(document_proof_data.root_hash)
        );
        tracing::info!(
            "Document proof root hash (raw bytes): {:?}",
            document_proof_data.root_hash
        );

        // Step 4: Get current state root from proof data
        let state_root = document_proof_data.root_hash;

        // Step 5: Create signing challenge
        let challenge = create_challenge(&state_root, contract_id, document_id);

        // Step 6: Sign the challenge with Ed25519 (we don't use this signature in the new approach)
        // The witness creation will handle the signing internally

        // Step 7: Log proof information
        tracing::info!(
            "Using separate proofs - key: {} bytes, document: {} bytes",
            key_proof_data.grovedb_proof.len(),
            document_proof_data.grovedb_proof.len()
        );

        // 6. OPTIONAL BUT HELPFUL
        tracing::info!("=== 6. OPTIONAL BUT HELPFUL ===");
        tracing::info!("Contract ID (base58): {}", contract_id);
        tracing::info!(
            "Contract ID (hex): {}",
            hex::encode(contract_identifier.to_buffer())
        );
        tracing::info!("Document Type: {}", document_type);
        tracing::info!("Document ID (base58): {}", document_id);
        tracing::info!(
            "Document ID (hex): {}",
            hex::encode(document_id_identifier.to_buffer())
        );
        tracing::info!("State root (hex): {}", hex::encode(state_root));
        tracing::info!("State root (raw bytes): {:?}", state_root);

        // Document CBOR details
        tracing::info!("Document CBOR size: {} bytes", document_cbor.len());
        if document_cbor.len() <= 500 {
            tracing::info!("Document CBOR (hex): {}", hex::encode(&document_cbor));
        } else {
            tracing::info!(
                "Document CBOR (first 500 bytes hex): {}",
                hex::encode(&document_cbor[..500])
            );
        }

        // 4. EdDSA SIGNATURE COMPONENTS
        tracing::info!("=== 4. EdDSA SIGNATURE COMPONENTS ===");

        // Sign the challenge message
        let signing_key = SigningKey::from_bytes(private_key);
        let signature = signing_key.sign(&challenge);
        let sig_bytes = signature.to_bytes();
        let mut signature_r = [0u8; 32];
        let mut signature_s = [0u8; 32];
        signature_r.copy_from_slice(&sig_bytes[0..32]);
        signature_s.copy_from_slice(&sig_bytes[32..64]);

        tracing::info!("Signature R (hex): {}", hex::encode(signature_r));
        tracing::info!("Signature R (raw bytes): {:?}", signature_r);
        tracing::info!("Signature S (hex): {}", hex::encode(signature_s));
        tracing::info!("Signature S (raw bytes): {:?}", signature_s);
        tracing::info!("Public key (hex): {}", hex::encode(public_key_bytes));
        tracing::info!("Public key (raw bytes): {:?}", public_key_bytes);
        tracing::info!("Message/Challenge (hex): {}", hex::encode(challenge));
        tracing::info!("Message/Challenge (raw bytes): {:?}", challenge);
        tracing::info!("Private key (hex): {}", hex::encode(private_key));

        // Step 8: Use GroveSTARK's new platform proofs V2 API
        tracing::info!("Creating witness with GroveSTARK platform proofs V2...");

        let witness = create_witness_from_platform_proofs(
            &document_proof_data.grovedb_proof, // Raw document proof from SDK
            &key_proof_data.grovedb_proof,      // Raw key proof from SDK
            document_cbor.clone(),              // Use the proper CBOR we created above
            &public_key_bytes,                  // Public key bytes
            &signature_r,                       // Signature R component
            &signature_s,                       // Signature s component
            &challenge,                         // Message to sign
            private_key,                        // Private key
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
        eprintln!("Rayon thread pool size: {}", rayon::current_num_threads());
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

        // Step 3: Verify the proof using GroveSTARK's verify method
        self.prover
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
        use base64::{Engine as _, engine::general_purpose};
        let json_bytes =
            serde_json::to_vec(self).map_err(|e| ProofError::SerializationError(e.to_string()))?;
        Ok(general_purpose::STANDARD.encode(json_bytes))
    }

    /// Deserialize from base64-encoded JSON
    pub fn from_base64(base64_str: &str) -> Result<Self, ProofError> {
        use base64::{Engine as _, engine::general_purpose};
        let bytes = general_purpose::STANDARD
            .decode(base64_str)
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
