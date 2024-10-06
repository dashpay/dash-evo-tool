use bincode::{Decode, Encode};
use dpp::identity::Identity;

#[derive(Encode, Decode)]
pub struct QualifiedIdentity {
    pub identity: Identity,
    pub alias: Option<String>,
    pub encrypted_voting_private_key: Option<Vec<u8>>,
    pub encrypted_owner_private_key: Option<Vec<u8>>,
}

impl QualifiedIdentity {
    /// Serializes the QualifiedIdentity to a vector of bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::encode_to_vec(self, bincode::config::standard())
            .expect("Failed to encode QualifiedIdentity")
    }

    /// Deserializes a QualifiedIdentity from a vector of bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::decode_from_slice(bytes, bincode::config::standard())
            .expect("Failed to decode QualifiedIdentity")
            .0
    }
}

impl From<Identity> for QualifiedIdentity {
    fn from(value: Identity) -> Self {
        QualifiedIdentity {
            identity: value,
            alias: None,
            encrypted_voting_private_key: None,
            encrypted_owner_private_key: None,
        }
    }
}
