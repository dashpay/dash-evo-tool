pub mod simple;

use std::fmt::Debug;

use dash_sdk::{
    dapi_client::mock::Key,
    dpp::identity::signer::Signer,
    platform::{types::identity::PublicKeyHash, IdentityPublicKey},
};

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

pub type Password = zeroize::Zeroizing<Vec<u8>>;
/// This trait defines the necessary methods for a KMS to handle key generation,
/// encryption, decryption, and key management operations.
pub trait Kms {
    type KeyHandle: Clone + Debug + TryFrom<IdentityPublicKey>;
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
    fn unlock(
        &self,
        password: &Password,
    ) -> Result<Box<dyn UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>>, Self::Error>;
}

/// This trait extends the Kms trait for operations that require access to private keys.
/// It provides methods for generating and deriving key pairs, decrypting data,
/// and signing data. This trait is intended to be used after the KMS has been unlocked.
///
/// It is important to ensure that dropping an instance of `UnlockedKMS` causes the KMS to be locked again,
/// and all sensitive data is cleared from memory to prevent unauthorized access.
pub trait UnlockedKMS: Kms + Signer {
    /// Generates a new key pair.
    fn generate_key_pair(&self) -> Result<Self::KeyHandle, Self::Error>;

    /// Derives a key pair from a given seed.
    fn derive_key_pair(&self, seed: &[u8]) -> Result<Self::KeyHandle, Self::Error>;

    /// Decrypts data using private key associated with the provided key ID.
    fn decrypt(
        &self,
        key: &Self::KeyHandle,
        encrypted_data: &EncryptedData,
    ) -> Result<PlainData, Self::Error>;
}
