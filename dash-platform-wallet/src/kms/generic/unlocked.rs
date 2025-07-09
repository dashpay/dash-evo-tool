use std::fmt::Debug;

use crate::kms::{
    Digest, EncryptedData, KVStore, KeyType, Kms, PlainData, PublicKey, Secret, Signature,
    UnlockedKMS,
    encryption::NONCE_SIZE,
    generic::{
        key_handle::KeyHandle,
        locked::{GenericKms, KmsError, StoredKeyRecord},
    },
};
use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{AeadMutInPlace, OsRng},
};
use argon2::MIN_SALT_LEN;
use bip39::rand::{RngCore, SeedableRng};
use dash_sdk::{
    dpp::{
        ProtocolError,
        dashcore::bip32::{DerivationPath, ExtendedPrivKey},
        identity::signer::Signer,
        platform_value::BinaryData,
        version::PlatformVersion,
    },
    platform::IdentityPublicKey,
};
use sha2::{Sha256, digest::generic_array::GenericArray};

/// AAD (Additional Authenticated Data) used for encrypting wallet seed.
/// This is used to ensure that the encrypted data can be verified and decrypted correctly.
pub(crate) const AAD: &[u8; 20] = b"dash_platform_wallet";

/// SimpleUnlockedKms is an unlocked KMS that allows operations on keys without requiring a password.
pub(super) struct GenericUnlockedKms<'a, S> {
    kms: &'a GenericKms,
    store: S,
    user_id: Vec<u8>,
    storage_key: Secret, // Derived key for encrypting/decrypting store
    platform_version: &'a PlatformVersion,
}
impl<'a, S: KVStore<KeyHandle, StoredKeyRecord>> GenericUnlockedKms<'a, S>
where
    KmsError: From<S::Error>,
{
    pub(crate) fn new(
        kms: &'a GenericKms,
        store: S,
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

        // Verify the password.
        me.verify_storage_key()?;

        Ok(me)
    }

    /// Get decrypted secret from the store for the given key handle, together with the original record
    fn get_from_store_with_metadata(
        &self,
        key: &KeyHandle,
    ) -> Result<Option<(Secret, StoredKeyRecord)>, KmsError> {
        let Some(record) = self.store.get(key)? else {
            return Ok(None);
        };

        let ciphertext = match &record {
            StoredKeyRecord::RawKey {
                encrypted_key,
                nonce,
                ..
            } => (encrypted_key, nonce),
            StoredKeyRecord::DerivationSeed {
                encrypted_seed,
                nonce,
                ..
            } => (encrypted_seed, nonce),
            StoredKeyRecord::DerivedKey { .. } => (&Vec::new(), &[0u8; NONCE_SIZE]),
            StoredKeyRecord::UserRecord {
                encrypted_master_key,
                nonce,
                ..
            } => (encrypted_master_key, nonce),
        };
        let secret = if !ciphertext.0.is_empty() {
            self.storage_decrypt(ciphertext.0, ciphertext.1)?
        } else {
            Secret::new([])?
        };

        record.verify_handle(key)?;

        Ok(Some((secret, record.clone())))
    }
    /// Verifies if the storage key works correctly.
    ///
    /// We do this by trying to decrypt any secret stored in the store.
    ///
    /// If the store is empty, it returns `Ok(())`.
    fn verify_storage_key(&self) -> Result<(), KmsError> {
        if let Some(handle) = self.store.keys()?.first() {
            // any error means that the password is incorrect, or we have some internal error
            self.get_from_store_with_metadata(handle).map_err(|e| {
                tracing::error!(
                    "cannot fetch key {:?} when trying to verify KMS password: {}",
                    handle,
                    e
                );
                KmsError::InvalidCredentials
            })?;
        };

        Ok(())
    }

    /// Get derived key pair for some handle.
    ///
    ///
    /// As we don't store derived private keys in the store,
    /// we need to derive the key pair from the master key every time
    /// we need it.
    ///
    /// ## Arguments
    ///
    /// * `handle`: The key handle for which to derive the key pair.
    ///   It MUST be a `KeyHandle::Derived` variant.
    fn get_derived_ecdsa_priv_key(
        &mut self,
        handle: &KeyHandle,
    ) -> Result<ExtendedPrivKey, KmsError> {
        let KeyHandle::Derived {
            derivation_path,
            network,
            ..
        } = handle
        else {
            return Err(KmsError::KeyRecordNotSupported(format!(
                "Cannot derive key from handle: {}",
                handle
            )));
        };

        let (seed, _) = self
            .get_from_store_with_metadata(handle)?
            .ok_or(KmsError::PrivateKeyNotFound(handle.clone()))?;

        // TODO: priv_key should be at least zeroized
        let priv_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(seed.as_ref(), *network)
            .map_err(|e| {
                KmsError::KeyGenerationError(format!(
                    "Failed to derive key pair from master key: {}",
                    e
                ))
            })?;
        Ok(priv_key)
    }

    /// Decrypts the encrypted data using the derived storage key and nonce.
    fn storage_decrypt(
        &self,
        encrypted_data: &[u8],
        nonce: &[u8; NONCE_SIZE],
    ) -> Result<Secret, KmsError> {
        use aes_gcm::KeyInit;
        let key = self.storage_key()?;

        let mut cipher = Aes256Gcm::new(key);

        let mut ciphertext = Secret::new(encrypted_data.to_vec())?;
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
    pub fn storage_encrypt(
        &self,
        mut message: Secret,
    ) -> Result<(Vec<u8>, [u8; NONCE_SIZE]), KmsError> {
        use aes_gcm::KeyInit;

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

impl<S: Debug> Debug for GenericUnlockedKms<'_, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleUnlockedKms")
            .field("user_id", &self.user_id)
            .field("store", &self.store)
            .finish()
    }
}

// Delegate Kms trait methods to SimpleUnlockedKms.kms
impl<S> Kms for GenericUnlockedKms<'_, S> {
    type KeyHandle = KeyHandle;
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

impl<S> Signer for GenericUnlockedKms<'_, S>
where
    S: KVStore<KeyHandle, StoredKeyRecord> + Debug + Send + Sync,
    KmsError: From<S::Error>,
{
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

impl<S> UnlockedKMS for GenericUnlockedKms<'_, S>
where
    S: KVStore<KeyHandle, StoredKeyRecord> + Debug + Send + Sync,
    KmsError: From<S::Error>,
{
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

        let (handle, record) = match key_type {
            KeyType::Raw {
                alogirhm: algorithm,
            } => {
                // FIXME: private key should be put into [Secret] or at least impl zeroize, not simply returned by a function
                let (pubkey, privkey) = algorithm
                    .random_public_and_private_key_data(&mut rng, self.platform_version)
                    .map_err(|e| {
                        KmsError::KeyGenerationError(format!("Failed to generate key pair: {}", e))
                    })?;

                let handle = KeyHandle::RawKey(pubkey.clone(), key_type);
                let (encrypted_key, nonce) = self.storage_encrypt(Secret::new(privkey)?)?;

                let record = StoredKeyRecord::RawKey {
                    encrypted_key,
                    nonce,
                    public_key: pubkey,
                };

                (handle, record)
            }
            KeyType::DerivationSeed { network } => {
                // Derived ECDSA keys are not generated here, they are derived from master keys
                let seed_hash = compute_seed_hash(&seed);
                // we need temporary handle to derive the key pair
                let handle = KeyHandle::DerivationSeed { seed_hash, network };

                let encrypted_seed = self.storage_encrypt(seed)?;
                let record = StoredKeyRecord::DerivationSeed {
                    encrypted_seed: encrypted_seed.0,
                    nonce: encrypted_seed.1,
                    seed_hash,
                    network,
                };

                (handle, record)
            }
        };

        self.store.set(handle.clone(), record)?;
        Ok(handle)
    }

    /// Derives a key pair from some master key using the provided derivation path
    /// and saves it to the store.
    ///
    /// Returns the handle of the derived key pair to be used in further operations.
    ///
    /// ## Arguments
    ///
    /// * `seed_handle`: Key handle of derivation seed to use; use [`UnlockedKMS::generate_key_pair`] to generate.
    /// * `key_type`: The type of key to derive; currently only [`KeyType::Raw`] is supported.
    /// * `path`: The derivation path to use for deriving the new key pair.
    fn derive_key_pair(
        &mut self,
        seed_handle: &Self::KeyHandle,
        key_type: KeyType,
        path: &DerivationPath,
    ) -> Result<Self::KeyHandle, Self::Error> {
        // parse the master key handle to get seed_hash and network
        let KeyHandle::DerivationSeed {
            seed_hash, network, ..
        } = seed_handle
        else {
            return Err(KmsError::KeyRecordNotSupported(format!(
                "Invalid derivation seed handle type: {}",
                seed_handle
            )));
        };

        let KeyType::Raw { alogirhm } = key_type else {
            return Err(KmsError::KeyRecordNotSupported(format!(
                "Invalid key type for deriving key pair: {:?}, only KeyType::Raw is supported",
                key_type
            )));
        };

        // ensure the wallet seed exists for this master key
        let (seed, _record) = self
            .get_from_store_with_metadata(seed_handle)?
            .ok_or(KmsError::PrivateKeyNotFound(seed_handle.clone()))?;

        // derive the actual key to get the public key
        match alogirhm {
            dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1 => {
                let extended_pub_key = path
                    .derive_pub_ecdsa_for_master_seed(seed.as_ref(), *network)
                    .map_err(|e| {
                        KmsError::KeyGenerationError(format!(
                            "Failed to derive key pair from seed: {}",
                            e
                        ))
                    })?;
                let public_key = extended_pub_key.to_pub().to_bytes();

                // now, define handle of the derived key
                let derived_key_handle = KeyHandle::Derived {
                    seed_hash: *seed_hash,
                    derivation_path: path.clone(),
                    network: *network,
                };

                let key_record = StoredKeyRecord::DerivedKey {
                    derivation_path: path.clone(),
                    seed_hash: *seed_hash,
                    public_key: public_key.to_vec(),
                    network: *network,
                };
                self.store.set(derived_key_handle.clone(), key_record)?;

                Ok(derived_key_handle)
            }
            _ => Err(KmsError::KeyRecordNotSupported(format!(
                "Unsupported key type for deriving key pair: {:?}",
                alogirhm
            ))),
        }
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

// Calulates the hash of the seed using SHA-256.
fn compute_seed_hash(seed: &Secret) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    hasher.update(seed);
    let result = hasher.finalize();
    let mut seed_hash = [0u8; 32];
    seed_hash.copy_from_slice(&result);
    seed_hash
}

#[cfg(test)]
mod tests {
    use dash_sdk::dpp::dashcore::{Network, bip32::DerivationPath};

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
        // generate seed key
        let seed = unlocked
            .generate_key_pair(
                super::KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                seed,
            )
            .expect("Failed to generate master key");
        // derive a key pair from the master key
        let derived_key = unlocked
            .derive_key_pair(&seed, KeyType::ecdsa_secp256k1(), &derivation_path)
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

        // let's check if we can find the master key by checking if it's in the keys list
        let keys: Vec<_> = kms.keys().expect("Failed to get keys").collect();
        assert!(
            keys.contains(&seed),
            "Master key should still exist in the store"
        );

        // derive the same key again
        let derived_key_2 = unlocked
            .derive_key_pair(&seed, KeyType::ecdsa_secp256k1(), &derivation_path)
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
        let seed = Secret::new(vec![42u8; 32]).expect("Failed to generate seed"); // Example seed, should be securely generated in real use cases
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
        let requested_key = KeyType::DerivationSeed {
            network: Network::Testnet,
        };
        let master_key = unlocked
            .generate_key_pair(requested_key, seed)
            .expect("Failed to generate master key");
        // derive a key pair from the master key
        let derived_key = unlocked
            .derive_key_pair(&master_key, KeyType::ecdsa_secp256k1(), &derivation_path)
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
