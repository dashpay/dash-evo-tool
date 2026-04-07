use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Default gap limit for DashPay address derivation
const DASHPAY_GAP_LIMIT: u32 = 20;

/// Information about a DashPay receiving address
#[derive(Debug, Clone)]
pub struct DashPayReceivingAddress {
    pub address: Address,
    pub contact_id: Identifier,
    pub owner_id: Identifier,
    pub address_index: u32,
}

/// Result of registering DashPay addresses
#[derive(Debug, Default)]
pub struct DashPayAddressRegistrationResult {
    pub addresses_registered: usize,
    pub contacts_processed: usize,
    pub errors: Vec<String>,
}

/// Derive the receiving addresses for a contact relationship
/// These are the addresses the CONTACT will use to pay US
/// Path: m/9'/coin'/15'/account'/(our_id)/(contact_id)/index
///
/// Uses platform-wallet's DIP-14 derivation via `derive_contact_xpub` and
/// `derive_contact_payment_address`.
pub fn derive_receiving_addresses_for_contact(
    key_wallet: &dash_sdk::dpp::key_wallet::wallet::Wallet,
    network: Network,
    our_identity_id: &Identifier,
    contact_id: &Identifier,
    start_index: u32,
    count: u32,
) -> Result<Vec<DashPayReceivingAddress>, String> {
    // For receiving payments, we derive from OUR xpub
    // Path: m/9'/coin'/15'/0'/(our_id)/(contact_id)
    let xpub_data = platform_wallet::derive_contact_xpub(
        key_wallet,
        network,
        0, // account 0
        our_identity_id,
        contact_id,
    )
    .map_err(|e| format!("Failed to derive contact xpub: {}", e))?;

    let addresses = platform_wallet::derive_contact_payment_addresses(
        &xpub_data.xpub,
        start_index,
        count,
        network,
    )
    .map_err(|e| format!("Failed to derive payment addresses: {}", e))?;

    Ok(addresses
        .into_iter()
        .enumerate()
        .map(|(i, address)| DashPayReceivingAddress {
            address,
            contact_id: *contact_id,
            owner_id: *our_identity_id,
            address_index: start_index + i as u32,
        })
        .collect())
}

/// Register DashPay receiving addresses for all contacts of an identity
/// This derives addresses up to the gap limit for each contact and registers them
/// with the wallet for transaction detection
pub async fn register_dashpay_addresses_for_identity(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
) -> Result<DashPayAddressRegistrationResult, String> {
    let mut result = DashPayAddressRegistrationResult::default();
    let our_identity_id = identity.identity.id();

    // Get the evo-tool wallet wrapper
    let wallet = identity
        .associated_wallets
        .values()
        .next()
        .ok_or("No wallet associated with identity")?;

    // Get the platform wallet's key-wallet Wallet for DIP-14 derivation
    let platform_wallet_arc = {
        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
        if !wallet_guard.is_open() {
            return Err("Wallet must be unlocked to register DashPay addresses".to_string());
        }
        wallet_guard
            .platform_wallet
            .clone()
            .ok_or("Platform wallet not available")?
    };

    // Load all contacts for this identity from the database
    let network_str = app_context.network.to_string();
    let contacts = app_context
        .db
        .load_dashpay_contacts(&our_identity_id, &network_str)
        .map_err(|e| format!("Failed to load contacts: {}", e))?;

    if contacts.is_empty() {
        return Ok(result);
    }

    // Load address indices for all contacts
    let address_indices = app_context
        .db
        .get_all_contact_address_indices(&our_identity_id)
        .map_err(|e| format!("Failed to load address indices: {}", e))?;

    // Create a map for quick lookup
    let indices_map: BTreeMap<Vec<u8>, _> = address_indices
        .into_iter()
        .map(|idx| (idx.contact_identity_id.clone(), idx))
        .collect();

    let network = app_context.network;

    // Acquire the key-wallet read guard for derivation
    let info_guard = platform_wallet_arc.core().state().await;
    let key_wallet_guard = &info_guard.wallet;

    for contact in contacts {
        let contact_id = match Identifier::from_bytes(&contact.contact_identity_id) {
            Ok(id) => id,
            Err(e) => {
                result.errors.push(format!("Invalid contact ID: {}", e));
                continue;
            }
        };

        // Get the current highest receive index for this contact
        let highest_receive_index = indices_map
            .get(&contact.contact_identity_id)
            .map(|idx| idx.highest_receive_index)
            .unwrap_or(0);

        // Get how many addresses are already registered with bloom filter
        let bloom_registered = indices_map
            .get(&contact.contact_identity_id)
            .map(|idx| idx.bloom_registered_count)
            .unwrap_or(0);

        // Calculate how many new addresses we need to derive
        // We want addresses from 0 to (highest_receive_index + GAP_LIMIT)
        let target_count = highest_receive_index.saturating_add(DASHPAY_GAP_LIMIT);

        // Only derive new addresses if we need more than what's registered
        if target_count <= bloom_registered {
            result.contacts_processed += 1;
            continue;
        }

        let start_index = bloom_registered;
        let count = target_count - bloom_registered;

        // Derive the receiving addresses
        match derive_receiving_addresses_for_contact(
            &key_wallet_guard,
            network,
            &our_identity_id,
            &contact_id,
            start_index,
            count,
        ) {
            Ok(addresses) => {
                // Register each address with the wallet
                for addr_info in &addresses {
                    if let Err(e) = register_dashpay_address(
                        app_context,
                        wallet,
                        &addr_info.address,
                        &our_identity_id,
                        &contact_id,
                        addr_info.address_index,
                    ) {
                        result.errors.push(format!(
                            "Failed to register address for contact {}: {}",
                            contact_id.to_string(Encoding::Base58),
                            e
                        ));
                    } else {
                        result.addresses_registered += 1;
                    }
                }

                // Update the bloom_registered_count in database
                if let Err(e) = app_context.db.update_bloom_registered_count(
                    &our_identity_id,
                    &contact_id,
                    target_count,
                ) {
                    result.errors.push(format!(
                        "Failed to update bloom count for contact {}: {}",
                        contact_id.to_string(Encoding::Base58),
                        e
                    ));
                }

                result.contacts_processed += 1;
            }
            Err(e) => {
                result.errors.push(format!(
                    "Failed to derive addresses for contact {}: {}",
                    contact_id.to_string(Encoding::Base58),
                    e
                ));
            }
        }
    }

    Ok(result)
}

/// Register a DashPay address mapping in the database.
///
/// Stores the address → (owner_id, contact_id, index) mapping so incoming
/// payments can be matched to the correct contact relationship.
/// Address monitoring is handled by ManagedWalletInfo's DashpayReceivingFunds
/// account pools — no additional address registration needed.
fn register_dashpay_address(
    app_context: &AppContext,
    _wallet: &Arc<std::sync::RwLock<crate::model::wallet::Wallet>>,
    address: &Address,
    owner_id: &Identifier,
    contact_id: &Identifier,
    address_index: u32,
) -> Result<(), String> {
    app_context
        .db
        .save_dashpay_address_mapping(owner_id, contact_id, address, address_index)
        .map_err(|e| format!("Failed to save address mapping: {}", e))
}

/// Match a received transaction to a DashPay contact
/// Returns the contact ID and payment details if the address belongs to a contact relationship
pub fn match_transaction_to_contact(
    app_context: &AppContext,
    address: &Address,
) -> Result<Option<(Identifier, Identifier, u32)>, String> {
    // Look up the address in the DashPay address mapping
    app_context
        .db
        .get_dashpay_address_mapping(address)
        .map_err(|e| format!("Failed to lookup address: {}", e))
}

/// Process an incoming transaction that was detected by SPV
/// This should be called when WalletEvent::TransactionReceived is received
pub async fn process_incoming_payment(
    app_context: &Arc<AppContext>,
    tx_id: &str,
    address: &Address,
    amount_duffs: u64,
) -> Result<Option<IncomingPaymentInfo>, String> {
    // Check if this address belongs to a DashPay contact relationship
    let mapping = match match_transaction_to_contact(app_context, address)? {
        Some(m) => m,
        None => return Ok(None), // Not a DashPay address
    };

    let (owner_id, contact_id, address_index) = mapping;

    // Update the highest receive index if needed
    let current_indices = app_context
        .db
        .get_contact_address_indices(&owner_id, &contact_id)
        .map_err(|e| format!("Failed to get address indices: {}", e))?;

    if address_index >= current_indices.highest_receive_index {
        app_context
            .db
            .update_highest_receive_index(&owner_id, &contact_id, address_index + 1)
            .map_err(|e| format!("Failed to update receive index: {}", e))?;
    }

    // Save the payment record
    app_context
        .db
        .save_payment(
            tx_id,
            &contact_id, // from contact
            &owner_id,   // to us
            amount_duffs as i64,
            None, // memo - not available for incoming
            "received",
        )
        .map_err(|e| format!("Failed to save payment: {}", e))?;

    Ok(Some(IncomingPaymentInfo {
        tx_id: tx_id.to_string(),
        from_contact_id: contact_id,
        to_identity_id: owner_id,
        address: address.clone(),
        amount_duffs,
        address_index,
    }))
}

/// Information about an incoming DashPay payment
#[derive(Debug, Clone)]
pub struct IncomingPaymentInfo {
    pub tx_id: String,
    pub from_contact_id: Identifier,
    pub to_identity_id: Identifier,
    pub address: Address,
    pub amount_duffs: u64,
    pub address_index: u32,
}
