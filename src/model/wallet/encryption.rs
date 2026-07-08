use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{self, Argon2};
use bip39::rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

const SALT_SIZE: usize = 16; // 128-bit salt
const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM

use crate::model::wallet::ClosedKeyItem;
use sha2::{Digest, Sha256};

/// An AES-256-GCM envelope: ciphertext plus the random salt and nonce needed
/// to reproduce the key and decrypt it. Produced by [`encrypt_message`] and
/// consumed by [`decrypt_message`].
pub(crate) struct EncryptedEnvelope {
    /// The AES-256-GCM ciphertext (authentication tag included).
    pub ciphertext: Vec<u8>,
    /// The random 128-bit salt fed to Argon2 for key derivation.
    pub salt: Vec<u8>,
    /// The random 96-bit AES-GCM nonce.
    pub nonce: Vec<u8>,
}

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

/// Encrypt `message` under `password` with AES-256-GCM, returning the
/// [`EncryptedEnvelope`] (ciphertext, salt, nonce).
#[allow(deprecated)]
pub(crate) fn encrypt_message(message: &[u8], password: &str) -> Result<EncryptedEnvelope, String> {
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
    let nonce_arr = Nonce::from_slice(&nonce);
    let ciphertext = cipher
        .encrypt(nonce_arr, message)
        .map_err(|e| e.to_string())?;

    Ok(EncryptedEnvelope {
        ciphertext,
        salt,
        nonce,
    })
}

/// Failure decrypting an AES-256-GCM envelope produced by [`encrypt_message`].
///
/// Two outcomes the callers must distinguish: an authentication failure
/// (`WrongPassword` — the supplied password is wrong or the ciphertext was
/// tampered with) versus a structurally invalid envelope (`Malformed` — bad
/// key derivation, cipher init, or nonce length; a corrupt at-rest blob). Each
/// caller maps these to its own typed domain error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecryptError {
    /// The AEAD tag did not verify — wrong password or tampered ciphertext.
    WrongPassword,
    /// The envelope is structurally invalid (key derivation, cipher init, or
    /// nonce length). Diagnostic detail is logged inside [`decrypt_message`].
    Malformed,
}

/// Decrypt an AES-256-GCM envelope (`ciphertext` + `salt` + `nonce`) under
/// `password` — the inverse of [`encrypt_message`]. Returns the plaintext in a
/// [`Zeroizing`] buffer so it wipes on drop; the caller validates its length.
///
/// Shared by every AES-GCM legacy-secret reader (the HD-seed migration reader,
/// the imported single-key entry, and the deprecated `ClosedKeyItem` seed
/// store) so the derive-key → init-cipher → checked-nonce → decrypt sequence
/// exists once. Structural failures are logged with `site` for context and
/// returned as [`DecryptError::Malformed`]; an authentication failure is
/// [`DecryptError::WrongPassword`] (no plaintext oracle).
pub(crate) fn decrypt_message(
    ciphertext: &[u8],
    salt: &[u8],
    nonce: &[u8],
    password: &str,
    site: &'static str,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let key = Zeroizing::new(derive_password_key(password, salt).map_err(|detail| {
        tracing::warn!(
            target = "model::wallet::encryption",
            site,
            %detail,
            "Argon2 key derivation failed during decrypt",
        );
        DecryptError::Malformed
    })?);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| {
        tracing::warn!(
            target = "model::wallet::encryption",
            site,
            %error,
            "AES-GCM init failed during decrypt",
        );
        DecryptError::Malformed
    })?;
    // Checked nonce conversion: an envelope with the wrong nonce length is a
    // corrupt at-rest blob, not a panic. `Nonce::from_slice` panics on a length
    // mismatch, which would poison the long-lived secret-store mutex.
    let nonce_bytes: &[u8; NONCE_SIZE] = nonce.try_into().map_err(|_| {
        tracing::warn!(
            target = "model::wallet::encryption",
            site,
            nonce_len = nonce.len(),
            "Envelope nonce is not the expected length",
        );
        DecryptError::Malformed
    })?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| DecryptError::WrongPassword)?;
    Ok(Zeroizing::new(plaintext))
}

impl ClosedKeyItem {
    pub fn compute_seed_hash(seed: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(seed);
        let result = hasher.finalize();
        let mut seed_hash = [0u8; 32];
        seed_hash.copy_from_slice(&result);
        seed_hash
    }

    /// Encrypt the seed using AES-256-GCM.
    pub(crate) fn encrypt_seed(seed: &[u8], password: &str) -> Result<EncryptedEnvelope, String> {
        encrypt_message(seed, password)
    }

    /// Decrypt the seed using AES-256-GCM via the shared [`decrypt_message`]
    /// reader.
    pub fn decrypt_seed(&self, password: &str) -> Result<[u8; 64], String> {
        let seed = decrypt_message(
            &self.encrypted_seed,
            &self.salt,
            &self.nonce,
            password,
            "closed_key_item::decrypt_seed",
        )
        .map_err(|e| match e {
            DecryptError::WrongPassword => "incorrect password".to_string(),
            DecryptError::Malformed => "failed to decrypt the wallet seed".to_string(),
        })?;

        seed.as_slice().try_into().map_err(|_| {
            format!(
                "invalid seed length, expected 64 bytes, got {} bytes",
                seed.len()
            )
        })
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
        let envelope = ClosedKeyItem::encrypt_seed(&seed, password).expect("Encryption failed");

        // Compute the seed hash
        let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);

        // Create a ClosedWalletSeed instance with the encrypted data
        let closed_wallet_seed = ClosedKeyItem {
            seed_hash,
            encrypted_seed: envelope.ciphertext,
            salt: envelope.salt,
            nonce: envelope.nonce,
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
        let envelope = ClosedKeyItem::encrypt_seed(&seed, password).expect("Encryption failed");

        // Compute the seed hash
        let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);

        // Create a ClosedWalletSeed instance with the encrypted data
        let closed_wallet_seed = ClosedKeyItem {
            seed_hash,
            encrypted_seed: envelope.ciphertext,
            salt: envelope.salt,
            nonce: envelope.nonce,
            password_hint: None,
        };

        // Attempt to decrypt with the wrong password
        let result = closed_wallet_seed.decrypt_seed(wrong_password);

        // Verify that decryption fails
        assert!(result.is_err());
    }
}
