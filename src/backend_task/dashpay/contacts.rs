use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath, ExtendedPrivKey};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use futures::future::join_all;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

// DashPay contract ID from the platform repo
pub const DASHPAY_CONTRACT_ID: [u8; 32] = [
    162, 161, 180, 172, 111, 239, 34, 234, 42, 26, 104, 232, 18, 54, 68, 179, 87, 135, 95, 107, 65,
    44, 24, 16, 146, 129, 193, 70, 231, 178, 113, 188,
];

pub async fn get_dashpay_contract(sdk: &Sdk) -> Result<Arc<DataContract>, String> {
    let contract_id = Identifier::from_bytes(&DASHPAY_CONTRACT_ID).map_err(|e| e.to_string())?;
    DataContract::fetch(sdk, contract_id)
        .await
        .map_err(|e| format!("Failed to fetch DashPay contract: {}", e))?
        .ok_or_else(|| "DashPay contract not found".to_string())
        .map(Arc::new)
}

/// Derive encryption keys for contactInfo using BIP32 CKDpriv as specified in DIP-0015.
///
/// DIP-0015 specifies:
/// - Key1 (for encToUserId): rootEncryptionKey/(2^16)'/index'
/// - Key2 (for privateData): rootEncryptionKey/(2^16 + 1)'/index'
///
/// We use the wallet's master seed to derive a root encryption key,
/// then apply BIP32 hardened derivation for the two encryption keys.
fn derive_contact_info_keys(
    identity: &QualifiedIdentity,
    derivation_index: u32,
) -> Result<([u8; 32], [u8; 32]), String> {
    // Get the wallet seed from the identity's associated wallet
    let wallet = identity
        .associated_wallets
        .values()
        .next()
        .ok_or("No wallet associated with identity for key derivation")?;

    let (seed, network) = {
        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
        if !wallet_guard.is_open() {
            return Err("Wallet must be unlocked to derive encryption keys".to_string());
        }
        let seed = wallet_guard
            .seed_bytes()
            .map_err(|e| format!("Wallet seed not available: {}", e))?
            .to_vec();
        (seed, identity.network)
    };

    // Create master extended private key from seed
    let master_xprv = ExtendedPrivKey::new_master(network, &seed)
        .map_err(|e| format!("Failed to create master key: {}", e))?;

    // Derive to the root encryption key path: m/9'/5'/15'/0'
    // This follows the DashPay derivation structure
    let root_path = DerivationPath::from_str("m/9'/5'/15'/0'")
        .map_err(|e| format!("Invalid derivation path: {}", e))?;

    let secp = dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new();
    let root_encryption_key = master_xprv
        .derive_priv(&secp, &root_path)
        .map_err(|e| format!("Failed to derive root encryption key: {}", e))?;

    // Derive Key1 for encToUserId: rootEncryptionKey/(2^16)'/index'
    // First derive at hardened index 2^16 (65536)
    let key1_level1 = root_encryption_key
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(65536)
                .map_err(|e| format!("Invalid hardened index: {}", e))?],
        )
        .map_err(|e| format!("Failed to derive key1 level1: {}", e))?;

    // Then derive at hardened derivation_index
    let key1_final = key1_level1
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(derivation_index)
                .map_err(|e| format!("Invalid hardened index: {}", e))?],
        )
        .map_err(|e| format!("Failed to derive key1 final: {}", e))?;

    // Derive Key2 for privateData: rootEncryptionKey/(2^16 + 1)'/index'
    // First derive at hardened index 2^16 + 1 (65537)
    let key2_level1 = root_encryption_key
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(65537)
                .map_err(|e| format!("Invalid hardened index: {}", e))?],
        )
        .map_err(|e| format!("Failed to derive key2 level1: {}", e))?;

    // Then derive at hardened derivation_index
    let key2_final = key2_level1
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(derivation_index)
                .map_err(|e| format!("Invalid hardened index: {}", e))?],
        )
        .map_err(|e| format!("Failed to derive key2 final: {}", e))?;

    // Extract the private key bytes (32 bytes) for encryption
    let key1_bytes: [u8; 32] = key1_final.private_key.secret_bytes();
    let key2_bytes: [u8; 32] = key2_final.private_key.secret_bytes();

    Ok((key1_bytes, key2_bytes))
}

/// Decrypt toUserId using AES-256-ECB as specified by DIP-0015.
///
/// DIP-0015 mandates ECB mode for encToUserId encryption because:
/// 1. The toUserId is derived from SHA256, making it appear random (no patterns)
/// 2. Keys are never reused (unique per contact via hardened BIP32 derivation)
/// 3. The data is fixed-size (32 bytes = exactly 2 AES blocks)
///
/// These properties eliminate typical ECB vulnerabilities (pattern leakage).
/// See: https://github.com/dashpay/dips/blob/master/dip-0015.md
#[allow(deprecated)]
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aead::generic_array::GenericArray;
    use aes_gcm::aes::Aes256;
    use aes_gcm::aes::cipher::{BlockDecrypt, KeyInit};

    if encrypted.len() != 32 {
        return Err("Invalid encrypted user ID length".to_string());
    }

    let cipher = Aes256::new(GenericArray::from_slice(key));

    // Split the 32-byte encrypted data into two 16-byte blocks for ECB mode
    let mut decrypted = [0u8; 32];

    let mut block1 = GenericArray::clone_from_slice(&encrypted[0..16]);
    let mut block2 = GenericArray::clone_from_slice(&encrypted[16..32]);

    cipher.decrypt_block(&mut block1);
    cipher.decrypt_block(&mut block2);

    decrypted[0..16].copy_from_slice(&block1);
    decrypted[16..32].copy_from_slice(&block2);

    Ok(decrypted)
}

// Helper function to decrypt private data using AES-256-CBC
fn decrypt_private_data(encrypted_data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use cbc::cipher::BlockDecryptMut;
    use cbc::cipher::KeyIvInit;
    use cbc::cipher::block_padding::Pkcs7;
    type Aes256CbcDec = cbc::Decryptor<aes_gcm::aes::Aes256>;

    if encrypted_data.len() < 16 {
        return Err("Encrypted data too short (no IV)".to_string());
    }

    // Extract IV and ciphertext
    let iv = &encrypted_data[0..16];
    let ciphertext = &encrypted_data[16..];

    // Decrypt
    let cipher = Aes256CbcDec::new(key.into(), iv.into());

    let mut buffer = ciphertext.to_vec();
    let decrypted = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|e| format!("Decryption failed: {:?}", e))?;

    Ok(decrypted.to_vec())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactData {
    pub identity_id: Identifier,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub is_hidden: bool,
    pub account_reference: u32,
    // Profile data (fetched from Platform)
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
}

pub async fn load_contacts(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, String> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for contact requests where we are the sender (ownerId)
    let mut outgoing_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    outgoing_query = outgoing_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    outgoing_query.limit = 100;

    // Query for contact requests where we are the recipient (toUserId)
    let mut incoming_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    incoming_query = incoming_query.with_where(WhereClause {
        field: "toUserId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });

    // Add orderBy workaround for Platform bug
    incoming_query = incoming_query.with_order_by(OrderClause {
        field: "$createdAt".to_string(),
        ascending: true,
    });
    incoming_query.limit = 100;

    // Fetch both incoming and outgoing contact requests
    let outgoing_docs = Document::fetch_many(sdk, outgoing_query)
        .await
        .map_err(|e| format!("Error fetching outgoing contacts: {}", e))?;

    let incoming_docs = Document::fetch_many(sdk, incoming_query)
        .await
        .map_err(|e| format!("Error fetching incoming contacts: {}", e))?;

    // Convert to vectors for easier processing
    let outgoing: Vec<(Identifier, Document)> = outgoing_docs
        .into_iter()
        .filter_map(|(id, doc)| doc.map(|d| (id, d)))
        .collect();

    let incoming: Vec<(Identifier, Document)> = incoming_docs
        .into_iter()
        .filter_map(|(id, doc)| doc.map(|d| (id, d)))
        .collect();

    // Find mutual contacts (where both parties have sent requests to each other)
    let mut contacts = HashSet::new();

    for (_, incoming_doc) in incoming.iter() {
        let from_id = incoming_doc.owner_id();

        // Check if we also sent a request to this person
        for (_, outgoing_doc) in outgoing.iter() {
            if let Some(Value::Identifier(to_id_bytes)) = outgoing_doc.properties().get("toUserId")
            {
                let to_id = Identifier::from_bytes(to_id_bytes.as_slice()).unwrap();
                if to_id == from_id {
                    // Mutual contact found
                    contacts.insert(from_id);
                }
            }
        }
    }

    // Now query for contact info documents
    let mut contact_info_query = DocumentQuery::new(dashpay_contract.clone(), "contactInfo")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    contact_info_query = contact_info_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    contact_info_query.limit = 100;

    let contact_info_docs = Document::fetch_many(sdk, contact_info_query)
        .await
        .map_err(|e| format!("Error fetching contact info: {}", e))?;

    // Build a map of contact ID to contact info
    let mut contact_info_map: HashMap<Identifier, ContactData> = HashMap::new();

    for (_doc_id, doc) in contact_info_docs.iter() {
        if let Some(doc) = doc {
            let props = doc.properties();

            // Get the derivation index used for this document
            if let Some(Value::U32(deriv_idx)) = props.get("derivationEncryptionKeyIndex") {
                // Derive keys for this document
                let (enc_user_id_key, private_data_key) =
                    match derive_contact_info_keys(&identity, *deriv_idx) {
                        Ok(keys) => keys,
                        Err(_) => continue,
                    };

                // Decrypt encToUserId to find which contact this is for
                if let Some(Value::Bytes(enc_user_id)) = props.get("encToUserId")
                    && let Ok(decrypted_id) = decrypt_to_user_id(enc_user_id, &enc_user_id_key)
                {
                    let contact_id = Identifier::from_bytes(&decrypted_id).unwrap();

                    // Decrypt private data if available
                    let mut nickname = None;
                    let mut note = None;
                    let mut is_hidden = false;
                    let mut account_reference = 0u32;

                    if let Some(Value::Bytes(encrypted_private)) = props.get("privateData")
                        && let Ok(decrypted_data) =
                            decrypt_private_data(encrypted_private, &private_data_key)
                    {
                        // Parse the decrypted data
                        // Simple format: version(4) + alias_len(1) + alias + note_len(1) + note + hidden(1) + accounts_len(1) + accounts
                        if decrypted_data.len() >= 8 {
                            let mut pos = 4; // Skip version

                            // Read alias
                            if pos < decrypted_data.len() {
                                let alias_len = decrypted_data[pos] as usize;
                                pos += 1;
                                if pos + alias_len <= decrypted_data.len() && alias_len > 0 {
                                    nickname = String::from_utf8(
                                        decrypted_data[pos..pos + alias_len].to_vec(),
                                    )
                                    .ok();
                                    pos += alias_len;
                                }
                            }

                            // Read note
                            if pos < decrypted_data.len() {
                                let note_len = decrypted_data[pos] as usize;
                                pos += 1;
                                if pos + note_len <= decrypted_data.len() && note_len > 0 {
                                    note = String::from_utf8(
                                        decrypted_data[pos..pos + note_len].to_vec(),
                                    )
                                    .ok();
                                    pos += note_len;
                                }
                            }

                            // Read hidden flag
                            if pos < decrypted_data.len() {
                                is_hidden = decrypted_data[pos] != 0;
                                pos += 1;
                            }

                            // Read accounts (simplified - just take first if available)
                            if pos < decrypted_data.len() {
                                let accounts_len = decrypted_data[pos] as usize;
                                pos += 1;
                                if accounts_len > 0 && pos + 4 <= decrypted_data.len() {
                                    account_reference = u32::from_le_bytes([
                                        decrypted_data[pos],
                                        decrypted_data[pos + 1],
                                        decrypted_data[pos + 2],
                                        decrypted_data[pos + 3],
                                    ]);
                                }
                            }
                        }
                    }

                    contact_info_map.insert(
                        contact_id,
                        ContactData {
                            identity_id: contact_id,
                            nickname,
                            note,
                            is_hidden,
                            account_reference,
                            username: None,
                            display_name: None,
                            avatar_url: None,
                            bio: None,
                        },
                    );
                }
            }
        }
    }

    // Build enriched contact list with basic data
    let mut contact_list: Vec<ContactData> = contacts
        .into_iter()
        .map(|contact_id| {
            contact_info_map
                .get(&contact_id)
                .cloned()
                .unwrap_or(ContactData {
                    identity_id: contact_id,
                    nickname: None,
                    note: None,
                    is_hidden: false,
                    account_reference: 0,
                    username: None,
                    display_name: None,
                    avatar_url: None,
                    bio: None,
                })
        })
        .collect();

    // Fetch profiles and usernames for all contacts in parallel with bounded concurrency.
    // Each contact requires two network queries (profile + DPNS username), so parallelizing
    // in chunks significantly reduces total load time for large contact lists.
    const CHUNK_SIZE: usize = 10;

    for chunk in contact_list.chunks_mut(CHUNK_SIZE) {
        let futures: Vec<_> = chunk
            .iter()
            .map(|contact| {
                let dashpay_contract = dashpay_contract.clone();
                let dpns_contract = app_context.dpns_contract.clone();
                let contact_id = contact.identity_id;

                async move {
                    let mut display_name = None;
                    let mut avatar_url = None;
                    let mut bio = None;
                    let mut username = None;

                    // Fetch profile
                    if let Ok(mut profile_query) = DocumentQuery::new(dashpay_contract, "profile") {
                        profile_query = profile_query.with_where(WhereClause {
                            field: "$ownerId".to_string(),
                            operator: WhereOperator::Equal,
                            value: Value::Identifier(contact_id.to_buffer()),
                        });
                        profile_query.limit = 1;

                        if let Ok(results) = Document::fetch_many(sdk, profile_query).await
                            && let Some((_, Some(doc))) = results.into_iter().next()
                        {
                            let props = doc.properties();
                            display_name = props
                                .get("displayName")
                                .and_then(|v| v.as_text())
                                .map(|s| s.to_string());
                            avatar_url = props
                                .get("avatarUrl")
                                .and_then(|v| v.as_text())
                                .map(|s| s.to_string());
                            bio = props
                                .get("bio")
                                .and_then(|v| v.as_text())
                                .map(|s| s.to_string());
                        }
                    }

                    // Fetch DPNS username
                    if let Ok(mut dpns_query) = DocumentQuery::new(dpns_contract, "domain") {
                        dpns_query = dpns_query.with_where(WhereClause {
                            field: "records.identity".to_string(),
                            operator: WhereOperator::Equal,
                            value: Value::Identifier(contact_id.to_buffer()),
                        });
                        dpns_query.limit = 1;

                        if let Ok(results) = Document::fetch_many(sdk, dpns_query).await
                            && let Some((_, Some(doc))) = results.into_iter().next()
                        {
                            let props = doc.properties();
                            if let Some(label) = props.get("label").and_then(|v| v.as_text()) {
                                username = Some(label.to_string());
                            }
                        }
                    }

                    (contact_id, display_name, avatar_url, bio, username)
                }
            })
            .collect();

        let results = join_all(futures).await;

        // Apply fetched data back to the contacts in this chunk
        for (contact_id, display_name, avatar_url, bio, username) in results {
            if let Some(contact) = chunk.iter_mut().find(|c| c.identity_id == contact_id) {
                contact.display_name = display_name;
                contact.avatar_url = avatar_url;
                contact.bio = bio;
                contact.username = username;
            }
        }
    }

    Ok(BackendTaskSuccessResult::DashPayContactsWithInfo(
        contact_list,
    ))
}

pub async fn add_contact(
    _app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    _identity: QualifiedIdentity,
    _contact_username: String,
    _account_label: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
    // TODO: Steps to implement:
    // 1. Resolve username to identity ID via DPNS
    // 2. Generate encryption keys for this contact relationship
    // 3. Create the contactRequest document with encrypted fields
    // 4. Broadcast the state transition
    Err("Adding contacts via username is not yet implemented. Use the contact request workflow instead.".to_string())
}

pub async fn remove_contact(
    _app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    _identity: QualifiedIdentity,
    _contact_id: Identifier,
) -> Result<BackendTaskSuccessResult, String> {
    // TODO: Implement contact removal
    // This would involve deleting the contactInfo document if it exists
    Err("Contact removal is not yet implemented".to_string())
}
