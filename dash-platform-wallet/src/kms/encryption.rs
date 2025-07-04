use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{self, Argon2};
use bip39::rand::{RngCore, rngs::OsRng};

const SALT_SIZE: usize = 16; // 128-bit salt
pub const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM

pub const DASH_SECRET_MESSAGE: &[u8; 19] = b"dash_secret_message";

use crate::secret::Secret;

/// Derive a key from the password and salt using Argon2.
///
/// As salt, we use user_id provided by the user.
pub fn derive_password_key(user_id: &[u8], password: &Secret) -> Result<Secret, String> {
    const KEY_SIZE: usize = 32; // For AES-256, we use a 256-bit key (32 bytes)

    let mut key = [0u8; KEY_SIZE];

    // Using Argon2 with default parameters
    let argon2 = Argon2::default();

    // Deriving the key
    argon2
        .hash_password_into(&password.as_ref(), user_id, &mut key)
        .map_err(|e| e.to_string())?;

    Secret::new(key).map_err(|e| e.to_string())
}

/// Encrypt the seed using AES-256-GCM.
#[allow(clippy::type_complexity)]
pub fn encrypt_message<'msg, 'aad, T>(
    message: T,
    user_id: &[u8],
    password: &Secret,
) -> Result<(Vec<u8>, Vec<u8>), String>
where
    T: Into<Payload<'msg, 'aad>>,
{
    // Derive the key
    let key = derive_password_key(user_id, password)?;

    // Generate a random nonce
    let mut nonce = vec![0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);

    // Create cipher instance
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| e.to_string())?;

    // Encrypt the seed
    let encrypted_seed = cipher
        .encrypt(Nonce::from_slice(&nonce), message)
        .map_err(|e| e.to_string())?;

    Ok((encrypted_seed, nonce))
}

pub fn decrypt_message(
    encrypted_data: &[u8],
    nonce: &[u8],
    key: &Secret,
) -> Result<Vec<u8>, String> {
    // Create cipher instance

    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| e.to_string())?;

    // Decrypt the data
    cipher
        .decrypt(Nonce::from_slice(nonce), encrypted_data)
        .map_err(|e| e.to_string())
}
