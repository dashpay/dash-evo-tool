use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::ContactAddressIndex;
use crate::model::dashpay_derivation::{derive_dashpay_incoming_xpub, derive_payment_address};
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
pub fn derive_receiving_addresses_for_contact(
    master_seed: &[u8],
    network: Network,
    our_identity_id: &Identifier,
    contact_id: &Identifier,
    start_index: u32,
    count: u32,
) -> Result<Vec<DashPayReceivingAddress>, String> {
    // For receiving payments, we derive from OUR xpub
    // Path: m/9'/coin'/15'/0'/(our_id)/(contact_id)
    // This is the key we sent to the contact in our contact request
    let xpub = derive_dashpay_incoming_xpub(
        master_seed,
        network,
        0, // account 0
        our_identity_id,
        contact_id,
    )
    .map_err(|e| e.to_string())?;

    let mut addresses = Vec::with_capacity(count as usize);
    for i in start_index..(start_index + count) {
        let address = derive_payment_address(&xpub, i).map_err(|e| e.to_string())?;
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
) -> Result<DashPayAddressRegistrationResult, TaskError> {
    let mut result = DashPayAddressRegistrationResult::default();
    let our_identity_id = identity.identity.id();

    // Select the DashPay wallet (lowest associated seed hash). The receive
    // side must pick the SAME wallet the send side published the contact-xpub
    // from, or the contact pays into addresses we never scan — both sides
    // resolve through `QualifiedIdentity::dashpay_wallet` (SEC-W-001).
    let (seed_hash, wallet) = identity.dashpay_wallet().ok_or(TaskError::WalletNotFound)?;
    let wallet = wallet.clone();

    // Load all contacts for this identity from the WalletBackend DashPay
    // adapter — the upstream-backed source of truth. After D4c there is no
    // DB fallback: registration is meaningful only once the wallet is
    // wired (it needs the wallet's seed and known-address map anyway).
    let backend = app_context.wallet_backend()?;
    let contacts = backend.dashpay_view().contacts(&our_identity_id).await;

    if contacts.is_empty() {
        return Ok(result);
    }

    // Hydrate the per-contact address-index cache from the k/v sidecar so
    // we don't pay a kv read per contact below.
    let mut indices_map: BTreeMap<Vec<u8>, ContactAddressIndex> = BTreeMap::new();
    for contact in &contacts {
        let contact_id = match Identifier::from_bytes(&contact.contact_identity_id) {
            Ok(id) => id,
            Err(_) => continue,
        };
        match backend.dashpay_get_address_index(&our_identity_id, &contact_id) {
            Ok(Some(idx)) => {
                indices_map.insert(contact.contact_identity_id.clone(), idx);
            }
            Ok(None) => {}
            Err(e) => {
                result.errors.push(format!(
                    "Failed to load address index for contact {}: {}",
                    contact_id.to_string(Encoding::Base58),
                    e
                ));
            }
        }
    }

    let network = app_context.network;

    // Fetch the HD seed just-in-time through the chokepoint and derive every
    // contact's receiving addresses inside the one session scope — the seed
    // is borrowed for this single registration run and zeroizes when the
    // closure returns; it never enters this layer by value.
    let derived = app_context
        .wallet_backend()?
        .secret_access()
        .with_secret_session(
            &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
            async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;

                let mut derived: Vec<(Identifier, u32, Vec<DashPayReceivingAddress>)> = Vec::new();
                for contact in &contacts {
                    let contact_id = match Identifier::from_bytes(&contact.contact_identity_id) {
                        Ok(id) => id,
                        Err(e) => {
                            result.errors.push(format!("Invalid contact ID: {}", e));
                            continue;
                        }
                    };

                    let highest_receive_index = indices_map
                        .get(&contact.contact_identity_id)
                        .map(|idx| idx.highest_receive_index)
                        .unwrap_or(0);
                    let bloom_registered = indices_map
                        .get(&contact.contact_identity_id)
                        .map(|idx| idx.bloom_registered_count)
                        .unwrap_or(0);

                    // Derive 0..(highest_receive_index + GAP_LIMIT), skipping
                    // what the bloom filter already covers.
                    let target_count = highest_receive_index.saturating_add(DASHPAY_GAP_LIMIT);
                    if target_count <= bloom_registered {
                        result.contacts_processed += 1;
                        continue;
                    }

                    let start_index = bloom_registered;
                    let count = target_count - bloom_registered;

                    match derive_receiving_addresses_for_contact(
                        seed,
                        network,
                        &our_identity_id,
                        &contact_id,
                        start_index,
                        count,
                    ) {
                        Ok(addresses) => derived.push((contact_id, target_count, addresses)),
                        Err(e) => {
                            result.errors.push(format!(
                                "Failed to derive addresses for contact {}: {}",
                                contact_id.to_string(Encoding::Base58),
                                e
                            ));
                        }
                    }
                }
                Ok(derived)
            },
        )
        .await?;

    // Register the derived addresses with the wallet outside the secret scope
    // — registration touches no plaintext seed.
    for (contact_id, target_count, addresses) in derived {
        for addr_info in &addresses {
            if let Err(e) = register_dashpay_address(
                app_context,
                &wallet,
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

        // Update the bloom_registered_count in the sidecar (RMW the shared
        // `ContactAddressIndex` record so we don't clobber a higher receive
        // cursor written by a concurrent payment).
        if let Err(e) =
            set_bloom_registered_count(&backend, &our_identity_id, &contact_id, target_count)
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

/// Helper: stamp `bloom_registered_count = count` onto the persisted
/// `ContactAddressIndex` for `(owner, contact)` without clobbering other
/// fields. Initialises a fresh record with the rest of the cursors at 0
/// when no entry exists yet.
fn set_bloom_registered_count(
    backend: &crate::wallet_backend::WalletBackend,
    owner: &Identifier,
    contact: &Identifier,
    count: u32,
) -> Result<(), TaskError> {
    let mut state = backend
        .dashpay_get_address_index(owner, contact)?
        .unwrap_or_else(|| ContactAddressIndex {
            owner_identity_id: owner.to_buffer().to_vec(),
            contact_identity_id: contact.to_buffer().to_vec(),
            next_send_index: 0,
            highest_receive_index: 0,
            bloom_registered_count: 0,
        });
    state.bloom_registered_count = count;
    backend.dashpay_set_address_index(owner, contact, &state)
}

/// Register a single DashPay address with the wallet
fn register_dashpay_address(
    app_context: &AppContext,
    wallet: &Arc<std::sync::RwLock<crate::model::wallet::Wallet>>,
    address: &Address,
    owner_id: &Identifier,
    contact_id: &Identifier,
    address_index: u32,
) -> Result<(), String> {
    use crate::model::wallet::{
        DerivationPathReference, DerivationPathType, coin_type_for_network,
    };
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};

    // Create a derivation path representation for DashPay addresses
    // m/9'/coin'/15'/0'/<owner_hash>/<contact_hash>/<index>
    // Note: We use a simplified representation since full 256-bit paths don't fit in standard BIP32
    let coin_type = coin_type_for_network(app_context.network);
    let path = DerivationPath::from(vec![
        ChildNumber::from_hardened_idx(9).unwrap(), // Feature purpose
        ChildNumber::from_hardened_idx(coin_type).unwrap(), // Coin type (per network)
        ChildNumber::from_hardened_idx(15).unwrap(), // DashPay feature
        ChildNumber::from_hardened_idx(0).unwrap(), // Account
        // For the identity indices, we use a hash to fit in u32
        ChildNumber::from_normal_idx(hash_identifier_to_u32(owner_id)).unwrap(),
        ChildNumber::from_normal_idx(hash_identifier_to_u32(contact_id)).unwrap(),
        ChildNumber::from_normal_idx(address_index).unwrap(),
    ]);

    // Store the DashPay address mapping in the k/v sidecar so the
    // incoming-payment detector can resolve `address → (contact, index)`.
    let backend = app_context
        .wallet_backend()
        .map_err(|e| format!("Wallet backend not yet available: {}", e))?;
    backend
        .dashpay_set_address_mapping(owner_id, &address.to_string(), contact_id, address_index)
        .map_err(|e| format!("Failed to save address mapping: {}", e))?;

    // Register with the wallet's known addresses
    let mut guard = wallet.write().map_err(|e| e.to_string())?;

    if guard.known_addresses.contains_key(address) {
        return Ok(()); // Already registered
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

/// Match a received transaction to a DashPay contact for `owner_id`.
///
/// Returns `(contact_id, address_index)` if the address is registered as
/// a DashPay receiving address for `owner_id`; `None` otherwise.
///
/// The k/v sidecar partitions the address map by owner, so the caller is
/// responsible for narrowing the search to the identity that observed the
/// transaction (typically the identity whose SPV bloom filter matched).
/// `address` is the Base58 receiving address — the same form the sidecar
/// is keyed by.
pub fn match_transaction_to_contact(
    app_context: &AppContext,
    owner_id: &Identifier,
    address: &str,
) -> Result<Option<(Identifier, u32)>, TaskError> {
    let backend = app_context.wallet_backend()?;
    backend.dashpay_get_address_mapping(owner_id, address)
}

/// Process a received output for one identity: if its address is a DashPay
/// contact-receiving address for `owner_id`, advance the receive cursor and
/// record the incoming payment through the upstream persist path.
///
/// `address` is the Base58 receiving address the output paid into. Returns
/// `Ok(None)` when the address is not a DashPay contact address for this
/// owner (the common case — most received outputs are plain wallet funds).
///
/// Idempotent: the receive cursor only ever advances, and the recording is
/// keyed by `(tx_id, vout)` with last-write-wins upstream, so a re-scan of the
/// same output neither double-credits nor double-counts. Keying by the output
/// index — not the bare `tx_id` — keeps a transaction that pays two different
/// contact addresses recording both, rather than the second clobbering the
/// first.
pub async fn process_incoming_payment(
    app_context: &Arc<AppContext>,
    owner_id: &Identifier,
    tx_id: &str,
    vout: u32,
    address: &str,
    amount_duffs: u64,
) -> Result<Option<IncomingPaymentInfo>, TaskError> {
    // Check if this address belongs to a DashPay contact relationship.
    let (contact_id, address_index) =
        match match_transaction_to_contact(app_context, owner_id, address)? {
            Some(m) => m,
            None => return Ok(None), // Not a DashPay address
        };

    // Bump the highest receive index if this address pushed past the cursor.
    let backend = app_context.wallet_backend()?;
    let mut state = backend
        .dashpay_get_address_index(owner_id, &contact_id)?
        .unwrap_or_else(|| ContactAddressIndex {
            owner_identity_id: owner_id.to_buffer().to_vec(),
            contact_identity_id: contact_id.to_buffer().to_vec(),
            next_send_index: 0,
            highest_receive_index: 0,
            bloom_registered_count: 0,
        });
    if address_index >= state.highest_receive_index {
        state.highest_receive_index = address_index + 1;
        backend.dashpay_set_address_index(owner_id, &contact_id, &state)?;
    }

    // Mirror the incoming payment through the WalletBackend adapter so the
    // upstream `ManagedIdentity` records it and the timestamp sidecar reflects
    // when DET observed it. Keyed per output so two contact outputs in one
    // transaction are both recorded.
    super::payments::mirror_incoming_payment_to_backend(
        app_context,
        owner_id,
        tx_id,
        vout,
        contact_id,
        amount_duffs,
    )
    .await;

    Ok(Some(IncomingPaymentInfo {
        tx_id: tx_id.to_string(),
        vout,
        from_contact_id: contact_id,
        to_identity_id: *owner_id,
        address: address.to_string(),
        amount_duffs,
        address_index,
    }))
}

/// Resolve a batch of received outputs against every local identity's DashPay
/// address map, recording the ones that pay a known contact. Returns the
/// number of payments recorded.
///
/// This is the detection driver wired to the [`EventBridge`]: it owns the
/// owner-scoped match the sync event callback cannot perform. The address map
/// is partitioned per owner, so each candidate output is tried against the
/// local identities until one claims it. A receiving address belongs to exactly
/// one owning identity, so the scan stops at the first match — trying every
/// identity afterwards would double-record an output if the same address ever
/// appeared under two owners' maps. The vast majority of outputs miss every
/// owner (a regular receiving address is not a contact address) and are skipped
/// via the `None` arm of [`process_incoming_payment`]. A per-output match error
/// is logged and the scan continues — one unreadable sidecar entry must not
/// drop the rest of a block's payments.
///
/// [`EventBridge`]: crate::wallet_backend::EventBridge
pub async fn detect_incoming_contact_payments(
    app_context: &Arc<AppContext>,
    outputs: &[crate::model::dashpay::DetectedIncomingOutput],
) -> Result<usize, TaskError> {
    if outputs.is_empty() {
        return Ok(0);
    }

    let identities = app_context.load_local_qualified_identities()?;
    if identities.is_empty() {
        return Ok(0);
    }

    let owner_ids: Vec<Identifier> = identities.iter().map(|i| i.identity.id()).collect();

    let mut recorded = 0usize;
    for output in outputs {
        for owner_id in &owner_ids {
            match process_incoming_payment(
                app_context,
                owner_id,
                &output.txid,
                output.vout,
                &output.address,
                output.amount_duffs,
            )
            .await
            {
                // An output's address belongs to one owner only — record it
                // and stop, so a cross-owner address collision can't
                // double-record the same output.
                Ok(Some(_info)) => {
                    recorded += 1;
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    // A single owner/output failure must not abort the batch;
                    // log and move on so other identities still get their
                    // matching payments recorded. No financial PII at this
                    // level — only the non-sensitive error detail.
                    tracing::debug!(
                        error = ?e,
                        "Incoming DashPay payment detection skipped one output"
                    );
                }
            }
        }
    }

    // Business event, no PII: counts only.
    tracing::debug!(
        candidate_outputs = outputs.len(),
        recorded,
        "Incoming DashPay contact-payment detection finished"
    );

    Ok(recorded)
}

/// Information about an incoming DashPay payment
#[derive(Debug, Clone)]
pub struct IncomingPaymentInfo {
    pub tx_id: String,
    /// Output index within the transaction this payment was recorded under.
    pub vout: u32,
    pub from_contact_id: Identifier,
    pub to_identity_id: Identifier,
    /// Base58 receiving address the payment landed on.
    pub address: String,
    pub amount_duffs: u64,
    pub address_index: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_identifier_to_u32() {
        let id = Identifier::random();
        let hash = hash_identifier_to_u32(&id);
        // Should be less than 2^31 (non-hardened range)
        assert!(hash < 0x80000000);
    }
}
