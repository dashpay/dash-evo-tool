use super::hd_derivation::derive_auto_accept_key;
use crate::model::qualified_identity::QualifiedIdentity;
use bip39::rand::{RngCore, SeedableRng, rngs::StdRng};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoAcceptProofData {
    pub identity_id: Identifier,
    pub proof_key: [u8; 32],
    pub account_reference: u32,
    pub expires_at: u64, // Unix timestamp
}

impl AutoAcceptProofData {
    pub fn to_qr_string(&self) -> String {
        // Format according to DIP-0015: dash:?du={username}&dapk={key_data}
        // Key data format: key_type (1 byte) + timestamp (4 bytes) + key_size (1 byte) + key (32 bytes)
        let mut key_data = Vec::new();
        key_data.push(0u8); // Key type 0 for ECDSA_SECP256K1
        key_data.extend_from_slice(&(self.expires_at as u32).to_be_bytes()); // Timestamp/expiration
        key_data.push(32u8); // Key size
        key_data.extend_from_slice(&self.proof_key); // The actual key

        // Encode key data in base58 using dashcore's base58 implementation
        use dash_sdk::dpp::dashcore::base58;
        let key_data_base58 = base58::encode_slice(&key_data);

        // For QR codes without username (identity-based)
        format!(
            "dash:?di={}&dapk={}",
            self.identity_id
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
            key_data_base58
        )
    }

    pub fn from_qr_string(qr_data: &str) -> Result<Self, String> {
        // Parse DIP-0015 format: dash:?du={username}&dapk={key_data} or dash:?di={identity}&dapk={key_data}
        if !qr_data.starts_with("dash:?") {
            return Err("Invalid QR code format - must start with 'dash:?'".to_string());
        }

        let query_string = &qr_data[6..]; // Skip "dash:?"
        let mut identity_id = None;
        let mut key_data_base58 = None;
        let mut account_reference = 0u32; // Default to account 0

        // Parse query parameters
        for param in query_string.split('&') {
            let parts: Vec<&str> = param.split('=').collect();
            if parts.len() != 2 {
                continue;
            }

            match parts[0] {
                "di" => {
                    identity_id = Some(
                        Identifier::from_string(
                            parts[1],
                            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                        )
                        .map_err(|e| format!("Invalid identity ID: {}", e))?,
                    )
                }
                "dapk" => {
                    key_data_base58 = Some(parts[1].to_string());
                }
                "account" => {
                    account_reference = parts[1]
                        .parse::<u32>()
                        .map_err(|e| format!("Invalid account reference: {}", e))?;
                }
                _ => {} // Ignore unknown parameters
            }
        }

        let identity_id = identity_id.ok_or("Missing identity ID in QR code".to_string())?;
        let key_data_base58 =
            key_data_base58.ok_or("Missing proof key data in QR code".to_string())?;

        // Decode the key data from base58
        use dash_sdk::dpp::dashcore::base58;
        let key_data = base58::decode(&key_data_base58)
            .map_err(|e| format!("Invalid base58 key data: {}", e))?;

        // Parse key data format: key_type (1) + timestamp (4) + key_size (1) + key (32-64)
        if key_data.len() < 38 {
            return Err("Key data too short".to_string());
        }

        let _key_type = key_data[0];
        let expires_at =
            u32::from_be_bytes([key_data[1], key_data[2], key_data[3], key_data[4]]) as u64;
        let key_size = key_data[5] as usize;

        if key_data.len() < 6 + key_size {
            return Err("Invalid key data length".to_string());
        }

        let mut proof_key = [0u8; 32];
        if key_size == 32 {
            proof_key.copy_from_slice(&key_data[6..38]);
        } else {
            return Err(format!("Unsupported key size: {}", key_size));
        }

        Ok(Self {
            identity_id,
            proof_key,
            account_reference,
            expires_at,
        })
    }
}

/// Generate an auto-accept proof for QR code sharing
///
/// According to DIP-0015, the autoAcceptProof is a signature that allows the recipient
/// to automatically accept the contact request and send one back without user interaction.
pub fn generate_auto_accept_proof(
    identity: &QualifiedIdentity,
    account_reference: u32,
    validity_hours: u32,
) -> Result<AutoAcceptProofData, String> {
    // Calculate expiration timestamp
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs()
        + (validity_hours as u64 * 3600);

    // Get wallet seed for HD derivation - use first available private key as seed
    let signing_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or("No suitable signing key found")?;

    let wallets: Vec<_> = identity.associated_wallets.values().cloned().collect();
    let wallet_seed = identity
        .private_keys
        .get_resolve(
            &(
                crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity,
                signing_key.id(),
            ),
            &wallets,
            identity.network,
        )
        .map_err(|e| format!("Error resolving private key: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or("Private key not found")?;

    // Determine network - for now use Testnet as default
    // TODO: Get actual network from app context or identity
    let network = Network::Testnet;

    // Derive the auto-accept key using DIP-0015 path: m/9'/5'/16'/timestamp'
    // Using expiration timestamp as the derivation index
    let auto_accept_xprv = derive_auto_accept_key(
        &wallet_seed,
        network,
        expires_at as u32, // Truncate to u32 for derivation
    )
    .map_err(|e| format!("Failed to derive auto-accept key: {}", e))?;

    // Extract the private key bytes (32 bytes)
    let proof_key = auto_accept_xprv.private_key.secret_bytes();

    Ok(AutoAcceptProofData {
        identity_id: identity.identity.id(),
        proof_key,
        account_reference,
        expires_at,
    })
}

/// Create the autoAcceptProof bytes for inclusion in a contact request
///
/// Format according to DIP-0015:
/// - key type (1 byte)
/// - key index (4 bytes) - the timestamp used for derivation
/// - signature size (1 byte)
/// - signature (32-96 bytes)
pub fn create_auto_accept_proof_bytes(
    proof_data: &AutoAcceptProofData,
    sender_id: &Identifier,
    recipient_id: &Identifier,
    account_reference: u32,
    wallet_seed: &[u8],
    network: Network,
) -> Result<Vec<u8>, String> {
    // Derive the auto-accept key
    let auto_accept_xprv =
        derive_auto_accept_key(wallet_seed, network, proof_data.expires_at as u32)
            .map_err(|e| format!("Failed to derive auto-accept key: {}", e))?;

    // Create the message to sign: ownerId + toUserId + accountReference
    let mut message_data = Vec::new();
    message_data.extend_from_slice(&sender_id.to_buffer());
    message_data.extend_from_slice(&recipient_id.to_buffer());
    message_data.extend_from_slice(&account_reference.to_le_bytes());

    // Hash the message
    let mut hasher = Sha256::new();
    hasher.update(&message_data);
    let message_hash = hasher.finalize();

    // Create secp256k1 message and sign
    let secp = Secp256k1::new();
    let message = Message::from_digest_slice(&message_hash)
        .map_err(|e| format!("Failed to create message: {}", e))?;

    let secret_key = SecretKey::from_slice(&auto_accept_xprv.private_key.secret_bytes())
        .map_err(|e| format!("Failed to create secret key: {}", e))?;

    let signature = secp.sign_ecdsa(&message, &secret_key);
    let sig_bytes = signature.serialize_compact();

    // Build the proof bytes
    let mut proof_bytes = Vec::new();
    proof_bytes.push(0u8); // Key type 0 for ECDSA_SECP256K1
    proof_bytes.extend_from_slice(&(proof_data.expires_at as u32).to_be_bytes()); // Key index (timestamp)
    proof_bytes.push(sig_bytes.len() as u8); // Signature size
    proof_bytes.extend_from_slice(&sig_bytes); // The signature

    Ok(proof_bytes)
}

/// Verify an auto-accept proof from a contact request
///
/// This would be called when receiving a contact request with an autoAcceptProof field
/// to determine if we should automatically accept and reciprocate.
pub fn verify_auto_accept_proof(
    proof_data: &[u8],
    sender_identity_id: Identifier,
    our_identity: &QualifiedIdentity,
    stored_proofs: &[StoredProof],
) -> Result<bool, String> {
    // The proof data should contain:
    // 1. The proof key we generated and shared
    // 2. A signature from the sender proving they received it from us

    if proof_data.len() < 32 {
        return Ok(false); // Invalid proof format
    }

    // Extract the proof key from the data
    let mut proof_key = [0u8; 32];
    proof_key.copy_from_slice(&proof_data[0..32]);

    // Check if this proof key matches any of our stored proofs
    for stored_proof in stored_proofs {
        if stored_proof.proof_key == proof_key {
            // Check if the proof hasn't expired
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("Time error: {}", e))?
                .as_secs();

            if now > stored_proof.expires_at {
                continue; // This proof has expired
            }

            // Verify the sender matches (if we restricted it)
            if let Some(expected_id) = &stored_proof.expected_identity_id {
                if expected_id != &sender_identity_id {
                    continue; // Wrong sender
                }
            }

            // Valid proof found!
            return Ok(true);
        }
    }

    Ok(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProof {
    pub proof_key: [u8; 32],
    pub identity_id: Identifier,
    pub expected_identity_id: Option<Identifier>, // If we want to restrict who can use this
    pub account_reference: u32,
    pub expires_at: u64,
    pub created_at: u64,
    pub used: bool,
}

/// Store a proof key that we've shared via QR code
///
/// This allows us to recognize incoming contact requests that include our proof
/// and automatically accept them.
pub fn store_shared_proof(
    proof_data: AutoAcceptProofData,
    identity: &QualifiedIdentity,
) -> Result<StoredProof, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs();

    let stored_proof = StoredProof {
        proof_key: proof_data.proof_key,
        identity_id: identity.identity.id(),
        expected_identity_id: None, // Can be set if we want to restrict usage
        account_reference: proof_data.account_reference,
        expires_at: proof_data.expires_at,
        created_at: now,
        used: false,
    };

    // In production, this would save to a database
    // For now, we return it for the caller to handle storage
    Ok(stored_proof)
}
