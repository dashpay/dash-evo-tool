use bincode::{Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::{signer, PubkeyHash};
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, Network, ScriptHash};
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::ed25519_dalek::Signer as EDDSASigner;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::identity::KeyType::{BIP13_SCRIPT_HASH, ECDSA_HASH160};
use dash_sdk::dpp::identity::{Identity, KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::BinaryData;
use dash_sdk::dpp::state_transition::errors::InvalidIdentityPublicKeyTypeError;
use dash_sdk::dpp::{bls_signatures, ed25519_dalek, ProtocolError};
use dash_sdk::platform::IdentityPublicKey;
use std::collections::{BTreeMap, HashSet};
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

impl From<Purpose> for EncryptedPrivateKeyTarget {
    fn from(value: Purpose) -> Self {
        match value {
            Purpose::VOTING => EncryptedPrivateKeyTarget::PrivateKeyOnVoterIdentity,
            _ => EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity,
        }
    }
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

impl Signer for QualifiedIdentity {
    fn sign(
        &self,
        identity_public_key: &IdentityPublicKey,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        let (_, private_key) = self
            .encrypted_private_keys
            .get(&(
                identity_public_key.purpose().into(),
                identity_public_key.id(),
            ))
            .ok_or(ProtocolError::Generic(format!(
                "{:?} not found in {:?}",
                identity_public_key, self
            )))?;
        match identity_public_key.key_type() {
            KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                let signature = signer::sign(data, private_key)?;
                Ok(signature.to_vec().into())
            }
            KeyType::BLS12_381 => {
                let pk =
                    bls_signatures::PrivateKey::from_bytes(private_key, false).map_err(|_e| {
                        ProtocolError::Generic(
                            "bls private key from bytes isn't correct".to_string(),
                        )
                    })?;
                Ok(pk.sign(data).to_bytes().to_vec().into())
            }
            KeyType::EDDSA_25519_HASH160 => {
                let key: [u8; 32] = private_key.clone().try_into().expect("expected 32 bytes");
                let pk = ed25519_dalek::SigningKey::try_from(&key).map_err(|_e| {
                    ProtocolError::Generic(
                        "eddsa 25519 private key from bytes isn't correct".to_string(),
                    )
                })?;
                Ok(pk.sign(data).to_vec().into())
            }
            // the default behavior from
            // https://github.com/dashevo/platform/blob/6b02b26e5cd3a7c877c5fdfe40c4a4385a8dda15/packages/js-dpp/lib/stateTransition/AbstractStateTransition.js#L187
            // is to return the error for the BIP13_SCRIPT_HASH
            KeyType::BIP13_SCRIPT_HASH => Err(ProtocolError::InvalidIdentityPublicKeyTypeError(
                InvalidIdentityPublicKeyTypeError::new(identity_public_key.key_type()),
            )),
        }
    }

    fn can_sign_with(&self, identity_public_key: &IdentityPublicKey) -> bool {
        self.encrypted_private_keys
            .get(&(
                identity_public_key.purpose().into(),
                identity_public_key.id(),
            ))
            .is_some()
    }
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

    pub fn masternode_payout_address(&self, network: Network) -> Option<Address> {
        self.identity
            .get_first_public_key_matching(
                Purpose::TRANSFER,
                [SecurityLevel::CRITICAL].into(),
                [ECDSA_HASH160, BIP13_SCRIPT_HASH].into(),
                false,
            )
            .and_then(|identity_public_key| {
                let key = identity_public_key.public_key_hash().ok()?;
                if identity_public_key.key_type() == BIP13_SCRIPT_HASH {
                    Some(Address::new(
                        network,
                        Payload::ScriptHash(ScriptHash::from_byte_array(key)),
                    ))
                } else {
                    Some(Address::new(
                        network,
                        Payload::PubkeyHash(PubkeyHash::from_byte_array(key)),
                    ))
                }
            })
    }

    pub fn can_sign_with_master_key(&self) -> Option<&IdentityPublicKey> {
        if self.identity_type != IdentityType::User {
            return None;
        }

        // Iterate through the encrypted private keys to check for a valid master key
        for ((target, _), (public_key, _)) in &self.encrypted_private_keys {
            if *target == EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity
                && public_key.purpose() == Purpose::AUTHENTICATION
                && public_key.security_level() == SecurityLevel::MASTER
            {
                return Some(public_key);
            }
        }

        None
    }

    pub fn document_signing_key(
        &self,
        document_type: &DocumentTypeRef,
    ) -> Option<&IdentityPublicKey> {
        self.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([document_type.security_level_requirement()]),
            HashSet::from(KeyType::all_key_types()),
            false,
        )
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
                    if target_type == &EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity {
                        if public_key.purpose() == Purpose::OWNER {
                            keys.push(public_key);
                        }
                        if public_key.purpose() == Purpose::TRANSFER {
                            keys.push(public_key);
                        }
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
