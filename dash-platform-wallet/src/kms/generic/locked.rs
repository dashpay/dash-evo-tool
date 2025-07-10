use core::fmt::Debug;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    kms::{
        KVStore, Kms, PublicKey, UnlockedKMS,
        file_store::{FileStore, JsonStoreError},
        generic::{NONCE_SIZE, key_handle::KeyHandle, unlocked::GenericUnlockedKms},
    },
    secret::{Secret, SecretError},
};

pub type Store = FileStore<KeyHandle, StoredKeyRecord>;
/// Simple Key Management Service (KMS) implementation for managing wallet keys.
#[derive(Clone)]
pub struct GenericKms {
    store: Store,
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
pub enum StoredKeyRecord {
    /// Represents a private key stored in the KMS. See [`KeyHandle::RawKey`].
    RawKey {
        encrypted_key: Vec<u8>,
        nonce: Nonce,
        public_key: PublicKey,
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
        public_key: PublicKey,
        network: dash_sdk::dpp::dashcore::Network,
    },
    /// Represents a user's encrypted copy of the master key.
    UserRecord {
        user_id: Vec<u8>,
        encrypted_master_key: Vec<u8>,
        nonce: Nonce,
        salt: Vec<u8>,   // For key derivation
        created_at: u64, // Timestamp
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
                KeyHandle::RawKey(requested_public_key),
            ) => {
                if !public_key.eq(requested_public_key) {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "Public key mismatch: requested: {:?}, stored: {:?}",
                        requested_public_key, public_key,
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
            // UserRecord: verify the user_id matches
            (
                StoredKeyRecord::UserRecord {
                    user_id: stored_user_id,
                    ..
                },
                KeyHandle::User(requested_user_id),
            ) => {
                if stored_user_id != requested_user_id {
                    return Err(KmsError::KeyIntegrityError(format!(
                        "User ID mismatch: requested: {}, stored: {}",
                        hex::encode(requested_user_id),
                        hex::encode(stored_user_id),
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

    /// Invalid username or password; note we don't distinguish between these two
    /// errors to ensure we don't leak information about user existence.
    #[error("Invalid username or password")]
    InvalidCredentials,

    #[error("Cannot remove the last user")]
    CannotRemoveLastUser,

    #[error("Master key not found")]
    MasterKeyNotFound,

    #[error("Migration error: {0}")]
    MigrationError(String),

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

    /// Unlocks the KMS for operations that require access to private keys.
    #[allow(private_interfaces)]
    pub fn login(
        &self,
        user_id: &[u8],
        password: Secret,
    ) -> Result<GenericUnlockedKms<'_, Store>, KmsError> {
        GenericUnlockedKms::new(self, self.store.clone(), user_id, password)
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
        self.login(user_id, password)
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
            KeyHandle::RawKey(public_key) => public_key.clone(),
            KeyHandle::Derived { .. } => {
                // For derived keys, we need to get the public key from the stored record
                match record {
                    StoredKeyRecord::DerivedKey { public_key, .. } => public_key,
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
            KeyHandle::User(_) => {
                return Err(KmsError::KeyRecordNotSupported(
                    "User record does not have a public key".to_string(),
                ));
            }
        };

        Ok(Some(pubkey))
    }

    /// Returns list of all public key handles in the KMS.
    ///
    /// Filters out internal keys (e.g. user password records).
    fn keys(&self) -> Result<impl Iterator<Item = Self::KeyHandle>, Self::Error> {
        let i = self
            .store
            .keys()?
            .into_iter()
            .filter(|k| !matches!(k, KeyHandle::User(_)));
        Ok(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        kms::{KeyType, Kms, UnlockedKMS},
        secret::Secret,
    };
    use dash_sdk::dpp::{
        dashcore::{Network, bip32::DerivationPath},
        identity::KeyType as IdentityKeyType,
    };
    use tempfile::TempDir;

    // Helper function to create a test KMS instance
    fn create_test_kms() -> (GenericKms, TempDir) {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
        let kms_path = temp_dir.path().join("test_wallet.json");
        let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");
        (kms, temp_dir)
    }

    // Helper function to create test credentials
    fn test_credentials() -> (Vec<u8>, Secret) {
        let user_id = b"test_user_123".to_vec();
        let password = Secret::new(b"secure_test_password_123".to_vec())
            .expect("Failed to create password secret");
        (user_id, password)
    }

    // Helper function to create test seed
    fn test_seed() -> Secret {
        Secret::new([42u8; 32]).expect("Failed to create test seed")
    }

    #[test]
    fn test_kms_creation() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
        let kms_path = temp_dir.path().join("test_creation.json");

        let kms = GenericKms::new(&kms_path);
        assert!(kms.is_ok(), "Failed to create KMS: {:?}", kms.err());
    }

    #[test]
    fn test_kms_unlock_with_empty_store() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();

        // Should succeed even with empty store
        let unlocked = kms.unlock(&user_id, password);
        assert!(
            unlocked.is_ok(),
            "Failed to unlock empty KMS: {:?}",
            unlocked.err()
        );
    }

    #[test]
    fn test_kms_unlock_wrong_password() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        // First, add a key to the store
        {
            let mut unlocked = kms
                .unlock(&user_id, password.clone())
                .expect("Failed to unlock KMS");

            let requested_key = KeyType::Raw {
                algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
            };
            let _key = unlocked
                .generate_key_pair(requested_key, seed)
                .expect("Failed to generate key");
        }

        // Now try with wrong password
        let wrong_password =
            Secret::new(b"wrong_password".to_vec()).expect("Failed to create wrong password");
        let result = kms.unlock(&user_id, wrong_password);
        assert!(result.is_err(), "Wrong password should fail");

        match result.err().unwrap() {
            KmsError::InvalidCredentials => {}
            other => panic!("Expected InvalidCredentials error, got: {:?}", other),
        }
    }

    #[test]
    fn test_generate_identity_key_ecdsa() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");
        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
        };
        let key_handle = unlocked
            .generate_key_pair(requested_key, seed)
            .expect("Failed to generate ECDSA key");

        // Just verify the key exists by checking if it's in the list
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert!(
            keys.contains(&key_handle),
            "Generated key should be in keys list"
        );
    }

    #[test]
    fn test_generate_identity_key_eddsa() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");
        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::EDDSA_25519_HASH160,
        };
        let key_handle = unlocked
            .generate_key_pair(requested_key, seed)
            .expect("Failed to generate EdDSA key");

        // Just verify the key exists by checking if it's in the list
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert!(
            keys.contains(&key_handle),
            "Generated key should be in keys list"
        );
    }

    #[test]
    fn test_generate_derivation_seed() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        let key_handle = unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                seed,
            )
            .expect("Failed to generate derivation seed");

        // Derivation seeds don't have public keys
        let pubkey_result = kms.public_key(&key_handle);
        assert!(
            pubkey_result.is_err(),
            "Derivation seed should not have public key"
        );
    }

    #[test]
    fn test_derive_key_pair() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // First generate a master key
        let master_key = unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                seed,
            )
            .expect("Failed to generate master key");

        // Derive a key from the master
        let derivation_path = DerivationPath::bip_44_payment_path(Network::Testnet, 0, false, 0);

        let derived_key = unlocked
            .derive_key_pair(&master_key, KeyType::ecdsa_secp256k1(), &derivation_path)
            .expect("Failed to derive key pair");

        // Verify the derived key is in the keys list
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert!(
            keys.contains(&derived_key),
            "Derived key should be in keys list"
        );
        assert_eq!(keys.len(), 2, "Should have master key and derived key");
    }

    #[test]
    fn test_derive_multiple_keys_same_path() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Generate master key
        let master_key = unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                seed,
            )
            .expect("Failed to generate master key");

        let derivation_path = DerivationPath::bip_44_payment_path(Network::Testnet, 0, false, 0);

        // Derive the same key twice
        let derived_key1 = unlocked
            .derive_key_pair(&master_key, KeyType::ecdsa_secp256k1(), &derivation_path)
            .expect("Failed to derive key pair 1");

        let derived_key2 = unlocked
            .derive_key_pair(&master_key, KeyType::ecdsa_secp256k1(), &derivation_path)
            .expect("Failed to derive key pair 2");

        // Both should produce the same key handle
        assert_eq!(derived_key1, derived_key2);

        // Should have only 2 keys total (master + derived, not duplicated)
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert_eq!(keys.len(), 2, "Should have 2 keys total (master + derived)");
    }

    #[test]
    fn test_derive_different_paths() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Generate master key
        let master_key = unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                seed,
            )
            .expect("Failed to generate master key");

        let path1 = DerivationPath::bip_44_payment_path(Network::Testnet, 0, false, 0);
        let path2 = DerivationPath::bip_44_payment_path(Network::Testnet, 0, false, 1);

        let derived_key1 = unlocked
            .derive_key_pair(&master_key, KeyType::ecdsa_secp256k1(), &path1)
            .expect("Failed to derive key pair 1");

        let derived_key2 = unlocked
            .derive_key_pair(&master_key, KeyType::ecdsa_secp256k1(), &path2)
            .expect("Failed to derive key pair 2");

        // Different paths should produce different keys
        assert_ne!(derived_key1, derived_key2);

        // Should have 3 keys total (master + 2 derived)
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert_eq!(keys.len(), 3, "Should have 3 keys total");
        assert!(keys.contains(&master_key), "Should contain master key");
        assert!(keys.contains(&derived_key1), "Should contain derived key 1");
        assert!(keys.contains(&derived_key2), "Should contain derived key 2");
    }

    #[test]
    fn test_list_keys_empty() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();

        let _unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert_eq!(keys.len(), 0, "Empty KMS should have no keys");
    }

    #[test]
    fn test_list_keys_with_content() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Generate multiple keys
        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
        };
        let key1 = unlocked
            .generate_key_pair(requested_key, seed.clone())
            .expect("Failed to generate key 1");

        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::EDDSA_25519_HASH160,
        };
        let key2 = unlocked
            .generate_key_pair(requested_key, seed.clone())
            .expect("Failed to generate key 2");

        let master_key = unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                seed,
            )
            .expect("Failed to generate master key");

        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert_eq!(keys.len(), 3, "Should have 3 keys");
        assert!(keys.contains(&key1), "Should contain key1");
        assert!(keys.contains(&key2), "Should contain key2");
        assert!(keys.contains(&master_key), "Should contain master key");
    }

    #[test]
    fn test_persistence_across_sessions() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
        let kms_path = temp_dir.path().join("persistence_test.json");
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let key_handle;

        // First session: create and store a key
        {
            let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");
            let mut unlocked = kms
                .unlock(&user_id, password.clone())
                .expect("Failed to unlock KMS");

            let requested_key = KeyType::Raw {
                algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
            };
            key_handle = unlocked
                .generate_key_pair(requested_key, seed)
                .expect("Failed to generate key");

            // Verify key exists in first session
            let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();
            assert!(
                keys.contains(&key_handle),
                "Key should exist in first session"
            );
        }

        // Second session: reload and verify
        {
            let kms = GenericKms::new(&kms_path).expect("Failed to reload KMS");
            let _unlocked = kms
                .unlock(&user_id, password)
                .expect("Failed to unlock reloaded KMS");

            let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

            assert_eq!(keys.len(), 1, "Should have 1 persisted key");
            assert!(
                keys.contains(&key_handle),
                "Should contain the original key"
            );
        }
    }

    #[test]
    fn test_invalid_seed_size() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
        };
        let short_seed = Secret::new(vec![42u8; 16]).expect("Failed to create short seed");
        let result = unlocked.generate_key_pair(requested_key, short_seed);

        assert!(result.is_err(), "Short seed should be rejected");

        // Test with too long seed
        let long_seed = Secret::new(vec![42u8; 64]).expect("Failed to create long seed");
        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
        };
        let result = unlocked.generate_key_pair(requested_key, long_seed);

        assert!(result.is_err(), "Long seed should be rejected");
    }

    #[test]
    fn test_derive_from_invalid_master() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Generate a regular identity key (not a derivation seed)
        let requested_key = KeyType::Raw {
            algorithm: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
        };
        let identity_key = unlocked
            .generate_key_pair(requested_key, seed)
            .expect("Failed to generate identity key");

        let derivation_path = DerivationPath::bip_44_payment_path(Network::Testnet, 0, false, 0);

        // Try to derive from the identity key (should fail)
        let result =
            unlocked.derive_key_pair(&identity_key, KeyType::ecdsa_secp256k1(), &derivation_path);
        assert!(
            result.is_err(),
            "Should not be able to derive from identity key"
        );
    }

    #[test]
    fn test_derive_from_nonexistent_master() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Create a fake master key handle that doesn't exist in store
        let fake_master = KeyHandle::DerivationSeed {
            seed_hash: [0u8; 32],
            network: Network::Testnet,
        };

        let derivation_path = DerivationPath::bip_44_payment_path(Network::Testnet, 0, false, 0);

        let result =
            unlocked.derive_key_pair(&fake_master, KeyType::ecdsa_secp256k1(), &derivation_path);
        assert!(
            result.is_err(),
            "Should fail to derive from nonexistent master"
        );

        match result.err().unwrap() {
            KmsError::PrivateKeyNotFound(_) => {}
            other => panic!("Expected PrivateKeyNotFound error, got: {:?}", other),
        }
    }

    #[test]
    fn test_get_public_key_nonexistent() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();

        let _unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Try to get public key for nonexistent key
        let fake_key = KeyHandle::RawKey(PublicKey::new(
            vec![1, 2, 3, 4],
            dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
        ));
        let result = kms
            .public_key(&fake_key)
            .expect("Should succeed but return None");

        assert!(result.is_none(), "Should return None for nonexistent key");
    }

    #[test]
    fn test_multiple_users_same_credentials() {
        let (kms, _temp_dir) = create_test_kms();
        let seed = test_seed();

        // Both users use the same credentials (this is the realistic scenario)
        let user_id = b"shared_user".to_vec();
        let password =
            Secret::new(b"shared_password".to_vec()).expect("Failed to create shared password");

        // User 1 creates a key
        let user1_key = {
            let mut unlocked = kms
                .unlock(&user_id, password.clone())
                .expect("Failed to unlock KMS for user1");

            unlocked
                .generate_key_pair(
                    KeyType::Raw {
                        algorithm: IdentityKeyType::ECDSA_SECP256K1,
                    },
                    seed.clone(),
                )
                .expect("Failed to generate key for user1")
        };

        // User 2 creates a key with same credentials
        let user2_key = {
            let mut unlocked = kms
                .unlock(&user_id, password.clone())
                .expect("Failed to unlock KMS for user2");

            unlocked
                .generate_key_pair(
                    KeyType::Raw {
                        algorithm: IdentityKeyType::EDDSA_25519_HASH160,
                    },
                    seed,
                )
                .expect("Failed to generate key for user2")
        };

        // Both keys should be listed (they share the same physical store and credentials)
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert_eq!(keys.len(), 2, "Should have 2 keys total");
        assert!(keys.contains(&user1_key), "Should contain user1's key");
        assert!(keys.contains(&user2_key), "Should contain user2's key");

        // Both users can unlock with the shared credentials
        {
            let _unlocked = kms
                .unlock(&user_id, password.clone())
                .expect("Failed to unlock KMS");
        }
    }

    #[test]
    fn test_different_users_different_stores() {
        // Test that different users should use different stores for true isolation
        let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
        let seed = test_seed();

        let user1_id = b"user1".to_vec();
        let user1_password =
            Secret::new(b"password1".to_vec()).expect("Failed to create user1 password");

        let user2_id = b"user2".to_vec();
        let user2_password =
            Secret::new(b"password2".to_vec()).expect("Failed to create user2 password");

        // User 1 gets their own store
        let user1_kms = {
            let user1_path = temp_dir.path().join("user1_wallet.json");
            GenericKms::new(&user1_path).expect("Failed to create user1 KMS")
        };

        // User 2 gets their own store
        let user2_kms = {
            let user2_path = temp_dir.path().join("user2_wallet.json");
            GenericKms::new(&user2_path).expect("Failed to create user2 KMS")
        };

        // User 1 creates a key in their store
        let user1_key = {
            let mut unlocked = user1_kms
                .unlock(&user1_id, user1_password.clone())
                .expect("Failed to unlock KMS for user1");

            unlocked
                .generate_key_pair(
                    KeyType::Raw {
                        algorithm: IdentityKeyType::ECDSA_SECP256K1,
                    },
                    seed.clone(),
                )
                .expect("Failed to generate key for user1")
        };

        // User 2 creates a key in their store
        let user2_key = {
            let mut unlocked = user2_kms
                .unlock(&user2_id, user2_password.clone())
                .expect("Failed to unlock KMS for user2");

            unlocked
                .generate_key_pair(
                    KeyType::Raw {
                        algorithm: IdentityKeyType::EDDSA_25519_HASH160,
                    },
                    seed,
                )
                .expect("Failed to generate key for user2")
        };

        // Each user can only see their own keys
        let user1_keys: Vec<_> = user1_kms
            .keys()
            .expect("Failed to list user1 keys")
            .collect();
        assert_eq!(user1_keys.len(), 1, "User1 should have 1 key");
        assert!(
            user1_keys.contains(&user1_key),
            "Should contain user1's key"
        );

        let user2_keys: Vec<_> = user2_kms
            .keys()
            .expect("Failed to list user2 keys")
            .collect();
        assert_eq!(user2_keys.len(), 1, "User2 should have 1 key");
        assert!(
            user2_keys.contains(&user2_key),
            "Should contain user2's key"
        );

        // Each user can still access their own store
        {
            let _unlocked1 = user1_kms
                .unlock(&user1_id, user1_password)
                .expect("Failed to unlock user1 KMS");
        }

        {
            let _unlocked2 = user2_kms
                .unlock(&user2_id, user2_password)
                .expect("Failed to unlock user2 KMS");
        }
    }

    #[test]
    fn test_key_record_verification() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        let mut unlocked = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS");

        // Generate a key and verify that it can be listed
        let key_handle = unlocked
            .generate_key_pair(
                KeyType::Raw {
                    algorithm: IdentityKeyType::ECDSA_SECP256K1,
                },
                seed,
            )
            .expect("Failed to generate key");

        // This should succeed - the key was just created
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert!(
            keys.contains(&key_handle),
            "Generated key should be in keys list"
        );
    }

    #[test]
    fn test_concurrent_access_simulation() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        // Simulate multiple concurrent accesses by creating multiple unlocked instances
        let mut unlocked1 = kms
            .unlock(&user_id, password.clone())
            .expect("Failed to unlock KMS 1");
        let mut unlocked2 = kms
            .unlock(&user_id, password)
            .expect("Failed to unlock KMS 2");

        // Both should be able to generate keys
        let key1 = unlocked1
            .generate_key_pair(
                KeyType::Raw {
                    algorithm: IdentityKeyType::ECDSA_SECP256K1,
                },
                seed.clone(),
            )
            .expect("Failed to generate key 1");

        let key2 = unlocked2
            .generate_key_pair(
                KeyType::Raw {
                    algorithm: IdentityKeyType::EDDSA_25519_HASH160,
                },
                seed,
            )
            .expect("Failed to generate key 2");

        // Both keys should be different
        assert_ne!(key1, key2, "Keys should be different");

        // Both should see all keys
        let keys: Vec<_> = kms.keys().expect("Failed to list keys").collect();

        assert_eq!(keys.len(), 2, "Should see both keys");
        assert!(keys.contains(&key1), "Should contain key1");
        assert!(keys.contains(&key2), "Should contain key2");
    }

    #[test]
    fn test_login_method_user_management() {
        let (kms, _temp_dir) = create_test_kms();
        let (admin_id, admin_password) = test_credentials();
        let user_id = b"new_user".to_vec();
        let user_password = Secret::new(b"user_password".to_vec()).unwrap();
        let seed = test_seed();

        // Create initial key and user through login
        let initial_key = {
            let mut admin_unlocked = kms
                .login(&admin_id, admin_password.clone())
                .expect("Failed to login as admin");
            let key = admin_unlocked
                .generate_key_pair(
                    KeyType::DerivationSeed {
                        network: Network::Testnet,
                    },
                    seed,
                )
                .expect("Failed to generate initial key");

            // Add new user
            admin_unlocked
                .add_user(&user_id, user_password.clone())
                .expect("Failed to add user");

            // Verify user list
            let users = admin_unlocked.list_users().expect("Failed to list users");
            assert_eq!(users.len(), 2, "Should have 2 users");
            assert!(users.contains(&admin_id), "Should contain admin");
            assert!(users.contains(&user_id), "Should contain new user");

            key
        };

        // New user should be able to login and see the same keys
        {
            let user_unlocked = kms
                .login(&user_id, user_password.clone())
                .expect("Failed to login as new user");
            let keys: Vec<_> = user_unlocked.keys().expect("Failed to get keys").collect();
            assert_eq!(keys.len(), 1, "New user should see the same key");
            assert_eq!(keys[0], initial_key, "Key should match");

            let users = user_unlocked.list_users().expect("Failed to list users");
            assert_eq!(users.len(), 2, "New user should see both users");
        }

        // Test password change through login
        let new_password = Secret::new(b"new_password".to_vec()).unwrap();
        {
            let mut user_unlocked = kms
                .login(&user_id, user_password.clone())
                .expect("Failed to login as user");
            user_unlocked
                .change_user_password(&user_id, new_password.clone())
                .expect("Failed to change password");
        }

        // Old password should no longer work
        let result = kms.login(&user_id, user_password);
        assert!(result.is_err(), "Old password should not work");

        // New password should work
        {
            let _user_unlocked = kms
                .login(&user_id, new_password)
                .expect("Failed to login with new password");
        }
    }

    #[test]
    fn test_login_method_error_handling() {
        let (kms, _temp_dir) = create_test_kms();
        let (user_id, password) = test_credentials();
        let seed = test_seed();

        // Create initial setup
        {
            let mut unlocked = kms
                .login(&user_id, password.clone())
                .expect("Failed to login");
            let _key = unlocked
                .generate_key_pair(
                    KeyType::DerivationSeed {
                        network: Network::Testnet,
                    },
                    seed,
                )
                .expect("Failed to generate key");
        }

        // Test invalid credentials
        let wrong_password = Secret::new(b"wrong_password".to_vec()).unwrap();
        let result = kms.login(&user_id, wrong_password);
        assert!(result.is_err(), "Wrong password should fail");
        match result.err().unwrap() {
            KmsError::InvalidCredentials => {}
            other => panic!("Expected InvalidCredentials, got: {:?}", other),
        }

        // Test non-existent user
        let nonexistent_user = b"nonexistent".to_vec();
        let result = kms.login(&nonexistent_user, password);
        assert!(result.is_err(), "Non-existent user should fail");
        match result.err().unwrap() {
            KmsError::InvalidCredentials => {}
            other => panic!("Expected InvalidCredentials, got: {:?}", other),
        }
    }

    #[test]
    fn test_login_method_duplicate_user_error() {
        let (kms, _temp_dir) = create_test_kms();
        let (admin_id, admin_password) = test_credentials();
        let user_id = b"duplicate_user".to_vec();
        let user_password = Secret::new(b"user_password".to_vec()).unwrap();

        // Create initial setup and add user
        {
            let mut admin_unlocked = kms
                .login(&admin_id, admin_password.clone())
                .expect("Failed to login as admin");
            admin_unlocked
                .add_user(&user_id, user_password.clone())
                .expect("Failed to add user");
        }

        // Try to add the same user again - should fail
        {
            let mut admin_unlocked = kms
                .login(&admin_id, admin_password)
                .expect("Failed to login as admin");
            let result = admin_unlocked.add_user(&user_id, user_password);
            assert!(result.is_err(), "Adding duplicate user should fail");
        }
    }

    #[test]
    fn test_login_method_user_isolation_different_stores() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
        let user1_id = b"user1".to_vec();
        let user1_password = Secret::new(b"password1".to_vec()).unwrap();
        let user2_id = b"user2".to_vec();
        let user2_password = Secret::new(b"password2".to_vec()).unwrap();
        let seed = test_seed();

        // Create separate KMS instances for different users
        let kms1 =
            GenericKms::new(&temp_dir.path().join("user1.json")).expect("Failed to create KMS1");
        let kms2 =
            GenericKms::new(&temp_dir.path().join("user2.json")).expect("Failed to create KMS2");

        // User 1 creates a key
        let user1_key = {
            let mut unlocked = kms1
                .login(&user1_id, user1_password.clone())
                .expect("Failed to login as user1");
            unlocked
                .generate_key_pair(
                    KeyType::DerivationSeed {
                        network: Network::Testnet,
                    },
                    seed.clone(),
                )
                .expect("Failed to generate key for user1")
        };

        // User 2 creates a key with different seed
        let seed2 = Secret::new([99u8; 32]).expect("Failed to create test seed 2");
        let user2_key = {
            let mut unlocked = kms2
                .login(&user2_id, user2_password.clone())
                .expect("Failed to login as user2");
            unlocked
                .generate_key_pair(
                    KeyType::DerivationSeed {
                        network: Network::Testnet,
                    },
                    seed2,
                )
                .expect("Failed to generate key for user2")
        };

        // Keys should be different (different seeds)
        assert_ne!(user1_key, user2_key, "Keys should be different");

        // Each user should only see their own keys
        {
            let unlocked1 = kms1
                .login(&user1_id, user1_password)
                .expect("Failed to login as user1");
            let keys1: Vec<_> = unlocked1.keys().expect("Failed to get keys").collect();
            assert_eq!(keys1.len(), 1, "User1 should see only 1 key");
            assert_eq!(keys1[0], user1_key, "Should be user1's key");
        }

        {
            let unlocked2 = kms2
                .login(&user2_id, user2_password)
                .expect("Failed to login as user2");
            let keys2: Vec<_> = unlocked2.keys().expect("Failed to get keys").collect();
            assert_eq!(keys2.len(), 1, "User2 should see only 1 key");
            assert_eq!(keys2[0], user2_key, "Should be user2's key");
        }
    }
}
