use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::wallet::kms::{Digest, EncryptedData, Kms, PlainData, UnlockedKMS};
use crate::model::wallet::{Wallet, WalletSeed, WalletSeedHash};
use bincode::de::{BorrowDecoder, Decoder};
use bincode::enc::Encoder;
use bincode::error::{DecodeError, EncodeError};
use bincode::{BorrowDecode, Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::bls_signatures::vsss_rs::elliptic_curve::bigint::Zero;
use dash_sdk::dpp::bls_signatures::{Bls12381G2Impl, SignatureSchemes};
use dash_sdk::dpp::dashcore::address::NetworkChecked;
use dash_sdk::dpp::dashcore::bip32::ChildNumber;
use dash_sdk::dpp::dashcore::secp256k1::hashes::hex::{Case, DisplayHex};
use dash_sdk::dpp::dashcore::{signer, Network};
use dash_sdk::dpp::ed25519_dalek::ed25519::signature::SignerMut;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::BinaryData;
use dash_sdk::dpp::state_transition::errors::{
    InvalidIdentityPublicKeyTypeError, InvalidSignaturePublicKeyError,
};
use dash_sdk::dpp::{bls_signatures, ed25519_dalek, ProtocolError};
use dash_sdk::platform::IdentityPublicKey;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, RwLock};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Clone, PartialEq)]
pub struct WalletDerivationPath {
    pub(crate) wallet_seed_hash: WalletSeedHash,
    pub(crate) derivation_path: DerivationPath,
}

impl Encode for WalletDerivationPath {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), EncodeError> {
        // Encode `wallet_seed_hash`
        self.wallet_seed_hash.encode(encoder)?;

        // Encode the length of the `DerivationPath`
        self.derivation_path.len().encode(encoder)?;

        // Encode each `ChildNumber` in the `DerivationPath`
        for child in &self.derivation_path {
            match child {
                ChildNumber::Normal { index } => {
                    0u8.encode(encoder)?; // Discriminant for Normal
                    index.encode(encoder)?;
                }
                ChildNumber::Hardened { index } => {
                    1u8.encode(encoder)?; // Discriminant for Hardened
                    index.encode(encoder)?;
                }
                ChildNumber::Normal256 { index } => {
                    2u8.encode(encoder)?; // Discriminant for Normal256
                    index.encode(encoder)?;
                }
                ChildNumber::Hardened256 { index } => {
                    3u8.encode(encoder)?; // Discriminant for Hardened256
                    index.encode(encoder)?;
                }
            }
        }

        Ok(())
    }
}

impl Decode for WalletDerivationPath {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, DecodeError> {
        // Decode `wallet_seed_hash`
        let wallet_seed_hash = WalletSeedHash::decode(decoder)?;

        // Decode the length of the `DerivationPath`
        let path_len = usize::decode(decoder)?;

        // Decode each `ChildNumber` in the `DerivationPath`
        let mut path = Vec::with_capacity(path_len);
        for _ in 0..path_len {
            let discriminant = u8::decode(decoder)?;
            let child_number = match discriminant {
                0 => ChildNumber::Normal {
                    index: u32::decode(decoder)?,
                },
                1 => ChildNumber::Hardened {
                    index: u32::decode(decoder)?,
                },
                2 => ChildNumber::Normal256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                3 => ChildNumber::Hardened256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                _ => return Err(DecodeError::OtherString("Invalid ChildNumber type".into())),
            };
            path.push(child_number);
        }

        let derivation_path = DerivationPath::from(path);
        Ok(Self {
            wallet_seed_hash,
            derivation_path,
        })
    }
}

impl<'de> BorrowDecode<'de> for WalletDerivationPath {
    fn borrow_decode<D: BorrowDecoder<'de>>(decoder: &mut D) -> Result<Self, DecodeError> {
        // Decode `wallet_seed_hash`
        let wallet_seed_hash = WalletSeedHash::decode(decoder)?;

        // Decode the length of the `DerivationPath`
        let path_len = usize::decode(decoder)?;

        // Decode each `ChildNumber` in the `DerivationPath`
        let mut path = Vec::with_capacity(path_len);
        for _ in 0..path_len {
            let discriminant = u8::decode(decoder)?;
            let child_number = match discriminant {
                0 => ChildNumber::Normal {
                    index: u32::decode(decoder)?,
                },
                1 => ChildNumber::Hardened {
                    index: u32::decode(decoder)?,
                },
                2 => ChildNumber::Normal256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                3 => ChildNumber::Hardened256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                _ => return Err(DecodeError::OtherString("Invalid ChildNumber type".into())),
            };
            path.push(child_number);
        }

        let derivation_path = DerivationPath::from(path);
        Ok(Self {
            wallet_seed_hash,
            derivation_path,
        })
    }
}

#[derive(Debug, Clone, Encode, Decode, PartialEq)]
pub enum PrivateKeyData {
    AlwaysClear([u8; 32]), // This is for keys that are MEDIUM security level
    Clear([u8; 32]),
    Encrypted(Vec<u8>),
    AtWalletDerivationPath(WalletDerivationPath),
}

impl Zeroize for PrivateKeyData {
    fn zeroize(&mut self) {
        match self {
            PrivateKeyData::AlwaysClear(data) | PrivateKeyData::Clear(data) => {
                data.zeroize();
            }
            PrivateKeyData::Encrypted(data) => {
                data.zeroize();
            }
            PrivateKeyData::AtWalletDerivationPath(_) => {}
        }
    }
}
impl Drop for PrivateKeyData {
    fn drop(&mut self) {
        self.zeroize();
    }
}
impl fmt::Display for PrivateKeyData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivateKeyData::Clear(data) => {
                write!(f, "Clear({})", hex::encode(data))
            }
            PrivateKeyData::Encrypted(data) => {
                write!(f, "Encrypted({} bytes)", data.len())
            }
            PrivateKeyData::AlwaysClear(data) => {
                write!(f, "Clear({})", hex::encode(data))
            }
            PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                wallet_seed_hash: wallet_seed,
                derivation_path,
            }) => {
                write!(
                    f,
                    "AtWalletDerivationPath({}/{})",
                    hex::encode(wallet_seed),
                    derivation_path
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyStorage {
    pub private_keys: BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, PrivateKeyData)>,
    seeds: BTreeMap<WalletSeedHash, WalletSeed>,
    pub network: Network,
}
// TODO: get rid of Encode/Decode, use own DB implementation
impl Encode for KeyStorage {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), EncodeError> {
        self.private_keys.encode(encoder)?;
        self.seeds.encode(encoder)?;
        self.network.magic().encode(encoder)?;

        Ok(())
    }
}
impl Decode for KeyStorage {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, DecodeError> {
        let private_keys = BTreeMap::decode(decoder)?;
        let seeds = BTreeMap::decode(decoder)?;
        let network_magic: u32 = u32::decode(decoder)?;
        let network = Network::from_magic(network_magic).ok_or_else(|| {
            DecodeError::OtherString(format!("Invalid network magic: {}", network_magic))
        })?;

        // let seeds = BTreeMap::decode(decoder)?;
        Ok(Self {
            private_keys,
            seeds,
            network,
        })
    }
}

impl From<BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, PrivateKeyData)>> for KeyStorage {
    fn from(value: BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, PrivateKeyData)>) -> Self {
        Self {
            private_keys: value,
            seeds: BTreeMap::new(), // TODO: we need to refactor key storage creation
            network: Network::Dash, // Default network, should be set properly later
        }
    }
}

impl From<BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, [u8; 32])>> for KeyStorage {
    fn from(value: BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, [u8; 32])>) -> Self {
        Self {
            seeds: BTreeMap::new(), // TODO: we need to refactor key storage creation
            network: Network::Dash, // Default network, should be set properly later
            private_keys: value
                .into_iter()
                .map(|(key, (qualified_identity_public_key, clear_key))| {
                    if qualified_identity_public_key
                        .identity_public_key
                        .security_level()
                        == SecurityLevel::MEDIUM
                    {
                        (
                            key,
                            (
                                qualified_identity_public_key,
                                PrivateKeyData::AlwaysClear(clear_key),
                            ),
                        )
                    } else {
                        (
                            key,
                            (
                                qualified_identity_public_key,
                                PrivateKeyData::Clear(clear_key),
                            ),
                        )
                    }
                })
                .collect(),
        }
    }
}

impl From<BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, WalletDerivationPath)>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<KeyIdentifier, (QualifiedIdentityPublicKey, WalletDerivationPath)>,
    ) -> Self {
        Self {
            network: Network::Dash, // Default network, should be set properly later
            private_keys: value
                .into_iter()
                .map(
                    |(key, (qualified_identity_public_key, wallet_derivation_path))| {
                        (
                            key,
                            (
                                qualified_identity_public_key,
                                PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                            ),
                        )
                    },
                )
                .collect(),
            seeds: BTreeMap::new(), // TODO: we need to refactor key storage creation
        }
    }
}

/// KeyHandle is an unique identifier for a key pair in the KeyStorage.
pub type KeyIdentifier = (PrivateKeyTarget, KeyID);

impl KeyStorage {
    // Allow dead_code: This method provides direct key access without password resolution,
    // useful for cases where keys are already decrypted or for debugging purposes
    #[allow(dead_code)]
    pub fn get(
        &self,
        key: &KeyIdentifier,
    ) -> Result<Option<(&QualifiedIdentityPublicKey, [u8; 32])>, String> {
        self.private_keys
            .get(key)
            .map(
                |(qualified_identity_public_key_data, private_key_data)| match private_key_data {
                    PrivateKeyData::AlwaysClear(clear) | PrivateKeyData::Clear(clear) => {
                        Ok((qualified_identity_public_key_data, *clear))
                    }
                    PrivateKeyData::Encrypted(_) => {
                        Err("Key is encrypted, please enter password".to_string())
                    }
                    PrivateKeyData::AtWalletDerivationPath(_) => {
                        Err("Key is not resolved, please enter password".to_string())
                    }
                },
            )
            .transpose()
    }

    pub fn get_resolve(
        &self,
        key: &KeyIdentifier,
    ) -> Result<Option<(QualifiedIdentityPublicKey, [u8; 32])>, String> {
        self.private_keys
            .get(key)
            .map(
                |(qualified_identity_public_key_data, private_key_data)| match private_key_data {
                    PrivateKeyData::AlwaysClear(clear) | PrivateKeyData::Clear(clear) => {
                        Ok((qualified_identity_public_key_data.clone(), *clear))
                    }
                    PrivateKeyData::Encrypted(_) => {
                        Err("Key is encrypted, please enter password".to_string())
                    }
                    PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                        wallet_seed_hash,
                        derivation_path,
                    }) => {
                        let seed = self.seeds.get(wallet_seed_hash).ok_or(format!(
                            "Wallet seed with hash {} not found",
                            wallet_seed_hash.to_hex_string(Case::Lower)
                        ))?;

                        let unlocked_seed = if let WalletSeed::Open(open) = seed {
                            open
                        } else {
                            return Err(format!(
                                "wallet seed with hash {} is not open",
                                wallet_seed_hash.to_hex_string(Case::Lower)
                            ));
                        };

                        let extended_private_key = derivation_path
                            .derive_priv_ecdsa_for_master_seed(&unlocked_seed.seed, self.network)
                            .map_err(|e| e.to_string())?;

                        Ok((
                            qualified_identity_public_key_data.clone(),
                            extended_private_key.private_key.secret_bytes(),
                        ))
                    }
                },
            )
            .transpose()
    }

    // Allow dead_code: This method provides access to raw private key data,
    // useful for inspecting key states and encryption status
    #[allow(dead_code)]
    pub fn get_private_key_data(&self, key: &KeyIdentifier) -> Option<&PrivateKeyData> {
        self.private_keys
            .get(key)
            .map(|(_, private_key_data)| private_key_data)
    }

    // Allow dead_code: This method provides combined access to private key data and wallet info,
    // useful for advanced key management and wallet integration scenarios
    #[allow(dead_code)]
    pub fn get_private_key_data_and_wallet_info(
        &self,
        key: &KeyIdentifier,
    ) -> Option<(&PrivateKeyData, &Option<WalletDerivationPath>)> {
        self.private_keys
            .get(key)
            .map(|(qualified_identity_public_key_data, private_key_data)| {
                (
                    private_key_data,
                    &qualified_identity_public_key_data.in_wallet_at_derivation_path,
                )
            })
    }

    pub fn get_cloned_private_key_data_and_wallet_info(
        &self,
        key: &KeyIdentifier,
    ) -> Option<(PrivateKeyData, Option<WalletDerivationPath>)> {
        self.private_keys
            .get(key)
            .map(|(qualified_identity_public_key_data, private_key_data)| {
                (
                    private_key_data.clone(),
                    qualified_identity_public_key_data
                        .in_wallet_at_derivation_path
                        .clone(),
                )
            })
    }

    pub fn find_master_key(&self) -> Option<&QualifiedIdentityPublicKey> {
        self.private_keys
            .values()
            .find(|(public_key, _)| {
                public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                    && public_key.identity_public_key.security_level() == SecurityLevel::MASTER
            })
            .map(|(public_key, _)| public_key)
    }

    pub fn has(&self, key: &KeyIdentifier) -> bool {
        self.private_keys.contains_key(key)
    }

    // Allow dead_code: This method returns all stored key identifiers,
    // useful for key enumeration and management operations
    #[allow(dead_code)]
    pub fn keys_set(&self) -> BTreeSet<KeyIdentifier> {
        self.private_keys.keys().cloned().collect()
    }

    pub fn identity_public_keys(&self) -> Vec<(&PrivateKeyTarget, &QualifiedIdentityPublicKey)> {
        self.private_keys
            .iter()
            .map(|((target, _), (key, _))| (target, key))
            .collect()
    }

    pub fn find_by_identity_public_key(
        &self,
        identity_public_key: &IdentityPublicKey,
    ) -> Option<KeyIdentifier> {
        self.private_keys
            .iter()
            .find_map(|((target, id), (key, _))| {
                if key.identity_public_key == *identity_public_key {
                    Some((target.clone(), *id))
                } else {
                    None
                }
            })
    }

    /// Inserts an unencrypted key into `ClearKeyStorage`. Returns an error if the storage is closed.
    pub fn insert_non_encrypted(
        &mut self,
        key: KeyIdentifier,
        value: (QualifiedIdentityPublicKey, [u8; 32]),
    ) {
        match value.0.identity_public_key.security_level() {
            SecurityLevel::MEDIUM => {
                self.private_keys
                    .insert(key, (value.0, PrivateKeyData::AlwaysClear(value.1)));
            }
            _ => {
                self.private_keys
                    .insert(key, (value.0, PrivateKeyData::Clear(value.1)));
            }
        }
    }
}

impl Kms for KeyStorage {
    type KeyHandle = KeyHandle;
    type Error = ProtocolError;

    // fn encrypt(
    //     &self,
    //     _key: Self::KeyHandle,
    //     _data: &PlainData,
    // ) -> Result<BinaryData, ProtocolError> {
    //     Err(ProtocolError::Generic(
    //         "KeyStorage does not support encryption".to_string(),
    //     ))
    // }

    // fn verify_signature(
    //     &self,
    //     _key: &Self::KeyHandle,
    //     _digest: &Digest,
    //     _signature: &[u8],
    // ) -> Result<bool, ProtocolError> {
    //     Err(ProtocolError::Generic(
    //         "KeyStorage does not support signature verification".to_string(),
    //     ))
    // }

    fn unlock(
        &self,
        _password: &zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<Box<dyn UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>>, Self::Error>
    {
        Err(ProtocolError::Generic(
            "KeyStorage does not support unlocking".to_string(),
        ))
    }
}

impl UnlockedKMS for KeyStorage {
    fn generate_key_pair(&self) -> Result<Self::KeyHandle, Self::Error> {
        Err(ProtocolError::Generic(
            "KeyStorage does not support key generation".to_string(),
        ))
    }

    fn derive_key_pair(&self, _seed: &[u8]) -> Result<Self::KeyHandle, ProtocolError> {
        Err(ProtocolError::Generic(
            "KeyStorage does not support key derivation".to_string(),
        ))
    }

    fn decrypt(
        &self,
        _key: &Self::KeyHandle,
        _encrypted_data: &EncryptedData,
    ) -> Result<PlainData, Self::Error> {
        Err(ProtocolError::Generic(
            "KeyStorage does not support decryption".to_string(),
        ))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct KeyHandle(pub KeyIdentifier);

impl From<KeyIdentifier> for KeyHandle {
    fn from(value: (PrivateKeyTarget, KeyID)) -> Self {
        Self(value)
    }
}

impl From<IdentityPublicKey> for KeyHandle {
    fn from(value: IdentityPublicKey) -> Self {
        Self((value.purpose().into(), value.id()))
    }
}

impl Signer for KeyStorage {
    fn sign(
        &self,
        identity_public_key: &IdentityPublicKey,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        let key_handle: <Self as Kms>::KeyHandle = self
            .find_by_identity_public_key(identity_public_key)
            .ok_or(ProtocolError::InvalidSignaturePublicKeyError(
                InvalidSignaturePublicKeyError::new(identity_public_key.data().to_vec()),
            ))?
            .into();

        let (_, private_key) = self
            .get_resolve(&(
                identity_public_key.purpose().into(),
                identity_public_key.id(),
            ))
            .map_err(ProtocolError::Generic)?
            .ok_or(ProtocolError::Generic(format!(
                "Key {} ({}) not found in identity",
                identity_public_key.id(),
                identity_public_key.purpose(),
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
                #[allow(clippy::useless_conversion)]
                let key: [u8; 32] = private_key.try_into().expect("expected 32 bytes");
                #[allow(clippy::unnecessary_fallible_conversions)]
                let mut pk = ed25519_dalek::SigningKey::try_from(&key).map_err(|_e| {
                    ProtocolError::Generic(
                        "eddsa 25519 private key from bytes isn't correct".to_string(),
                    )
                })?;
                pk.try_sign(data).map(|x| x.to_vec().into()).map_err(|e| {
                    ProtocolError::Generic(format!("Failed to sign with eddsa 25519: {}", e))
                })
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
        self.has(&(
            identity_public_key.purpose().into(),
            identity_public_key.id(),
        ))
    }
}
