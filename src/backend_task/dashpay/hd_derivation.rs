use dash_sdk::dpp::dashcore::hashes::{Hash, HashEngine};
use dash_sdk::dpp::dashcore::{
    Network,
    bip32::{ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey},
};
use dash_sdk::platform::Identifier;
use std::str::FromStr;

/// DashPay feature index as defined in DIP-0009
const DASHPAY_INCOMING_FUNDS_FEATURE: u32 = 15;
/// DashPay auto-accept proof feature index
const DASHPAY_AUTO_ACCEPT_FEATURE: u32 = 16;

/// Derive the DashPay incoming funds extended public key for a contact relationship
/// Path: m/9'/5'/15'/account'/(sender_id)/(recipient_id)
///
/// This creates a unique derivation path for each contact relationship,
/// allowing for unique payment addresses between any two identities.
pub fn derive_dashpay_incoming_xpub(
    master_seed: &[u8],
    network: Network,
    account: u32,
    sender_id: &Identifier,
    recipient_id: &Identifier,
) -> Result<ExtendedPubKey, String> {
    // Create extended private key from seed
    let master_xprv = ExtendedPrivKey::new_master(network, master_seed)
        .map_err(|e| format!("Failed to create master key: {}", e))?;

    // Build derivation path: m/9'/5'/15'/account'
    let base_path = DerivationPath::from_str(&format!(
        "m/9'/5'/{}'/{}'/",
        DASHPAY_INCOMING_FUNDS_FEATURE, account
    ))
    .map_err(|e| format!("Invalid derivation path: {}", e))?;

    // Derive to the account level
    let account_xprv = master_xprv
        .derive_priv(
            &dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new(),
            &base_path,
        )
        .map_err(|e| format!("Failed to derive account key: {}", e))?;

    // For the identity-based derivation, we need to use 256-bit derivation as per DIP-0014
    // Convert identity IDs to child numbers (non-hardened)
    let sender_child = identity_to_child_number(sender_id, false)?;
    let recipient_child = identity_to_child_number(recipient_id, false)?;

    // Derive: account_key/sender_id
    let sender_level = derive_256bit_child(&account_xprv, sender_child)?;

    // Derive: sender_level/recipient_id
    let contact_xprv = derive_256bit_child(&sender_level, recipient_child)?;

    // Convert to extended public key
    let xpub = ExtendedPubKey::from_priv(
        &dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new(),
        &contact_xprv,
    );

    Ok(xpub)
}

/// Derive a specific payment address for a contact
/// Path: ..../index (where index is the address index)
pub fn derive_payment_address(
    contact_xpub: &ExtendedPubKey,
    index: u32,
) -> Result<dash_sdk::dpp::dashcore::Address, String> {
    let secp = dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new();

    // Derive the specific address key
    let address_key = contact_xpub
        .derive_pub(
            &secp,
            &[ChildNumber::from_normal_idx(index).map_err(|e| format!("Invalid index: {}", e))?],
        )
        .map_err(|e| format!("Failed to derive address key: {}", e))?;

    // Convert to Dash address
    // The ExtendedPubKey's public_key is a secp256k1::PublicKey
    // We need to convert it to dashcore::PublicKey
    let secp_pubkey = address_key.public_key;
    let pubkey = dash_sdk::dpp::dashcore::PublicKey::new(secp_pubkey);
    let address = dash_sdk::dpp::dashcore::Address::p2pkh(&pubkey, contact_xpub.network);

    Ok(address)
}

/// Convert an Identifier to a ChildNumber for 256-bit derivation
/// According to DIP-0014, we use the full 256-bit identifier
fn identity_to_child_number(id: &Identifier, hardened: bool) -> Result<ChildNumber, String> {
    let id_bytes = id.to_buffer();

    // For 256-bit derivation, we need to handle this specially
    // The standard BIP32 only supports 32-bit indices, so we need to chain multiple derivations
    // or use a custom implementation. For now, we'll use a truncated version.

    // Take first 4 bytes and convert to u32 (this is a simplification)
    // In production, you'd want to implement full 256-bit derivation per DIP-0014
    let mut index_bytes = [0u8; 4];
    index_bytes.copy_from_slice(&id_bytes[..4]);
    let index = u32::from_be_bytes(index_bytes);

    if hardened {
        ChildNumber::from_hardened_idx(index).map_err(|e| format!("Invalid hardened index: {}", e))
    } else {
        ChildNumber::from_normal_idx(index).map_err(|e| format!("Invalid normal index: {}", e))
    }
}

/// Perform 256-bit child key derivation as specified in DIP-0014
/// This is a simplified version - full implementation would need proper 256-bit handling
fn derive_256bit_child(
    parent: &ExtendedPrivKey,
    child: ChildNumber,
) -> Result<ExtendedPrivKey, String> {
    let secp = dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new();

    parent
        .derive_priv(&secp, &[child])
        .map_err(|e| format!("Failed to derive child: {}", e))
}

/// Generate the extended public key data for a contact request
/// Returns (parent_fingerprint, chain_code, public_key_bytes)
#[allow(clippy::type_complexity)]
pub fn generate_contact_xpub_data(
    master_seed: &[u8],
    network: Network,
    account: u32,
    sender_id: &Identifier,
    recipient_id: &Identifier,
) -> Result<([u8; 4], [u8; 32], [u8; 33]), String> {
    // Derive the extended public key for this contact
    let xpub =
        derive_dashpay_incoming_xpub(master_seed, network, account, sender_id, recipient_id)?;

    // Extract the components needed for the contact request
    let parent_fingerprint = xpub.parent_fingerprint.to_bytes();
    let chain_code = xpub.chain_code.to_bytes();

    // Get the public key bytes (33 bytes compressed)
    let public_key_bytes = xpub.public_key.serialize();

    Ok((parent_fingerprint, chain_code, public_key_bytes))
}

/// Derive auto-accept proof key according to DIP-0015
/// Path: m/9'/5'/16'/timestamp'
pub fn derive_auto_accept_key(
    master_seed: &[u8],
    network: Network,
    timestamp: u32,
) -> Result<ExtendedPrivKey, String> {
    // Create extended private key from seed
    let master_xprv = ExtendedPrivKey::new_master(network, master_seed)
        .map_err(|e| format!("Failed to create master key: {}", e))?;

    // Build derivation path: m/9'/5'/16'/timestamp'
    let path = DerivationPath::from_str(&format!(
        "m/9'/5'/{}'/{}'",
        DASHPAY_AUTO_ACCEPT_FEATURE, timestamp
    ))
    .map_err(|e| format!("Invalid derivation path: {}", e))?;

    // Derive the key
    let auto_accept_key = master_xprv
        .derive_priv(&dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new(), &path)
        .map_err(|e| format!("Failed to derive auto-accept key: {}", e))?;

    Ok(auto_accept_key)
}

/// Calculate account reference as specified in DIP-0015
pub fn calculate_account_reference(
    sender_secret_key: &[u8],
    extended_public_key: &ExtendedPubKey,
    account: u32,
    version: u32,
) -> u32 {
    use dash_sdk::dpp::dashcore::hashes::hmac::{Hmac, HmacEngine};
    use dash_sdk::dpp::dashcore::hashes::sha256;

    // Serialize the extended public key
    let xpub_bytes = extended_public_key.encode();

    // Create HMAC-SHA256(senderSecretKey, extendedPublicKey)
    let mut engine = HmacEngine::<sha256::Hash>::new(sender_secret_key);
    engine.input(&xpub_bytes);
    let ask = Hmac::<sha256::Hash>::from_engine(engine);

    // Take the 28 most significant bits
    let ask_bytes = ask.to_byte_array();
    let ask28 = u32::from_be_bytes([ask_bytes[0], ask_bytes[1], ask_bytes[2], ask_bytes[3]]) >> 4;

    // Prepare account reference
    let shortened_account_bits = account & 0x0FFFFFFF;
    let version_bits = version << 28;

    // Combine: Version | (ASK28 XOR ShortenedAccountBits)
    version_bits | (ask28 ^ shortened_account_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashpay_derivation_path() {
        // Test that we can create valid derivation paths
        let path = DerivationPath::from_str("m/9'/5'/15'/0'/").unwrap();
        assert_eq!(path.len(), 4);
    }

    #[test]
    fn test_account_reference_calculation() {
        // Test account reference calculation
        let secret_key = [1u8; 32];
        let network = Network::Testnet;
        let master_seed = [2u8; 64];

        let master_xprv = ExtendedPrivKey::new_master(network, &master_seed).unwrap();
        let xpub = ExtendedPubKey::from_priv(
            &dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new(),
            &master_xprv,
        );

        let account_ref = calculate_account_reference(&secret_key, &xpub, 0, 0);

        // Verify it's a valid u32
        assert!(account_ref <= u32::MAX);
        // Verify version bits are in the right place
        assert_eq!(account_ref >> 28, 0);
    }
}
