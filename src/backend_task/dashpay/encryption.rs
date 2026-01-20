use aes_gcm::aes::Aes256;
use bip39::rand::{self, RngCore};
use cbc;
use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};
use dash_sdk::dpp::identity::IdentityPublicKey;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use sha2::{Digest, Sha256};

/// Generate ECDH shared key according to DashPay DIP-15
/// Uses libsecp256k1_ecdh method: SHA256((y[31]&0x1|0x2) || x)
pub fn generate_ecdh_shared_key(
    private_key: &[u8],
    public_key: &IdentityPublicKey,
) -> Result<[u8; 32], String> {
    let _secp = Secp256k1::new();

    // Parse the private key
    let secret_key =
        SecretKey::from_slice(private_key).map_err(|e| format!("Invalid private key: {}", e))?;

    // Get the public key data - only works for full secp256k1 keys
    match public_key.key_type() {
        KeyType::ECDSA_SECP256K1 => {
            let public_key_data = public_key.data();
            let public_key = PublicKey::from_slice(public_key_data.as_slice())
                .map_err(|e| format!("Invalid public key: {}", e))?;

            // Perform ECDH to get shared secret
            let shared_secret = dash_sdk::dpp::dashcore::secp256k1::ecdh::shared_secret_point(&public_key, &secret_key);

            // Extract x and y coordinates (64 bytes total: 32 + 32)
            let x = &shared_secret[..32];
            let y = &shared_secret[32..];

            // Determine the prefix based on y coordinate parity
            let prefix = if y[31] & 0x1 == 1 { 0x03u8 } else { 0x02u8 };

            // Create the input for SHA256: prefix || x
            let mut hasher = Sha256::new();
            hasher.update([prefix]);
            hasher.update(x);

            let result = hasher.finalize();
            let mut shared_key = [0u8; 32];
            shared_key.copy_from_slice(&result);

            Ok(shared_key)
        }
        KeyType::ECDSA_HASH160 => {
            Err("Cannot perform ECDH with ECDSA_HASH160 key type - only hash is available, not full public key".to_string())
        }
        _ => {
            Err(format!("Unsupported key type for ECDH: {:?}", public_key.key_type()))
        }
    }
}

/// Create encrypted extended public key according to DashPay DIP-15
/// Format: IV (16 bytes) + Encrypted Data (80 bytes) = 96 bytes total
/// Uses CBC-AES-256 as specified in the DIP
pub fn encrypt_extended_public_key(
    parent_fingerprint: [u8; 4],
    chain_code: [u8; 32],
    public_key: [u8; 33],
    shared_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};

    // Create the extended public key data (69 bytes)
    let mut xpub_data = Vec::with_capacity(69);
    xpub_data.extend_from_slice(&parent_fingerprint);
    xpub_data.extend_from_slice(&chain_code);
    xpub_data.extend_from_slice(&public_key);

    // Generate random IV (16 bytes for CBC)
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);

    // Encrypt using CBC-AES-256 with PKCS7 padding
    type Aes256CbcEnc = cbc::Encryptor<Aes256>;
    let cipher = Aes256CbcEnc::new(shared_key.into(), &iv.into());

    // The xpub_data is 69 bytes, which will be padded to 80 bytes (next multiple of 16)
    // We need to create a buffer with room for padding
    let mut buffer = vec![0u8; 80]; // 69 bytes padded to 80 (next multiple of 16)
    buffer[..xpub_data.len()].copy_from_slice(&xpub_data);

    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, xpub_data.len())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    // Verify the ciphertext is exactly 80 bytes
    if ciphertext.len() != 80 {
        return Err(format!(
            "Unexpected ciphertext length: {} (expected 80)",
            ciphertext.len()
        ));
    }

    // Combine IV and ciphertext (16 + 80 = 96 bytes total)
    let mut result = Vec::with_capacity(96);
    result.extend_from_slice(&iv);
    result.extend_from_slice(ciphertext);

    Ok(result)
}

/// Encrypt account label according to DashPay DIP-15
/// Format: IV (16 bytes) + Encrypted Data (32-64 bytes) = 48-80 bytes total
/// Uses CBC-AES-256 as specified in the DIP
///
/// Note: Maximum label length is 62 bytes due to the internal format:
/// - 1 byte length prefix + label bytes + PKCS7 padding
/// - For 63 bytes: 1 + 63 = 64, PKCS7 adds 16 = 80 byte ciphertext = 96 total (exceeds limit)
/// - For 62 bytes: 1 + 62 = 63, PKCS7 adds 1 = 64 byte ciphertext = 80 total (at limit)
pub fn encrypt_account_label(label: &str, shared_key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};

    let label_bytes = label.as_bytes();

    // Label length check
    // Max 62 bytes due to 1-byte length prefix + PKCS7 padding constraints
    if label_bytes.is_empty() {
        return Err("Account label cannot be empty".to_string());
    }
    if label_bytes.len() > 62 {
        return Err("Account label too long (max 62 bytes)".to_string());
    }

    // To ensure minimum ciphertext size of 32 bytes, pad the label to at least 16 bytes
    // This way, with PKCS7 padding, we'll get at least 32 bytes of ciphertext
    // We use a simple length prefix approach: [len][label][zeros...]
    let min_label_len = 16;
    let padded_label = if label_bytes.len() < min_label_len {
        let mut padded = vec![label_bytes.len() as u8]; // Store original length as first byte
        padded.extend_from_slice(label_bytes);
        // Pad with zeros to reach min_label_len
        padded.resize(min_label_len, 0);
        padded
    } else {
        // For longer labels, just prepend the length
        let mut padded = vec![label_bytes.len() as u8];
        padded.extend_from_slice(label_bytes);
        padded
    };

    // Generate random IV (16 bytes for CBC)
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);

    // Encrypt using CBC-AES-256 with PKCS7 padding
    type Aes256CbcEnc = cbc::Encryptor<Aes256>;
    let cipher = Aes256CbcEnc::new(shared_key.into(), &iv.into());

    // Calculate buffer size for PKCS7 padding
    let padded_len = if padded_label.len() % 16 == 0 {
        padded_label.len() + 16 // Add full padding block
    } else {
        ((padded_label.len() / 16) + 1) * 16 // Round up to next multiple of 16
    };

    let mut buffer = vec![0u8; padded_len];
    buffer[..padded_label.len()].copy_from_slice(&padded_label);

    // Encrypt with PKCS7 padding
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, padded_label.len())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    // Combine IV and ciphertext
    let mut result = Vec::with_capacity(16 + ciphertext.len());
    result.extend_from_slice(&iv);
    result.extend_from_slice(ciphertext);

    // Verify the final result is within expected range (48-80 bytes as per validation)
    // IV: 16 bytes + ciphertext: 32-64 bytes = 48-80 bytes total
    if result.len() < 48 || result.len() > 80 {
        return Err(format!(
            "Unexpected encrypted result length: {} (expected 48-80)",
            result.len()
        ));
    }

    Ok(result)
}

/// Decrypt extended public key using CBC-AES-256
#[allow(clippy::type_complexity)]
pub fn decrypt_extended_public_key(
    encrypted_data: &[u8],
    shared_key: &[u8; 32],
) -> Result<(Vec<u8>, [u8; 32], [u8; 33]), String> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};

    // Expected format: IV (16 bytes) + Encrypted Data (80 bytes) = 96 bytes
    if encrypted_data.len() != 96 {
        return Err(format!(
            "Invalid encrypted public key length: {} (expected 96)",
            encrypted_data.len()
        ));
    }

    // Extract IV and ciphertext
    let iv = &encrypted_data[..16];
    let ciphertext = &encrypted_data[16..];

    // Decrypt using CBC-AES-256 with PKCS7 padding
    type Aes256CbcDec = cbc::Decryptor<Aes256>;
    let cipher = Aes256CbcDec::new(shared_key.into(), iv.into());

    let mut buffer = ciphertext.to_vec();
    let decrypted = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|e| format!("Decryption failed: {:?}", e))?;

    // Should decrypt to exactly 69 bytes after removing padding
    if decrypted.len() != 69 {
        return Err(format!(
            "Invalid decrypted data length: {} (expected 69)",
            decrypted.len()
        ));
    }

    let parent_fingerprint = decrypted[..4].to_vec();
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&decrypted[4..36]);
    let mut public_key = [0u8; 33];
    public_key.copy_from_slice(&decrypted[36..69]);

    Ok((parent_fingerprint, chain_code, public_key))
}

/// Decrypt account label using CBC-AES-256
pub fn decrypt_account_label(
    encrypted_data: &[u8],
    shared_key: &[u8; 32],
) -> Result<String, String> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};

    // Expected format: IV (16 bytes) + Encrypted Data (32-64 bytes) = 48-80 bytes
    if encrypted_data.len() < 48 || encrypted_data.len() > 80 {
        return Err(format!(
            "Invalid encrypted label length: {} (expected 48-80)",
            encrypted_data.len()
        ));
    }

    // Extract IV and ciphertext
    let iv = &encrypted_data[..16];
    let ciphertext = &encrypted_data[16..];

    // Decrypt using CBC-AES-256 with PKCS7 padding
    type Aes256CbcDec = cbc::Decryptor<Aes256>;
    let cipher = Aes256CbcDec::new(shared_key.into(), iv.into());

    let mut buffer = ciphertext.to_vec();
    let decrypted = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|e| format!("Decryption failed: {:?}", e))?;

    // Extract the actual label from our custom format: [len][label][padding...]
    if decrypted.is_empty() {
        return Err("Decrypted data is empty".to_string());
    }

    let label_len = decrypted[0] as usize;
    if label_len == 0 || label_len > decrypted.len() - 1 {
        return Err(format!("Invalid label length: {}", label_len));
    }

    // Extract the actual label bytes
    let label_bytes = &decrypted[1..=label_len];

    // Convert to string
    String::from_utf8(label_bytes.to_vec())
        .map_err(|e| format!("Invalid UTF-8 in decrypted label: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bip39::rand::{self, RngCore};
    use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn generate_test_shared_key() -> [u8; 32] {
        let mut shared_key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut shared_key);
        shared_key
    }

    fn generate_test_key_pair() -> (SecretKey, PublicKey) {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C,
            0x1D, 0x1E, 0x1F, 0x20,
        ])
        .unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        (secret_key, public_key)
    }

    #[test]
    fn test_encrypt_decrypt_extended_public_key_roundtrip() {
        // Generate test data
        let parent_fingerprint = [0x12, 0x34, 0x56, 0x78];
        let mut chain_code = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut chain_code);

        let (_, public_key) = generate_test_key_pair();
        let public_key_bytes = public_key.serialize();

        let shared_key = generate_test_shared_key();

        // Encrypt
        let encrypted = encrypt_extended_public_key(
            parent_fingerprint,
            chain_code,
            public_key_bytes,
            &shared_key,
        )
        .expect("Encryption should succeed");

        // Verify encrypted data length is 96 bytes (16 IV + 80 encrypted)
        assert_eq!(encrypted.len(), 96, "Encrypted data should be 96 bytes");

        // Decrypt
        let (decrypted_fingerprint, decrypted_chain_code, decrypted_public_key) =
            decrypt_extended_public_key(&encrypted, &shared_key)
                .expect("Decryption should succeed");

        // Verify decrypted data matches original
        assert_eq!(
            decrypted_fingerprint,
            parent_fingerprint.to_vec(),
            "Parent fingerprint should match"
        );
        assert_eq!(
            decrypted_chain_code, chain_code,
            "Chain code should match"
        );
        assert_eq!(
            decrypted_public_key, public_key_bytes,
            "Public key should match"
        );
    }

    #[test]
    fn test_encrypt_decrypt_account_label_roundtrip() {
        let shared_key = generate_test_shared_key();

        // Test various label lengths
        let test_labels = vec![
            "Personal",
            "Business Account",
            "Savings - Long Term Investment Fund 2024",
            "Short",
        ];

        for label in test_labels {
            let encrypted = encrypt_account_label(label, &shared_key)
                .expect("Encryption should succeed");

            // Verify encrypted length is in expected range (48-80 bytes)
            assert!(
                encrypted.len() >= 48 && encrypted.len() <= 80,
                "Encrypted label length {} should be 48-80",
                encrypted.len()
            );

            let decrypted = decrypt_account_label(&encrypted, &shared_key)
                .expect("Decryption should succeed");

            assert_eq!(decrypted, label, "Decrypted label should match original");
        }
    }

    #[test]
    fn test_account_label_with_unicode() {
        let shared_key = generate_test_shared_key();

        // Test with unicode characters
        let label = "你好世界"; // "Hello World" in Chinese

        let encrypted = encrypt_account_label(label, &shared_key)
            .expect("Encryption should succeed");

        let decrypted = decrypt_account_label(&encrypted, &shared_key)
            .expect("Decryption should succeed");

        assert_eq!(decrypted, label, "Unicode label should roundtrip correctly");
    }

    #[test]
    fn test_account_label_length_validation() {
        let shared_key = generate_test_shared_key();

        // Test empty label - should fail
        let result = encrypt_account_label("", &shared_key);
        assert!(result.is_err(), "Empty label should be rejected");
        assert!(
            result.unwrap_err().contains("empty"),
            "Error should mention empty"
        );

        // Test label that's too long (> 62 bytes) - should fail
        let long_label = "x".repeat(63);
        let result = encrypt_account_label(&long_label, &shared_key);
        assert!(result.is_err(), "Label > 62 bytes should be rejected");
        assert!(
            result.unwrap_err().contains("too long"),
            "Error should mention too long"
        );

        // Test label at exactly the limit (62 bytes) - should succeed
        let max_label = "x".repeat(62);
        let result = encrypt_account_label(&max_label, &shared_key);
        assert!(result.is_ok(), "Label of 62 bytes should be accepted");

        // Test label just under the limit - should succeed
        let valid_label = "x".repeat(45);
        let result = encrypt_account_label(&valid_label, &shared_key);
        assert!(result.is_ok(), "Label of 45 bytes should be accepted");
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let shared_key = generate_test_shared_key();
        let wrong_key = generate_test_shared_key();

        // Generate test data
        let parent_fingerprint = [0x12, 0x34, 0x56, 0x78];
        let mut chain_code = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut chain_code);

        let (_, public_key) = generate_test_key_pair();
        let public_key_bytes = public_key.serialize();

        // Encrypt with correct key
        let encrypted = encrypt_extended_public_key(
            parent_fingerprint,
            chain_code,
            public_key_bytes,
            &shared_key,
        )
        .expect("Encryption should succeed");

        // Try to decrypt with wrong key - should fail
        let result = decrypt_extended_public_key(&encrypted, &wrong_key);
        assert!(
            result.is_err(),
            "Decryption with wrong key should fail"
        );
    }

    #[test]
    fn test_decrypt_account_label_with_wrong_key_fails() {
        let shared_key = generate_test_shared_key();
        let wrong_key = generate_test_shared_key();

        let encrypted = encrypt_account_label("Test Label", &shared_key)
            .expect("Encryption should succeed");

        let result = decrypt_account_label(&encrypted, &wrong_key);
        assert!(
            result.is_err(),
            "Decryption with wrong key should fail"
        );
    }

    #[test]
    fn test_invalid_encrypted_data_length() {
        let shared_key = generate_test_shared_key();

        // Test extended public key with wrong length
        let too_short = vec![0u8; 50];
        let result = decrypt_extended_public_key(&too_short, &shared_key);
        assert!(result.is_err(), "Too short data should be rejected");

        let too_long = vec![0u8; 100];
        let result = decrypt_extended_public_key(&too_long, &shared_key);
        assert!(result.is_err(), "Too long data should be rejected");

        // Test account label with wrong length
        let too_short_label = vec![0u8; 30];
        let result = decrypt_account_label(&too_short_label, &shared_key);
        assert!(result.is_err(), "Too short label data should be rejected");

        let too_long_label = vec![0u8; 100];
        let result = decrypt_account_label(&too_long_label, &shared_key);
        assert!(result.is_err(), "Too long label data should be rejected");
    }

    #[test]
    fn test_encryption_produces_different_ciphertext() {
        let shared_key = generate_test_shared_key();

        // Encrypt the same data twice
        let parent_fingerprint = [0x12, 0x34, 0x56, 0x78];
        let chain_code = [0xAB; 32];
        let (_, public_key) = generate_test_key_pair();
        let public_key_bytes = public_key.serialize();

        let encrypted1 = encrypt_extended_public_key(
            parent_fingerprint,
            chain_code,
            public_key_bytes,
            &shared_key,
        )
        .expect("Encryption should succeed");

        let encrypted2 = encrypt_extended_public_key(
            parent_fingerprint,
            chain_code,
            public_key_bytes,
            &shared_key,
        )
        .expect("Encryption should succeed");

        // Due to random IV, the ciphertexts should be different
        assert_ne!(
            encrypted1, encrypted2,
            "Random IVs should produce different ciphertexts"
        );

        // But both should decrypt to the same value
        let (fp1, cc1, pk1) =
            decrypt_extended_public_key(&encrypted1, &shared_key).unwrap();
        let (fp2, cc2, pk2) =
            decrypt_extended_public_key(&encrypted2, &shared_key).unwrap();

        assert_eq!(fp1, fp2);
        assert_eq!(cc1, cc2);
        assert_eq!(pk1, pk2);
    }
}
