use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
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

    let network = app_context.network;

    // Acquire the key-wallet read guard for derivation. Note: this
    // function derives addresses via the standalone DIP-14 helpers
    // without touching the key-wallet account pool. Phase 10 will
    // consolidate this with `bootstrap_dashpay_contact_accounts` and
    // the pool's `maintain_gap_limit` mechanism so all DashPay
    // address generation flows through one code path.
    let info_guard = platform_wallet_arc.state().await;
    let key_wallet_guard = info_guard.wallet();

    for contact in contacts {
        let contact_id = match Identifier::from_bytes(&contact.contact_identity_id) {
            Ok(id) => id,
            Err(e) => {
                result.errors.push(format!("Invalid contact ID: {}", e));
                continue;
            }
        };

        // Always derive the first `DASHPAY_GAP_LIMIT` addresses for
        // each contact. DIP-14 derivation is deterministic so
        // repeated calls produce the same addresses — callers can
        // invoke this idempotently without duplicating work on the
        // key-wallet side.
        match derive_receiving_addresses_for_contact(
            key_wallet_guard,
            network,
            &our_identity_id,
            &contact_id,
            0,
            DASHPAY_GAP_LIMIT,
        ) {
            Ok(addresses) => {
                result.addresses_registered += addresses.len();
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
