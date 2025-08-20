use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::HashSet;
use bip39::rand::{SeedableRng, RngCore, rngs::StdRng};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoAcceptProofData {
    pub identity_id: Identifier,
    pub proof_key: [u8; 32],
    pub account_reference: u32,
    pub expires_at: u64, // Unix timestamp
}

impl AutoAcceptProofData {
    pub fn to_qr_string(&self) -> String {
        // Format: dashpay:{identity_id}:{proof_key_hex}:{account}:{expires}
        format!(
            "dashpay:{}:{}:{}:{}",
            self.identity_id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
            hex::encode(&self.proof_key),
            self.account_reference,
            self.expires_at
        )
    }
    
    pub fn from_qr_string(qr_data: &str) -> Result<Self, String> {
        let parts: Vec<&str> = qr_data.split(':').collect();
        if parts.len() != 5 || parts[0] != "dashpay" {
            return Err("Invalid QR code format".to_string());
        }
        
        let identity_id = Identifier::from_string(parts[1], dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            .map_err(|e| format!("Invalid identity ID: {}", e))?;
        
        let proof_key_bytes = hex::decode(parts[2])
            .map_err(|e| format!("Invalid proof key hex: {}", e))?;
        
        if proof_key_bytes.len() != 32 {
            return Err("Proof key must be 32 bytes".to_string());
        }
        
        let mut proof_key = [0u8; 32];
        proof_key.copy_from_slice(&proof_key_bytes);
        
        let account_reference = parts[3].parse::<u32>()
            .map_err(|e| format!("Invalid account reference: {}", e))?;
        
        let expires_at = parts[4].parse::<u64>()
            .map_err(|e| format!("Invalid expiration timestamp: {}", e))?;
        
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
    // Get a signing key from the identity
    let signing_key = identity.identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL, SecurityLevel::HIGH, SecurityLevel::MEDIUM]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or("No suitable signing key found")?;
    
    // Get the private key for signing
    let wallets: Vec<_> = identity.associated_wallets.values().cloned().collect();
    let private_key = identity.private_keys
        .get_resolve(
            &(crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity, signing_key.id()),
            &wallets,
            identity.network,
        )
        .map_err(|e| format!("Error resolving private key: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or("Private key not found")?;
    
    // Generate a random nonce for this proof
    let mut rng = StdRng::from_entropy();
    let mut nonce = [0u8; 16];
    rng.fill_bytes(&mut nonce);
    
    // Create proof key by hashing private key + nonce + account reference
    let mut hasher = Sha256::new();
    hasher.update(&private_key);
    hasher.update(&nonce);
    hasher.update(&account_reference.to_le_bytes());
    
    // Add expiration time
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs() + (validity_hours as u64 * 3600);
    hasher.update(&expires_at.to_le_bytes());
    
    let proof_key_hash = hasher.finalize();
    let mut proof_key = [0u8; 32];
    proof_key.copy_from_slice(&proof_key_hash);
    
    Ok(AutoAcceptProofData {
        identity_id: identity.identity.id(),
        proof_key,
        account_reference,
        expires_at,
    })
}

/// Verify an auto-accept proof from a contact request
/// 
/// This would be called when receiving a contact request with an autoAcceptProof field
/// to determine if we should automatically accept and reciprocate.
pub fn verify_auto_accept_proof(
    _proof_data: &[u8],
    _sender_identity_id: Identifier,
    _our_identity: &QualifiedIdentity,
) -> Result<bool, String> {
    // TODO: Implement proof verification
    // This would involve:
    // 1. Deserializing the proof data
    // 2. Checking the signature against the sender's public key
    // 3. Verifying the proof hasn't expired
    // 4. Checking if we have a matching pre-shared proof key
    
    // For now, return false (don't auto-accept)
    Ok(false)
}

/// Store a proof key that we've shared via QR code
/// 
/// This allows us to recognize incoming contact requests that include our proof
/// and automatically accept them.
pub fn store_shared_proof(
    _proof_data: AutoAcceptProofData,
    _identity: &QualifiedIdentity,
) -> Result<(), String> {
    // TODO: Store in local database for later verification
    // This would be used when someone scans our QR code and sends us a contact request
    
    Ok(())
}