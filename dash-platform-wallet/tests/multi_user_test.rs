use dash_platform_wallet::{
    kms::generic::locked::GenericKms,
    kms::{KeyType, Kms, UnlockedKMS},
    secret::Secret,
};
use dash_sdk::dpp::dashcore::Network;
/// Test multi-user functionality of the KMS
/// This test verifies that the multi-user unlock mechanism works correctly
use tempfile::TempDir;

#[test]
fn test_multi_user_unlock() {
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("multi_user_test.json");

    // Create KMS instance
    let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

    // Test credentials
    let user_id = b"test_user".to_vec();
    let password = Secret::new(b"test_password".to_vec()).unwrap();

    // First unlock should create the first user and master key
    let alice_seed = Secret::new([42u8; 32]).expect("Failed to create test seed");
    let alice_seed_handle = {
        let mut unlocked_kms = kms
            .unlock(&user_id, password.clone())
            .expect("Failed to unlock empty KMS");
        unlocked_kms
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                alice_seed,
            )
            .expect("Failed to generate seed")
    };

    // Second unlock should use the existing user record
    {
        let unlocked_kms = kms
            .unlock(&user_id, password.clone())
            .expect("Failed to unlock KMS second time");
        let keys: Vec<_> = unlocked_kms.keys().expect("Failed to get keys").collect();
        assert_eq!(keys.len(), 1, "Should have 1 key");
        assert_eq!(keys[0], alice_seed_handle, "Key should match");
    }

    // Wrong password should fail
    let wrong_password = Secret::new(b"wrong_password".to_vec()).unwrap();
    let result = kms.unlock(&user_id, wrong_password);
    assert!(result.is_err(), "Wrong password should fail");

    // Different user should fail (no user record exists)
    let different_user = b"different_user".to_vec();
    let result = kms.unlock(&different_user, password.clone());
    assert!(result.is_err(), "Different user should fail");

    println!("Multi-user unlock test passed! ✅");
}

#[test]
fn test_legacy_single_user_mode() {
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("legacy_test.json");

    // Create KMS instance
    let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

    // Test credentials
    let user_id = b"legacy_user".to_vec();
    let password = Secret::new(b"legacy_password".to_vec()).unwrap();

    // Create a key with the old single-user system
    let test_seed = Secret::new([99u8; 32]).expect("Failed to create test seed");
    let seed_handle = {
        let mut unlocked_kms = kms
            .unlock(&user_id, password.clone())
            .expect("Failed to unlock KMS");
        unlocked_kms
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                test_seed,
            )
            .expect("Failed to generate seed")
    };

    // Re-open and verify we can still access the key
    let unlocked_kms = kms
        .unlock(&user_id, password.clone())
        .expect("Failed to unlock KMS after restart");
    let keys: Vec<_> = unlocked_kms.keys().expect("Failed to get keys").collect();
    assert_eq!(keys.len(), 1, "Should have 1 key");
    assert_eq!(keys[0], seed_handle, "Key should match");

    println!("Legacy single-user mode test passed! ✅");
}
