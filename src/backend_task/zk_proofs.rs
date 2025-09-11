use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::proofs::grovestark_integration::{GroveStarkIntegration, ProofDataOutput};
use dash_sdk::Sdk;
use std::sync::Arc;

pub async fn process_zk_proof_task(
    task: BackendTask,
    sdk: &Sdk,
    _app_context: &Arc<AppContext>,
) -> Result<BackendTaskSuccessResult, String> {
    match task {
        BackendTask::ZKProofTask(zk_task) => match zk_task {
            ZKProofTask::GenerateProof {
                identity_id,
                contract_id,
                document_type,
                document_id,
                key_id,
                private_key,
                public_key,
                security_level,
                mut grinding_bits,
            } => {
                // Enforce minimum grinding bits of 16
                grinding_bits = grinding_bits.max(16);
                let integration = GroveStarkIntegration::new(security_level, grinding_bits);

                match integration
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
                    .await
                {
                    Ok(proof_data) => Ok(BackendTaskSuccessResult::GeneratedZKProof(proof_data)),
                    Err(e) => Err(format!("Failed to generate proof: {}", e)),
                }
            }
            ZKProofTask::VerifyProof {
                proof_data,
                security_level,
            } => {
                let integration = GroveStarkIntegration::new(security_level, 16);

                match integration.verify_proof(&proof_data) {
                    Ok(is_valid) => Ok(BackendTaskSuccessResult::VerifiedZKProof(
                        is_valid, proof_data,
                    )),
                    Err(e) => Err(format!("Failed to verify proof: {}", e)),
                }
            }
        },
        _ => Err("Invalid task type for ZK proof handler".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ZKProofTask {
    GenerateProof {
        identity_id: String,
        contract_id: String,
        document_type: String,
        document_id: String,
        key_id: u32,
        private_key: [u8; 32],
        public_key: [u8; 32], // Add public key from storage
        security_level: u32,
        grinding_bits: u32,
    },
    VerifyProof {
        proof_data: ProofDataOutput,
        security_level: u32,
    },
}
