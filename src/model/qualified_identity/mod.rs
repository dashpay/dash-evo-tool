pub mod encrypted_key_storage;
pub mod qualified_identity_public_key;

use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::{Wallet, WalletSeed, WalletSeedHash};
use bincode::{Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::{signer, PubkeyHash};
use dash_sdk::dpp::bls_signatures::{Bls12381G2Impl, SignatureSchemes};
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
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::BinaryData;
use dash_sdk::dpp::state_transition::errors::InvalidIdentityPublicKeyTypeError;
use dash_sdk::dpp::{bls_signatures, ed25519_dalek, ProtocolError};
use dash_sdk::platform::IdentityPublicKey;
use std::collections::{BTreeMap, HashSet};
use std::fmt::{Display, Formatter};
use std::sync::{Arc, RwLock};

#[derive(Debug, Encode, Decode, PartialEq, Clone, Copy)]
pub enum IdentityType {
    User,
    Masternode,
    Evonode,
}

impl IdentityType {
    pub fn vote_strength(&self) -> u64 {
        match self {
            IdentityType::User => 1,
            IdentityType::Masternode => 1,
            IdentityType::Evonode => 4,
        }
    }

    pub fn default_encoding(&self) -> Encoding {
        match self {
            IdentityType::User => Encoding::Base58,
            IdentityType::Masternode => Encoding::Hex,
            IdentityType::Evonode => Encoding::Hex,
        }
    }
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
pub enum PrivateKeyTarget {
    PrivateKeyOnMainIdentity,
    PrivateKeyOnVoterIdentity,
    PrivateKeyOnOperatorIdentity,
}

impl From<Purpose> for PrivateKeyTarget {
    fn from(value: Purpose) -> Self {
        match value {
            Purpose::VOTING => PrivateKeyTarget::PrivateKeyOnVoterIdentity,
            _ => PrivateKeyTarget::PrivateKeyOnMainIdentity,
        }
    }
}

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct DPNSNameInfo {
    pub name: String,
    pub acquired_at: u64,
}

#[derive(Debug, Clone)]
pub struct QualifiedIdentity {
    pub identity: Identity,
    pub associated_voter_identity: Option<(Identity, IdentityPublicKey)>,
    pub associated_operator_identity: Option<(Identity, IdentityPublicKey)>,
    pub associated_owner_key_id: Option<KeyID>,
    pub identity_type: IdentityType,
    pub alias: Option<String>,
    pub private_keys: KeyStorage,
    pub dpns_names: Vec<DPNSNameInfo>,
    pub associated_wallets: BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
    /// The index used to register the identity
    pub wallet_index: Option<u32>,
    pub top_ups: BTreeMap<u32, u32>,
}

impl PartialEq for QualifiedIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.associated_voter_identity == other.associated_voter_identity
            && self.associated_operator_identity == other.associated_operator_identity
            && self.associated_owner_key_id == other.associated_owner_key_id
            && self.identity_type == other.identity_type
            && self.wallet_index == other.wallet_index
            && self.alias == other.alias
            && self.private_keys == other.private_keys
            && self.dpns_names == other.dpns_names
        // `associated_wallets` is ignored in this comparison
    }
}

// Implement Encode manually for QualifiedIdentity, excluding decrypted_wallets
impl Encode for QualifiedIdentity {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        self.identity.encode(encoder)?;
        self.associated_voter_identity.encode(encoder)?;
        self.associated_operator_identity.encode(encoder)?;
        self.associated_owner_key_id.encode(encoder)?;
        self.identity_type.encode(encoder)?;
        self.alias.encode(encoder)?;
        self.private_keys.encode(encoder)?;
        self.dpns_names.encode(encoder)?;
        // `decrypted_wallets` is skipped
        Ok(())
    }
}

// Implement Decode manually for QualifiedIdentity, excluding decrypted_wallets
impl Decode for QualifiedIdentity {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        Ok(Self {
            identity: Identity::decode(decoder)?,
            associated_voter_identity: Option::<(Identity, IdentityPublicKey)>::decode(decoder)?,
            associated_operator_identity: Option::<(Identity, IdentityPublicKey)>::decode(decoder)?,
            associated_owner_key_id: Option::<KeyID>::decode(decoder)?,
            identity_type: IdentityType::decode(decoder)?,
            alias: Option::<String>::decode(decoder)?,
            private_keys: KeyStorage::decode(decoder)?,
            dpns_names: Vec::<DPNSNameInfo>::decode(decoder)?,
            associated_wallets: BTreeMap::new(), // Initialize with an empty vector
            wallet_index: None,
            top_ups: Default::default(),
        })
    }
}

impl Signer for QualifiedIdentity {
    fn sign(
        &self,
        identity_public_key: &IdentityPublicKey,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        let (_, private_key) = self
            .private_keys
            .get_resolve(
                &(
                    identity_public_key.purpose().into(),
                    identity_public_key.id(),
                ),
                self.associated_wallets
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .map_err(|e| ProtocolError::Generic(e))?
            .ok_or(ProtocolError::Generic(format!(
                "Key {} ({}) not found in identity {:?}",
                identity_public_key.id(),
                identity_public_key.purpose(),
                self.identity.id().to_string(Encoding::Base58)
            )))?;
        match identity_public_key.key_type() {
            KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                let signature = signer::sign(data, &private_key)?;
                Ok(signature.to_vec().into())
            }
            KeyType::BLS12_381 => {
                let pk = bls_signatures::SecretKey::<Bls12381G2Impl>::from_be_bytes(&private_key)
                    .into_option()
                    .ok_or(ProtocolError::Generic(
                        "bls private key from bytes isn't correct".to_string(),
                    ))?;
                Ok(pk
                    .sign(SignatureSchemes::Basic, data)?
                    .as_raw_value()
                    .to_compressed()
                    .to_vec()
                    .into())
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
        self.private_keys.has(&(
            identity_public_key.purpose().into(),
            identity_public_key.id(),
        ))
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

    pub fn display_string(&self) -> String {
        self.alias
            .clone()
            .unwrap_or(self.identity.id().to_string(Encoding::Base58))
    }

    pub fn display_short_string(&self) -> String {
        self.alias.clone().unwrap_or_else(|| {
            let id_str = self.identity.id().to_string(Encoding::Base58);
            id_str.chars().take(5).collect()
        })
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

    pub fn can_sign_with_master_key(&self) -> Option<&QualifiedIdentityPublicKey> {
        if self.identity_type != IdentityType::User {
            return None;
        }

        // Iterate through the encrypted private keys to check for a valid master key
        for (target, public_key) in self.private_keys.identity_public_keys() {
            if *target == PrivateKeyTarget::PrivateKeyOnMainIdentity
                && public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                && public_key.identity_public_key.security_level() == SecurityLevel::MASTER
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

    pub fn available_withdrawal_keys(&self) -> Vec<&QualifiedIdentityPublicKey> {
        let mut keys = vec![];

        // Check the main identity's public keys
        for (target, public_key) in self.private_keys.identity_public_keys() {
            match (self.identity_type, target) {
                (IdentityType::User, PrivateKeyTarget::PrivateKeyOnMainIdentity) => {
                    if public_key.identity_public_key.purpose() == Purpose::TRANSFER {
                        keys.push(public_key);
                    }
                }
                (IdentityType::Masternode | IdentityType::Evonode, target_type) => {
                    if target_type == &PrivateKeyTarget::PrivateKeyOnMainIdentity {
                        if public_key.identity_public_key.purpose() == Purpose::OWNER {
                            keys.push(public_key);
                        }
                        if public_key.identity_public_key.purpose() == Purpose::TRANSFER {
                            keys.push(public_key);
                        }
                    }
                }
                _ => {}
            }
        }

        keys
    }

    pub fn available_transfer_keys(&self) -> Vec<&QualifiedIdentityPublicKey> {
        let mut keys = vec![];

        // Check the main identity's public keys
        for (_, public_key) in self.private_keys.identity_public_keys() {
            if public_key.identity_public_key.purpose() == Purpose::TRANSFER {
                keys.push(public_key);
            }
        }

        keys
    }

    pub fn available_authentication_keys(&self) -> Vec<&QualifiedIdentityPublicKey> {
        let mut keys = vec![];

        // Check the main identity's public keys
        for (_, public_key) in self.private_keys.identity_public_keys() {
            if public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION {
                keys.push(public_key);
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
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: Default::default(),
        }
    }
}
