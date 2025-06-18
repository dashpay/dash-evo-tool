use std::{collections::BTreeMap, ops::DerefMut};

use dash_sdk::{
    dapi_client::mock::Key, dpp::dashcore::bip32::DerivationPath, platform::IdentityPublicKey,
};
use serde::{Deserialize, Serialize};
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
        todo!("move code from DET");
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
