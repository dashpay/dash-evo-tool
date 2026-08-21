use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::model::grovestark_prover::{ProofDataOutput, ProofMetadata, PublicInputsData};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, KeyType};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::documents::document_query::DocumentQuery;
use dash_sdk::platform::{
    Document, DriveDocumentQuery, Fetch, FetchMany, IdentityKeysQuery, IdentityPublicKey,
};
use ed25519_dalek::{Signer, SigningKey};
use grovestark::{
    GroveSTARK, PublicInputs, STARKConfig, STARKProof, create_witness_from_platform_proofs,
};
use std::time::Instant;

pub async fn run_grovestark_task(
    task: GroveSTARKTask,
    sdk: &Sdk,
) -> Result<BackendTaskSuccessResult, TaskError> {
    match task {
        GroveSTARKTask::GenerateProof {
            identity,
            contract_id,
            document_type,
            document_id,
            key_id,
        } => {
            let identity_id = identity.identity.id().to_string(Encoding::Base58);

            // Resolve the signing key through the JIT chokepoint (no parked-seed
            // read), then derive its ed25519 public key. EDDSA_25519_HASH160
            // stores only the 20-byte hash on Platform, so the verifying key is
            // recovered from the resolved private key rather than read back.
            //
            // The key id is resolved to the identity's own published key first,
            // so a request naming a key this identity does not have fails here
            // rather than reaching the vault.
            //
            // TODO(grovestark-unpublished-key): resolve_private_key_bytes now
            // requires the key be published on the main identity; a
            // locally-added-but-not-yet-broadcast key (a normal state per
            // docs/ai-design/2026-07-30-key-placement-resolution/design.md §2)
            // fails here indistinguishably from "no such key".
            let signing_key = identity
                .identity
                .get_public_key_by_id(key_id)
                .ok_or(TaskError::WalletKeyLookupFailed)?;
            let (_, private_key) = identity
                .resolve_private_key_bytes(signing_key)
                .await?
                .ok_or(TaskError::WalletKeyLookupFailed)?;

            let public_key = {
                use dash_sdk::dpp::ed25519_dalek::SigningKey;
                let signing_key = SigningKey::from_bytes(&private_key);
                *signing_key.verifying_key().as_bytes()
            };

            let prover = GroveSTARKProver::new();

            let proof_data = prover
                .generate_proof(
                    sdk,
                    &identity_id,
                    &contract_id,
                    &document_type,
                    &document_id,
                    key_id,
                    &private_key,
                    &public_key,
                )
                .await?;

            Ok(BackendTaskSuccessResult::GeneratedZKProof(proof_data))
        }
        GroveSTARKTask::VerifyProof { proof_data } => {
            let prover = GroveSTARKProver::new();

            let is_valid = prover.verify_proof(&proof_data)?;

            Ok(BackendTaskSuccessResult::VerifiedZKProof(
                is_valid, proof_data,
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroveSTARKTask {
    GenerateProof {
        // Boxed: `QualifiedIdentity` is large, and boxing it keeps the enum
        // (and the wrapping `BackendTask`) small.
        identity: Box<QualifiedIdentity>,
        contract_id: String,
        document_type: String,
        document_id: String,
        key_id: u32,
    },
    VerifyProof {
        proof_data: ProofDataOutput,
    },
}

struct GroveSTARKProver {
    prover: GroveSTARK,
}

impl GroveSTARKProver {
    fn new() -> Self {
        Self {
            prover: GroveSTARK::with_config(STARKConfig::default()),
        }
    }

    /// Generate a proof of document ownership.
    #[allow(clippy::too_many_arguments)]
    async fn generate_proof(
        &self,
        sdk: &Sdk,
        identity_id: &str,
        contract_id: &str,
        document_type: &str,
        document_id: &str,
        key_id: u32,
        private_key: &[u8; 32],
        public_key: &[u8; 32],
    ) -> Result<ProofDataOutput, GroveSTARKError> {
        if cfg!(debug_assertions) {
            return Err(GroveSTARKError::UnsupportedBuild);
        }

        let start_time = Instant::now();
        tracing::debug!(%identity_id, %contract_id, %document_type, %document_id, "Starting ZK proof generation");

        // Step 1: Parse identifiers.
        let identity_identifier = Identifier::from_string(identity_id, Encoding::Base58)
            .map_err(GroveSTARKError::InvalidIdentityId)?;
        let contract_identifier = Identifier::from_string(contract_id, Encoding::Base58)
            .map_err(GroveSTARKError::InvalidContractId)?;

        // Step 2: Fetch the specific key with proof.
        let specific_key_ids: Vec<KeyID> = vec![key_id];
        let keys_query = IdentityKeysQuery::new(identity_identifier, specific_key_ids);
        let (specific_keys, _metadata, key_proof) =
            IdentityPublicKey::fetch_many_with_metadata_and_proof(sdk, keys_query, None)
                .await
                .map_err(|e| GroveSTARKError::Platform(Box::new(e)))?;

        let identity_key = specific_keys
            .get(&key_id)
            .and_then(|maybe_key| maybe_key.as_ref())
            .ok_or(GroveSTARKError::PrivateKeyNotAvailable)?;

        if identity_key.key_type() != KeyType::EDDSA_25519_HASH160 {
            return Err(GroveSTARKError::NonEddsaKey);
        }

        let public_key_bytes = *public_key;

        // Step 3: Fetch the contract and build a document query.
        let contract = dash_sdk::platform::DataContract::fetch(sdk, contract_identifier)
            .await
            .map_err(|e| GroveSTARKError::Platform(Box::new(e)))?
            .ok_or(GroveSTARKError::ContractNotFound)?;

        let document_id_identifier = Identifier::from_string(document_id, Encoding::Base58)
            .map_err(GroveSTARKError::InvalidDocumentId)?;

        let query = DocumentQuery::new(contract, document_type)
            .map_err(|e| GroveSTARKError::Platform(Box::new(e.into())))?
            .with_document_id(&document_id_identifier);

        let (document_opt, _metadata, proof) =
            Document::fetch_with_metadata_and_proof(sdk, query.clone(), None)
                .await
                .map_err(|e| GroveSTARKError::Platform(Box::new(e)))?;

        let document = document_opt.ok_or(GroveSTARKError::DocumentNotFound)?;

        let document_cbor =
            serde_json::to_vec(&document).map_err(GroveSTARKError::Serialization)?;

        // Ownership check: a mismatch here means the proof will fail.
        if document.owner_id() != identity_identifier {
            tracing::warn!(
                "Document owner does not match the proving identity; proof is expected to fail"
            );
        }

        // Step 4: Recover the current state root from the document proof.
        let drive_document_query: DriveDocumentQuery =
            (&query)
                .try_into()
                .map_err(|e: dash_sdk::dash_platform_queries::Error| {
                    GroveSTARKError::Platform(Box::new(e.into()))
                })?;
        let (state_root, _documents) = drive_document_query
            .verify_proof(&proof.grovedb_proof, sdk.version())
            .map_err(|e| GroveSTARKError::Platform(Box::new(dash_sdk::Error::Drive(e))))?;

        // Step 5: Sign the challenge with Ed25519.
        let challenge = create_challenge(&state_root, contract_id, document_id);
        let signing_key = SigningKey::from_bytes(private_key);
        let sig_bytes = signing_key.sign(&challenge).to_bytes();
        let mut signature_r = [0u8; 32];
        let mut signature_s = [0u8; 32];
        signature_r.copy_from_slice(&sig_bytes[0..32]);
        signature_s.copy_from_slice(&sig_bytes[32..64]);

        // Step 6: Build the witness.
        let witness = create_witness_from_platform_proofs(
            &proof.grovedb_proof,
            &key_proof.grovedb_proof,
            document_cbor,
            &public_key_bytes,
            &signature_r,
            &signature_s,
            &challenge,
        )
        .map_err(|e| GroveSTARKError::WitnessCreation(format!("{e:?}")))?;

        // Step 7: Assemble public inputs and prove.
        let public_inputs = PublicInputs {
            state_root,
            contract_id: contract_identifier.to_buffer(),
            message_hash: challenge,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(GroveSTARKError::Time)?
                .as_secs(),
        };

        tracing::debug!(
            threads = rayon::current_num_threads(),
            "Generating STARK proof (normally ~10s)"
        );
        let proof = self
            .prover
            .prove(witness, public_inputs.clone())
            .map_err(|e| GroveSTARKError::ProofGeneration(e.to_string()))?;

        let serialized_proof =
            serde_json::to_vec(&proof).map_err(GroveSTARKError::Serialization)?;

        let generation_time = start_time.elapsed();
        tracing::debug!(
            elapsed_s = generation_time.as_secs_f32(),
            "STARK proof generated"
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
                security_level: 128,
            },
        })
    }

    /// Verify a previously generated proof.
    fn verify_proof(&self, proof_data: &ProofDataOutput) -> Result<bool, GroveSTARKError> {
        if cfg!(debug_assertions) {
            return Err(GroveSTARKError::UnsupportedBuild);
        }

        let stark_proof: STARKProof =
            serde_json::from_slice(&proof_data.proof).map_err(GroveSTARKError::Deserialization)?;

        let public_inputs = PublicInputs {
            state_root: proof_data.public_inputs.state_root,
            contract_id: proof_data.public_inputs.contract_id,
            message_hash: proof_data.public_inputs.message_hash,
            timestamp: proof_data.public_inputs.timestamp,
        };

        self.prover
            .verify(&stark_proof, &public_inputs)
            .map_err(|e| GroveSTARKError::Verification(e.to_string()))
    }
}

/// Bind the state root and document identity into a 32-byte signing challenge.
fn create_challenge(state_root: &[u8; 32], contract_id: &str, document_id: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(state_root);
    hasher.update(contract_id.as_bytes());
    hasher.update(document_id.as_bytes());

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hasher.finalize());
    hash
}

#[derive(Debug, thiserror::Error)]
pub enum GroveSTARKError {
    #[error("Could not reach Dash Platform. Please check your connection and try again.")]
    Platform(#[source] Box<dash_sdk::Error>),

    #[error("The identity ID is not valid. Please check it and try again.")]
    InvalidIdentityId(#[source] dash_sdk::dpp::platform_value::Error),

    #[error("The contract ID is not valid. Please check it and try again.")]
    InvalidContractId(#[source] dash_sdk::dpp::platform_value::Error),

    #[error("The document ID is not valid. Please check it and try again.")]
    InvalidDocumentId(#[source] dash_sdk::dpp::platform_value::Error),

    #[error("The contract could not be found. Please check the contract ID and try again.")]
    ContractNotFound,

    #[error("The document could not be found. Please check the document ID and try again.")]
    DocumentNotFound,

    #[error("The selected key cannot be used for this proof. Choose an EdDSA key and try again.")]
    NonEddsaKey,

    #[error("The signing key is not available. Unlock the wallet and try again.")]
    PrivateKeyNotAvailable,

    #[error("Could not build the proof witness. Please try again.")]
    WitnessCreation(String),

    #[error("Could not generate the proof. Please try again.")]
    ProofGeneration(String),

    #[error("Could not verify the proof.")]
    Verification(String),

    #[error("Could not read the proof data.")]
    Serialization(#[source] serde_json::Error),

    #[error("Could not read the proof data.")]
    Deserialization(#[source] serde_json::Error),

    #[error("The system clock is set incorrectly. Fix the clock and try again.")]
    Time(#[source] std::time::SystemTimeError),

    #[error("Proof generation requires a release build (run with cargo run --release).")]
    UnsupportedBuild,
}
