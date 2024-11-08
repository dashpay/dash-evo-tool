use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{self, Argon2};
use rand::rngs::OsRng;
use rand::RngCore;

const SALT_SIZE: usize = 16; // 128-bit salt
const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM

pub const DASH_SECRET_MESSAGE: &[u8; 19] = b"dash_secret_message";

use crate::model::wallet::ClosedWalletSeed;
use sha2::{Digest, Sha256};

/// Derive a key from the password and salt using Argon2.
pub fn derive_password_key(password: &str, salt: &[u8]) -> Result<Vec<u8>, String> {
    let key_length = 32; // For AES-256, we use a 256-bit key (32 bytes)

    let mut key = vec![0u8; key_length];

    // Using Argon2 with default parameters
    let argon2 = Argon2::default();

    // Deriving the key
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;

    Ok(key)
}

/// Encrypt the seed using AES-256-GCM.
pub fn encrypt_message(
    message: &[u8],
    password: &str,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    // Generate a random salt
    let mut salt = vec![0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    // Derive the key
    let key = derive_password_key(password, &salt)?;

    // Generate a random nonce
    let mut nonce = vec![0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);

    // Create cipher instance
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;

    // Encrypt the seed
    let encrypted_seed = cipher
        .encrypt(Nonce::from_slice(&nonce), message)
        .map_err(|e| e.to_string())?;

    Ok((encrypted_seed, salt, nonce))
}

impl ClosedWalletSeed {
    pub fn compute_seed_hash(seed: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(seed);
        let result = hasher.finalize();
        let mut seed_hash = [0u8; 32];
        seed_hash.copy_from_slice(&result);
        seed_hash
    }

    /// Encrypt the seed using AES-256-GCM.
    pub(crate) fn encrypt_seed(
        seed: &[u8],
        password: &str,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
        encrypt_message(seed, password)
    }

    /// Decrypt the seed using AES-256-GCM.
    pub fn decrypt_seed(&self, password: &str) -> Result<[u8; 64], String> {
        // Derive the key
        let key = derive_password_key(password, &self.salt)?;

        // Create cipher instance
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;

        // Decrypt the seed
        let seed = cipher
            .decrypt(
                Nonce::from_slice(&self.nonce),
                self.encrypted_seed.as_slice(),
            )
            .map_err(|e| e.to_string())?;

        let sized_seed = seed.try_into().map_err(|e: Vec<u8>| {
            format!(
                "invalid seed length, expected 64 bytes, got {} bytes",
                e.len()
            )
        })?;

        Ok(sized_seed)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_seed() {
        let seed = [42u8; 64]; // A 64-byte seed filled with the value 42
        let password = "securepassword";

        // Encrypt the seed using the encrypt_seed method
        let (encrypted_seed, salt, nonce) =
            ClosedWalletSeed::encrypt_seed(&seed, password).expect("Encryption failed");

        // Compute the seed hash
        let seed_hash = ClosedWalletSeed::compute_seed_hash(&seed);

        // Create a ClosedWalletSeed instance with the encrypted data
        let closed_wallet_seed = ClosedWalletSeed {
            seed_hash,
            encrypted_seed,
            salt,
            nonce,
            password_hint: None, // Set password hint if needed
        };

        // Decrypt the seed using the instance method
        let decrypted_seed = closed_wallet_seed
            .decrypt_seed(password)
            .expect("Decryption failed");

        // Verify that the decrypted seed matches the original seed
        assert_eq!(seed, decrypted_seed);
    }

    #[test]
    fn test_incorrect_password() {
        let seed = [42u8; 64]; // A 64-byte seed
        let password = "securepassword";
        let wrong_password = "wrongpassword";

        // Encrypt the seed using the encrypt_seed method
        let (encrypted_seed, salt, nonce) =
            ClosedWalletSeed::encrypt_seed(&seed, password).expect("Encryption failed");

        // Compute the seed hash
        let seed_hash = ClosedWalletSeed::compute_seed_hash(&seed);

        // Create a ClosedWalletSeed instance with the encrypted data
        let closed_wallet_seed = ClosedWalletSeed {
            seed_hash,
            encrypted_seed,
            salt,
            nonce,
            password_hint: None,
        };

        // Attempt to decrypt with the wrong password
        let result = closed_wallet_seed.decrypt_seed(wrong_password);

        // Verify that decryption fails
        assert!(result.is_err());
    }
}
