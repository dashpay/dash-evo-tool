use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose};
use dash_sdk::platform::IdentityPublicKey;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

#[derive(Debug, Encode, Decode, PartialEq, Clone, Copy)]
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

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct QualifiedIdentity {
    pub identity: Identity,
    pub associated_voter_identity: Option<(Identity, IdentityPublicKey)>,
    pub associated_operator_identity: Option<(Identity, IdentityPublicKey)>,
    pub associated_owner_key_id: Option<KeyID>,
    pub identity_type: IdentityType,
    pub alias: Option<String>,
    pub encrypted_private_keys:
        BTreeMap<(EncryptedPrivateKeyTarget, KeyID), (IdentityPublicKey, Vec<u8>)>,
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

    pub fn available_withdrawal_keys(&self) -> Vec<&IdentityPublicKey> {
        let mut keys = vec![];

        // Check the main identity's public keys
        for ((target, _), (public_key, _)) in &self.encrypted_private_keys {
            match (self.identity_type, target) {
                (IdentityType::User, EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity) => {
                    if public_key.purpose() == Purpose::TRANSFER {
                        keys.push(public_key);
                    }
                }
                (IdentityType::Masternode | IdentityType::Evonode, target_type) => {
                    match target_type {
                        EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity => {
                            if public_key.purpose() == Purpose::OWNER {
                                keys.push(public_key);
                            }
                        }
                        EncryptedPrivateKeyTarget::PrivateKeyOnVoterIdentity => {
                            if public_key.purpose() == Purpose::TRANSFER {
                                keys.push(public_key);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        keys
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
