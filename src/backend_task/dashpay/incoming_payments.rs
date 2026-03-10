use super::hd_derivation::{derive_dashpay_incoming_xpub, derive_payment_address};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashPayWalletScanResult {
    pub saved_payments: usize,
    pub mappings_available: bool,
}

#[derive(Clone, Copy)]
pub enum DashPayWalletScanMode<'a> {
    Full {
        active_seed_bytes: &'a [u8; 64],
        active_wallet: &'a Arc<std::sync::RwLock<Wallet>>,
    },
    Delta {
        active_seed_bytes: Option<&'a [u8; 64]>,
        active_wallet: Option<&'a Arc<std::sync::RwLock<Wallet>>>,
    },
}

/// Derive the receiving addresses for a contact relationship
/// These are the addresses the CONTACT will use to pay US
/// Path: m/9'/5'/15'/account'/(our_id)/(contact_id)/index
pub fn derive_receiving_addresses_for_contact(
    master_seed: &[u8],
    network: Network,
    our_identity_id: &Identifier,
    contact_id: &Identifier,
    start_index: u32,
    count: u32,
) -> Result<Vec<DashPayReceivingAddress>, String> {
    // For receiving payments, we derive from OUR xpub
    // Path: m/9'/5'/15'/0'/(our_id)/(contact_id)
    // This is the key we sent to the contact in our contact request
    let xpub = derive_dashpay_incoming_xpub(
        master_seed,
        network,
        0, // account 0
        our_identity_id,
        contact_id,
    )?;

    let mut addresses = Vec::with_capacity(count as usize);
    for i in start_index..(start_index + count) {
        let address = derive_payment_address(&xpub, i)?;
        addresses.push(DashPayReceivingAddress {
            address,
            contact_id: *contact_id,
            owner_id: *our_identity_id,
            address_index: i,
        });
    }

    Ok(addresses)
}

/// Register DashPay receiving addresses for all contacts of an identity
/// This derives addresses up to the gap limit for each contact and registers them
/// with the wallet for transaction detection
pub async fn register_dashpay_addresses_for_identity(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
) -> Result<DashPayAddressRegistrationResult, DashPayScanError> {
    register_dashpay_addresses_for_identity_impl(app_context, identity)
}

fn register_dashpay_addresses_for_identity_impl(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
) -> Result<DashPayAddressRegistrationResult, DashPayScanError> {
    let mut result = DashPayAddressRegistrationResult::default();
    let our_identity_id = identity.identity.id();

    // Load all contacts for this identity from the database
    let network_str = app_context.network.to_string();
    let contacts = app_context
        .db
        .load_dashpay_contacts(&our_identity_id, &network_str)
        .map_err(DashPayScanError::LoadContacts)?;

    if contacts.is_empty() {
        return Ok(result);
    }

    // Load address indices for all contacts
    let address_indices = app_context
        .db
        .get_all_contact_address_indices(&our_identity_id)
        .map_err(DashPayScanError::LoadContactAddressIndices)?;

    // Create a map for quick lookup
    let indices_map: BTreeMap<Vec<u8>, _> = address_indices
        .into_iter()
        .map(|idx| (idx.contact_identity_id.clone(), idx))
        .collect();

    let network = app_context.network;
    let wallet_seeds = collect_wallet_seeds(&identity.associated_wallets)?;

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
        let mut contact_registration_succeeded = true;

        // Derive the receiving addresses
        for wallet_seed in &wallet_seeds {
            match derive_receiving_addresses_for_contact(
                &wallet_seed.seed,
                network,
                &our_identity_id,
                &contact_id,
                start_index,
                count,
            ) {
                Ok(addresses) => {
                    for addr_info in &addresses {
                        if let Err(e) = register_dashpay_address(
                            app_context,
                            &wallet_seed.wallet,
                            &wallet_seed.seed_hash,
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
                            contact_registration_succeeded = false;
                        } else {
                            result.addresses_registered += 1;
                        }
                    }
                }
                Err(e) => {
                    result.errors.push(format!(
                        "Failed to derive addresses for contact {}: {}",
                        contact_id.to_string(Encoding::Base58),
                        e
                    ));
                    contact_registration_succeeded = false;
                }
            }
        }

        if contact_registration_succeeded
            && let Err(e) = app_context.db.update_bloom_registered_count(
                &our_identity_id,
                &contact_id,
                target_count,
            )
        {
            result.errors.push(format!(
                "Failed to update bloom count for contact {}: {}",
                contact_id.to_string(Encoding::Base58),
                e
            ));
        }

        result.contacts_processed += 1;
    }

    Ok(result)
}

struct WalletSeedRegistration {
    wallet: Arc<std::sync::RwLock<Wallet>>,
    seed_hash: WalletSeedHash,
    seed: [u8; 64],
}

fn collect_wallet_seeds(
    wallets: &BTreeMap<WalletSeedHash, Arc<std::sync::RwLock<Wallet>>>,
) -> Result<Vec<WalletSeedRegistration>, DashPayScanError> {
    if wallets.is_empty() {
        return Err(DashPayScanError::NoAssociatedWallet);
    }

    wallets
        .values()
        .map(|wallet| {
            let wallet_guard = wallet
                .read()
                .map_err(|_| DashPayScanError::WalletLockPoisoned)?;
            if !wallet_guard.is_open() {
                return Err(DashPayScanError::WalletLocked);
            }
            let seed = *wallet_guard
                .seed_bytes()
                .map_err(|e| DashPayScanError::WalletSeedUnavailable(e.to_string()))?;
            Ok(WalletSeedRegistration {
                wallet: Arc::clone(wallet),
                seed_hash: wallet_guard.seed_hash(),
                seed,
            })
        })
        .collect()
}

fn wallet_seed_hash(
    wallet: &Arc<std::sync::RwLock<Wallet>>,
) -> Result<WalletSeedHash, DashPayScanError> {
    let guard = wallet
        .read()
        .map_err(|_| DashPayScanError::WalletLockPoisoned)?;
    Ok(guard.seed_hash())
}

#[cfg(test)]
fn compute_seed_hash(seed_bytes: &[u8; 64]) -> WalletSeedHash {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(seed_bytes);
    hasher.finalize().into()
}

fn contacts_with_target_counts(
    db: &crate::database::Database,
    network: Network,
    identity_id: &Identifier,
) -> Result<Vec<(Identifier, u32)>, DashPayScanError> {
    let network_str = network.to_string();
    let contacts = db
        .load_dashpay_contacts(identity_id, &network_str)
        .map_err(DashPayScanError::LoadContacts)?;
    if contacts.is_empty() {
        return Ok(Vec::new());
    }

    let address_indices = db
        .get_all_contact_address_indices(identity_id)
        .map_err(DashPayScanError::LoadContactAddressIndices)?;
    let indices_map: BTreeMap<Vec<u8>, _> = address_indices
        .into_iter()
        .map(|idx| (idx.contact_identity_id.clone(), idx))
        .collect();

    contacts
        .into_iter()
        .map(|contact| {
            let contact_id = Identifier::from_bytes(&contact.contact_identity_id).map_err(|e| {
                DashPayScanError::MalformedContactId {
                    identity_id: *identity_id,
                    details: e.to_string(),
                }
            })?;
            let target_count = indices_map
                .get(&contact.contact_identity_id)
                .map(|idx| idx.highest_receive_index.saturating_add(DASHPAY_GAP_LIMIT))
                .unwrap_or(DASHPAY_GAP_LIMIT);
            Ok((contact_id, target_count))
        })
        .collect()
}

fn derive_dashpay_address_mappings_for_seed(
    owner_identity_id: &Identifier,
    active_seed_bytes: &[u8],
    network: Network,
    contacts: &[(Identifier, u32)],
) -> Result<Vec<DashPayReceivingAddress>, DashPayScanError> {
    let mut addresses = Vec::new();
    for (contact_id, target_count) in contacts {
        if *target_count == 0 {
            continue;
        }
        addresses.extend(
            derive_receiving_addresses_for_contact(
                active_seed_bytes,
                network,
                owner_identity_id,
                contact_id,
                0,
                *target_count,
            )
            .map_err(|details| DashPayScanError::DeriveReceivingAddresses {
                contact_id: *contact_id,
                details,
            })?,
        );
    }
    Ok(addresses)
}

fn ensure_dashpay_address_mappings_for_scan(
    db: &crate::database::Database,
    network: Network,
    identity_id: &Identifier,
    active_seed_hash: &WalletSeedHash,
    active_seed_bytes: &[u8],
    active_wallet: Option<&Arc<std::sync::RwLock<Wallet>>>,
) -> Result<bool, DashPayScanError> {
    let contacts = contacts_with_target_counts(db, network, identity_id)?;
    if contacts.is_empty() {
        return Ok(false);
    }

    let addresses = derive_dashpay_address_mappings_for_seed(
        identity_id,
        active_seed_bytes,
        network,
        &contacts,
    )?;
    if addresses.is_empty() {
        return Ok(false);
    }

    for addr_info in addresses {
        if let Some(active_wallet) = active_wallet {
            db.save_dashpay_address_mapping(
                identity_id,
                &addr_info.contact_id,
                active_seed_hash,
                &addr_info.address,
                addr_info.address_index,
            )
            .map_err(DashPayScanError::SaveAddressMapping)?;
            register_dashpay_address_in_wallet(
                active_wallet,
                &addr_info.address,
                identity_id,
                &addr_info.contact_id,
                addr_info.address_index,
            )?;
        } else {
            db.save_dashpay_address_mapping(
                identity_id,
                &addr_info.contact_id,
                active_seed_hash,
                &addr_info.address,
                addr_info.address_index,
            )
            .map_err(DashPayScanError::SaveAddressMapping)?;
        }
    }

    Ok(true)
}

fn dashpay_address_mappings_cover_targets(
    db: &crate::database::Database,
    network: Network,
    identity_id: &Identifier,
    seed_hash: &WalletSeedHash,
) -> Result<bool, DashPayScanError> {
    let contacts = contacts_with_target_counts(db, network, identity_id)?;
    if contacts.is_empty() {
        return Ok(true);
    }

    let mappings = db
        .get_dashpay_address_mappings_for_wallet(identity_id, seed_hash)
        .map_err(DashPayScanError::LoadAddressMappings)?;
    let mut coverage: HashMap<Identifier, HashSet<u32>> = HashMap::new();
    for (_, contact_id, address_index) in mappings {
        coverage
            .entry(contact_id)
            .or_default()
            .insert(address_index);
    }

    Ok(contacts.into_iter().all(|(contact_id, target_count)| {
        if target_count == 0 {
            return true;
        }

        let Some(indices) = coverage.get(&contact_id) else {
            return false;
        };
        (0..target_count).all(|index| indices.contains(&index))
    }))
}

fn register_existing_dashpay_address_mappings_in_wallet(
    db: &crate::database::Database,
    network: Network,
    identity_id: &Identifier,
    seed_hash: &WalletSeedHash,
    active_wallet: &Arc<std::sync::RwLock<Wallet>>,
) -> Result<bool, DashPayScanError> {
    use std::str::FromStr;

    let mappings = db
        .get_dashpay_address_mappings_for_wallet(identity_id, seed_hash)
        .map_err(DashPayScanError::LoadAddressMappings)?;

    if mappings.is_empty() {
        return Ok(false);
    }

    for (address_str, contact_id, address_index) in mappings {
        let address = Address::from_str(&address_str)
            .map_err(|err| DashPayScanError::MalformedStoredAddressMapping {
                address: address_str.clone(),
                details: err.to_string(),
            })?
            .require_network(network)
            .map_err(|err| DashPayScanError::MalformedStoredAddressMapping {
                address: address_str,
                details: err.to_string(),
            })?;

        register_dashpay_address_in_wallet(
            active_wallet,
            &address,
            identity_id,
            &contact_id,
            address_index,
        )?;
    }

    Ok(true)
}

/// Register a single DashPay address with the wallet
fn register_dashpay_address(
    app_context: &AppContext,
    wallet: &Arc<std::sync::RwLock<crate::model::wallet::Wallet>>,
    seed_hash: &WalletSeedHash,
    address: &Address,
    owner_id: &Identifier,
    contact_id: &Identifier,
    address_index: u32,
) -> Result<(), DashPayScanError> {
    // Store the DashPay address mapping in the database
    app_context
        .db
        .save_dashpay_address_mapping(owner_id, contact_id, seed_hash, address, address_index)
        .map_err(DashPayScanError::SaveAddressMapping)?;

    register_dashpay_address_in_wallet(wallet, address, owner_id, contact_id, address_index)
}

fn register_dashpay_address_in_wallet(
    wallet: &Arc<std::sync::RwLock<crate::model::wallet::Wallet>>,
    address: &Address,
    owner_id: &Identifier,
    contact_id: &Identifier,
    address_index: u32,
) -> Result<(), DashPayScanError> {
    use crate::model::wallet::{DerivationPathReference, DerivationPathType};
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};

    let path = DerivationPath::from(vec![
        ChildNumber::from_hardened_idx(9).unwrap(),
        ChildNumber::from_hardened_idx(5).unwrap(),
        ChildNumber::from_hardened_idx(15).unwrap(),
        ChildNumber::from_hardened_idx(0).unwrap(),
        ChildNumber::from_normal_idx(hash_identifier_to_u32(owner_id)).unwrap(),
        ChildNumber::from_normal_idx(hash_identifier_to_u32(contact_id)).unwrap(),
        ChildNumber::from_normal_idx(address_index).unwrap(),
    ]);

    let mut guard = wallet
        .write()
        .map_err(|_| DashPayScanError::WalletLockPoisoned)?;

    if guard.known_addresses.contains_key(address) {
        return Ok(());
    }

    guard.known_addresses.insert(address.clone(), path.clone());
    guard.watched_addresses.insert(
        path,
        crate::model::wallet::AddressInfo {
            address: address.clone(),
            path_type: DerivationPathType::DASHPAY,
            path_reference: DerivationPathReference::ContactBasedFunds,
        },
    );

    Ok(())
}

/// Hash an identifier to a u32 for use in derivation path representation
fn hash_identifier_to_u32(id: &Identifier) -> u32 {
    use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};
    let hash = sha256::Hash::hash(&id.to_buffer());
    let bytes = hash.to_byte_array();
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & 0x7FFFFFFF
}

/// Match a received transaction to a DashPay contact
/// Returns the contact ID and payment details if the address belongs to a contact relationship
pub fn match_transaction_to_contact(
    app_context: &AppContext,
    address: &Address,
) -> Result<Option<(Identifier, Identifier, u32)>, DashPayScanError> {
    // Look up the address in the DashPay address mapping
    app_context
        .db
        .get_dashpay_address_mapping(address)
        .map_err(DashPayScanError::LookupAddressMapping)
}

/// Record a single incoming DashPay payment in the database.
///
/// Saves the payment record and updates the highest receive index for the
/// contact.  This is the shared core of [`process_incoming_payment`] (real-time
/// SPV path) and [`scan_wallet_transactions_for_dashpay_payments`] (retroactive
/// scan).
#[derive(Debug, Error)]
pub enum DashPayScanError {
    #[error("failed to load identity for scan: {0}")]
    LoadIdentity(#[source] rusqlite::Error),
    #[error("identity not found for scan: {0}")]
    IdentityNotFound(Identifier),
    #[error("wallet lock was poisoned while preparing scan")]
    WalletLockPoisoned,
    #[error("wallet must be unlocked to scan DashPay payments")]
    WalletLocked,
    #[error("wallet seed not available: {0}")]
    WalletSeedUnavailable(String),
    #[error("no wallet associated with identity")]
    NoAssociatedWallet,
    #[error("failed to load DashPay contacts: {0}")]
    LoadContacts(#[source] rusqlite::Error),
    #[error("failed to load DashPay contact address indices: {0}")]
    LoadContactAddressIndices(#[source] rusqlite::Error),
    #[error("failed to load DashPay address mappings: {0}")]
    LoadAddressMappings(#[source] rusqlite::Error),
    #[error("failed to lookup DashPay address mapping: {0}")]
    LookupAddressMapping(#[source] rusqlite::Error),
    #[error("failed to save DashPay address mapping: {0}")]
    SaveAddressMapping(#[source] rusqlite::Error),
    #[error("stored DashPay address mapping is malformed for address {address}: {details}")]
    MalformedStoredAddressMapping { address: String, details: String },
    #[error("malformed contact identifier for owner {identity_id}: {details}")]
    MalformedContactId {
        identity_id: Identifier,
        details: String,
    },
    #[error("failed to derive receiving addresses for contact {contact_id}: {details}")]
    DeriveReceivingAddresses {
        contact_id: Identifier,
        details: String,
    },
    #[error("failed to save payment: {0}")]
    SavePayment(#[source] rusqlite::Error),
    #[error("failed to update receive index: {0}")]
    UpdateReceiveIndex(#[source] rusqlite::Error),
}

#[allow(clippy::too_many_arguments)]
fn record_incoming_payment(
    db: &crate::database::Database,
    tx_id: &str,
    output_index: Option<u32>,
    owner_id: &Identifier,
    contact_id: &Identifier,
    amount_duffs: u64,
    address_index: u32,
    created_at: Option<i64>,
) -> Result<bool, DashPayScanError> {
    let save_result = db
        .save_payment_with_output_index(
            tx_id,
            output_index,
            contact_id, // from contact
            owner_id,   // to us
            amount_duffs as i64,
            None, // memo — not available for incoming
            "received",
            created_at,
        )
        .map_err(DashPayScanError::SavePayment)?;

    if save_result.changed() {
        db.update_highest_receive_index(owner_id, contact_id, address_index + 1)
            .map_err(DashPayScanError::UpdateReceiveIndex)?;
    }

    Ok(save_result.changed())
}

/// Process an incoming transaction that was detected by SPV
/// This should be called when WalletEvent::TransactionReceived is received
pub async fn process_incoming_payment(
    app_context: &Arc<AppContext>,
    tx_id: &str,
    address: &Address,
    amount_duffs: u64,
    output_index: Option<u32>,
) -> Result<Option<IncomingPaymentInfo>, DashPayScanError> {
    // Check if this address belongs to a DashPay contact relationship
    let mapping = match match_transaction_to_contact(app_context, address)? {
        Some(m) => m,
        None => return Ok(None), // Not a DashPay address
    };

    let (owner_id, contact_id, address_index) = mapping;

    // Record the payment and update address index.
    // Real-time path: pass None so created_at defaults to the current time.
    record_incoming_payment(
        &app_context.db,
        tx_id,
        output_index,
        &owner_id,
        &contact_id,
        amount_duffs,
        address_index,
        None,
    )?;

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

/// A payment match produced by scanning wallet transactions against the
/// DashPay address mapping table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DashPayScanMatch {
    tx_id: String,
    output_index: u32,
    contact_id: Identifier,
    address_index: u32,
    amount_duffs: u64,
    /// Block/tx timestamp, used as the payment row's `created_at`.
    timestamp: u64,
}

/// Pure matching logic for the retroactive DashPay scan.
///
/// Returns one match per output that hits a known DashPay receiving address.
/// Returns an empty vector if the transaction should be skipped:
/// - transaction is outgoing (address_map only contains receive-side addresses,
///   so matching an output of an outgoing tx does NOT indicate a payment to a
///   contact — it would likely be our own change address or unrelated),
/// - no output matches a known DashPay receiving address.
fn match_dashpay_payment_in_transaction(
    wtx: &crate::model::wallet::WalletTransaction,
    address_map: &std::collections::HashMap<String, (Identifier, u32)>,
    network: Network,
) -> Vec<DashPayScanMatch> {
    if !wtx.is_incoming() {
        return Vec::new();
    }

    let tx_id = wtx.txid.to_string();
    let mut matches = Vec::new();
    for (output_index, output) in wtx.transaction.output.iter().enumerate() {
        let Ok(addr) = Address::from_script(&output.script_pubkey, network) else {
            continue;
        };
        if let Some((contact_id, address_index)) = address_map.get(&addr.to_string()) {
            matches.push(DashPayScanMatch {
                tx_id: tx_id.clone(),
                output_index: output_index as u32,
                contact_id: *contact_id,
                address_index: *address_index,
                amount_duffs: output.value,
                timestamp: wtx.timestamp,
            });
        }
    }

    matches
}

/// Scan wallet transactions against DashPay address mappings and save any
/// matched payments to the `dashpay_payments` table.
///
/// This is the retroactive counterpart of [`process_incoming_payment`]: it
/// iterates over *all* wallet transactions (already synced via SPV) and checks
/// every output address against the stored address-mapping table. Calling this
/// function repeatedly is safe and idempotent because inserts are keyed by
/// transaction/contact/output identity and ignored on conflict.
///
/// It is designed to be called from:
/// - the SPV reconcile flow (after wallet transactions are updated), and
/// - the `LoadPaymentHistory` backend task (for immediate retroactive
///   detection when the user opens the Payment History screen).
pub fn scan_wallet_transactions_for_dashpay_payments(
    app_context: &AppContext,
    identity_id: &Identifier,
    transactions: &[crate::model::wallet::WalletTransaction],
    scan_mode: DashPayWalletScanMode<'_>,
) -> Result<DashPayWalletScanResult, DashPayScanError> {
    scan_wallet_transactions_for_dashpay_payments_impl(
        &app_context.db,
        app_context.network,
        identity_id,
        transactions,
        scan_mode,
    )
}

fn scan_wallet_transactions_for_dashpay_payments_impl(
    db: &crate::database::Database,
    network: Network,
    identity_id: &Identifier,
    transactions: &[crate::model::wallet::WalletTransaction],
    scan_mode: DashPayWalletScanMode<'_>,
) -> Result<DashPayWalletScanResult, DashPayScanError> {
    let mappings_ensured = match scan_mode {
        DashPayWalletScanMode::Full {
            active_seed_bytes,
            active_wallet,
        } => ensure_dashpay_address_mappings_for_scan(
            db,
            network,
            identity_id,
            &wallet_seed_hash(active_wallet)?,
            active_seed_bytes,
            Some(active_wallet),
        )?,
        DashPayWalletScanMode::Delta {
            active_seed_bytes,
            active_wallet,
        } => match active_wallet {
            Some(active_wallet) => {
                let active_seed_hash = wallet_seed_hash(active_wallet)?;
                if dashpay_address_mappings_cover_targets(
                    db,
                    network,
                    identity_id,
                    &active_seed_hash,
                )? {
                    register_existing_dashpay_address_mappings_in_wallet(
                        db,
                        network,
                        identity_id,
                        &active_seed_hash,
                        active_wallet,
                    )?
                } else if let Some(active_seed_bytes) = active_seed_bytes {
                    ensure_dashpay_address_mappings_for_scan(
                        db,
                        network,
                        identity_id,
                        &active_seed_hash,
                        active_seed_bytes,
                        Some(active_wallet),
                    )?
                } else {
                    false
                }
            }
            None => false,
        },
    };

    // Build a lookup set of addresses → (owner_id, contact_id, address_index)
    let mappings = match scan_mode {
        DashPayWalletScanMode::Full { active_wallet, .. } => db
            .get_dashpay_address_mappings_for_wallet(identity_id, &wallet_seed_hash(active_wallet)?)
            .map_err(DashPayScanError::LoadAddressMappings)?,
        DashPayWalletScanMode::Delta {
            active_wallet: Some(active_wallet),
            ..
        } => db
            .get_dashpay_address_mappings_for_wallet(identity_id, &wallet_seed_hash(active_wallet)?)
            .map_err(DashPayScanError::LoadAddressMappings)?,
        DashPayWalletScanMode::Delta {
            active_wallet: None,
            ..
        } => Vec::new(),
    };

    if mappings.is_empty() {
        return Ok(DashPayWalletScanResult {
            saved_payments: 0,
            mappings_available: false,
        });
    }

    // address string → (contact_id, address_index)
    let address_map: HashMap<String, (Identifier, u32)> = mappings
        .into_iter()
        .map(|(addr_str, contact_id, idx)| (addr_str, (contact_id, idx)))
        .collect();

    let mut saved = 0usize;

    for wtx in transactions {
        for m in match_dashpay_payment_in_transaction(wtx, &address_map, network) {
            let created_at = (m.timestamp != 0).then_some(m.timestamp as i64);
            if record_incoming_payment(
                db,
                &m.tx_id,
                Some(m.output_index),
                identity_id,
                &m.contact_id,
                m.amount_duffs,
                m.address_index,
                created_at,
            )? {
                saved += 1;
                tracing::info!(
                    tx_id = %m.tx_id,
                    vout = m.output_index,
                    contact = %m.contact_id.to_string(Encoding::Base58),
                    amount = m.amount_duffs,
                    direction = "received",
                    address_index = m.address_index,
                    "Saved DashPay payment from wallet transaction scan"
                );
            }
        }
    }

    if saved > 0 {
        tracing::info!(
            identity = %identity_id.to_string(Encoding::Base58),
            new_payments = saved,
            "DashPay wallet transaction scan complete"
        );
    }

    Ok(DashPayWalletScanResult {
        saved_payments: saved,
        mappings_available: mappings_ensured || !address_map.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;
    use crate::model::wallet::{TransactionStatus, Wallet, WalletTransaction};
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{PublicKey, Transaction, TxOut, Txid};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, RwLock};

    #[test]
    fn test_hash_identifier_to_u32() {
        let id = Identifier::random();
        let hash = hash_identifier_to_u32(&id);
        // Should be less than 2^31 (non-hardened range)
        assert!(hash < 0x80000000);
    }

    fn address_from_seed(byte: u8) -> Address {
        let pubkey_bytes = [byte; 33];
        // Start from a valid compressed secp256k1 point header (0x02) and tweak
        // the x coordinate. Fall back to bumping the byte if unlucky.
        let mut attempt = pubkey_bytes;
        attempt[0] = 0x02;
        attempt[1] = byte;
        loop {
            if let Ok(pk) = PublicKey::from_slice(&attempt) {
                return Address::p2pkh(&pk, Network::Testnet);
            }
            attempt[1] = attempt[1].wrapping_add(1);
        }
    }

    fn make_wtx(
        txid_byte: u8,
        outputs: Vec<TxOut>,
        net_amount: i64,
        timestamp: u64,
    ) -> WalletTransaction {
        WalletTransaction {
            txid: Txid::from_slice(&[txid_byte; 32]).unwrap(),
            transaction: Transaction {
                version: 2,
                lock_time: 0,
                input: vec![],
                output: outputs,
                special_transaction_payload: None,
            },
            timestamp,
            height: Some(100),
            block_hash: None,
            net_amount,
            fee: None,
            label: None,
            is_ours: true,
            status: TransactionStatus::Confirmed,
        }
    }

    fn wallet_from_seed(seed_byte: u8) -> Arc<RwLock<Wallet>> {
        let wallet = Wallet::new_from_seed([seed_byte; 64], Network::Testnet, None, None)
            .expect("wallet from seed");
        Arc::new(RwLock::new(wallet))
    }

    fn seed_from_wallet(wallet: &Arc<RwLock<Wallet>>) -> [u8; 64] {
        let guard = wallet.read().expect("wallet read");
        *guard.seed_bytes().expect("seed bytes")
    }

    fn save_contact(
        db: &crate::database::Database,
        owner_id: &Identifier,
        contact_id: &Identifier,
    ) {
        db.save_dashpay_contact(
            owner_id,
            contact_id,
            &Network::Testnet.to_string(),
            Some("contact"),
            None,
            None,
            None,
            "accepted",
        )
        .expect("save contact");
    }

    fn save_mappings_for_seed(
        db: &crate::database::Database,
        owner_id: &Identifier,
        seed: &[u8; 64],
        mappings: &[DashPayReceivingAddress],
    ) {
        let seed_hash = compute_seed_hash(seed);
        for mapping in mappings {
            db.save_dashpay_address_mapping(
                owner_id,
                &mapping.contact_id,
                &seed_hash,
                &mapping.address,
                mapping.address_index,
            )
            .expect("save mapping");
        }
    }

    #[test]
    fn collect_wallet_seeds_returns_all_associated_wallets() {
        let wallet_a = wallet_from_seed(0x21);
        let wallet_b = wallet_from_seed(0x22);
        let seed_a = seed_from_wallet(&wallet_a);
        let seed_b = seed_from_wallet(&wallet_b);

        let mut wallets = BTreeMap::new();
        wallets.insert(
            wallet_a.read().expect("wallet a").seed_hash(),
            Arc::clone(&wallet_a),
        );
        wallets.insert(
            wallet_b.read().expect("wallet b").seed_hash(),
            Arc::clone(&wallet_b),
        );

        let collected = collect_wallet_seeds(&wallets).expect("collect wallet seeds");
        assert_eq!(collected.len(), 2);
        assert!(collected.iter().any(|entry| entry.seed == seed_a));
        assert!(collected.iter().any(|entry| entry.seed == seed_b));
        assert!(
            collected
                .iter()
                .any(|entry| entry.seed_hash == compute_seed_hash(&seed_a))
        );
        assert!(
            collected
                .iter()
                .any(|entry| entry.seed_hash == compute_seed_hash(&seed_b))
        );
    }

    #[test]
    fn ensure_mappings_for_scan_registers_wallet_known_and_watched_addresses() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let wallet = wallet_from_seed(0x31);
        let initial_known = wallet.read().expect("wallet read").known_addresses.len();
        let seed = seed_from_wallet(&wallet);

        save_contact(&db, &owner_id, &contact_id);
        db.update_highest_receive_index(&owner_id, &contact_id, 3)
            .expect("seed receive index");

        let ensured = ensure_dashpay_address_mappings_for_scan(
            &db,
            Network::Testnet,
            &owner_id,
            &compute_seed_hash(&seed),
            &seed,
            Some(&wallet),
        )
        .expect("ensure mappings");
        assert!(ensured);

        let expected = derive_receiving_addresses_for_contact(
            &seed,
            Network::Testnet,
            &owner_id,
            &contact_id,
            0,
            23,
        )
        .expect("derive expected addresses");
        let stored = db
            .get_all_dashpay_address_mappings(&owner_id)
            .expect("load mappings");
        assert_eq!(stored.len(), expected.len());

        let guard = wallet.read().expect("wallet read");
        assert_eq!(guard.known_addresses.len(), initial_known + expected.len());
        for addr in &expected {
            assert!(guard.known_addresses.contains_key(&addr.address));
            assert!(
                guard
                    .watched_addresses
                    .values()
                    .any(|info| info.address == addr.address)
            );
        }
    }

    #[test]
    fn delta_scan_with_seed_backfills_mappings_and_records_payment() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let wallet = wallet_from_seed(0x41);
        let seed = seed_from_wallet(&wallet);

        save_contact(&db, &owner_id, &contact_id);

        let derived = derive_receiving_addresses_for_contact(
            &seed,
            Network::Testnet,
            &owner_id,
            &contact_id,
            0,
            1,
        )
        .expect("derive one address");
        let payment_address = derived[0].address.clone();
        let wtx = make_wtx(
            0x51,
            vec![TxOut {
                value: 42_000,
                script_pubkey: payment_address.script_pubkey(),
            }],
            42_000,
            1_700_000_123,
        );

        let result = scan_wallet_transactions_for_dashpay_payments_impl(
            &db,
            Network::Testnet,
            &owner_id,
            std::slice::from_ref(&wtx),
            DashPayWalletScanMode::Delta {
                active_seed_bytes: Some(&seed),
                active_wallet: Some(&wallet),
            },
        )
        .expect("delta scan");

        assert_eq!(result.saved_payments, 1);
        assert!(result.mappings_available);
        assert_eq!(
            db.get_all_dashpay_address_mappings(&owner_id)
                .expect("load mappings")
                .len(),
            DASHPAY_GAP_LIMIT as usize
        );

        let guard = wallet.read().expect("wallet read");
        assert!(guard.known_addresses.contains_key(&payment_address));
        assert!(
            guard
                .watched_addresses
                .values()
                .any(|info| info.address == payment_address)
        );
        drop(guard);

        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments");
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].tx_id, wtx.txid.to_string());
    }

    #[test]
    fn delta_scan_with_complete_db_mappings_re_registers_wallet_addresses() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let original_wallet = wallet_from_seed(0x42);
        let original_seed = seed_from_wallet(&original_wallet);

        save_contact(&db, &owner_id, &contact_id);
        ensure_dashpay_address_mappings_for_scan(
            &db,
            Network::Testnet,
            &owner_id,
            &compute_seed_hash(&original_seed),
            &original_seed,
            Some(&original_wallet),
        )
        .expect("seed mappings");

        assert!(
            dashpay_address_mappings_cover_targets(
                &db,
                Network::Testnet,
                &owner_id,
                &compute_seed_hash(&original_seed),
            )
            .expect("coverage check")
        );

        let mapping_count = db
            .get_all_dashpay_address_mappings(&owner_id)
            .expect("load mappings")
            .len();
        let fresh_wallet = wallet_from_seed(0x42);
        let fresh_seed = seed_from_wallet(&fresh_wallet);
        let fresh_wallet_known_before = fresh_wallet
            .read()
            .expect("wallet read")
            .known_addresses
            .len();

        let payment_address = derive_receiving_addresses_for_contact(
            &original_seed,
            Network::Testnet,
            &owner_id,
            &contact_id,
            0,
            1,
        )
        .expect("derive known payment address")[0]
            .address
            .clone();
        let wtx = make_wtx(
            0x52,
            vec![TxOut {
                value: 84_000,
                script_pubkey: payment_address.script_pubkey(),
            }],
            84_000,
            1_700_000_124,
        );

        let result = scan_wallet_transactions_for_dashpay_payments_impl(
            &db,
            Network::Testnet,
            &owner_id,
            std::slice::from_ref(&wtx),
            DashPayWalletScanMode::Delta {
                active_seed_bytes: Some(&fresh_seed),
                active_wallet: Some(&fresh_wallet),
            },
        )
        .expect("delta scan with existing mappings");

        assert_eq!(result.saved_payments, 1);
        assert!(result.mappings_available);
        assert_eq!(
            db.get_all_dashpay_address_mappings(&owner_id)
                .expect("reload mappings")
                .len(),
            mapping_count
        );

        let guard = fresh_wallet.read().expect("wallet read");
        assert!(guard.known_addresses.len() > fresh_wallet_known_before);
        assert!(guard.known_addresses.contains_key(&payment_address));
        assert!(
            guard
                .watched_addresses
                .values()
                .any(|info| info.address == payment_address)
        );
    }

    #[test]
    fn delta_reregistration_only_imports_addresses_for_active_wallet_seed() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let wallet_a = wallet_from_seed(0x43);
        let wallet_b = wallet_from_seed(0x44);
        let seed_a = seed_from_wallet(&wallet_a);
        let seed_b = seed_from_wallet(&wallet_b);

        save_contact(&db, &owner_id, &contact_id);

        let mappings_a = derive_receiving_addresses_for_contact(
            &seed_a,
            Network::Testnet,
            &owner_id,
            &contact_id,
            0,
            DASHPAY_GAP_LIMIT,
        )
        .expect("derive wallet A mappings");
        let mappings_b = derive_receiving_addresses_for_contact(
            &seed_b,
            Network::Testnet,
            &owner_id,
            &contact_id,
            0,
            DASHPAY_GAP_LIMIT,
        )
        .expect("derive wallet B mappings");
        save_mappings_for_seed(&db, &owner_id, &seed_a, &mappings_a);
        save_mappings_for_seed(&db, &owner_id, &seed_b, &mappings_b);

        let fresh_wallet_a = wallet_from_seed(0x43);
        let result = scan_wallet_transactions_for_dashpay_payments_impl(
            &db,
            Network::Testnet,
            &owner_id,
            &[],
            DashPayWalletScanMode::Delta {
                active_seed_bytes: Some(&seed_a),
                active_wallet: Some(&fresh_wallet_a),
            },
        )
        .expect("delta scan");

        assert_eq!(result.saved_payments, 0);
        assert!(result.mappings_available);

        let guard = fresh_wallet_a.read().expect("wallet read");
        assert!(
            mappings_a
                .iter()
                .all(|mapping| guard.known_addresses.contains_key(&mapping.address))
        );
        assert!(
            mappings_b
                .iter()
                .all(|mapping| !guard.known_addresses.contains_key(&mapping.address))
        );
    }

    #[test]
    fn delta_scan_without_seed_bytes_re_registers_existing_scoped_mappings() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let original_wallet = wallet_from_seed(0x45);
        let seed = seed_from_wallet(&original_wallet);
        let seed_hash = compute_seed_hash(&seed);

        save_contact(&db, &owner_id, &contact_id);
        ensure_dashpay_address_mappings_for_scan(
            &db,
            Network::Testnet,
            &owner_id,
            &seed_hash,
            &seed,
            Some(&original_wallet),
        )
        .expect("seed mappings");

        let fresh_wallet = wallet_from_seed(0x45);
        let known_before = fresh_wallet
            .read()
            .expect("wallet read")
            .known_addresses
            .len();

        let result = scan_wallet_transactions_for_dashpay_payments_impl(
            &db,
            Network::Testnet,
            &owner_id,
            &[],
            DashPayWalletScanMode::Delta {
                active_seed_bytes: None,
                active_wallet: Some(&fresh_wallet),
            },
        )
        .expect("delta scan");

        assert_eq!(result.saved_payments, 0);
        assert!(result.mappings_available);
        assert!(
            fresh_wallet
                .read()
                .expect("wallet read")
                .known_addresses
                .len()
                > known_before
        );
    }

    #[test]
    fn coverage_check_is_scoped_to_the_active_wallet_seed() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let seed_a = [0x46; 64];
        let seed_b = [0x47; 64];

        save_contact(&db, &owner_id, &contact_id);

        let mappings_b = derive_receiving_addresses_for_contact(
            &seed_b,
            Network::Testnet,
            &owner_id,
            &contact_id,
            0,
            DASHPAY_GAP_LIMIT,
        )
        .expect("derive wallet B mappings");
        save_mappings_for_seed(&db, &owner_id, &seed_b, &mappings_b);

        assert!(
            !dashpay_address_mappings_cover_targets(
                &db,
                Network::Testnet,
                &owner_id,
                &compute_seed_hash(&seed_a),
            )
            .expect("wallet A coverage")
        );
        assert!(
            dashpay_address_mappings_cover_targets(
                &db,
                Network::Testnet,
                &owner_id,
                &compute_seed_hash(&seed_b),
            )
            .expect("wallet B coverage")
        );
    }

    #[test]
    fn scan_helper_skips_outgoing_transactions() {
        let addr = address_from_seed(1);
        let contact_id = Identifier::random();
        let mut address_map = HashMap::new();
        address_map.insert(addr.to_string(), (contact_id, 5u32));

        let output = TxOut {
            value: 50_000,
            script_pubkey: addr.script_pubkey(),
        };
        // net_amount < 0 → outgoing
        let wtx = make_wtx(0x11, vec![output], -10_000, 1_700_000_000);

        let result = match_dashpay_payment_in_transaction(&wtx, &address_map, Network::Testnet);
        assert!(result.is_empty(), "outgoing transactions must be skipped");
    }

    #[test]
    fn scan_helper_records_all_matching_outputs_per_tx() {
        let addr1 = address_from_seed(3);
        let addr2 = address_from_seed(4);
        let contact_a = Identifier::random();
        let contact_b = Identifier::random();

        let mut address_map = HashMap::new();
        address_map.insert(addr1.to_string(), (contact_a, 7u32));
        address_map.insert(addr2.to_string(), (contact_b, 9u32));

        let outputs = vec![
            TxOut {
                value: 11_111,
                script_pubkey: addr1.script_pubkey(),
            },
            TxOut {
                value: 22_222,
                script_pubkey: addr2.script_pubkey(),
            },
        ];
        let wtx = make_wtx(0x33, outputs, 33_333, 1_700_000_100);

        let matches = match_dashpay_payment_in_transaction(&wtx, &address_map, Network::Testnet);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].contact_id, contact_a);
        assert_eq!(matches[0].output_index, 0);
        assert_eq!(matches[0].address_index, 7);
        assert_eq!(matches[0].amount_duffs, 11_111);
        assert_eq!(matches[0].timestamp, 1_700_000_100);
        assert_eq!(matches[1].contact_id, contact_b);
        assert_eq!(matches[1].output_index, 1);
        assert_eq!(matches[1].address_index, 9);
        assert_eq!(matches[1].amount_duffs, 22_222);
    }

    #[test]
    fn scan_helper_records_same_contact_multiple_outputs_per_tx() {
        let addr1 = address_from_seed(5);
        let addr2 = address_from_seed(6);
        let contact_id = Identifier::random();

        let mut address_map = HashMap::new();
        address_map.insert(addr1.to_string(), (contact_id, 3u32));
        address_map.insert(addr2.to_string(), (contact_id, 4u32));

        let outputs = vec![
            TxOut {
                value: 40_000,
                script_pubkey: addr1.script_pubkey(),
            },
            TxOut {
                value: 60_000,
                script_pubkey: addr2.script_pubkey(),
            },
        ];
        let wtx = make_wtx(0x34, outputs, 100_000, 1_700_000_101);

        let matches = match_dashpay_payment_in_transaction(&wtx, &address_map, Network::Testnet);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches.iter().map(|m| m.amount_duffs).sum::<u64>(), 100_000);
    }

    #[test]
    fn record_incoming_payment_advances_highest_receive_index() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();

        // Record at index 3 → highest should become 4 (index + 1).
        record_incoming_payment(
            &db,
            "txid_a",
            Some(0),
            &owner_id,
            &contact_id,
            1_000,
            3,
            Some(1_700_000_000),
        )
        .expect("record 1");
        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices");
        assert_eq!(indices.highest_receive_index, 4);

        // Record at a HIGHER index 10 → should advance to 11.
        record_incoming_payment(
            &db,
            "txid_b",
            Some(1),
            &owner_id,
            &contact_id,
            2_000,
            10,
            Some(1_700_000_100),
        )
        .expect("record 2");
        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices");
        assert_eq!(indices.highest_receive_index, 11);

        // Record at a LOWER index 2 → must NOT regress (MAX semantics).
        record_incoming_payment(
            &db,
            "txid_c",
            Some(2),
            &owner_id,
            &contact_id,
            3_000,
            2,
            Some(1_700_000_200),
        )
        .expect("record 3");
        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices");
        assert_eq!(indices.highest_receive_index, 11);

        // Backfilled payments should store the provided timestamp, not the scan time.
        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments");
        let row_a = payments
            .iter()
            .find(|p| p.tx_id == "txid_a")
            .expect("txid_a saved");
        assert_eq!(row_a.created_at, 1_700_000_000);
    }

    #[test]
    fn record_incoming_payment_skips_duplicate_output_and_reports_false() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();

        let inserted = record_incoming_payment(
            &db,
            "txid_dup",
            Some(1),
            &owner_id,
            &contact_id,
            5_000,
            8,
            Some(1_700_000_000),
        )
        .expect("first insert");
        assert!(inserted);

        let inserted_again = record_incoming_payment(
            &db,
            "txid_dup",
            Some(1),
            &owner_id,
            &contact_id,
            5_000,
            8,
            Some(1_700_000_000),
        )
        .expect("duplicate insert");
        assert!(!inserted_again);

        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments");
        assert_eq!(payments.len(), 1);

        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices");
        assert_eq!(indices.highest_receive_index, 9);
    }

    #[test]
    fn record_incoming_payment_upgrades_legacy_row_and_reports_true() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();

        db.save_payment_with_output_index(
            "txid_legacy",
            None,
            &contact_id,
            &owner_id,
            5_000,
            Some("legacy memo"),
            "received",
            Some(1_700_000_000),
        )
        .expect("seed legacy row");

        let changed = record_incoming_payment(
            &db,
            "txid_legacy",
            Some(2),
            &owner_id,
            &contact_id,
            5_000,
            4,
            Some(1_700_000_100),
        )
        .expect("upgrade legacy row");
        assert!(changed);

        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments");
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].output_index, 2);
        assert_eq!(payments[0].memo.as_deref(), Some("legacy memo"));
        assert_eq!(payments[0].created_at, 1_700_000_000);

        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices");
        assert_eq!(indices.highest_receive_index, 5);
    }

    #[test]
    fn record_incoming_payment_saves_multiple_outputs_from_same_transaction() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();

        assert!(
            record_incoming_payment(
                &db,
                "txid_multi",
                Some(0),
                &owner_id,
                &contact_id,
                40_000,
                2,
                Some(1_700_000_010),
            )
            .expect("first output")
        );
        assert!(
            record_incoming_payment(
                &db,
                "txid_multi",
                Some(1),
                &owner_id,
                &contact_id,
                60_000,
                3,
                Some(1_700_000_010),
            )
            .expect("second output")
        );

        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments");
        assert_eq!(payments.len(), 2);
        assert_eq!(
            payments.iter().map(|payment| payment.amount).sum::<i64>(),
            100_000
        );
    }

    #[test]
    fn record_incoming_payment_uses_db_default_timestamp_when_backfill_timestamp_is_zero() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();

        record_incoming_payment(
            &db,
            "txid_zero_ts",
            Some(0),
            &owner_id,
            &contact_id,
            7_500,
            1,
            None,
        )
        .expect("insert");

        let payment = db
            .load_payment_history(&owner_id, 1)
            .expect("load payments")
            .into_iter()
            .next()
            .expect("payment row");
        assert_ne!(payment.created_at, 0);
    }

    #[test]
    fn full_scan_mappings_are_derived_per_active_wallet_seed() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let contacts = vec![(contact_id, DASHPAY_GAP_LIMIT)];
        let seed_a = [7u8; 64];
        let seed_b = [8u8; 64];

        let mappings_a = derive_dashpay_address_mappings_for_seed(
            &owner_id,
            &seed_a,
            Network::Testnet,
            &contacts,
        )
        .expect("derive mappings for wallet A");
        let mappings_b = derive_dashpay_address_mappings_for_seed(
            &owner_id,
            &seed_b,
            Network::Testnet,
            &contacts,
        )
        .expect("derive mappings for wallet B");

        assert_eq!(mappings_a.len(), DASHPAY_GAP_LIMIT as usize);
        assert_eq!(mappings_b.len(), DASHPAY_GAP_LIMIT as usize);
        assert!(
            mappings_a
                .iter()
                .zip(mappings_b.iter())
                .all(|(a, b)| a.address != b.address),
            "distinct wallet seeds must contribute distinct DashPay receive mappings"
        );

        save_mappings_for_seed(&db, &owner_id, &seed_a, &mappings_a);
        save_mappings_for_seed(&db, &owner_id, &seed_b, &mappings_b);

        let stored = db
            .get_all_dashpay_address_mappings(&owner_id)
            .expect("load mappings");
        assert_eq!(stored.len(), (DASHPAY_GAP_LIMIT as usize) * 2);
    }
}
