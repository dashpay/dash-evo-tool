use std::fmt::Debug;

use crate::kms::{
    Digest, EncryptedData, KVStore, KeyType, Kms, PlainData, PublicKey, Secret, Signature,
    UnlockedKMS,
    generic::{
        NONCE_SIZE,
        key_handle::KeyHandle,
        locked::{GenericKms, KmsError, StoredKeyRecord},
    },
};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{AeadMutInPlace, OsRng},
};
use argon2::MIN_SALT_LEN;
use bip39::rand::{RngCore, SeedableRng};
use dash_sdk::{
    dpp::{
        ProtocolError,
        dashcore::bip32::{DerivationPath, ExtendedPrivKey},
        ed25519_dalek::Signer as EDDSASigner,
        identity::{
            identity_public_key::accessors::v0::IdentityPublicKeyGettersV0, signer::Signer,
        },
        platform_value::BinaryData,
        version::PlatformVersion,
    },
    platform::IdentityPublicKey,
};
use sha2::Sha256;

/// AAD (Additional Authenticated Data) used for encrypting wallet seed.
/// This is used to ensure that the encrypted data can be verified and decrypted correctly.
pub(crate) const AAD: &[u8; 20] = b"dash_platform_wallet";

/// SimpleUnlockedKms is an unlocked KMS that allows operations on keys without requiring a password.
pub struct GenericUnlockedKms<'a, S> {
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
        if store.keys()?.is_empty() {
            // If the store is empty, we need to create the first user with a new master key
            return Self::init(kms, store, user_id, password);
        }

        // Check password and decode user record
        let user_handle = KeyHandle::User(user_id.to_vec());
        if let Some(user_record) = store.get(&user_handle)? {
            // Multi-user mode: decrypt master key from user record
            let master_key =
                Self::decrypt_master_key_from_user_record(&user_record, user_id, &password)?;

            let me = Self {
                kms,
                store,
                user_id: user_id.to_vec(),
                storage_key: master_key,
                platform_version: PlatformVersion::desired(),
            };

            // Verify the master key works by trying to decrypt any existing key
            me.verify_master_key()?;

            Ok(me)
        } else {
            Err(KmsError::InvalidCredentials)
        }
    }
    /// Initialize the unlocked KMS with a new user and master key.
    fn init(
        kms: &'a GenericKms,
        store: S,
        user_id: &[u8],
        password: Secret,
    ) -> Result<Self, KmsError> {
        if user_id.is_empty() {
            return Err(KmsError::InvalidCredentials);
        }

        // Empty store: create the first user with a new master key
        let master_key = Self::generate_master_key()?;

        let mut me = Self {
            kms,
            store,
            user_id: user_id.to_vec(),
            storage_key: master_key,
            platform_version: PlatformVersion::desired(),
        };

        // Create the first user record
        let user_record = me.create_user_record(user_id, &password)?;

        // Try multi-user mode first
        let user_handle = KeyHandle::User(user_id.to_vec());
        me.store.set(user_handle, user_record)?;

        Ok(me)
    }

    /// Decrypt master key from user record
    fn decrypt_master_key_from_user_record(
        user_record: &StoredKeyRecord,
        user_id: &[u8],
        password: &Secret,
    ) -> Result<Secret, KmsError> {
        let StoredKeyRecord::UserRecord {
            encrypted_master_key,
            nonce,
            ..
        } = user_record
        else {
            return Err(KmsError::KeyIntegrityError(
                "Invalid user record".to_string(),
            ));
        };

        // Derive the user's storage key
        let user_storage_key = derive_storage_key(user_id.to_vec(), password)?;

        // Decrypt the master key - map decryption errors to InvalidCredentials
        let master_key =
            Self::storage_decrypt_with_key(&user_storage_key, encrypted_master_key, nonce)
                .map_err(|e| {
                    tracing::error!("Failed to decrypt master key for user: {}", e);
                    KmsError::InvalidCredentials
                })?;

        Ok(master_key)
    }

    /// Verify that the master key works by trying to decrypt any existing key
    ///
    /// If no keys are found, it assumes the master key is valid.
    fn verify_master_key(&self) -> Result<(), KmsError> {
        // Try to decrypt any non-user key to verify master key works; if it doesn't, it will return an error.
        if let Some(key_handle) = self.keys()?.next() {
            self.get_from_store_with_metadata(&key_handle)
                .map_err(|e| {
                    tracing::error!("Cannot decrypt key {:?} with master key: {}", key_handle, e);
                    KmsError::InvalidCredentials
                })?;
        }

        Ok(())
    }

    /// Static method to decrypt data with a specific key
    fn storage_decrypt_with_key(
        key: &Secret,
        encrypted_data: &[u8],
        nonce: &[u8; NONCE_SIZE],
    ) -> Result<Secret, KmsError> {
        use aes_gcm::KeyInit;
        let mut cipher = Aes256Gcm::new(key.as_ref());
        let nonce = Nonce::from_slice(nonce);

        // we don't need to zeroize encrypted_data since it's not a real secret, we just put it here to benefit
        // from _in_place functions
        let mut buf = Secret::new(encrypted_data.to_vec())?;
        cipher
            .decrypt_in_place(nonce, AAD, buf.as_mut())
            .map_err(|e| KmsError::DecryptionError(format!("Failed to decrypt data: {}", e)))?;

        Ok(buf)
    }

    /// Add a new user to the multi-user key store
    pub fn add_user(&mut self, user_id: &[u8], password: Secret) -> Result<(), KmsError> {
        if user_id.is_empty() {
            return Err(KmsError::InvalidCredentials);
        }

        // Check if user already exists
        let user_handle = KeyHandle::User(user_id.to_vec());
        if self.store.get(&user_handle)?.is_some() {
            return Err(KmsError::InvalidCredentials);
        }

        // Create user record with encrypted master key
        let user_record = self.create_user_record(user_id, &password)?;

        // Store the user record
        self.store.set(user_handle, user_record)?;

        Ok(())
    }

    /// Remove a user from the multi-user key store
    pub fn remove_user(&mut self, user_id: &[u8]) -> Result<(), KmsError> {
        if user_id.is_empty() {
            return Err(KmsError::InvalidCredentials);
        }

        let user_handle = KeyHandle::User(user_id.to_vec());
        if self.store.get(&user_handle)?.is_none() {
            return Err(KmsError::InvalidCredentials);
        }

        // Check if this is the last user
        let user_count = self
            .store
            .keys()?
            .iter()
            .filter(|h| matches!(h, KeyHandle::User(_)))
            .count();

        if user_count <= 1 {
            return Err(KmsError::CannotRemoveLastUser);
        }

        if !self.store.delete(&user_handle)? {
            // For now, return error since KVStore doesn't have delete method
            Err(KmsError::InvalidCredentials)
        } else {
            Ok(())
        }
    }

    /// Change password for an existing user
    pub fn change_user_password(
        &mut self,
        user_id: &[u8],
        new_password: Secret,
    ) -> Result<(), KmsError> {
        if user_id.is_empty() {
            return Err(KmsError::InvalidCredentials);
        }

        let user_handle = KeyHandle::User(user_id.to_vec());
        if self.store.get(&user_handle)?.is_none() {
            return Err(KmsError::InvalidCredentials);
        }

        // Create new user record with new password
        let new_user_record = self.create_user_record(user_id, &new_password)?;

        // Update the user record
        self.store.set(user_handle, new_user_record)?;

        Ok(())
    }

    /// List all users in the key store
    pub fn list_users(&self) -> Result<Vec<Vec<u8>>, KmsError> {
        let user_ids = self
            .store
            .keys()?
            .iter()
            .filter_map(|handle| match handle {
                KeyHandle::User(user_id) => Some(user_id.clone()),
                _ => None,
            })
            .collect();

        Ok(user_ids)
    }

    /// Create a user record with encrypted master key
    fn create_user_record(
        &self,
        user_id: &[u8],
        password: &Secret,
    ) -> Result<StoredKeyRecord, KmsError> {
        let nonce = {
            let mut nonce = [0u8; NONCE_SIZE];
            OsRng.fill_bytes(&mut nonce);
            nonce
        };

        // Derive the user's storage key
        let user_storage_key = derive_storage_key(user_id.to_vec(), password)?;

        // Encrypt the master key with the user's storage key
        let encrypted_master_key =
            Self::storage_encrypt_with_key(&user_storage_key, &self.storage_key, &nonce)?;

        let salt = {
            use bip39::rand::RngCore;
            let mut salt = vec![0u8; 32];
            OsRng.fill_bytes(&mut salt);
            salt
        };

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(StoredKeyRecord::UserRecord {
            user_id: user_id.to_vec(),
            encrypted_master_key,
            nonce,
            salt,
            created_at,
        })
    }

    /// Static method to encrypt data with a specific key
    fn storage_encrypt_with_key(
        key: &Secret,
        message: &Secret,
        nonce: &[u8; NONCE_SIZE],
    ) -> Result<Vec<u8>, KmsError> {
        use aes_gcm::KeyInit;

        if key.len() != 32 {
            return Err(KmsError::StorageKeyError(format!(
                "Invalid storage key size, expected 32 bytes, got: {}",
                key.len()
            )));
        }

        let mut cipher = Aes256Gcm::new(key.as_ref());
        let nonce = Nonce::from_slice(nonce);

        let mut buf = message.clone();
        cipher
            .encrypt_in_place(nonce, AAD, buf.as_mut())
            .map_err(|e| KmsError::EncryptionError(format!("Failed to encrypt data: {}", e)))?;

        // message contains the encrypted data now, so it's safe to return it as a Vec<u8>
        Ok(buf.to_vec())
    }

    /// Generate a new master key
    fn generate_master_key() -> Result<Secret, KmsError> {
        use bip39::rand::RngCore;

        let mut master_key = Secret::new([0u8; 32])?;
        OsRng.fill_bytes(master_key.as_mut());

        Ok(master_key)
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
    fn get_derived_ecdsa_priv_key(&self, handle: &KeyHandle) -> Result<ExtendedPrivKey, KmsError> {
        let KeyHandle::Derived {
            derivation_path,
            network,
            seed_hash,
            ..
        } = handle
        else {
            return Err(KmsError::KeyRecordNotSupported(format!(
                "Cannot derive key from handle: {}",
                handle
            )));
        };

        // Find the derivation seed handle that matches this derived key
        let seed_handle = KeyHandle::DerivationSeed {
            seed_hash: *seed_hash,
            network: *network,
        };

        let (seed, _) = self
            .get_from_store_with_metadata(&seed_handle)?
            .ok_or(KmsError::PrivateKeyNotFound(seed_handle.clone()))?;

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
        Self::storage_decrypt_with_key(&self.storage_key, encrypted_data, nonce)
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
        let key = &self.storage_key;
        let mut cipher = Aes256Gcm::new(key.as_ref());

        // Encrypt the message
        cipher
            .encrypt_in_place(Nonce::from_slice(&nonce), AAD, message.as_mut())
            .map_err(|e| KmsError::EncryptionError(format!("Encryption failed: {}", e)))?;

        // Now the message contains the ciphertext, which is safe to be retrieved as a vector.
        Ok(((message.as_ref() as &[u8]).to_vec(), nonce))
    }

    /// Find a key handle that matches the given identity public key
    fn find_key_handle_for_identity_public_key(
        &self,
        identity_public_key: &IdentityPublicKey,
    ) -> Option<KeyHandle> {
        let public_key: PublicKey = identity_public_key.clone().into();

        let raw_key_handle = KeyHandle::RawKey(public_key.clone());

        // Check if this key exists in our store
        if let Ok(keys) = self.store.keys() {
            if keys.contains(&raw_key_handle) {
                return Some(raw_key_handle);
            }

            // If not found as raw key, try to find among derived keys
            for key_handle in keys {
                if let KeyHandle::Derived { .. } = key_handle {
                    // Check if the public key matches
                    if let Ok(Some(stored_pubkey)) = self.kms.public_key(&key_handle) {
                        if stored_pubkey == public_key {
                            return Some(key_handle);
                        }
                    }
                }
            }
        }

        None
    }

    /// Sign data with an ECDSA key
    fn sign_with_ecdsa_key(
        &self,
        key_handle: &KeyHandle,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        // Get the private key for this handle
        let private_key = match key_handle {
            KeyHandle::RawKey(_) => {
                // For raw keys, we need to get the private key from the store
                let (secret, _) = self
                    .get_from_store_with_metadata(key_handle)
                    .map_err(|e| {
                        ProtocolError::Generic(format!("Failed to get private key: {}", e))
                    })?
                    .ok_or_else(|| ProtocolError::Generic("Private key not found".to_string()))?;
                secret
            }
            KeyHandle::Derived { .. } => {
                // For derived keys, we need to derive the private key
                let extended_priv_key =
                    self.get_derived_ecdsa_priv_key(key_handle).map_err(|e| {
                        ProtocolError::Generic(format!("Failed to derive private key: {}", e))
                    })?;
                Secret::new(extended_priv_key.private_key.secret_bytes()).map_err(|e| {
                    ProtocolError::Generic(format!("Failed to create secret: {}", e))
                })?
            }
            _ => {
                return Err(ProtocolError::Generic(
                    "Invalid key handle for ECDSA".to_string(),
                ));
            }
        };

        // Use the dashcore signer to sign the data
        let signature = dash_sdk::dpp::dashcore::signer::sign(data, private_key.as_ref())
            .map_err(|e| ProtocolError::Generic(format!("ECDSA signing failed: {}", e)))?;

        Ok(signature.to_vec().into())
    }

    /// Sign data with a BLS key
    fn sign_with_bls_key(
        &self,
        key_handle: &KeyHandle,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        // Get the private key for this handle
        let (secret, _) = self
            .get_from_store_with_metadata(key_handle)
            .map_err(|e| ProtocolError::Generic(format!("Failed to get private key: {}", e)))?
            .ok_or_else(|| ProtocolError::Generic("Private key not found".to_string()))?;

        // Use BLS signing
        let private_key_bytes: &[u8] = secret.as_ref();
        let key_array: [u8; 32] = private_key_bytes
            .try_into()
            .map_err(|_| ProtocolError::Generic("Invalid BLS private key length".to_string()))?;

        let pk = dash_sdk::dpp::bls_signatures::SecretKey::<
            dash_sdk::dpp::bls_signatures::Bls12381G2Impl,
        >::from_be_bytes(&key_array)
        .into_option()
        .ok_or_else(|| ProtocolError::Generic("Invalid BLS private key".to_string()))?;

        let signature = pk
            .sign(dash_sdk::dpp::bls_signatures::SignatureSchemes::Basic, data)
            .map_err(|e| ProtocolError::Generic(format!("BLS signing failed: {}", e)))?;

        Ok(signature.as_raw_value().to_compressed().to_vec().into())
    }

    /// Sign data with an EdDSA key
    fn sign_with_eddsa_key(
        &self,
        key_handle: &KeyHandle,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        // Get the private key for this handle
        let (secret, _) = self
            .get_from_store_with_metadata(key_handle)
            .map_err(|e| ProtocolError::Generic(format!("Failed to get private key: {}", e)))?
            .ok_or_else(|| ProtocolError::Generic("Private key not found".to_string()))?;

        // Use EdDSA signing
        let private_key_bytes: &[u8] = secret.as_ref();
        let key: [u8; 32] = private_key_bytes
            .try_into()
            .map_err(|_| ProtocolError::Generic("Invalid EdDSA private key length".to_string()))?;

        let signing_key = dash_sdk::dpp::ed25519_dalek::SigningKey::from(&key);

        let signature = signing_key.sign(data);
        Ok(signature.to_vec().into())
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
        let key_handle = self
            .find_key_handle_for_identity_public_key(identity_public_key)
            .ok_or_else(|| {
                ProtocolError::Generic(format!(
                    "No key found for identity public key: {}",
                    identity_public_key.id()
                ))
            })?;
        self.sign_digest(&key_handle, &data)
            .map(|signature| signature.into())
            .map_err(|e| ProtocolError::Generic(format!("Failed to sign data: {}", e)))
    }

    fn can_sign_with(&self, identity_public_key: &IdentityPublicKey) -> bool {
        self.find_key_handle_for_identity_public_key(identity_public_key)
            .is_some()
    }
}

impl<S> UnlockedKMS for GenericUnlockedKms<'_, S>
where
    S: KVStore<KeyHandle, StoredKeyRecord> + Debug + Send + Sync,
    KmsError: From<S::Error>,
{
    fn sign_digest(
        &self,
        key: &Self::KeyHandle,
        digest: &Digest,
    ) -> Result<Signature, Self::Error> {
        let key_type = match key {
            KeyHandle::RawKey(PublicKey(_, key_type)) => key_type,
            KeyHandle::DerivationSeed { .. } => {
                return Err(KmsError::KeyRecordNotSupported(format!(
                    "Cannot sign with derivation seed directly: {}",
                    key
                )));
            }
            KeyHandle::Derived { .. } => {
                &crate::kms::IdentityKeyType::ECDSA_SECP256K1 // Assuming ECDSA for derived keys
            }
            KeyHandle::User(..) => {
                return Err(KmsError::KeyRecordNotSupported(format!(
                    "Cannot sign with user key: {}",
                    key
                )));
            }
        };

        // Convert the key type and handle the signing based on the key type
        let signature_result = match key_type {
            dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1
            | dash_sdk::dpp::identity::KeyType::ECDSA_HASH160 => {
                // For ECDSA keys, we need to get the private key and sign directly
                self.sign_with_ecdsa_key(key, digest).map_err(|e| {
                    KmsError::SigningError(format!("Failed to sign with ECDSA key: {}", e))
                })
            }
            dash_sdk::dpp::identity::KeyType::BLS12_381 => {
                // For BLS keys, we need to get the private key and sign directly
                self.sign_with_bls_key(key, digest).map_err(|e| {
                    KmsError::SigningError(format!("Failed to sign with BLS key: {}", e))
                })
            }
            dash_sdk::dpp::identity::KeyType::EDDSA_25519_HASH160 => {
                // For EdDSA keys, we need to get the private key and sign directly
                self.sign_with_eddsa_key(key, digest).map_err(|e| {
                    KmsError::SigningError(format!("Failed to sign with EdDSA key: {}", e))
                })
            }
            dash_sdk::dpp::identity::KeyType::BIP13_SCRIPT_HASH => {
                Err(KmsError::KeyRecordNotSupported(format!(
                    "Cannot sign with BIP13_SCRIPT_HASH key type: {}",
                    key
                )))
            }
        };

        signature_result.map(|sig| {
            // Convert the signature to BinaryData
            Signature::from(sig)
        })
    }

    fn decrypt(
        &self,
        _key: &Self::KeyHandle,
        _encrypted_data: &EncryptedData,
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
            KeyType::Raw { algorithm } => {
                // FIXME: private key should be put into [Secret] or at least impl zeroize, not simply returned by a function
                let (pubkey, privkey) = algorithm
                    .random_public_and_private_key_data(&mut rng, self.platform_version)
                    .map_err(|e| {
                        KmsError::KeyGenerationError(format!("Failed to generate key pair: {}", e))
                    })
                    .map(|(pubkey, privkey)| {
                        // Convert public key to PublicKey type
                        let pubkey = PublicKey::new(pubkey, algorithm);
                        (pubkey, privkey)
                    })?;

                let handle = KeyHandle::RawKey(pubkey.clone());
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

        let KeyType::Raw { algorithm } = key_type else {
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
        match algorithm {
            dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1 => {
                let extended_pub_key = path
                    .derive_pub_ecdsa_for_master_seed(seed.as_ref(), *network)
                    .map_err(|e| {
                        KmsError::KeyGenerationError(format!(
                            "Failed to derive key pair from seed: {}",
                            e
                        ))
                    })?;
                let public_key = PublicKey::new(extended_pub_key.to_pub().to_bytes(), algorithm);

                // now, define handle of the derived key
                let derived_key_handle = KeyHandle::Derived {
                    seed_hash: *seed_hash,
                    derivation_path: path.clone(),
                    network: *network,
                };

                let key_record = StoredKeyRecord::DerivedKey {
                    derivation_path: path.clone(),
                    seed_hash: *seed_hash,
                    public_key,
                    network: *network,
                };
                self.store.set(derived_key_handle.clone(), key_record)?;

                Ok(derived_key_handle)
            }
            _ => Err(KmsError::KeyRecordNotSupported(format!(
                "Unsupported key type for deriving key pair: {:?}",
                algorithm
            ))),
        }
    }

    fn export(&self, _encryption_key: Secret) -> Result<Vec<u8>, Self::Error> {
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
    use dash_sdk::dpp::{
        dashcore::{Network, bip32::DerivationPath},
        identity::{Purpose, identity_public_key::v0::IdentityPublicKeyV0},
    };

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

    #[test]
    fn test_signer_sign_with_identity_public_key() {
        use dash_sdk::dpp::identity::IdentityPublicKey;
        use dash_sdk::dpp::identity::signer::Signer;

        let user_id = b"user123".to_vec();
        let password: Secret =
            Secret::new(b"securepassword".to_vec()).expect("Failed to create Secret");

        let kms_db_dir =
            tempfile::tempdir().expect("Failed to create temporary file for KMS database");
        let kms_db = kms_db_dir.path().join("wallet.json");
        let kms = GenericKms::new(&kms_db).expect("Failed to create KMS instance");
        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Generate a raw ECDSA key directly
        let seed2 = Secret::new([43u8; 32]).expect("Failed to generate seed");
        let _raw_key = unlocked
            .generate_key_pair(KeyType::ecdsa_secp256k1(), seed2)
            .expect("Failed to generate raw key");

        // Get the public key (we're not using it for matching, just ensuring it works)
        let public_key_data = kms
            .public_key(&_raw_key)
            .expect("Failed to get public key")
            .expect("Public key should exist");

        // Create an IdentityPublicKey from the public key data
        let identity_public_key_v0 = IdentityPublicKeyV0 {
            data: public_key_data.into(),
            id: 1,
            purpose: Purpose::AUTHENTICATION,
            key_type: dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1,
            read_only: false,
            contract_bounds: None,
            disabled_at: None,
            security_level: dash_sdk::dpp::identity::SecurityLevel::CRITICAL,
        };

        let identity_public_key = IdentityPublicKey::from(identity_public_key_v0);

        assert!(
            unlocked.can_sign_with(&identity_public_key),
            "Should  be able to sign with the correct key"
        );

        // Test signing with an unknown key (should fail)
        let data_to_sign = b"test message to sign";
        let signature: Signature = Signer::sign(&unlocked, &identity_public_key, data_to_sign)
            .expect("Failed to sign data with the correct key")
            .into();

        // Verify the signature
        signature
            .verify(identity_public_key.clone(), data_to_sign)
            .expect("Failed to verify signature");

        // modify the signature and ensure it's wrong
        let mut modified_signature = signature.signature.clone();
        modified_signature[5] ^= 1; // flip some byte to modify the signature
        let invalid_signature = Signature::new(modified_signature, signature.key_type);
        invalid_signature
            .verify(identity_public_key, data_to_sign)
            .expect_err("Modified signature validation should fail");
    }
}
