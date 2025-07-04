use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use aes_gcm::{
    Aes256Gcm, AesGcm, Key, KeyInit, Nonce,
    aead::{AeadMutInPlace, OsRng},
    aes::cipher,
};
use argon2::MIN_SALT_LEN;
use bip39::rand::{RngCore, SeedableRng};
use dash_sdk::{
    dpp::{
        ProtocolError,
        dashcore::bip32::DerivationPath,
        identity::{KeyType, signer::Signer},
        platform_value::BinaryData,
        version::PlatformVersion,
    },
    platform::IdentityPublicKey,
};
use sha2::digest::generic_array::GenericArray;
use zeroize::{Zeroize, Zeroizing};

use crate::kms::{
    Digest, EncryptedData, Error, KVStore, Kms, PlainData, PublicKey, Secret, Signature,
    UnlockedKMS,
    encryption::{NONCE_SIZE, decrypt_message},
    file_store::{FileStore, JsonStoreError},
    generic_kms::{GenericKeyHandle, GenericKms, KeyRecord, KmsError},
};

/// AAD (Additional Authenticated Data) used for encrypting wallet seed.
/// This is used to ensure that the encrypted data can be verified and decrypted correctly.
pub(crate) const AAD: &[u8; 20] = b"dash_platform_wallet";

/// SimpleUnlockedKms is an unlocked KMS that allows operations on keys without requiring a password.
pub struct GenericUnlockedKms<'a> {
    kms: &'a GenericKms,
    store: FileStore<GenericKeyHandle, KeyRecord>,
    user_id: Vec<u8>,
    storage_key: Secret, // Derived key for encrypting/decrypting store
    platform_version: &'a PlatformVersion,
}
impl<'a> GenericUnlockedKms<'a> {
    pub(crate) fn new(
        kms: &'a GenericKms,
        store: FileStore<GenericKeyHandle, KeyRecord>,
        user_id: &[u8],
        password: Secret,
    ) -> Result<Self, KmsError> {
        let storage_key =
            derive_storage_key(user_id.to_vec(), &password).expect("Failed to derive storage key");

        let me = Self {
            kms,
            store,
            user_id: user_id.to_vec(),
            storage_key,
            platform_version: PlatformVersion::desired(),
        };

        // find any item in the store and try to decrypt it to verify the password
        if let Some(key) = me.store.keys()?.iter().next() {
            if let Some(record) = me.store.get(key)? {
                match record {
                    KeyRecord::EncryptedPrivateKey {
                        ref encrypted_key,
                        nonce,
                    } => me
                        .storage_decrypt(encrypted_key, &nonce)
                        .map_err(|e| KmsError::InvalidCredentials)?,
                    KeyRecord::WalletSeed => {
                        // Wallet seed is not encrypted, so we don't need to decrypt it
                        unimplemented!("Wallet seed decryption is not implemented yet");
                    }
                };
            };
        };

        Ok(me)
    }

    /// Decrypts the encrypted data using the derived storage key and nonce.
    fn storage_decrypt(
        &self,
        encrypted_data: &Vec<u8>,
        nonce: &[u8; NONCE_SIZE],
    ) -> Result<Secret, KmsError> {
        let key = self.storage_key()?;
        let mut cipher = Aes256Gcm::new(key);

        let mut ciphertext = Secret::new(encrypted_data.clone())?;
        let buffer = ciphertext.as_mut();
        // Decrypt the data
        cipher
            .decrypt_in_place(Nonce::from_slice(nonce), AAD, buffer)
            .map_err(|e| KmsError::DecryptionError(format!("Decryption failed: {}", e)))?;

        Ok(ciphertext)
    }

    /// Encrypts the message using AES-256-GCM.
    ///
    /// ## Returns
    ///
    /// Returns a tuple containing the encrypted message and the nonce used for encryption.
    ///
    /// ## Panics
    ///
    /// Panics if the encrypted data is bigger than 4096 bytes,
    /// which is the maximum size of a [`Secret`].
    pub fn storage_encrypt<'msg, 'aad>(
        &self,
        mut message: Secret,
    ) -> Result<(Vec<u8>, [u8; NONCE_SIZE]), KmsError> {
        // Generate a random nonce
        let mut nonce = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce);

        // Create cipher instance
        let key = self.storage_key()?;
        let mut cipher = Aes256Gcm::new(key);

        // Encrypt the message
        cipher
            .encrypt_in_place(Nonce::from_slice(&nonce), AAD, message.as_mut())
            .map_err(|e| KmsError::EncryptionError(format!("Encryption failed: {}", e)))?;

        // Now the message contains the ciphertext, which is safe to be retrieved as a vector.
        Ok(((message.as_ref() as &[u8]).to_vec(), nonce))
    }

    fn storage_key(&self) -> Result<&Key<Aes256Gcm>, KmsError> {
        if self.storage_key.len() != 32 {
            return Err(KmsError::StorageKeyError(format!(
                "Invalid storage key size, expected 32 bytes, got: {}",
                self.storage_key.len()
            )));
        }
        let key_bytes: &[u8; 32] = self.storage_key.as_ref();
        let key = GenericArray::from_slice(key_bytes);

        Ok(key)
    }
}

impl Debug for GenericUnlockedKms<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleUnlockedKms")
            .field("user_id", &self.user_id)
            .field("store", &self.store)
            .finish()
    }
}

// Delegate Kms trait methods to SimpleUnlockedKms.kms
impl Kms for GenericUnlockedKms<'_> {
    type KeyHandle = GenericKeyHandle;
    type Error = KmsError;

    /// Unlocks the KMS for operations that require access to private keys.
    fn unlock(
        &self,
        user_id: &[u8],
        password: Secret,
    ) -> Result<impl UnlockedKMS<KeyHandle = Self::KeyHandle, Error = Self::Error>, Self::Error>
    {
        self.kms.unlock(user_id, password)
    }

    fn public_key(&self, key: &Self::KeyHandle) -> Result<Option<PublicKey>, Self::Error> {
        self.kms.public_key(key)
    }

    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error> {
        self.kms.keys()
    }
}

impl Signer for GenericUnlockedKms<'_> {
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

impl<'a> UnlockedKMS for GenericUnlockedKms<'a> {
    fn sign(&self, key: &Self::KeyHandle, digest: &Digest) -> Result<Signature, Self::Error> {
        todo!();
    }

    fn decrypt(
        &self,
        key: &Self::KeyHandle,
        encrypted_data: &EncryptedData,
    ) -> Result<PlainData, Self::Error> {
        todo!();
    }

    /// Generates a new key pair.
    fn generate_key_pair(
        &mut self,
        key_type: KeyType,
        seed: Secret,
    ) -> Result<Self::KeyHandle, Self::Error> {
        if seed.len() != 32 {
            return Err(KmsError::KeyGenerationError(
                "Seed must be exactly 32 bytes long".to_string(),
            ));
        }

        let seed_bytes: &[u8; 32] = seed.as_ref();
        let mut rng = bip39::rand::prelude::StdRng::from_seed(*seed_bytes);
        // FIXME: private key should be put into mlocked buffer, not simply returned here
        let (pubkey, privkey) = key_type
            .random_public_and_private_key_data(&mut rng, self.platform_version)
            .map_err(|e| {
                KmsError::KeyGenerationError(format!("Failed to generate key pair: {}", e))
            })?;

        let handle = GenericKeyHandle::PublicKeyBytes(pubkey);
        let (encrypted_key, nonce) = self.storage_encrypt(Secret::new(privkey)?)?;

        let record = KeyRecord::EncryptedPrivateKey {
            encrypted_key,
            nonce,
        };

        self.store.set(handle.clone(), record)?;
        Ok(handle)
    }

    /// Derives a key pair from a given seed.
    fn derive_key_pair(
        &mut self,
        master_key: &Self::KeyHandle,
        path: &DerivationPath,
    ) -> Result<Self::KeyHandle, Self::Error> {
        todo!()
    }

    fn export(&self, encryption_key: Secret) -> Result<Vec<u8>, Self::Error> {
        todo!();
    }
}

/// Derive encryption key used for encrypting and decrypting data in the JSON store.
///
/// ## Arguments
///
/// * `user_id`: A vector of bytes representing the user ID; this is used as a salt for key derivation.
///   Cannot be empty.
/// * `password`: A `Secret` containing the user's password. As a special case, an empty password
///   is allowed, to allow stores without a password.
fn derive_storage_key(user_id: Vec<u8>, password: &Secret) -> Result<Secret, KmsError> {
    use argon2::Argon2;

    if user_id.is_empty() {
        return Err(KmsError::InvalidCredentials);
    }

    // Ensure that the salt is at least [MIN_SALT_LEN] bytes long
    let mut salt = user_id.clone();
    // special case for empty user_id
    if salt.is_empty() {
        salt = [0u8].repeat(MIN_SALT_LEN);
    }
    while salt.len() < MIN_SALT_LEN {
        salt.extend_from_slice(&user_id);
    }

    let mut output_key_material = Secret::new([0u8; 32])?; // Can be any desired size
    Argon2::default()
        .hash_password_into(password.as_ref(), &salt, output_key_material.as_mut())
        .map_err(|e| {
            KmsError::EncryptionError(format!("Failed to derive encryption key: {}", e))
        })?;

    Ok(output_key_material)
}

#[cfg(test)]
mod tests {
    use dash_sdk::dpp::dashcore::bip32::DerivationPath;

    use crate::kms::{Kms, UnlockedKMS, generic::locked::GenericKms};

    use super::*;

    #[test]
    fn test_unlock_kms() {
        let derivation_path = DerivationPath::master();
        let user_id = b"user123".to_vec();
        let password: Secret =
            Secret::new(b"securepassword".to_vec()).expect("Failed to create Secret");
        let seed = Secret::new([42u8; 32]).expect("Failed to generate seed"); // Example seed, should be securely generated in real use cases
        let kms_db_dir =
            tempfile::tempdir().expect("Failed to create temporary file for KMS database");
        let kms_db = kms_db_dir.path().join("wallet.json");
        let kms = GenericKms::new(&kms_db).expect("Failed to create KMS instance");
        let mut unlocked = kms
            .unlock(&user_id, password.clone())
            .expect("Failed to unlock KMS");
        // generate master key
        let master_key = unlocked
            .generate_key_pair(dash_sdk::dpp::identity::KeyType::EDDSA_25519_HASH160, seed)
            .expect("Failed to generate master key");
        // derive a key pair from the master key
        let derived_key = unlocked
            .derive_key_pair(&master_key, &derivation_path)
            .expect("Failed to derive key pair from master key");

        let derived_pubkey = kms
            .public_key(&derived_key)
            .expect("Failed to get derived key")
            .expect("Derived key should exist");

        // Now close and reopen the KMS, checking that the master key is still available
        drop(unlocked);
        drop(kms);

        let kms = GenericKms::new(&kms_db).expect("Failed to create KMS instance");
        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // let's check if we can find the master key
        unlocked
            .public_key(&master_key)
            .expect("Failed to get master key");
        // derive the same key again
        let derived_key_2 = unlocked
            .derive_key_pair(&master_key, &derivation_path)
            .expect("Failed to derive key pair from master key");
        let derived_pubkey_2 = kms
            .public_key(&derived_key_2)
            .expect("Failed to get derived key")
            .expect("Derived key should exist");

        // compare the keys
        assert_eq!(derived_key, derived_key_2);
        assert_eq!(derived_pubkey, derived_pubkey_2);
    }

    #[test]
    fn test_incorrect_password() {
        let seed = Secret::new(vec![42u8; 48]).expect("Failed to generate seed"); // Example seed, should be securely generated in real use cases
        let derivation_path = DerivationPath::master();
        let user_id = b"user123".to_vec();
        let password = Secret::new(b"securepassword".to_vec()).expect("Failed to create Secret");
        let kms_db_dir =
            tempfile::tempdir().expect("Failed to create temporary file for KMS database");
        let kms_db = kms_db_dir.path().join("wallet.json");
        let kms = GenericKms::new(&kms_db).expect("Failed to create KMS instance");
        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");
        // generate master key
        let master_key = unlocked
            .generate_key_pair(dash_sdk::dpp::identity::KeyType::EDDSA_25519_HASH160, seed)
            .expect("Failed to generate master key");
        // derive a key pair from the master key
        let derived_key = unlocked
            .derive_key_pair(&master_key, &derivation_path)
            .expect("Failed to derive key pair from master key");

        let _ = kms
            .public_key(&derived_key)
            .expect("Failed to get derived key")
            .expect("Derived key should exist");

        // Now close and reopen the KMS using wrong password
        drop(unlocked);

        let kms = GenericKms::new(&kms_db).expect("Failed to create KMS instance");
        let wrong_password =
            Secret::new(b"invalidpassword".to_vec()).expect("Failed to create Secret");
        // Attempt to unlock with the wrong password

        let _unlocked = kms
            .unlock(&user_id, wrong_password)
            .expect_err("Invalid password accepted");
    }
}
