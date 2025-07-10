use dash_platform_wallet::{
    kms::generic::locked::GenericKms,
    kms::{KeyType, Kms, UnlockedKMS},
    secret::Secret,
};
use dash_sdk::dpp::dashcore::Network;
/// Test multi-user functionality of the KMS
/// This test verifies that multiple users can access the same key store and user management
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

#[test]
fn test_add_multiple_users() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("multi_user_add_test.json");
    let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

    // First user (admin) credentials
    let admin_id = b"admin".to_vec();
    let admin_password = Secret::new(b"admin_password".to_vec()).unwrap();

    // Create initial key with admin user
    let initial_seed = Secret::new([11u8; 32]).expect("Failed to create seed");
    let initial_key = {
        let mut unlocked = kms
            .login(&admin_id, admin_password.clone())
            .expect("Failed to login as admin");
        unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                initial_seed,
            )
            .expect("Failed to generate initial key")
    };

    // Add multiple users
    let user1_id = b"user1".to_vec();
    let user1_password = Secret::new(b"user1_password".to_vec()).unwrap();
    let user2_id = b"user2".to_vec();
    let user2_password = Secret::new(b"user2_password".to_vec()).unwrap();
    let user3_id = b"user3".to_vec();
    let user3_password = Secret::new(b"user3_password".to_vec()).unwrap();

    {
        let mut admin_unlocked = kms
            .login(&admin_id, admin_password.clone())
            .expect("Failed to login as admin");
        admin_unlocked
            .add_user(&user1_id, user1_password.clone())
            .expect("Failed to add user1");
        admin_unlocked
            .add_user(&user2_id, user2_password.clone())
            .expect("Failed to add user2");
        admin_unlocked
            .add_user(&user3_id, user3_password.clone())
            .expect("Failed to add user3");

        // Verify user list
        let users = admin_unlocked.list_users().expect("Failed to list users");
        assert_eq!(users.len(), 4, "Should have 4 users (admin + 3 added)");
        assert!(users.contains(&admin_id), "Should contain admin");
        assert!(users.contains(&user1_id), "Should contain user1");
        assert!(users.contains(&user2_id), "Should contain user2");
        assert!(users.contains(&user3_id), "Should contain user3");
    }

    // Verify all users can access the same keys
    for (user_id, password) in [
        (&admin_id, &admin_password),
        (&user1_id, &user1_password),
        (&user2_id, &user2_password),
        (&user3_id, &user3_password),
    ] {
        let unlocked = kms
            .login(user_id, password.clone())
            .expect(&format!("Failed to login as {:?}", user_id));
        let keys: Vec<_> = unlocked.keys().expect("Failed to get keys").collect();
        assert_eq!(keys.len(), 1, "Each user should see the same key");
        assert_eq!(keys[0], initial_key, "Key should match initial key");
    }

    // Each user should see the same user list
    for (user_id, password) in [(&user1_id, &user1_password), (&user2_id, &user2_password)] {
        let unlocked = kms
            .login(user_id, password.clone())
            .expect(&format!("Failed to login as {:?}", user_id));
        let users = unlocked.list_users().expect("Failed to list users");
        assert_eq!(users.len(), 4, "All users should see the same user list");
    }

    println!("Add multiple users test passed! ✅");
}

#[test]
fn test_user_password_change() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("password_change_test.json");
    let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

    // Initial user
    let user_id = b"changeable_user".to_vec();
    let old_password = Secret::new(b"old_password_123".to_vec()).unwrap();
    let new_password = Secret::new(b"new_password_456".to_vec()).unwrap();

    // Create initial setup
    let test_seed = Secret::new([33u8; 32]).expect("Failed to create seed");
    let test_key = {
        let mut unlocked = kms
            .login(&user_id, old_password.clone())
            .expect("Failed to login with old password");
        unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                test_seed,
            )
            .expect("Failed to generate test key")
    };

    // Change password
    {
        let mut unlocked = kms
            .login(&user_id, old_password.clone())
            .expect("Failed to login with old password");
        unlocked
            .change_user_password(&user_id, new_password.clone())
            .expect("Failed to change password");
    }

    // Old password should no longer work
    let result = kms.login(&user_id, old_password);
    assert!(result.is_err(), "Old password should no longer work");

    // New password should work and access same keys
    {
        let unlocked = kms
            .login(&user_id, new_password.clone())
            .expect("Failed to login with new password");
        let keys: Vec<_> = unlocked.keys().expect("Failed to get keys").collect();
        assert_eq!(keys.len(), 1, "Should still have the same key");
        assert_eq!(keys[0], test_key, "Key should be unchanged");
    }

    // Change password again to ensure it works multiple times
    let newer_password = Secret::new(b"newer_password_789".to_vec()).unwrap();
    {
        let mut unlocked = kms
            .login(&user_id, new_password.clone())
            .expect("Failed to login with new password");
        unlocked
            .change_user_password(&user_id, newer_password.clone())
            .expect("Failed to change password again");
    }

    // Verify newest password works
    {
        let unlocked = kms
            .login(&user_id, newer_password)
            .expect("Failed to login with newest password");
        let keys: Vec<_> = unlocked.keys().expect("Failed to get keys").collect();
        assert_eq!(keys.len(), 1, "Should still have the same key");
        assert_eq!(keys[0], test_key, "Key should be unchanged");
    }

    // New password should no longer work
    let result = kms.login(&user_id, new_password);
    assert!(result.is_err(), "New password should no longer work");

    println!("User password change test passed! ✅");
}

#[test]
fn test_shared_key_store_multi_user() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("shared_store_test.json");
    let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

    // Multiple users
    let alice_id = b"alice".to_vec();
    let alice_password = Secret::new(b"alice_secure_password".to_vec()).unwrap();
    let bob_id = b"bob".to_vec();
    let bob_password = Secret::new(b"bob_secure_password".to_vec()).unwrap();
    let charlie_id = b"charlie".to_vec();
    let charlie_password = Secret::new(b"charlie_secure_password".to_vec()).unwrap();

    // Alice creates initial key and adds other users
    let alice_key = {
        let mut alice_unlocked = kms
            .login(&alice_id, alice_password.clone())
            .expect("Failed to login as Alice");
        let key = alice_unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                Secret::new([77u8; 32]).expect("Failed to create seed"),
            )
            .expect("Failed to generate Alice's key");

        alice_unlocked
            .add_user(&bob_id, bob_password.clone())
            .expect("Failed to add Bob");
        alice_unlocked
            .add_user(&charlie_id, charlie_password.clone())
            .expect("Failed to add Charlie");
        key
    };

    // Bob creates another key - should be visible to all users
    let bob_key = {
        let mut bob_unlocked = kms
            .login(&bob_id, bob_password.clone())
            .expect("Failed to login as Bob");
        bob_unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                Secret::new([88u8; 32]).expect("Failed to create seed"),
            )
            .expect("Failed to generate Bob's key")
    };

    // Charlie creates a third key
    let charlie_key = {
        let mut charlie_unlocked = kms
            .login(&charlie_id, charlie_password.clone())
            .expect("Failed to login as Charlie");
        charlie_unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                Secret::new([99u8; 32]).expect("Failed to create seed"),
            )
            .expect("Failed to generate Charlie's key")
    };

    // All users should see all three keys
    for (user_id, password, user_name) in [
        (&alice_id, &alice_password, "Alice"),
        (&bob_id, &bob_password, "Bob"),
        (&charlie_id, &charlie_password, "Charlie"),
    ] {
        let unlocked = kms
            .login(user_id, password.clone())
            .expect(&format!("Failed to login as {}", user_name));
        let keys: Vec<_> = unlocked.keys().expect("Failed to get keys").collect();
        assert_eq!(keys.len(), 3, "{} should see all 3 keys", user_name);
        assert!(
            keys.contains(&alice_key),
            "{} should see Alice's key",
            user_name
        );
        assert!(
            keys.contains(&bob_key),
            "{} should see Bob's key",
            user_name
        );
        assert!(
            keys.contains(&charlie_key),
            "{} should see Charlie's key",
            user_name
        );

        // All users should see the same user list
        let users = unlocked.list_users().expect("Failed to list users");
        assert_eq!(users.len(), 3, "{} should see all 3 users", user_name);
        assert!(
            users.contains(&alice_id),
            "{} should see Alice in user list",
            user_name
        );
        assert!(
            users.contains(&bob_id),
            "{} should see Bob in user list",
            user_name
        );
        assert!(
            users.contains(&charlie_id),
            "{} should see Charlie in user list",
            user_name
        );
    }

    println!("Shared key store multi-user test passed! ✅");
}

#[test]
fn test_user_management_errors() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("user_errors_test.json");
    let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

    let admin_id = b"admin".to_vec();
    let admin_password = Secret::new(b"admin_password".to_vec()).unwrap();
    let user_id = b"test_user".to_vec();
    let user_password = Secret::new(b"user_password".to_vec()).unwrap();

    // Create initial setup
    {
        let mut admin_unlocked = kms
            .login(&admin_id, admin_password.clone())
            .expect("Failed to login as admin");
        admin_unlocked
            .add_user(&user_id, user_password.clone())
            .expect("Failed to add user");
    }

    // Test duplicate user addition
    {
        let mut admin_unlocked = kms
            .login(&admin_id, admin_password.clone())
            .expect("Failed to login as admin");
        let result = admin_unlocked.add_user(&user_id, user_password.clone());
        assert!(result.is_err(), "Adding duplicate user should fail");
        // Note: The specific error type depends on implementation
    }

    // Test changing password for non-existent user
    {
        let mut admin_unlocked = kms
            .login(&admin_id, admin_password.clone())
            .expect("Failed to login as admin");
        let nonexistent_user = b"nonexistent".to_vec();
        let new_password = Secret::new(b"new_password".to_vec()).unwrap();
        let result = admin_unlocked.change_user_password(&nonexistent_user, new_password);
        assert!(
            result.is_err(),
            "Changing password for non-existent user should fail"
        );
    }

    // Test invalid credentials
    let wrong_password = Secret::new(b"wrong_password".to_vec()).unwrap();
    let result = kms.login(&admin_id, wrong_password);
    assert!(result.is_err(), "Login with wrong password should fail");

    let nonexistent_user = b"nonexistent".to_vec();
    let result = kms.login(&nonexistent_user, admin_password.clone());
    assert!(result.is_err(), "Login with non-existent user should fail");

    println!("User management errors test passed! ✅");
}

#[test]
fn test_user_isolation() {
    // Test that users with different credentials cannot access each other's data
    let temp_dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let temp_dir2 = TempDir::new().expect("Failed to create temp dir 2");

    let kms1_path = temp_dir1.path().join("user1_store.json");
    let kms2_path = temp_dir2.path().join("user2_store.json");

    let kms1 = GenericKms::new(&kms1_path).expect("Failed to create KMS 1");
    let kms2 = GenericKms::new(&kms2_path).expect("Failed to create KMS 2");

    let user1_id = b"isolated_user1".to_vec();
    let user1_password = Secret::new(b"user1_password".to_vec()).unwrap();
    let user2_id = b"isolated_user2".to_vec();
    let user2_password = Secret::new(b"user2_password".to_vec()).unwrap();

    // User 1 creates a key in their store
    let user1_key = {
        let mut unlocked = kms1
            .login(&user1_id, user1_password.clone())
            .expect("Failed to login user1");
        unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                Secret::new([111u8; 32]).expect("Failed to create seed"),
            )
            .expect("Failed to generate user1 key")
    };

    // User 2 creates a key in their store
    let user2_key = {
        let mut unlocked = kms2
            .login(&user2_id, user2_password.clone())
            .expect("Failed to login user2");
        unlocked
            .generate_key_pair(
                KeyType::DerivationSeed {
                    network: Network::Testnet,
                },
                Secret::new([222u8; 32]).expect("Failed to create seed"),
            )
            .expect("Failed to generate user2 key")
    };

    // Each user can only see their own keys
    {
        let unlocked1 = kms1
            .login(&user1_id, user1_password)
            .expect("Failed to login user1");
        let keys1: Vec<_> = unlocked1.keys().expect("Failed to get keys").collect();
        assert_eq!(keys1.len(), 1, "User1 should see only their key");
        assert_eq!(keys1[0], user1_key, "User1 should see their own key");

        let users1 = unlocked1.list_users().expect("Failed to list users");
        assert_eq!(users1.len(), 1, "User1 should see only themselves");
        assert_eq!(
            users1[0], user1_id,
            "User1 should see themselves in the list"
        );
    }

    {
        let unlocked2 = kms2
            .login(&user2_id, user2_password)
            .expect("Failed to login user2");
        let keys2: Vec<_> = unlocked2.keys().expect("Failed to get keys").collect();
        assert_eq!(keys2.len(), 1, "User2 should see only their key");
        assert_eq!(keys2[0], user2_key, "User2 should see their own key");

        let users2 = unlocked2.list_users().expect("Failed to list users");
        assert_eq!(users2.len(), 1, "User2 should see only themselves");
        assert_eq!(
            users2[0], user2_id,
            "User2 should see themselves in the list"
        );
    }

    println!("User isolation test passed! ✅");
}

#[test]
fn test_persistence_and_recovery() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let kms_path = temp_dir.path().join("persistence_test.json");

    let admin_id = b"admin".to_vec();
    let admin_password = Secret::new(b"admin_password".to_vec()).unwrap();
    let user1_id = b"user1".to_vec();
    let user1_password = Secret::new(b"user1_password".to_vec()).unwrap();
    let user2_id = b"user2".to_vec();
    let user2_password = Secret::new(b"user2_password".to_vec()).unwrap();

    let initial_key;
    let added_key;

    // First session: create users and keys
    {
        let kms = GenericKms::new(&kms_path).expect("Failed to create KMS");

        // Admin creates initial key and adds users
        {
            let mut admin_unlocked = kms
                .login(&admin_id, admin_password.clone())
                .expect("Failed to login as admin");
            initial_key = admin_unlocked
                .generate_key_pair(
                    KeyType::DerivationSeed {
                        network: Network::Testnet,
                    },
                    Secret::new([55u8; 32]).expect("Failed to create seed"),
                )
                .expect("Failed to generate initial key");

            admin_unlocked
                .add_user(&user1_id, user1_password.clone())
                .expect("Failed to add user1");
            admin_unlocked
                .add_user(&user2_id, user2_password.clone())
                .expect("Failed to add user2");
        }

        // User1 creates another key
        {
            let mut user1_unlocked = kms
                .login(&user1_id, user1_password.clone())
                .expect("Failed to login as user1");
            added_key = user1_unlocked
                .generate_key_pair(
                    KeyType::DerivationSeed {
                        network: Network::Testnet,
                    },
                    Secret::new([66u8; 32]).expect("Failed to create seed"),
                )
                .expect("Failed to generate user1 key");
        }
    }

    // Second session: reload and verify everything persisted
    {
        let kms = GenericKms::new(&kms_path).expect("Failed to reload KMS");

        // All users should still exist and work
        for (user_id, password, user_name) in [
            (&admin_id, &admin_password, "admin"),
            (&user1_id, &user1_password, "user1"),
            (&user2_id, &user2_password, "user2"),
        ] {
            let unlocked = kms
                .login(user_id, password.clone())
                .expect(&format!("Failed to login as {}", user_name));

            let keys: Vec<_> = unlocked.keys().expect("Failed to get keys").collect();
            assert_eq!(keys.len(), 2, "{} should see both keys", user_name);
            assert!(
                keys.contains(&initial_key),
                "{} should see initial key",
                user_name
            );
            assert!(
                keys.contains(&added_key),
                "{} should see added key",
                user_name
            );

            let users = unlocked.list_users().expect("Failed to list users");
            assert_eq!(users.len(), 3, "{} should see all 3 users", user_name);
        }

        // Test user management still works after reload
        {
            let mut admin_unlocked = kms
                .login(&admin_id, admin_password.clone())
                .expect("Failed to login as admin");
            let new_password = Secret::new(b"new_user2_password".to_vec()).unwrap();
            admin_unlocked
                .change_user_password(&user2_id, new_password.clone())
                .expect("Failed to change user2 password");

            // Old password should not work
            let result = kms.login(&user2_id, user2_password);
            assert!(result.is_err(), "Old password should not work");

            // New password should work
            let unlocked = kms
                .login(&user2_id, new_password)
                .expect("Failed to login with new password");
            let keys: Vec<_> = unlocked.keys().expect("Failed to get keys").collect();
            assert_eq!(keys.len(), 2, "User2 should still see both keys");
        }
    }

    println!("Persistence and recovery test passed! ✅");
}
