use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::{Wallet, WalletSeedHash};
use bincode::de::{BorrowDecoder, Decoder};
use bincode::enc::Encoder;
use bincode::error::{DecodeError, EncodeError};
use bincode::{BorrowDecode, Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, Purpose, SecurityLevel};
use dash_sdk::key_wallet::bip32::ChildNumber;
use dash_sdk::key_wallet::bip32::DerivationPath;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, RwLock};

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

#[derive(Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct KeyStorage {
    pub private_keys:
        BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>,
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>,
    ) -> Self {
        Self {
            private_keys: value,
        }
    }
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>,
    ) -> Self {
        Self {
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

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, WalletDerivationPath)>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<
            (PrivateKeyTarget, KeyID),
            (QualifiedIdentityPublicKey, WalletDerivationPath),
        >,
    ) -> Self {
        Self {
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
        }
    }
}

impl KeyStorage {
    // Allow dead_code: This method provides direct key access without password resolution,
    // useful for cases where keys are already decrypted or for debugging purposes
    #[allow(dead_code)]
    pub fn get(
        &self,
        key: &(PrivateKeyTarget, KeyID),
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
        key: &(PrivateKeyTarget, KeyID),
        wallets: &[Arc<RwLock<Wallet>>],
        network: Network,
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
                        let derived_key = Wallet::derive_private_key_in_arc_rw_lock_slice(
                            wallets,
                            *wallet_seed_hash,
                            derivation_path,
                            network,
                        )?
                        .ok_or(format!(
                            "Wallet for key at derivation path {} not present, we have {} wallets",
                            derivation_path,
                            wallets.len()
                        ))?;
                        // match qualified_identity_public_key_data
                        //     .identity_public_key
                        //     .security_level()
                        // {
                        //     SecurityLevel::MEDIUM => {
                        //         *private_key_data = PrivateKeyData::AlwaysClear(derived_key)
                        //     }
                        //     _ => *private_key_data = PrivateKeyData::Clear(derived_key),
                        // }
                        Ok((qualified_identity_public_key_data.clone(), derived_key))
                    }
                },
            )
            .transpose()
    }

    // Allow dead_code: This method provides access to raw private key data,
    // useful for inspecting key states and encryption status
    #[allow(dead_code)]
    pub fn get_private_key_data(&self, key: &(PrivateKeyTarget, KeyID)) -> Option<&PrivateKeyData> {
        self.private_keys
            .get(key)
            .map(|(_, private_key_data)| private_key_data)
    }

    // Allow dead_code: This method provides combined access to private key data and wallet info,
    // useful for advanced key management and wallet integration scenarios
    #[allow(dead_code)]
    pub fn get_private_key_data_and_wallet_info(
        &self,
        key: &(PrivateKeyTarget, KeyID),
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
        key: &(PrivateKeyTarget, KeyID),
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

    pub fn has(&self, key: &(PrivateKeyTarget, KeyID)) -> bool {
        self.private_keys.contains_key(key)
    }

    // Allow dead_code: This method returns all stored key identifiers,
    // useful for key enumeration and management operations
    #[allow(dead_code)]
    pub fn keys_set(&self) -> BTreeSet<(PrivateKeyTarget, KeyID)> {
        self.private_keys.keys().cloned().collect()
    }

    pub fn identity_public_keys(&self) -> Vec<(&PrivateKeyTarget, &QualifiedIdentityPublicKey)> {
        self.private_keys
            .iter()
            .map(|((target, _), (key, _))| (target, key))
            .collect()
    }

    /// Inserts an unencrypted key into `ClearKeyStorage`. Returns an error if the storage is closed.
    pub fn insert_non_encrypted(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
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
