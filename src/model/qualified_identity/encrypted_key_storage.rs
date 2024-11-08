use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::PrivateKeyTarget;
use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, Purpose, SecurityLevel};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub enum KeyStorage {
    Open(ClearKeyStorage),
    Closed(ClosedKeyStorage),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrivateKeyData {
    Clear([u8; 32]),
    Encrypted(Vec<u8>),
}

impl fmt::Display for PrivateKeyData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivateKeyData::Clear(data) => {
                write!(f, "Clear({:?})", hex::encode(data))
            }
            PrivateKeyData::Encrypted(data) => {
                write!(f, "Encrypted({} bytes)", data.len())
            }
        }
    }
}

impl Default for KeyStorage {
    fn default() -> Self {
        Self::Closed(ClosedKeyStorage::default())
    }
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>,
    ) -> Self {
        Self::Open(ClearKeyStorage::from(value))
    }
}

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct ClearKeyStorage {
    pub private_keys: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>,
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>>
    for ClearKeyStorage
{
    fn from(
        value: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>,
    ) -> Self {
        Self {
            private_keys: value,
        }
    }
}

impl ClearKeyStorage {
    pub fn get(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<&(QualifiedIdentityPublicKey, [u8; 32])> {
        self.private_keys.get(key)
    }

    pub fn insert(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, [u8; 32]),
    ) {
        self.private_keys.insert(key, value);
    }
}

#[derive(Debug, Default, Encode, Decode, Clone, PartialEq)]
pub struct ClosedKeyStorage {
    pub encrypted_private_keys:
        BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, Vec<u8>)>,
}

impl ClosedKeyStorage {
    pub fn get(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<&(QualifiedIdentityPublicKey, Vec<u8>)> {
        self.encrypted_private_keys.get(key)
    }
    pub fn insert(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, Vec<u8>),
    ) {
        self.encrypted_private_keys.insert(key, value);
    }
}

impl KeyStorage {
    pub fn get(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Result<Option<&(QualifiedIdentityPublicKey, [u8; 32])>, String> {
        match self {
            KeyStorage::Open(open) => Ok(open.get(key)),
            KeyStorage::Closed(_) => Err("Key is encrypted, please enter password".to_string()),
        }
    }

    pub fn get_private_key_data(&self, key: &(PrivateKeyTarget, KeyID)) -> Option<PrivateKeyData> {
        match self {
            KeyStorage::Open(open) => open.get(key).map(|(_, k)| PrivateKeyData::Clear(*k)),
            KeyStorage::Closed(closed) => closed
                .get(key)
                .map(|(_, k)| PrivateKeyData::Encrypted(k.clone())),
        }
    }

    pub fn find_master_key(&self) -> Option<&QualifiedIdentityPublicKey> {
        match self {
            KeyStorage::Open(open) => open
                .private_keys
                .values()
                .find(|(public_key, _)| {
                    public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                        && public_key.identity_public_key.security_level() == SecurityLevel::MASTER
                })
                .map(|(public_key, _)| public_key),

            KeyStorage::Closed(closed) => closed
                .encrypted_private_keys
                .values()
                .find(|(public_key, _)| {
                    public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                        && public_key.identity_public_key.security_level() == SecurityLevel::MASTER
                })
                .map(|(public_key, _)| public_key),
        }
    }

    pub fn has(&self, key: &(PrivateKeyTarget, KeyID)) -> bool {
        match self {
            KeyStorage::Open(open) => open.private_keys.contains_key(key),
            KeyStorage::Closed(closed) => closed.encrypted_private_keys.contains_key(key),
        }
    }

    pub fn keys_set(&self) -> BTreeSet<(PrivateKeyTarget, KeyID)> {
        match self {
            KeyStorage::Open(open) => open.private_keys.keys().cloned().collect(),
            KeyStorage::Closed(closed) => closed.encrypted_private_keys.keys().cloned().collect(),
        }
    }

    pub fn identity_public_keys(&self) -> Vec<(&PrivateKeyTarget, &QualifiedIdentityPublicKey)> {
        match self {
            KeyStorage::Open(open) => open
                .private_keys
                .iter()
                .map(|((target, _), (key, _))| (target, key))
                .collect(),
            KeyStorage::Closed(closed) => closed
                .encrypted_private_keys
                .iter()
                .map(|((target, _), (key, _))| (target, key))
                .collect(),
        }
    }

    /// Inserts an unencrypted key into `ClearKeyStorage`. Returns an error if the storage is closed.
    pub fn insert_non_encrypted(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, [u8; 32]),
    ) -> Result<(), String> {
        match self {
            KeyStorage::Open(open) => {
                open.insert(key, value);
                Ok(())
            }
            KeyStorage::Closed(_) => {
                Err("Cannot insert non-encrypted key into closed storage".to_string())
            }
        }
    }

    /// Inserts an encrypted key into `ClosedKeyStorage`. Returns an error if the storage is open.
    pub fn insert_encrypted(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, Vec<u8>),
    ) -> Result<(), String> {
        match self {
            KeyStorage::Closed(closed) => {
                closed.insert(key, value);
                Ok(())
            }
            KeyStorage::Open(_) => Err("Cannot insert encrypted key into open storage".to_string()),
        }
    }
}
