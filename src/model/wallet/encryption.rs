use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{self, Argon2};
use bip39::rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

const SALT_SIZE: usize = 16; // 128-bit salt
const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM
const TAG_SIZE: usize = 16; // 128-bit authentication tag for AES-256-GCM

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

/// A failure encrypting or decrypting wallet secret material with AES-256-GCM.
///
/// `Display` is a complete, jargon-free sentence for the Everyday User; the
/// variant preserves which class of failure occurred so callers can branch
/// (e.g. wrong password vs a corrupt at-rest blob) without parsing strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EncryptionError {
    /// The AEAD tag did not verify — wrong password or tampered ciphertext.
    #[error("The password is incorrect. Please check it and try again.")]
    WrongPassword,
    /// The stored data is structurally invalid (bad envelope lengths or a
    /// corrupt at-rest blob).
    #[error(
        "This wallet's saved data appears to be damaged and could not be read. Re-add it from its recovery phrase to restore it."
    )]
    Malformed,
    /// The Argon2 key-derivation step failed.
    #[error("The encryption key could not be prepared. Please try again.")]
    KeyDerivation,
    /// The AES-256-GCM cipher failed to encrypt the data.
    #[error("The data could not be encrypted. Please try again.")]
    Encryption,
}

/// Derive a 32-byte AES-256 key from the password and salt using Argon2.
///
/// The key is returned in a [`Zeroizing`] buffer so the derived secret wipes on
/// drop instead of lingering in a plain `Vec<u8>` after the caller is done.
///
/// # Errors
///
/// Returns [`EncryptionError::KeyDerivation`] if Argon2 hashing fails.
pub fn derive_password_key(
    password: &str,
    salt: &[u8],
) -> Result<Zeroizing<Vec<u8>>, EncryptionError> {
    let key_length = 32; // For AES-256, we use a 256-bit key (32 bytes)

    let mut key = Zeroizing::new(vec![0u8; key_length]);

    // Using Argon2 with default parameters
    let argon2 = Argon2::default();

    // Deriving the key
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| EncryptionError::KeyDerivation)?;

    Ok(key)
}

/// Encrypt `message` under `password` with AES-256-GCM, returning the
/// [`EncryptedEnvelope`] (ciphertext, salt, nonce).
#[allow(deprecated)]
pub(crate) fn encrypt_message(
    message: &[u8],
    password: &str,
) -> Result<EncryptedEnvelope, EncryptionError> {
    // Generate a random salt
    let mut salt = vec![0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    // Derive the key
    let key = derive_password_key(password, &salt)?;

    // Generate a random nonce
    let mut nonce = vec![0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);

    // Create cipher instance
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| EncryptionError::Encryption)?;

    // Encrypt the seed
    let nonce_arr = Nonce::from_slice(&nonce);
    let ciphertext = cipher
        .encrypt(nonce_arr, message)
        .map_err(|_| EncryptionError::Encryption)?;

    Ok(EncryptedEnvelope {
        ciphertext,
        salt,
        nonce,
    })
}

/// Failure decrypting an AES-256-GCM envelope produced by [`encrypt_message`].
///
/// Two outcomes the callers must distinguish: an authentication failure
/// (`WrongPassword`) versus a structurally invalid envelope (`Malformed`).
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
/// `expected_plaintext_len` lets the shared reader reject an impossible
/// ciphertext/tag length before an AEAD failure can be mistaken for a wrong
/// password. Structural failures are logged with `site`; an authentication
/// failure on a well-formed envelope is [`DecryptError::WrongPassword`].
pub(crate) fn decrypt_message(
    ciphertext: &[u8],
    salt: &[u8],
    nonce: &[u8],
    expected_plaintext_len: usize,
    password: &str,
    site: &'static str,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let expected_ciphertext_len = expected_plaintext_len
        .checked_add(TAG_SIZE)
        .ok_or(DecryptError::Malformed)?;
    if ciphertext.len() != expected_ciphertext_len {
        tracing::warn!(
            target = "model::wallet::encryption",
            site,
            ciphertext_len = ciphertext.len(),
            expected_ciphertext_len,
            "Envelope ciphertext is not the expected length",
        );
        return Err(DecryptError::Malformed);
    }
    if salt.len() != SALT_SIZE {
        tracing::warn!(
            target = "model::wallet::encryption",
            site,
            salt_len = salt.len(),
            "Envelope salt is not the expected length",
        );
        return Err(DecryptError::Malformed);
    }
    let key = derive_password_key(password, salt).map_err(|error| {
        tracing::warn!(
            target = "model::wallet::encryption",
            site,
            %error,
            "Argon2 key derivation failed during decrypt",
        );
        DecryptError::Malformed
    })?;
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
    pub(crate) fn encrypt_seed(
        seed: &[u8],
        password: &str,
    ) -> Result<EncryptedEnvelope, EncryptionError> {
        encrypt_message(seed, password)
    }

    /// Decrypt the seed using AES-256-GCM via the shared [`decrypt_message`]
    /// reader.
    ///
    /// The 64-byte seed is returned in a [`Zeroizing`] buffer so it wipes on
    /// drop, matching every other seed reader. The bytes are copied straight
    /// into the zeroizing array — no plain `[u8; 64]` stack copy is left behind.
    pub fn decrypt_seed(&self, password: &str) -> Result<Zeroizing<[u8; 64]>, EncryptionError> {
        let seed = decrypt_message(
            &self.encrypted_seed,
            &self.salt,
            &self.nonce,
            64,
            password,
            "closed_key_item::decrypt_seed",
        )
        .map_err(|e| match e {
            DecryptError::WrongPassword => EncryptionError::WrongPassword,
            DecryptError::Malformed => EncryptionError::Malformed,
        })?;

        // A decrypted seed of the wrong length is a corrupt at-rest blob.
        if seed.len() != 64 {
            return Err(EncryptionError::Malformed);
        }
        let mut out = Zeroizing::new([0u8; 64]);
        out.copy_from_slice(&seed);
        Ok(out)
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
            encrypted_seed: Zeroizing::new(envelope.ciphertext),
            salt: envelope.salt,
            nonce: envelope.nonce,
            password_hint: None, // Set password hint if needed
        };

        // Decrypt the seed using the instance method
        let decrypted_seed = closed_wallet_seed
            .decrypt_seed(password)
            .expect("Decryption failed");

        // Verify that the decrypted seed matches the original seed
        assert_eq!(seed, *decrypted_seed);
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
            encrypted_seed: Zeroizing::new(envelope.ciphertext),
            salt: envelope.salt,
            nonce: envelope.nonce,
            password_hint: None,
        };

        // Attempt to decrypt with the wrong password
        let result = closed_wallet_seed.decrypt_seed(wrong_password);

        // Verify that decryption fails with the typed wrong-password variant.
        assert_eq!(result.unwrap_err(), EncryptionError::WrongPassword);
    }

    #[test]
    fn wrong_password_is_typed_variant() {
        let envelope = encrypt_message(b"payload", "right").expect("encrypt");
        let err = decrypt_message(
            &envelope.ciphertext,
            &envelope.salt,
            &envelope.nonce,
            b"payload".len(),
            "wrong",
            "test",
        )
        .unwrap_err();
        assert_eq!(err, DecryptError::WrongPassword);
    }

    #[test]
    fn malformed_envelope_is_typed_variant() {
        let envelope = encrypt_message(b"payload", "pw").expect("encrypt");
        // A nonce of the wrong length is a structurally corrupt blob, not a
        // wrong password — it must surface as Malformed, never panic.
        let err = decrypt_message(
            &envelope.ciphertext,
            &envelope.salt,
            &[0u8; 4],
            b"payload".len(),
            "pw",
            "test",
        )
        .unwrap_err();
        assert_eq!(err, DecryptError::Malformed);
    }

    #[test]
    fn decrypt_seed_wrong_length_blob_is_malformed() {
        // Encrypt a 10-byte payload, then decrypt_seed it: the plaintext is not
        // 64 bytes, so it is a corrupt seed blob → Malformed (no panic).
        let envelope = encrypt_message(&[9u8; 10], "pw").expect("encrypt");
        let item = ClosedKeyItem {
            seed_hash: [0u8; 32],
            encrypted_seed: Zeroizing::new(envelope.ciphertext),
            salt: envelope.salt,
            nonce: envelope.nonce,
            password_hint: None,
        };
        assert_eq!(
            item.decrypt_seed("pw").unwrap_err(),
            EncryptionError::Malformed
        );
    }
}
