use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::{Identity, KeyID};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

#[derive(Debug, Encode, Decode, PartialEq, Clone)]
pub enum IdentityType {
    User,
    Masternode,
    Evonode,
}

impl Display for IdentityType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityType::User => write!(f, "User"),
            IdentityType::Masternode => write!(f, "Masternode"),
            IdentityType::Evonode => write!(f, "Evonode"),
        }
    }
}

#[derive(Debug, Encode, Decode, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub enum EncryptedPrivateKeyTarget {
    PrivateKeyOnMainIdentity,
    PrivateKeyOnVoterIdentity,
    PrivateKeyOnOperatorIdentity,
}

#[derive(Debug, Encode, Decode, Clone)]
pub struct QualifiedIdentity {
    pub identity: Identity,
    pub associated_voter_identity: Option<(Identity, KeyID)>,
    pub associated_operator_identity: Option<(Identity, KeyID)>,
    pub associated_owner_key_id: Option<KeyID>,
    pub identity_type: IdentityType,
    pub alias: Option<String>,
    pub encrypted_private_keys: BTreeMap<(EncryptedPrivateKeyTarget, KeyID), Vec<u8>>,
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
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            encrypted_private_keys: Default::default(),
        }
    }
}
