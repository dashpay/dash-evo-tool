use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::model::grovestark_prover::{GroveSTARKProver, ProofDataOutput};
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;

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
            let (_, private_key) = identity
                .resolve_private_key_bytes(PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id)
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
