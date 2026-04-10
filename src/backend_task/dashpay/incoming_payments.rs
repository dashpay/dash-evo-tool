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
            key_wallet_guard,
            network,
            &our_identity_id,
            &contact_id,
            start_index,
            count,
        ) {
            Ok(addresses) => {
                // Addresses are tracked by key-wallet's
                // `DashpayReceivingFunds` account pool (registered at
                // contact establishment time via
                // `DashPayWallet::register_contact_account`), so there's
                // no separate evo-tool mapping table to populate any
                // more (Phase 9b-4). Just bump the result counter so
                // the caller can log how many addresses were derived.
                result.addresses_registered += addresses.len();

                // Update the bloom_registered_count via the platform
                // wallet — the persister catches the changeset and
                // writes to `dashpay_contact_address_indices` on
                // flush (Phase 9b-3).
                super::platform_wallet_cache::cache_contact_bloom_registered_count(
                    app_context,
                    &our_identity_id,
                    &contact_id,
                    target_count,
                )
                .await;

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
/// Match a received transaction output's address to a DashPay
/// contact relationship by iterating every registered platform
/// wallet and asking it whether the address belongs to one of its
/// `DashpayReceivingFunds` accounts (Phase 9b-4).
///
/// Returns `Some((owner_identity_id, contact_identity_id, address_index))`
/// on match, or `None` if the address isn't a DashPay contact
/// address for any wallet in this app context.
///
/// Uses `DashPayWallet::match_incoming_dashpay_address_blocking`
/// under the hood — safe to call from sync contexts (egui frame
/// loop, ZMQ listeners) as long as no tokio runtime is in scope.
pub fn match_transaction_to_contact(
    app_context: &AppContext,
    address: &Address,
) -> Result<Option<(Identifier, Identifier, u32)>, String> {
    let wallets = app_context
        .wallets
        .read()
        .map_err(|e| format!("Failed to read wallets: {}", e))?;
    for wallet_arc in wallets.values() {
        let pw = {
            let guard = match wallet_arc.read() {
                Ok(g) => g,
                Err(_) => continue,
            };
            match guard.platform_wallet.clone() {
                Some(pw) => pw,
                None => continue,
            }
        };
        if let Some(m) = pw
            .dashpay()
            .match_incoming_dashpay_address_blocking(address)
        {
            return Ok(Some((
                m.user_identity_id,
                m.friend_identity_id,
                m.address_index,
            )));
        }
    }
    Ok(None)
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
        // Bump the highest_receive_index via the platform wallet —
        // persister catches the changeset and writes to
        // `dashpay_contact_address_indices` on flush (Phase 9b-3).
        // Monotonic — if the in-memory value is already ≥ this
        // index, the mutation emits an empty changeset.
        super::platform_wallet_cache::cache_contact_highest_receive_index(
            app_context,
            &owner_id,
            &contact_id,
            address_index + 1,
        )
        .await;
    }

    // Record the received payment via the platform wallet so the
    // persister writes it to `dashpay_payments` on flush (Phase 9b-2).
    // We have the owner's `Identifier` but not its `QualifiedIdentity`
    // — load it by id so `platform_wallet_for_identity` can resolve
    // the wallet seed hash.
    let wallets_snapshot: std::collections::BTreeMap<
        crate::model::wallet::WalletSeedHash,
        Arc<std::sync::RwLock<crate::model::wallet::Wallet>>,
    > = if let Ok(wallets) = app_context.wallets.read() {
        wallets.clone()
    } else {
        tracing::warn!("Failed to snapshot wallets for platform-wallet payment cache");
        return Ok(Some(IncomingPaymentInfo {
            tx_id: tx_id.to_string(),
            from_contact_id: contact_id,
            to_identity_id: owner_id,
            address: address.clone(),
            amount_duffs,
            address_index,
        }));
    };
    match app_context
        .db
        .get_identity_by_id(&owner_id, app_context, &wallets_snapshot)
    {
        Ok(Some(owner_qi)) => {
            crate::backend_task::dashpay::payments::cache_payment_via_platform_wallet(
                app_context,
                &owner_qi,
                tx_id.to_string(),
                platform_wallet::wallet::dashpay::PaymentEntry::new_received(
                    contact_id,
                    amount_duffs,
                    None,
                ),
            )
            .await;
        }
        Ok(None) => {
            tracing::warn!(
                identity = %owner_id,
                "skipping platform-wallet payment cache: owner identity not found"
            );
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "skipping platform-wallet payment cache: identity lookup failed"
            );
        }
    }

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
