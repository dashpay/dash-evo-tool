use core::{fmt::Debug, todo};
use dash_sdk::{
    dpp::{
        ProtocolError, dashcore::bip32::DerivationPath, identity::signer::Signer,
        platform_value::BinaryData,
    },
    platform::IdentityPublicKey,
};
use serde::{Deserialize, Serialize};
use std::ops::DerefMut;
use thiserror::Error;
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::kms::{
    KVStore, Kms, Secret, UnlockedKMS,
    json_store::{JsonStore, JsonStoreError},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd, Eq, Ord)]
pub enum KeyHandle {
    IdentityPublicKey(IdentityPublicKey),
    Derived {
        seed_hash: Vec<u8>,              // Hash of the seed to use to derive the key
        derivation_path: DerivationPath, // Derivation path for the key; TODO:
    },
}

/// Simple Key Management Service (KMS) implementation for managing wallet keys.
pub struct SimpleKms {
    store: JsonStore<KeyHandle, KeyRecord>,
}

#[derive(Serialize, Deserialize, Clone, ZeroizeOnDrop)]
enum KeyRecord {
    EncryptedPrivateKey(Vec<u8>), // Encrypted private key, serialized as bytes
    WalletSeed,                   // TODO: Move from DET wallet/mod.rs , use the ClosedWalletSeed
}

#[derive(Debug, Error)]
pub enum KmsError {
    #[error("Backing storage error: {0}")]
    StorageError(#[from] JsonStoreError),
    #[error("Encryption error: {0}")]
    EncryptionError(String),
    #[error("Signing error: {0}")]
    SigningError(String),
}

impl SimpleKms {
    /// Creates a new instance of `SimpleKms` that uses the specified path for storage.
    pub fn new(path: std::path::PathBuf) -> Result<Self, KmsError> {
        Ok(SimpleKms {
            store: JsonStore::new(path)?,
        })
    }
}

impl Kms for SimpleKms {
    type KeyHandle = KeyHandle;
    type Error = KmsError;

    /// Unlocks the KMS for operations that require access to private keys.
    fn unlock(
        &self,
        user_id: Vec<u8>,
        password: &Secret,
    ) -> Result<impl UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>, Self::Error>
    {
        Ok(SimpleUnlockedKms {
            kms: self,
            store: self.store.clone(),
            user_id,
            password: Zeroizing::new(password.clone()),
        })
    }

    fn get(&self, key: &Self::KeyHandle) -> Result<Option<super::PublicKey>, Self::Error> {
        todo!();
        // let record = self.store.get(key)?;
        // Ok(record.map(|_| IdentityPublicKey::default())) // Placeholder for actual key retrieval logic
    }

    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error> {
        let i = self.store.keys()?.into_iter();
        Ok(i)
    }
}
/// Derive encryption key used for encrypting and decrypting data in the JSON store.
fn derive_storage_key(
    user_id: Vec<u8>,
    password: &Secret,
) -> Result<Zeroizing<[u8; 32]>, KmsError> {
    use argon2::Argon2;

    let mut output_key_material = Zeroizing::new([0u8; 32]); // Can be any desired size
    Argon2::default()
        .hash_password_into(password, &user_id, output_key_material.deref_mut())
        .map_err(|e| {
            KmsError::EncryptionError(format!("Failed to derive encryption key: {}", e))
        })?;
    Ok(output_key_material)
}

/// SimpleUnlockedKms is an unlocked KMS that allows operations on keys without requiring a password.
pub struct SimpleUnlockedKms<'a> {
    kms: &'a SimpleKms,
    store: JsonStore<KeyHandle, KeyRecord>,
    user_id: Vec<u8>,
    password: Zeroizing<Secret>,
}

impl Debug for SimpleUnlockedKms<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleUnlockedKms")
            .field("user_id", &self.user_id)
            .field("store", &self.store)
            .finish()
    }
}

// Delegate Kms trait methods to SimpleUnlockedKms.kms
impl<'a> Kms for SimpleUnlockedKms<'a> {
    type KeyHandle = KeyHandle;
    type Error = KmsError;

    /// Unlocks the KMS for operations that require access to private keys.
    fn unlock(
        &self,
        user_id: Vec<u8>,
        password: &Secret,
    ) -> Result<impl UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>, Self::Error>
    {
        self.kms.unlock(user_id, password)
    }

    fn get(&self, key: &Self::KeyHandle) -> Result<Option<super::PublicKey>, Self::Error> {
        self.kms.get(key)
    }

    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error> {
        self.kms.keys()
    }
}

impl<'a> Signer for SimpleUnlockedKms<'a> {
    fn sign(
        &self,
        identity_public_key: &IdentityPublicKey,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        todo!();
    }

    fn can_sign_with(&self, identity_public_key: &IdentityPublicKey) -> bool {
        todo!();
    }
}

impl<'a> UnlockedKMS for SimpleUnlockedKms<'a> {
    fn decrypt(
        &self,
        key: &Self::KeyHandle,
        encrypted_data: &super::EncryptedData,
    ) -> Result<super::PlainData, Self::Error> {
        todo!();
    }
    fn derive_key_pair(&self, seed: &[u8]) -> Result<Self::KeyHandle, Self::Error> {
        todo!();
    }
    fn export(&self, encryption_key: Secret) -> Result<Vec<u8>, Self::Error> {
        todo!();
    }
    fn generate_key_pair(&self) -> Result<Self::KeyHandle, Self::Error> {
        todo!();
    }
}
