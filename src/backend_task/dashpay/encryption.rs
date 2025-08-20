use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, KeyInit}};
use aes_gcm::aes::Aes256;
use cbc;
use dash_sdk::dpp::identity::IdentityPublicKey;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::KeyType;
use bip39::rand::{self, RngCore};
use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, SecretKey, Secp256k1};
use sha2::{Sha256, Digest};

/// Generate ECDH shared key according to DashPay DIP-15
/// Uses libsecp256k1_ecdh method: SHA256((y[31]&0x1|0x2) || x)
pub fn generate_ecdh_shared_key(
    private_key: &[u8],
    public_key: &IdentityPublicKey,
) -> Result<[u8; 32], String> {
    let _secp = Secp256k1::new();
    
    // Parse the private key
    let secret_key = SecretKey::from_slice(private_key)
        .map_err(|e| format!("Invalid private key: {}", e))?;
    
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
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    
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
    
    let ciphertext = cipher.encrypt_padded_mut::<Pkcs7>(&mut buffer, xpub_data.len())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;
    
    // Verify the ciphertext is exactly 80 bytes
    if ciphertext.len() != 80 {
        return Err(format!("Unexpected ciphertext length: {} (expected 80)", ciphertext.len()));
    }
    
    // Combine IV and ciphertext (16 + 80 = 96 bytes total)
    let mut result = Vec::with_capacity(96);
    result.extend_from_slice(&iv);
    result.extend_from_slice(&ciphertext);
    
    Ok(result)
}

/// Encrypt account label according to DashPay DIP-15
/// Format: IV (16 bytes) + Encrypted Data (32-64 bytes) = 48-80 bytes total
/// Uses CBC-AES-256 as specified in the DIP
pub fn encrypt_account_label(
    label: &str,
    shared_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    
    let label_bytes = label.as_bytes();
    
    // Label length check (must fit in 32-64 bytes after padding)
    if label_bytes.len() > 64 {
        return Err("Account label too long (max 64 characters)".to_string());
    }
    
    // Generate random IV (16 bytes for CBC)
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);
    
    // Encrypt using CBC-AES-256 with PKCS7 padding
    type Aes256CbcEnc = cbc::Encryptor<Aes256>;
    let cipher = Aes256CbcEnc::new(shared_key.into(), &iv.into());
    
    // Encrypt with padding (will be padded to next multiple of 16)
    // Calculate padded length (next multiple of 16) with extra space for padding
    let padded_len = ((label_bytes.len() + 16) / 16) * 16; // Add full block for padding
    let mut buffer = vec![0u8; padded_len];
    buffer[..label_bytes.len()].copy_from_slice(label_bytes);
    
    let ciphertext = cipher.encrypt_padded_mut::<Pkcs7>(&mut buffer, label_bytes.len())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;
    
    // Verify the ciphertext is within expected range (32-64 bytes)
    if ciphertext.len() < 32 || ciphertext.len() > 64 {
        return Err(format!("Unexpected ciphertext length: {} (expected 32-64)", ciphertext.len()));
    }
    
    // Combine IV and ciphertext (16 + (32-64) = 48-80 bytes total)
    let mut result = Vec::with_capacity(16 + ciphertext.len());
    result.extend_from_slice(&iv);
    result.extend_from_slice(&ciphertext);
    
    Ok(result)
}

/// Decrypt extended public key
pub fn decrypt_extended_public_key(
    encrypted_data: &[u8],
    shared_key: &[u8; 32],
) -> Result<(Vec<u8>, [u8; 32], [u8; 33]), String> {
    if encrypted_data.len() < 12 + 16 { // nonce + minimum encrypted data + auth tag
        return Err("Invalid encrypted public key length".to_string());
    }
    
    let nonce_bytes = &encrypted_data[..12];
    let ciphertext = &encrypted_data[12..];
    let nonce = Nonce::from_slice(nonce_bytes);
    
    // Decrypt using AES-256-GCM
    let key = Key::<Aes256Gcm>::from_slice(shared_key);
    let cipher = Aes256Gcm::new(key);
    
    let decrypted = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;
    
    if decrypted.len() != 69 {
        return Err("Invalid decrypted data length".to_string());
    }
    
    let parent_fingerprint = decrypted[..4].to_vec();
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&decrypted[4..36]);
    let mut public_key = [0u8; 33];
    public_key.copy_from_slice(&decrypted[36..69]);
    
    Ok((parent_fingerprint, chain_code, public_key))
}

/// Decrypt account label
pub fn decrypt_account_label(
    encrypted_data: &[u8],
    shared_key: &[u8; 32],
) -> Result<String, String> {
    if encrypted_data.len() < 12 + 16 { // nonce + minimum data + auth tag
        return Err("Invalid encrypted label length".to_string());
    }
    
    let nonce_bytes = &encrypted_data[..12];
    let ciphertext = &encrypted_data[12..];
    let nonce = Nonce::from_slice(nonce_bytes);
    
    // Decrypt using AES-256-GCM
    let key = Key::<Aes256Gcm>::from_slice(shared_key);
    let cipher = Aes256Gcm::new(key);
    
    let decrypted = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;
    
    // Convert to string
    String::from_utf8(decrypted)
        .map_err(|e| format!("Invalid UTF-8 in decrypted label: {}", e))
}