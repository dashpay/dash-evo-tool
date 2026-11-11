//! Public immutable review and registration outcome data. No signer or secret is persisted.

use dash_sdk::dpp::data_contract::{
    accessors::{v0::DataContractV0Getters, v1::DataContractV1Getters},
    serialized_version::DataContractInSerializationFormat,
};
use dash_sdk::dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructureUntrusted;
use dash_sdk::dpp::{
    ProtocolError,
    dashcore::Network,
    version::{PlatformVersion, TryFromPlatformVersioned},
};
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use serde::{Deserialize, Serialize};

/// An exact contract and the current owner/key binding approved in Review.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRegistration {
    pub contract: DataContract,
    pub nonce: u64,
    pub key: IdentityPublicKey,
    pub network: Network,
    pub protocol_version: u32,
    pub alias: String,
    pub registration_cost: u64,
    pub owner_balance: u64,
}

/// Only confirmed acceptance or proven zero dispatches release the paid-registration lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationStatus {
    NotSubmitted,
    Submitted,
    Unknown,
    Confirmed,
    Conflict,
}

/// Versioned public recovery data, retained even if the originating screen closes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationRecord {
    pub attempt_id: [u8; 32],
    pub network: Network,
    pub owner: Identifier,
    pub contract_id: Identifier,
    pub token_id: Identifier,
    pub nonce: u64,
    pub key_id: u32,
    pub protocol_version: u32,
    pub contract_bytes: Vec<u8>,
    pub alias: String,
    pub status: RegistrationStatus,
}

impl RegistrationRecord {
    /// Whether another paid attempt remains unsafe.
    pub fn blocks_registration(&self) -> bool {
        matches!(
            self.status,
            RegistrationStatus::Submitted
                | RegistrationStatus::Unknown
                | RegistrationStatus::Conflict
        )
    }
    /// Decodes an already validated recovery payload with bounded allocation and its original version.
    pub fn contract(&self) -> Result<DataContract, ProtocolError> {
        DataContract::versioned_deserialize_untrusted(
            &self.contract_bytes,
            false,
            PlatformVersion::get(self.protocol_version)?,
        )
    }
    /// Acceptance is about every authored field, not just the deterministic ID.
    pub fn matches_contract(&self, accepted: &DataContract) -> Result<bool, ProtocolError> {
        let expected = self.contract()?;
        let version = PlatformVersion::get(self.protocol_version)?;
        let normalize =
            |contract: &DataContract| -> Result<DataContractInSerializationFormat, ProtocolError> {
                let mut shape = DataContractInSerializationFormat::try_from_platform_versioned(
                    contract, version,
                )?;
                if let DataContractInSerializationFormat::V1(ref mut data) = shape {
                    data.created_at = None;
                    data.updated_at = None;
                    data.created_at_block_height = None;
                    data.updated_at_block_height = None;
                    data.created_at_epoch = None;
                    data.updated_at_epoch = None;
                }
                Ok(shape)
            };
        Ok(accepted.id() == self.contract_id
            && accepted.owner_id() == self.owner
            && accepted.token_id(0) == Some(self.token_id)
            && normalize(&expected)? == normalize(accepted)?)
    }
}

/// A confirmed receipt remains a success when saving the local token cache fails.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistrationOutcome {
    pub record: RegistrationRecord,
    pub local_cache_saved: bool,
    pub journal_saved: bool,
}

/// Typed serialization failures preserve their original diagnostic cause.
#[derive(Debug, thiserror::Error)]
pub enum ReviewJsonError {
    #[error("The contract preview could not be generated. Prepare the review again.")]
    Protocol(#[source] Box<ProtocolError>),
    #[error("The contract JSON could not be generated. Prepare the review again.")]
    Json(#[source] serde_json::Error),
}

/// Stable version-explicit serialization used by review, signing and recovery.
pub fn review_json(
    contract: &DataContract,
    version: &PlatformVersion,
) -> Result<String, ReviewJsonError> {
    let shape = DataContractInSerializationFormat::try_from_platform_versioned(contract, version)
        .map_err(|source| ReviewJsonError::Protocol(Box::new(source)))?;
    serde_json::to_string_pretty(&shape).map_err(ReviewJsonError::Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::token_creator::{TokenDraft, contract::canonical_contract};
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Setters;
    use dash_sdk::dpp::serialization::PlatformSerializableWithPlatformVersion;

    fn fixture() -> (RegistrationRecord, DataContract) {
        let version = PlatformVersion::latest();
        let owner = Identifier::from([9; 32]);
        let params = TokenDraft::example(false)
            .parse(owner, Network::Testnet, version, 0)
            .unwrap();
        let contract = canonical_contract(owner, 8, params, version).unwrap();
        let record = RegistrationRecord {
            attempt_id: [1; 32],
            network: Network::Testnet,
            owner,
            contract_id: contract.id(),
            token_id: contract.token_id(0).unwrap(),
            nonce: 8,
            key_id: 1,
            protocol_version: version.protocol_version,
            contract_bytes: contract
                .serialize_to_bytes_with_platform_version(version)
                .unwrap(),
            alias: "Cloud".into(),
            status: RegistrationStatus::Unknown,
        };
        (record, contract)
    }
    #[test]
    fn token_registration_same_id_different_payload_is_a_conflict() {
        let (record, mut other) = fixture();
        other.keywords_mut().unwrap().push("different".into());
        assert!(!record.matches_contract(&other).unwrap());
        assert!(record.blocks_registration());
    }
    #[test]
    fn token_registration_acceptance_allows_only_server_creation_metadata() {
        let (record, mut accepted) = fixture();
        accepted.set_created_at(Some(123));
        accepted.set_created_at_block_height(Some(50));
        accepted.set_created_at_epoch(Some(1));
        assert!(record.matches_contract(&accepted).unwrap());
        assert_eq!(record.contract().unwrap().id(), accepted.id());
        assert_eq!(record.token_id, accepted.token_id(0).unwrap());
    }
}
