mod encryption;
pub mod file_store;
pub mod generic_kms;
pub mod generic_unlocked_kms;
use dash_sdk::{
    dpp::{
        dashcore::bip32::DerivationPath,
        identity::{KeyType, signer::Signer},
    },
    platform::IdentityPublicKey,
};
use std::{fmt::Debug, ptr::copy};
use zeroize::Zeroize;
mod wallet_seed;

use crate::{kms::generic_kms::GenericKeyHandle, secret::Secret};

/// Key Management Service (KMS) trait for managing cryptographic keys.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KMSError {
    #[error("Key generation failed")]
    KeyGenerationError(String),
    #[error("Encryption error: {0}")]
    EncryptionError(String),
    #[error("Decryption error: {0}")]
    DecryptionError(String),
    #[error("Signing error: {0}")]
    SigningError(String),
    #[error("Signature verification error: {0}")]
    SignatureVerificationError(String),
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub type EncryptedData = Vec<u8>;
pub type PlainData = Vec<u8>;
pub type Signature = Vec<u8>;
pub type Digest = Vec<u8>;

/// Represents a public key in the KMS context.
pub type PublicKey = IdentityPublicKey;

/// This trait defines the necessary methods for a KMS to handle key generation,
/// encryption, decryption, and key management operations.
pub trait Kms {
    type KeyHandle: Clone + Debug;
    type Error: Debug + std::error::Error;

    // /// Encrypts data using public key associated with the provided key ID.
    // fn encrypt(&self, key: Self::KeyHandle, data: &PlainData)
    //     -> Result<EncryptedData, Self::Error>;

    // /// Verifies a signature against the provided data and key.
    // fn verify_signature(
    //     &self,
    //     key: &Self::KeyHandle,
    //     digest: &Digest,
    //     signature: &Signature,
    // ) -> Result<bool, Self::Error>;

    /// Unlocks the KMS for operations that require access to private keys.
    ///
    /// Consumes the password to ensure it is not stored in memory after unlocking.
    fn unlock(
        &self,
        user_id: &[u8],
        password: Secret,
        // ) -> Result<Box<dyn UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>>, Self::Error>;
    ) -> Result<impl UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>, Self::Error>;

    /// List all keys managed by the KMS.
    ///
    /// Returns a vector of key handles representing the keys managed by the KMS.
    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error>;

    /// Retrieves a public key associated with the provided key handle.
    fn public_key(&self, key: &Self::KeyHandle) -> Result<Option<PublicKey>, Self::Error>;
}

/// This trait extends the Kms trait for operations that require access to private keys.
/// It provides methods for generating and deriving key pairs, decrypting data,
/// and signing data. This trait is intended to be used after the KMS has been unlocked.
///
/// It is important to ensure that dropping an instance of `UnlockedKMS` causes the KMS to be locked again,
/// and all sensitive data is cleared from memory to prevent unauthorized access.
pub trait UnlockedKMS: Kms + Signer {
    /// Generates a new key pair.
    fn generate_key_pair(
        &mut self,
        key_type: KeyType,
        seed: Secret,
    ) -> Result<Self::KeyHandle, Self::Error>;

    /// Derives a key pair for a given derivation path from a master key.
    fn derive_key_pair(
        &mut self,
        master_key: &Self::KeyHandle,
        path: &DerivationPath,
    ) -> Result<Self::KeyHandle, Self::Error>;

    /// Decrypts data using private key associated with the provided key ID.
    fn decrypt(
        &self,
        key: &Self::KeyHandle,
        encrypted_data: &EncryptedData,
    ) -> Result<PlainData, Self::Error>;

    fn sign(&self, key: &Self::KeyHandle, digest: &Digest) -> Result<Signature, Self::Error>;

    /// Exports a backup of the KMS.
    ///
    /// Backup should be encrypted and should not contain any sensitive data in plaintext.
    fn export(&self, encryption_key: Secret) -> Result<Vec<u8>, Self::Error>;
}

/// Generic Key-Value Store trait for backing storage of keys and their associated data.
pub trait KVStore<K: Clone + std::fmt::Debug, V: Clone> {
    type Error: std::error::Error + Send + Sync;

    /// Retrieves a value associated with the given key.
    fn get(&self, key: &K) -> Result<Option<V>, Self::Error>;

    /// Stores a key-value pair.
    fn set(&mut self, key: K, value: V) -> Result<(), Self::Error>;

    /// Removes a key-value pair.
    fn delete(&mut self, key: &K) -> Result<bool, Self::Error>;

    /// Lists all keys in the store.
    fn keys(&self) -> Result<Vec<K>, Self::Error>;

    /// Checks if a key exists in the store.
    fn contains_key(&self, key: &K) -> Result<bool, Self::Error>;

    /// Clears all key-value pairs from the store.
    fn clear(&mut self) -> Result<(), Self::Error>;
}
