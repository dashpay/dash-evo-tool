use crate::backend_task::BackendTaskSuccessResult;
use crate::proofs::grovestark_integration::{GroveStarkIntegration, ProofDataOutput};
use dash_sdk::Sdk;

pub async fn run_grovestark_task(
    task: ZKProofTask,
    sdk: &Sdk,
) -> Result<BackendTaskSuccessResult, String> {
    match task {
        ZKProofTask::GenerateProof {
            identity_id,
            contract_id,
            document_type,
            document_id,
            key_id,
            private_key,
            public_key,
        } => {
            let integration = GroveStarkIntegration::new();

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
        ZKProofTask::VerifyProof { proof_data } => {
            let integration = GroveStarkIntegration::new();

            match integration.verify_proof(&proof_data) {
                Ok(is_valid) => Ok(BackendTaskSuccessResult::VerifiedZKProof(
                    is_valid, proof_data,
                )),
                Err(e) => Err(format!("Failed to verify proof: {}", e)),
            }
        }
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
        public_key: [u8; 32],
    },
    VerifyProof {
        proof_data: ProofDataOutput,
    },
}
