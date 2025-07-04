use core::{fmt::Debug, todo};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::ZeroizeOnDrop;

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
    store: FileStore<KeyHandle, KeyRecord>,
}

impl Debug for GenericKms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KMSImpl")
            .field("store", &self.store)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Clone, ZeroizeOnDrop)]
pub(super) enum KeyRecord {
    EncryptedPrivateKey {
        encrypted_key: Vec<u8>,
        nonce: [u8; NONCE_SIZE],
    }, // Encrypted private key, serialized as bytes, + nonce
    WalletSeed, // TODO: Move from DET wallet/mod.rs , use the ClosedWalletSeed
}

#[derive(Debug, Error)]
pub enum KmsError {
    #[error("Key not supported: {0}")]
    KeyRecordNotSupported(String),

    #[error("Backing storage error: {0}")]
    StorageError(#[from] JsonStoreError),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("Key generation error: {0}")]
    KeyGenerationError(String),

    #[error("Cannot determine storage encryption key: {0}")]
    StorageKeyError(String),

    #[error("Invalid username or password")]
    InvalidCredentials,

    #[error("Signing error: {0}")]
    SigningError(String),

    #[error("Error while manipulating secrets: {0}")]
    SecretError(#[from] SecretError),
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

    fn public_key(&self, key: &Self::KeyHandle) -> Result<Option<PublicKey>, Self::Error> {
        todo!();
        // let record = self.store.get(key)?;
        // Ok(record.map(|_| IdentityPublicKey::default())) // Placeholder for actual key retrieval logic
    }

    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error> {
        let i = self.store.keys()?.into_iter();
        Ok(i)
    }
}
