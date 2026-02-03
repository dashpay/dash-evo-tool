//! Key Exchange Protocol (YAPPR) implementation
//!
//! This module implements the Dash Platform Application Key Exchange Protocol,
//! enabling web apps to securely obtain deterministic login keys from the wallet.
//!
//! Protocol flow:
//! 1. Web app generates ephemeral keypair and displays `dash-key:` URI
//! 2. User pastes URI into wallet, selects identity, approves
//! 3. Wallet derives login key (BIP32 + HKDF), encrypts with ECDH, publishes to Platform
//! 4. Web app polls for response document, decrypts login key

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::key_exchange_request::KeyExchangeRequest;
use crate::model::qualified_identity::QualifiedIdentity;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use bip39::rand::{RngCore, SeedableRng, rngs::StdRng};
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{Document as DppDocument, DocumentV0};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, ExtendedPrivKey};
use dash_sdk::dpp::platform_value::{Bytes32, Value};
use dash_sdk::platform::Identifier;
use hkdf::Hkdf;
use sha2::Sha256;
use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

/// Feature index for key exchange (m/9'/coin_type'/21'/...)
const KEY_EXCHANGE_FEATURE: u32 = 21;

/// HKDF salt for deriving the shared secret
const ECDH_HKDF_SALT: &[u8] = b"dash:key-exchange:v1";

/// AES-GCM nonce size in bytes
const NONCE_SIZE: usize = 12;

/// AES-GCM tag size in bytes
#[allow(dead_code)]
const TAG_SIZE: usize = 16;

/// Handle a key exchange request from the user.
///
/// This is the main entry point called when the user approves a key exchange request.
/// It:
/// 1. Derives the deterministic login key for the contract
/// 2. Generates an ephemeral keypair for ECDH
/// 3. Encrypts the login key with the shared secret
/// 4. Publishes the response document to Platform
pub async fn handle_key_exchange_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
    request: &KeyExchangeRequest,
) -> Result<BackendTaskSuccessResult, String> {
    // Get wallet seed for key derivation
    let wallet_seed = get_wallet_seed(identity)?;

    // Derive the login key
    let login_key = derive_login_key(
        &wallet_seed,
        app_context.network,
        &identity.identity.id(),
        &request.contract_id,
        request.key_index,
    )?;

    // Generate wallet's ephemeral keypair for ECDH
    let (wallet_ephemeral_priv, wallet_ephemeral_pub) = generate_ephemeral_keypair()?;

    // Parse app's ephemeral public key
    let app_ephemeral_pub = PublicKey::from_slice(&request.app_ephemeral_pub_key)
        .map_err(|e| format!("Invalid app ephemeral public key: {}", e))?;

    // Derive ECDH shared secret
    let shared_secret = ecdh_shared_secret(&wallet_ephemeral_priv, &app_ephemeral_pub)?;

    // Encrypt the login key
    let encrypted_payload = encrypt_login_key(&login_key, &shared_secret)?;

    // Publish the response document
    publish_login_key_response(
        app_context,
        sdk,
        identity,
        &request.contract_id,
        request.key_index,
        &wallet_ephemeral_pub.serialize(),
        &encrypted_payload,
    )
    .await?;

    Ok(BackendTaskSuccessResult::KeyExchangeComplete {
        contract_id: request.contract_id,
        app_label: request.label.clone(),
    })
}

/// Get the wallet seed for HD derivation.
fn get_wallet_seed(identity: &QualifiedIdentity) -> Result<Vec<u8>, String> {
    // Use ENCRYPTION key for derivation (matches DashPay pattern)
    let encryption_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::ENCRYPTION,
            HashSet::from([SecurityLevel::MEDIUM]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or("No suitable ENCRYPTION key found for key derivation")?;

    let wallets: Vec<_> = identity.associated_wallets.values().cloned().collect();

    let seed = identity
        .private_keys
        .get_resolve(
            &(
                crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity,
                encryption_key.id(),
            ),
            &wallets,
            identity.network,
        )
        .map_err(|e| format!("Error resolving wallet seed: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or("Wallet seed not found")?;

    Ok(seed.to_vec())
}

/// Derive the deterministic login key for a contract.
///
/// Derivation path: m/9'/coin_type'/21'/account'
/// Then HKDF with identity_id as salt and contract_id || key_index as info
///
/// Note: coin_type = 5 for mainnet, 1 for testnet/devnet
pub fn derive_login_key(
    wallet_seed: &[u8],
    network: Network,
    identity_id: &Identifier,
    contract_id: &Identifier,
    key_index: u32,
) -> Result<[u8; 32], String> {
    // Determine coin type based on network
    let coin_type = match network {
        Network::Dash => 5,
        Network::Testnet | Network::Devnet | Network::Regtest => 1,
        _ => return Err(format!("Unsupported network: {:?}", network)),
    };

    // Build derivation path: m/9'/coin_type'/21'/0' (account 0)
    let path =
        DerivationPath::from_str(&format!("m/9'/{}'/{}'/0'", coin_type, KEY_EXCHANGE_FEATURE))
            .map_err(|e| format!("Invalid derivation path: {}", e))?;

    // Create master key from seed
    let master_key = ExtendedPrivKey::new_master(network, wallet_seed)
        .map_err(|e| format!("Failed to create master key: {}", e))?;

    // Derive the base key
    let secp = Secp256k1::new();
    let base_key = master_key
        .derive_priv(&secp, &path)
        .map_err(|e| format!("Failed to derive base key: {}", e))?;

    // Extract the private key bytes
    let base_private_key = base_key.private_key.secret_bytes();

    // Build HKDF info: contract_id (32 bytes) || key_index (4 bytes LE)
    let mut info = Vec::with_capacity(36);
    info.extend_from_slice(&contract_id.to_buffer());
    info.extend_from_slice(&key_index.to_le_bytes());

    // Use identity_id as salt
    let salt = identity_id.to_buffer();

    // Derive the login key using HKDF-SHA256
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), &base_private_key);
    let mut login_key = [0u8; 32];
    hkdf.expand(&info, &mut login_key)
        .map_err(|_| "HKDF expansion failed")?;

    Ok(login_key)
}

/// Generate a fresh ephemeral keypair for ECDH.
pub fn generate_ephemeral_keypair() -> Result<(SecretKey, PublicKey), String> {
    let secp = Secp256k1::new();
    let mut rng = StdRng::from_entropy();

    // Generate random 32 bytes for the private key
    let mut key_bytes = [0u8; 32];
    rng.fill_bytes(&mut key_bytes);

    let secret_key = SecretKey::from_slice(&key_bytes)
        .map_err(|e| format!("Failed to create secret key: {}", e))?;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);

    Ok((secret_key, public_key))
}

/// Derive ECDH shared secret from private key and public key.
///
/// Uses HKDF to derive a 32-byte shared secret suitable for AES-256-GCM.
pub fn ecdh_shared_secret(
    private_key: &SecretKey,
    public_key: &PublicKey,
) -> Result<[u8; 32], String> {
    // Perform ECDH multiplication
    let shared_point =
        dash_sdk::dpp::dashcore::secp256k1::ecdh::shared_secret_point(public_key, private_key);

    // Extract x coordinate (first 32 bytes)
    let shared_x = &shared_point[..32];

    // Derive the shared secret using HKDF
    let hkdf = Hkdf::<Sha256>::new(Some(ECDH_HKDF_SALT), shared_x);
    let mut shared_secret = [0u8; 32];
    hkdf.expand(&[], &mut shared_secret)
        .map_err(|_| "HKDF expansion failed for shared secret")?;

    Ok(shared_secret)
}

/// Encrypt the login key using AES-256-GCM.
///
/// Returns: nonce (12 bytes) || ciphertext || tag (16 bytes)
pub fn encrypt_login_key(
    login_key: &[u8; 32],
    shared_secret: &[u8; 32],
) -> Result<Vec<u8>, String> {
    // Create AES-256-GCM cipher
    let cipher = Aes256Gcm::new_from_slice(shared_secret)
        .map_err(|e| format!("Failed to create cipher: {}", e))?;

    // Generate random nonce
    let mut rng = StdRng::from_entropy();
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt the login key
    let ciphertext = cipher
        .encrypt(nonce, login_key.as_ref())
        .map_err(|e| format!("Encryption failed: {}", e))?;

    // Build output: nonce || ciphertext (includes tag)
    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

/// Publish the login key response document to Platform.
///
/// Document type: loginKeyResponse on the keyExchange contract
/// Index: unique on ($ownerId, contractId)
///
/// Per YAPPR spec Section 7.3, this function checks for an existing document
/// and updates it if found, or creates a new one if not.
#[allow(clippy::too_many_arguments)]
async fn publish_login_key_response(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
    target_contract_id: &Identifier,
    key_index: u32,
    wallet_ephemeral_pub_key: &[u8; 33],
    encrypted_payload: &[u8],
) -> Result<(), String> {
    use dash_sdk::dpp::document::DocumentV0Setters;
    use dash_sdk::drive::query::{WhereClause, WhereOperator};
    use dash_sdk::platform::{DataContract, Document, DocumentQuery, Fetch, FetchMany};

    // Get the key exchange contract ID
    let contract_id = app_context
        .key_exchange_contract_id()
        .ok_or("Key exchange contract ID not configured")?;

    tracing::info!(
        "Fetching key exchange contract: {}",
        contract_id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
    );

    // Fetch the key exchange contract from the network
    let key_exchange_contract = DataContract::fetch(sdk, contract_id)
        .await
        .map_err(|e| format!("Failed to fetch key exchange contract: {}", e))?
        .ok_or("Key exchange contract not found on network")?;

    // Store the contract in the database so the SDK's context provider can find it
    // during document validation
    app_context
        .db
        .insert_contract_if_not_exists(
            &key_exchange_contract,
            Some("keyExchange"),
            crate::database::contracts::InsertTokensToo::NoTokensShouldBeAdded,
            app_context,
        )
        .map_err(|e| format!("Failed to store key exchange contract: {}", e))?;

    // Log contract details for debugging
    tracing::info!(
        "Fetched contract ID: {}, document types: {:?}",
        key_exchange_contract
            .id()
            .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
        key_exchange_contract
            .document_types()
            .keys()
            .collect::<Vec<_>>()
    );

    let key_exchange_contract = Arc::new(key_exchange_contract);

    // Get signing key
    let signing_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or("No suitable signing key found")?
        .clone();

    // Helper to query for existing document
    let query_existing = || async {
        let mut query = DocumentQuery::new(key_exchange_contract.clone(), "loginKeyResponse")
            .map_err(|e| format!("Failed to create query: {}", e))?;

        query = query
            .with_where(WhereClause {
                field: "$ownerId".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(identity.identity.id().to_buffer()),
            })
            .with_where(WhereClause {
                field: "contractId".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(target_contract_id.to_buffer()),
            });
        query.limit = 1;

        let existing_docs = Document::fetch_many(sdk, query)
            .await
            .map_err(|e| format!("Error checking for existing response: {}", e))?;

        Ok::<_, String>(existing_docs.into_iter().find_map(|(_, doc)| doc))
    };

    // Try to update with retry on revision conflict
    const MAX_RETRIES: u32 = 3;
    let mut last_error: Option<String> = None;

    for attempt in 1..=MAX_RETRIES {
        // Query for existing document (fresh query each attempt)
        let existing_doc = query_existing().await?;

        if let Some(mut doc) = existing_doc {
            // Update existing document
            use dash_sdk::dpp::document::DocumentV0Getters;

            let current_revision = doc.revision();
            tracing::info!(
                "Attempt {}/{}: Found existing loginKeyResponse document with revision {:?}, updating it",
                attempt,
                MAX_RETRIES,
                current_revision
            );

            // Update properties
            doc.set(
                "walletEphemeralPubKey",
                Value::Bytes(wallet_ephemeral_pub_key.to_vec()),
            );
            doc.set("encryptedPayload", Value::Bytes(encrypted_payload.to_vec()));
            doc.set("keyIndex", Value::U32(key_index));

            // Bump revision - this should increment by 1
            doc.bump_revision();

            let new_revision = doc.revision();
            tracing::info!("After bump_revision: revision is now {:?}", new_revision);

            // Create replacement transition
            use dash_sdk::platform::documents::transitions::DocumentReplaceTransitionBuilder;
            let mut builder = DocumentReplaceTransitionBuilder::new(
                key_exchange_contract.clone(),
                "loginKeyResponse".to_string(),
                doc,
            );

            // Add state transition options if available
            if let Some(options) = app_context.state_transition_options() {
                builder = builder.with_state_transition_creation_options(options);
            }

            // Submit to Platform
            match sdk.document_replace(builder, &signing_key, identity).await {
                Ok(_) => {
                    tracing::info!(
                        "Updated key exchange response for contract {}",
                        target_contract_id.to_string(
                            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58
                        )
                    );
                    return Ok(());
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // Check if this is a revision conflict error
                    if error_str.contains("invalid revision")
                        || error_str.contains("desired revision")
                    {
                        tracing::warn!(
                            "Attempt {}/{}: Revision conflict, will retry: {}",
                            attempt,
                            MAX_RETRIES,
                            error_str
                        );
                        last_error = Some(error_str);
                        // Small delay before retry to allow Platform state to settle
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        continue;
                    } else {
                        return Err(format!("Failed to update key exchange response: {}", e));
                    }
                }
            }
        } else {
            // No existing document - create new one
            tracing::info!("No existing loginKeyResponse document found, creating new one");

            // Build document properties
            let mut properties = BTreeMap::new();
            properties.insert(
                "contractId".to_string(),
                Value::Identifier(target_contract_id.to_buffer()),
            );
            properties.insert(
                "walletEphemeralPubKey".to_string(),
                Value::Bytes(wallet_ephemeral_pub_key.to_vec()),
            );
            properties.insert(
                "encryptedPayload".to_string(),
                Value::Bytes(encrypted_payload.to_vec()),
            );
            properties.insert("keyIndex".to_string(), Value::U32(key_index));

            // Generate entropy for document ID
            let mut rng = StdRng::from_entropy();
            let entropy = Bytes32::random_with_rng(&mut rng);

            // Generate document ID
            let document_id = Document::generate_document_id_v0(
                &key_exchange_contract.id(),
                &identity.identity.id(),
                "loginKeyResponse",
                entropy.as_slice(),
            );

            // Create the document
            let document = DppDocument::V0(DocumentV0 {
                id: document_id,
                owner_id: identity.identity.id(),
                creator_id: None,
                properties,
                revision: Some(1),
                created_at: None,
                updated_at: None,
                transferred_at: None,
                created_at_block_height: None,
                updated_at_block_height: None,
                transferred_at_block_height: None,
                created_at_core_block_height: None,
                updated_at_core_block_height: None,
                transferred_at_core_block_height: None,
            });

            // Build and submit the document
            let mut builder =
                dash_sdk::platform::documents::transitions::DocumentCreateTransitionBuilder::new(
                    key_exchange_contract.clone(),
                    "loginKeyResponse".to_string(),
                    document,
                    entropy
                        .as_slice()
                        .try_into()
                        .expect("entropy should be 32 bytes"),
                );

            // Add state transition options if available
            if let Some(options) = app_context.state_transition_options() {
                builder = builder.with_state_transition_creation_options(options);
            }

            // Submit to Platform
            sdk.document_create(builder, &signing_key, identity)
                .await
                .map_err(|e| format!("Failed to publish key exchange response: {}", e))?;

            tracing::info!(
                "Published new key exchange response for contract {}",
                target_contract_id
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            );

            return Ok(());
        }
    }

    // If we get here, all retries failed
    Err(last_error
        .unwrap_or_else(|| "Failed to update key exchange response after retries".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_login_key_deterministic() {
        let seed = [0x42u8; 64];
        let identity_id = Identifier::from_bytes(&[0x11u8; 32]).unwrap();
        let contract_id = Identifier::from_bytes(&[0x22u8; 32]).unwrap();

        // Derive twice with same inputs
        let key1 = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id, 0)
            .expect("Should derive");
        let key2 = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id, 0)
            .expect("Should derive");

        assert_eq!(key1, key2, "Same inputs should produce same key");
    }

    #[test]
    fn test_derive_login_key_different_contracts() {
        let seed = [0x42u8; 64];
        let identity_id = Identifier::from_bytes(&[0x11u8; 32]).unwrap();
        let contract_id_1 = Identifier::from_bytes(&[0x22u8; 32]).unwrap();
        let contract_id_2 = Identifier::from_bytes(&[0x33u8; 32]).unwrap();

        let key1 = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id_1, 0)
            .expect("Should derive");
        let key2 = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id_2, 0)
            .expect("Should derive");

        assert_ne!(
            key1, key2,
            "Different contracts should produce different keys"
        );
    }

    #[test]
    fn test_derive_login_key_different_indices() {
        let seed = [0x42u8; 64];
        let identity_id = Identifier::from_bytes(&[0x11u8; 32]).unwrap();
        let contract_id = Identifier::from_bytes(&[0x22u8; 32]).unwrap();

        let key1 = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id, 0)
            .expect("Should derive");
        let key2 = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id, 1)
            .expect("Should derive");

        assert_ne!(
            key1, key2,
            "Different indices should produce different keys"
        );
    }

    #[test]
    fn test_derive_login_key_different_identities() {
        let seed = [0x42u8; 64];
        let identity_id_1 = Identifier::from_bytes(&[0x11u8; 32]).unwrap();
        let identity_id_2 = Identifier::from_bytes(&[0x44u8; 32]).unwrap();
        let contract_id = Identifier::from_bytes(&[0x22u8; 32]).unwrap();

        let key1 = derive_login_key(&seed, Network::Testnet, &identity_id_1, &contract_id, 0)
            .expect("Should derive");
        let key2 = derive_login_key(&seed, Network::Testnet, &identity_id_2, &contract_id, 0)
            .expect("Should derive");

        assert_ne!(
            key1, key2,
            "Different identities should produce different keys"
        );
    }

    #[test]
    fn test_generate_ephemeral_keypair() {
        let (priv1, pub1) = generate_ephemeral_keypair().expect("Should generate");
        let (priv2, pub2) = generate_ephemeral_keypair().expect("Should generate");

        // Verify they're different (random)
        assert_ne!(
            priv1.secret_bytes(),
            priv2.secret_bytes(),
            "Different calls should produce different keys"
        );
        assert_ne!(
            pub1.serialize(),
            pub2.serialize(),
            "Different calls should produce different public keys"
        );

        // Verify public key is compressed (33 bytes starting with 02 or 03)
        let pub_bytes = pub1.serialize();
        assert_eq!(pub_bytes.len(), 33);
        assert!(pub_bytes[0] == 0x02 || pub_bytes[0] == 0x03);
    }

    #[test]
    fn test_ecdh_shared_secret() {
        let (priv_a, pub_a) = generate_ephemeral_keypair().expect("Should generate");
        let (priv_b, pub_b) = generate_ephemeral_keypair().expect("Should generate");

        // Both parties should derive the same shared secret
        let secret_a = ecdh_shared_secret(&priv_a, &pub_b).expect("Should derive");
        let secret_b = ecdh_shared_secret(&priv_b, &pub_a).expect("Should derive");

        assert_eq!(secret_a, secret_b, "ECDH should produce same shared secret");
    }

    #[test]
    fn test_encrypt_login_key() {
        let login_key = [0xAB; 32];
        let shared_secret = [0xCD; 32];

        let encrypted = encrypt_login_key(&login_key, &shared_secret).expect("Should encrypt");

        // Expected size: 12 (nonce) + 32 (ciphertext) + 16 (tag) = 60 bytes
        assert_eq!(encrypted.len(), NONCE_SIZE + 32 + TAG_SIZE);

        // Encrypt again - should produce different result due to random nonce
        let encrypted2 = encrypt_login_key(&login_key, &shared_secret).expect("Should encrypt");
        assert_ne!(
            encrypted, encrypted2,
            "Different nonces should produce different ciphertext"
        );
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let login_key = [0x42; 32];
        let shared_secret = [0xEF; 32];

        let encrypted = encrypt_login_key(&login_key, &shared_secret).expect("Should encrypt");

        // Manually decrypt to verify
        let nonce = &encrypted[..NONCE_SIZE];
        let ciphertext = &encrypted[NONCE_SIZE..];

        let cipher = Aes256Gcm::new_from_slice(&shared_secret).expect("Should create cipher");
        let decrypted = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .expect("Should decrypt");

        assert_eq!(decrypted.as_slice(), &login_key);
    }

    #[test]
    fn test_coin_type_selection() {
        let seed = [0x42u8; 64];
        let identity_id = Identifier::from_bytes(&[0x11u8; 32]).unwrap();
        let contract_id = Identifier::from_bytes(&[0x22u8; 32]).unwrap();

        // Mainnet and testnet should produce different keys due to different coin types
        let key_mainnet = derive_login_key(&seed, Network::Dash, &identity_id, &contract_id, 0)
            .expect("Should derive");
        let key_testnet = derive_login_key(&seed, Network::Testnet, &identity_id, &contract_id, 0)
            .expect("Should derive");

        assert_ne!(
            key_mainnet, key_testnet,
            "Different networks should produce different keys"
        );
    }
}
