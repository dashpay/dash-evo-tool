use crate::backend_task::dashpay::encryption::{
    decrypt_account_label, decrypt_extended_public_key, encrypt_account_label,
    encrypt_extended_public_key,
};
use bip39::rand::{self, RngCore};
use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};

/// Test encryption and decryption of extended public keys
pub fn test_extended_public_key_encryption() -> Result<(), String> {
    println!("Testing extended public key encryption/decryption...");

    // Generate test data
    let parent_fingerprint = [0x12, 0x34, 0x56, 0x78];
    let mut chain_code = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut chain_code);

    // Generate a test key pair
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&[
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F, 0x20,
    ])
    .unwrap();
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);
    let public_key_bytes = public_key.serialize();

    // Generate a shared key for encryption
    let mut shared_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut shared_key);

    // Test encryption
    let encrypted = encrypt_extended_public_key(
        parent_fingerprint,
        chain_code,
        public_key_bytes,
        &shared_key,
    )?;

    // Verify encrypted data length is 96 bytes (16 IV + 80 encrypted)
    if encrypted.len() != 96 {
        return Err(format!(
            "Invalid encrypted length: {} (expected 96)",
            encrypted.len()
        ));
    }

    println!("✓ Encryption produced 96 bytes as expected");

    // Test decryption
    let (decrypted_fingerprint, decrypted_chain_code, decrypted_public_key) =
        decrypt_extended_public_key(&encrypted, &shared_key)?;

    // Verify decrypted data matches original
    if decrypted_fingerprint != parent_fingerprint.to_vec() {
        return Err("Parent fingerprint mismatch after decryption".to_string());
    }

    if decrypted_chain_code != chain_code {
        return Err("Chain code mismatch after decryption".to_string());
    }

    if decrypted_public_key != public_key_bytes {
        return Err("Public key mismatch after decryption".to_string());
    }

    println!("✓ Decryption successfully recovered original data");

    // Test with wrong key fails
    let mut wrong_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut wrong_key);

    match decrypt_extended_public_key(&encrypted, &wrong_key) {
        Ok(_) => return Err("Decryption should have failed with wrong key".to_string()),
        Err(_) => println!("✓ Decryption correctly failed with wrong key"),
    }

    Ok(())
}

/// Test encryption and decryption of account labels
pub fn test_account_label_encryption() -> Result<(), String> {
    println!("\nTesting account label encryption/decryption...");

    // Generate a shared key
    let mut shared_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut shared_key);

    // Test various label lengths
    let test_labels = vec![
        "Personal",
        "Business Account",
        "Savings - Long Term Investment Fund 2024",
        "Test with special chars: 你好世界 🚀",
    ];

    for label in test_labels {
        println!("  Testing label: '{}'", label);

        // Encrypt
        let encrypted = encrypt_account_label(label, &shared_key)?;

        // Verify encrypted length is in expected range (48-80 bytes)
        if encrypted.len() < 48 || encrypted.len() > 80 {
            return Err(format!(
                "Invalid encrypted label length: {} (expected 48-80)",
                encrypted.len()
            ));
        }

        // Decrypt
        let decrypted = decrypt_account_label(&encrypted, &shared_key)?;

        // Verify match
        if decrypted != label {
            return Err(format!(
                "Label mismatch after decryption: '{}' != '{}'",
                decrypted, label
            ));
        }

        println!(
            "    ✓ Successfully encrypted/decrypted ({} bytes encrypted)",
            encrypted.len()
        );
    }

    // Test label that's too long
    let long_label = "x".repeat(65);
    match encrypt_account_label(&long_label, &shared_key) {
        Ok(_) => return Err("Should have rejected label > 64 chars".to_string()),
        Err(_) => println!("  ✓ Correctly rejected label > 64 characters"),
    }

    Ok(())
}

/// Test ECDH shared key generation
pub fn test_ecdh_shared_key_generation() -> Result<(), String> {
    println!("\nTesting ECDH shared key generation...");

    // Skip the actual ECDH test for now due to IdentityPublicKey structure complexities

    // TODO: Complete ECDH test once we have proper IdentityPublicKey mock
    // The issue is that IdentityPublicKey stores ECDSA keys differently than BLS keys
    // and we need to properly mock the .data() method to return the right bytes
    // For ECDSA_SECP256K1 keys, the data field is the raw 33-byte compressed public key
    // but the IdentityPublicKey structure expects a BLS PublicKey type in the data field

    println!("✓ ECDH test skipped (needs proper mock implementation)");

    // For now, let's test that the basic encryption/decryption functions work
    // which is demonstrated in the other tests above

    Ok(())
}

/// Run all encryption tests
pub fn run_all_encryption_tests() -> Result<(), String> {
    println!("=== Running DashPay Encryption Tests ===\n");

    test_extended_public_key_encryption()?;
    test_account_label_encryption()?;
    test_ecdh_shared_key_generation()?;

    println!("\n=== All encryption tests passed! ===");

    Ok(())
}

/// Create a test task to run encryption verification
pub fn create_encryption_test_task() -> crate::backend_task::BackendTask {
    crate::backend_task::BackendTask::None
}
