use core::fmt::Debug;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    kms::{
        KVStore, Kms, PublicKey, UnlockedKMS,
        encryption::NONCE_SIZE,
        file_store::{FileStore, JsonStoreError},
        generic::{key_handle::KeyHandle, unlocked::GenericUnlockedKms},
    },
    secret::{Secret, SecretError},
};

/// Simple Key Management Service (KMS) implementation for managing wallet keys.
#[derive(Clone)]
pub struct GenericKms {
    store: FileStore<KeyHandle, StoredKeyRecord>,
}

impl Debug for GenericKms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KMSImpl")
            .field("store", &self.store)
            .finish()
    }
}

pub type Nonce = [u8; NONCE_SIZE];
#[derive(Serialize, Deserialize, Clone)]
pub(super) enum StoredKeyRecord {
    /// Represents a private key stored in the KMS. See [`KeyHandle::RawKey`].
    RawKey {
        encrypted_key: Vec<u8>,
        nonce: Nonce,
        public_key: Vec<u8>,
    },
    DerivationSeed {
        encrypted_seed: Vec<u8>,
        nonce: Nonce,
        seed_hash: [u8; 32],
        network: dash_sdk::dpp::dashcore::Network,
    },
    /// Represents a key derived from some seed.
    DerivedKey {
        derivation_path: DerivationPath,
        seed_hash: [u8; 32],
        public_key: Vec<u8>,
        network: dash_sdk::dpp::dashcore::Network,
    },
}
impl StoredKeyRecord {
    /// Verifies that the key record matches the provided key handle.
    pub fn verify_handle(&self, key_handle: &KeyHandle) -> Result<(), KmsError> {
        match (self, key_handle) {
            // PrivateKey record: verify the public key bytes match the derived public key from the private key
            (
                StoredKeyRecord::RawKey {
                    encrypted_key: _,
                    nonce: _,
                    public_key,
                },
                KeyHandle::RawKey(public_key_bytes, _typ),
            ) => {
                if !public_key.eq(public_key_bytes) {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "Public key mismatch: requested: {}, stored: {}",
                        hex::encode(public_key_bytes),
                        hex::encode(public_key),
                    )));
                }
            }
            // WalletSeed record: verify the seed_hash matches
            (
                StoredKeyRecord::DerivationSeed {
                    seed_hash,
                    network: stored_network,
                    ..
                },
                KeyHandle::DerivationSeed {
                    seed_hash: requested_seed_hash,
                    network,
                },
            ) => {
                if network != stored_network {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "Network mismatch: requested: {}, stored: {}",
                        network, stored_network,
                    )));
                }
                if seed_hash != requested_seed_hash {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "Seed hash mismatch: requested: {}, stored: {}",
                        hex::encode(requested_seed_hash),
                        hex::encode(seed_hash),
                    )));
                }
            }
            // DerivedKey record: verify the derivation path matches (if possible)
            (
                StoredKeyRecord::DerivedKey {
                    derivation_path,
                    seed_hash,
                    ..
                },
                KeyHandle::Derived {
                    derivation_path: requested_derivation_path,
                    seed_hash: requested_seed_hash,
                    ..
                },
            ) => {
                if !derivation_path.eq(requested_derivation_path) {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "Derivation path mismatch: requested: {}, stored: {}",
                        requested_derivation_path, derivation_path,
                    )));
                }

                if seed_hash != requested_seed_hash {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "Seed hash mismatch: requested: {}, stored: {}",
                        hex::encode(requested_seed_hash),
                        hex::encode(seed_hash),
                    )));
                }
            }
            _ => {
                return Err(KmsError::KeyRecordNotSupported(
                    "KeyRecord does not match KeyHandle".to_string(),
                ));
            }
        };

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum KmsError {
    #[error("Key not supported: {0}")]
    KeyRecordNotSupported(String),

    #[error("Backing storage error: {0}")]
    StorageError(String),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("Key generation error: {0}")]
    KeyGenerationError(String),

    #[error("Key integrity error: {0}")]
    KeyIntegrityError(String),

    #[error("Cannot determine storage encryption key: {0}")]
    StorageKeyError(String),

    #[error("Invalid username or password")]
    InvalidCredentials,

    #[error("Signing error: {0}")]
    SigningError(String),

    #[error("Error while manipulating secrets: {0}")]
    SecretError(#[from] SecretError),

    #[error("Private key not found for key handle {0}")]
    PrivateKeyNotFound(KeyHandle),
}

impl From<JsonStoreError> for KmsError {
    fn from(err: JsonStoreError) -> Self {
        KmsError::StorageError(err.to_string())
    }
}

impl GenericKms {
    /// Creates a new instance of `SimpleKms` that uses the specified path for storage.
    pub fn new(path: &std::path::Path) -> Result<Self, KmsError> {
        Ok(GenericKms {
            store: FileStore::new(path)?,
        })
    }
}

impl Kms for GenericKms {
    type KeyHandle = KeyHandle;
    type Error = KmsError;

    /// Unlocks the KMS for operations that require access to private keys.
    fn unlock(
        &self,
        user_id: &[u8],
        password: Secret,
    ) -> Result<impl UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>, Self::Error>
    {
        GenericUnlockedKms::new(self, self.store.clone(), user_id, password)
    }

    fn public_key(&self, key_handle: &Self::KeyHandle) -> Result<Option<PublicKey>, Self::Error> {
        // we need to retrieve the record from the store, even if we have pubkey inside the key handle
        // because we need to check if the key exists in the store
        let Some(record) = self.store.get(key_handle)? else {
            return Ok(None);
        };

        // some checks
        record.verify_handle(key_handle)?;

        let pubkey = match key_handle {
            KeyHandle::RawKey(public_key_bytes, _typ) => {
                // This is a public key, we can return it directly
                public_key_bytes.clone()
            }
            KeyHandle::Derived { .. } => {
                // For derived keys, we need to get the public key from the stored record
                match record {
                    StoredKeyRecord::DerivedKey { public_key, .. } => public_key.clone(),
                    _ => {
                        return Err(KmsError::KeyRecordNotSupported(format!(
                            "Unexpected key record type retrieved for handle {}",
                            key_handle
                        )));
                    }
                }
            }
            KeyHandle::DerivationSeed { .. } => {
                return Err(KmsError::KeyRecordNotSupported(
                    "Derivation seed does not have a public key".to_string(),
                ));
            }
        };

        Ok(Some(pubkey))
    }

    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error> {
        let i = self.store.keys()?.into_iter();
        Ok(i)
    }
}
