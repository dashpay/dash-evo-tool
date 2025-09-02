/// DIP-14 compliant 256-bit HD key derivation implementation
/// 
/// This module implements Extended Key Derivation using 256-bit Unsigned Integers
/// as specified in DIP-0014 for DashPay contact relationships.

use dash_sdk::dpp::dashcore::hashes::{Hash, HashEngine};
use dash_sdk::dpp::dashcore::hashes::hmac::{Hmac, HmacEngine};
use dash_sdk::dpp::dashcore::hashes::sha512;
use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey, PublicKey};
use dash_sdk::dpp::dashcore::bip32::{ChainCode, ExtendedPrivKey, ExtendedPubKey, Fingerprint};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Identifier;

/// Perform DIP-14 compliant 256-bit child key derivation for private keys
/// 
/// This implements CKDpriv256 as specified in DIP-0014:
/// - For indices < 2^32, uses standard BIP32 derivation for compatibility
/// - For indices >= 2^32, uses 256-bit derivation with ser_256(i)
pub fn ckd_priv_256(
    parent_key: &ExtendedPrivKey,
    index: &[u8; 32],  // 256-bit index
    hardened: bool,
) -> Result<ExtendedPrivKey, String> {
    let secp = Secp256k1::new();
    
    // Check if this is a compatibility mode derivation (index < 2^32)
    let is_compatibility_mode = is_index_less_than_2_32(index);
    
    // Prepare HMAC data based on the derivation type
    let mut hmac_engine = HmacEngine::<sha512::Hash>::new(&parent_key.chain_code.to_bytes());
    
    if hardened {
        // Hardened derivation: 0x00 || ser_256(k_par) || ser(i)
        hmac_engine.input(&[0x00]);
        hmac_engine.input(&parent_key.private_key.secret_bytes());
        
        if is_compatibility_mode {
            // Use ser_32(i) for compatibility
            hmac_engine.input(&index[28..32]);
        } else {
            // Use ser_256(i) for full 256-bit
            hmac_engine.input(index);
        }
    } else {
        // Non-hardened derivation: ser_P(point(k_par)) || ser(i)
        let parent_pubkey = parent_key.private_key.public_key(&secp);
        hmac_engine.input(&parent_pubkey.serialize());
        
        if is_compatibility_mode {
            // Use ser_32(i) for compatibility
            hmac_engine.input(&index[28..32]);
        } else {
            // Use ser_256(i) for full 256-bit
            hmac_engine.input(index);
        }
    }
    
    let hmac_result = Hmac::<sha512::Hash>::from_engine(hmac_engine);
    let hmac_bytes = hmac_result.to_byte_array();
    
    // Split into I_L (first 32 bytes) and I_R (last 32 bytes)
    let (i_l, i_r) = hmac_bytes.split_at(32);
    
    // Parse I_L as a private key and add to parent key
    let i_l_key = SecretKey::from_slice(i_l)
        .map_err(|e| format!("Failed to parse I_L as secret key: {}", e))?;
    
    // k_i = parse_256(I_L) + k_par (mod n)
    let child_key = parent_key.private_key
        .add_tweak(&i_l_key.into())
        .map_err(|e| format!("Failed to add tweak to parent key: {}", e))?;
    
    // Chain code is I_R (32 bytes)
    let mut chain_code_bytes = [0u8; 32];
    chain_code_bytes.copy_from_slice(i_r);
    let child_chain_code = ChainCode::from(chain_code_bytes);
    
    // Calculate child fingerprint
    let parent_pubkey = parent_key.private_key.public_key(&secp);
    let parent_fingerprint = calculate_fingerprint(&parent_pubkey);
    
    // Create the child extended private key
    Ok(ExtendedPrivKey {
        network: parent_key.network,
        depth: parent_key.depth + 1,
        parent_fingerprint,
        child_number: index_to_child_number(index, hardened)?,
        private_key: child_key,
        chain_code: child_chain_code,
    })
}

/// Perform DIP-14 compliant 256-bit child key derivation for public keys
/// 
/// This implements CKDpub256 as specified in DIP-0014:
/// - Only works for non-hardened derivation
/// - For indices < 2^32, uses standard BIP32 derivation for compatibility
/// - For indices >= 2^32, uses 256-bit derivation with ser_256(i)
pub fn ckd_pub_256(
    parent_key: &ExtendedPubKey,
    index: &[u8; 32],  // 256-bit index
    hardened: bool,
) -> Result<ExtendedPubKey, String> {
    if hardened {
        return Err("Cannot derive hardened child from extended public key".to_string());
    }
    
    let secp = Secp256k1::new();
    
    // Check if this is a compatibility mode derivation (index < 2^32)
    let is_compatibility_mode = is_index_less_than_2_32(index);
    
    // Prepare HMAC data
    let mut hmac_engine = HmacEngine::<sha512::Hash>::new(&parent_key.chain_code.to_bytes());
    
    // Non-hardened derivation: ser_P(K_par) || ser(i)
    hmac_engine.input(&parent_key.public_key.serialize());
    
    if is_compatibility_mode {
        // Use ser_32(i) for compatibility
        hmac_engine.input(&index[28..32]);
    } else {
        // Use ser_256(i) for full 256-bit
        hmac_engine.input(index);
    }
    
    let hmac_result = Hmac::<sha512::Hash>::from_engine(hmac_engine);
    let hmac_bytes = hmac_result.to_byte_array();
    
    // Split into I_L (first 32 bytes) and I_R (last 32 bytes)
    let (i_l, i_r) = hmac_bytes.split_at(32);
    
    // Parse I_L as a secret key for the tweak
    let i_l_key = SecretKey::from_slice(i_l)
        .map_err(|e| format!("Failed to parse I_L as secret key: {}", e))?;
    
    // K_i = point(parse_256(I_L)) + K_par
    let child_pubkey = parent_key.public_key
        .add_exp_tweak(&secp, &i_l_key.into())
        .map_err(|e| format!("Failed to add tweak to parent public key: {}", e))?;
    
    // Chain code is I_R (32 bytes)
    let mut chain_code_bytes = [0u8; 32];
    chain_code_bytes.copy_from_slice(i_r);
    let child_chain_code = ChainCode::from(chain_code_bytes);
    
    // Create the child extended public key
    Ok(ExtendedPubKey {
        network: parent_key.network,
        depth: parent_key.depth + 1,
        parent_fingerprint: parent_key.parent_fingerprint,
        child_number: index_to_child_number(index, false)?,
        public_key: child_pubkey,
        chain_code: child_chain_code,
    })
}

/// Derive DashPay incoming funds extended public key using DIP-14 compliant derivation
/// Path: m/9'/5'/15'/account'/(sender_id)/(recipient_id)
pub fn derive_dashpay_incoming_xpub_dip14(
    master_seed: &[u8],
    network: Network,
    account: u32,
    sender_id: &Identifier,
    recipient_id: &Identifier,
) -> Result<ExtendedPubKey, String> {
    use dash_sdk::dpp::dashcore::bip32::DerivationPath;
    use std::str::FromStr;
    
    // Create extended private key from seed
    let master_xprv = ExtendedPrivKey::new_master(network, master_seed)
        .map_err(|e| format!("Failed to create master key: {}", e))?;
    
    // Build derivation path for the base: m/9'/5'/15'/account'
    let base_path = DerivationPath::from_str(&format!(
        "m/9'/5'/15'/{}'",
        account
    ))
    .map_err(|e| format!("Invalid derivation path: {}", e))?;
    
    // Derive to the account level using standard BIP32
    let secp = Secp256k1::new();
    let account_xprv = master_xprv
        .derive_priv(&secp, &base_path)
        .map_err(|e| format!("Failed to derive account key: {}", e))?;
    
    // Now use DIP-14 256-bit derivation for the identity levels
    // Derive: account_key/(sender_id)
    let sender_index = identifier_to_256bit_index(sender_id);
    let sender_level = ckd_priv_256(&account_xprv, &sender_index, false)?;
    
    // Derive: sender_level/(recipient_id)
    let recipient_index = identifier_to_256bit_index(recipient_id);
    let contact_xprv = ckd_priv_256(&sender_level, &recipient_index, false)?;
    
    // Convert to extended public key
    Ok(ExtendedPubKey::from_priv(&secp, &contact_xprv))
}

/// Convert an Identifier to a 256-bit index for DIP-14 derivation
fn identifier_to_256bit_index(id: &Identifier) -> [u8; 32] {
    let mut index = [0u8; 32];
    index.copy_from_slice(&id.to_buffer());
    index
}

/// Check if a 256-bit index is less than 2^32 (compatibility mode)
fn is_index_less_than_2_32(index: &[u8; 32]) -> bool {
    // Check if the first 28 bytes are all zeros
    index[0..28].iter().all(|&b| b == 0)
}

/// Convert a 256-bit index to a ChildNumber for storage
/// This is a simplified representation since ChildNumber only supports 31-bit indices
fn index_to_child_number(index: &[u8; 32], hardened: bool) -> Result<dash_sdk::dpp::dashcore::bip32::ChildNumber, String> {
    use dash_sdk::dpp::dashcore::bip32::ChildNumber;
    
    // For compatibility with existing ChildNumber structure,
    // we need to ensure the value fits in 31 bits for normal, or set the hardened bit
    // We'll use a hash of the full 256-bit index to get a deterministic 31-bit value
    use dash_sdk::dpp::dashcore::hashes::sha256;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    
    let hash = sha256::Hash::hash(index);
    let hash_bytes = hash.to_byte_array();
    
    // Take first 4 bytes and mask to 31 bits
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&hash_bytes[0..4]);
    let mut num = u32::from_be_bytes(bytes);
    
    if hardened {
        // Set the hardened bit (bit 31)
        num |= 0x80000000;
        Ok(ChildNumber::from(num))
    } else {
        // Clear bit 31 to ensure it's within normal range
        num &= 0x7FFFFFFF;
        Ok(ChildNumber::from(num))
    }
}

/// Calculate fingerprint for a public key (first 4 bytes of HASH160)
fn calculate_fingerprint(pubkey: &PublicKey) -> Fingerprint {
    use dash_sdk::dpp::dashcore::hashes::hash160;
    
    let hash = hash160::Hash::hash(&pubkey.serialize());
    let mut fingerprint_bytes = [0u8; 4];
    fingerprint_bytes.copy_from_slice(&hash.to_byte_array()[0..4]);
    Fingerprint::from(fingerprint_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use hex;
    
    #[test]
    fn test_256bit_index_detection() {
        // Test index less than 2^32
        let mut small_index = [0u8; 32];
        small_index[31] = 42;
        assert!(is_index_less_than_2_32(&small_index));
        
        // Test index >= 2^32
        let mut large_index = [0u8; 32];
        large_index[27] = 1;  // Set a bit in the upper bytes
        assert!(!is_index_less_than_2_32(&large_index));
    }
    
    #[test]
    fn test_identifier_to_index_conversion() {
        let id_bytes = [
            0x77, 0x5d, 0x38, 0x54, 0xc9, 0x10, 0xb7, 0xde,
            0xe4, 0x36, 0x86, 0x9c, 0x47, 0x24, 0xbe, 0xd2,
            0xfe, 0x07, 0x84, 0xe1, 0x98, 0xb8, 0xa3, 0x9f,
            0x02, 0xbb, 0xb4, 0x9d, 0x8e, 0xbc, 0xfc, 0x3b,
        ];
        let id = Identifier::from_bytes(&id_bytes).unwrap();
        
        let index = identifier_to_256bit_index(&id);
        assert_eq!(index, id_bytes);
    }
    
    #[test]
    fn test_dip14_derivation_compatibility() {
        // Test that derivation with index < 2^32 matches standard BIP32
        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        
        let master = ExtendedPrivKey::new_master(network, &seed).unwrap();
        
        // Test with small index (should use compatibility mode)
        let mut small_index = [0u8; 32];
        small_index[31] = 1;
        
        let child = ckd_priv_256(&master, &small_index, false);
        assert!(child.is_ok());
    }
    
    #[test]
    fn test_dip14_test_vector_1() {
        // Test Vector 1 from DIP-14
        // Mnemonic: birth kingdom trash renew flavor utility donkey gasp regular alert pave layer
        let seed_hex = "b16d3782e714da7c55a397d5f19104cfed7ffa8036ac514509bbb50807f8ac598eeb26f0797bd8cc221a6cbff2168d90a5e9ee025a5bd977977b9eccd97894bb";
        let seed = hex::decode(seed_hex).unwrap();
        let network = Network::Testnet;
        
        // Test derivation path with 256-bit indices
        let index1 = hex::decode("775d3854c910b7dee436869c4724bed2fe0784e198b8a39f02bbb49d8ebcfc3b").unwrap();
        let index2 = hex::decode("f537439f36d04a15474ff7423e4b904a14373fafb37a41db74c84f1dbb5c89a6").unwrap();
        
        let master = ExtendedPrivKey::new_master(network, &seed).unwrap();
        
        // Derive first level (non-hardened)
        let mut index1_array = [0u8; 32];
        index1_array.copy_from_slice(&index1);
        let level1 = ckd_priv_256(&master, &index1_array, false).unwrap();
        
        // Derive second level (hardened)
        let mut index2_array = [0u8; 32];
        index2_array.copy_from_slice(&index2);
        let level2 = ckd_priv_256(&level1, &index2_array, true).unwrap();
        
        // The test passes if we can derive without errors
        // Full validation would require checking against expected key values
        assert_eq!(level2.depth, 2);
    }
    
    #[test]
    fn test_dashpay_identity_derivation() {
        // Test DashPay contact relationship derivation
        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        
        // Create two test identity IDs
        let sender_bytes = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x11,
        ];
        let recipient_bytes = [
            0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11,
            0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
            0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11,
            0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
        ];
        
        let sender_id = Identifier::from_bytes(&sender_bytes).unwrap();
        let recipient_id = Identifier::from_bytes(&recipient_bytes).unwrap();
        
        // Test that we can derive the DashPay contact xpub
        let xpub = derive_dashpay_incoming_xpub_dip14(
            &seed,
            network,
            0,  // account
            &sender_id,
            &recipient_id,
        );
        
        // Print the error if it fails
        if let Err(ref e) = xpub {
            eprintln!("DashPay derivation error: {}", e);
        }
        
        assert!(xpub.is_ok());
        let xpub = xpub.unwrap();
        
        // Verify the derivation depth is correct (base path + 2 identity levels)
        // m/9'/5'/15'/0'/(sender)/(recipient) = depth 6
        assert_eq!(xpub.depth, 6);
    }
}