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

                    let bloom_registered = indices_map
                        .get(&contact.contact_identity_id)
                        .map(|idx| idx.bloom_registered_count)
                        .unwrap_or(0);

                    // Derive 0..GAP_LIMIT, skipping what is already registered.
                    // platform-wallet's receiving account extends its own pool
                    // as payments use addresses.
                    let target_count = DASHPAY_GAP_LIMIT;
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
    // Every index below is a valid BIP32 child index (< 2^31): the hardened
    // constants (9, 15, 0) and the small per-network coin type are in range,
    // and `hash_identifier_to_u32` masks its output with `& 0x7FFFFFFF`, so the
    // identity indices are non-hardened by construction. `address_index` is a
    // non-hardened receiving index, also below 2^31 by invariant.
    let idx = "invariant: BIP32 child index is below 2^31";
    let path = DerivationPath::from(vec![
        ChildNumber::from_hardened_idx(9).expect(idx), // Feature purpose
        ChildNumber::from_hardened_idx(coin_type).expect(idx), // Coin type (per network)
        ChildNumber::from_hardened_idx(15).expect(idx), // DashPay feature
        ChildNumber::from_hardened_idx(0).expect(idx), // Account
        // For the identity indices, we use a hash to fit in u32
        ChildNumber::from_normal_idx(hash_identifier_to_u32(owner_id)).expect(idx),
        ChildNumber::from_normal_idx(hash_identifier_to_u32(contact_id)).expect(idx),
        ChildNumber::from_normal_idx(address_index).expect(idx),
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
