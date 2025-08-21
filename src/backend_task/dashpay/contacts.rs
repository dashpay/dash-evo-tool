use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
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

// Helper function to derive encryption keys for contactInfo
fn derive_contact_info_keys(
    identity: &QualifiedIdentity,
    derivation_index: u32,
) -> Result<([u8; 32], [u8; 32]), String> {
    // Get a key from the identity to use as root
    let root_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or("No suitable key found for encryption")?;

    // Get the private key for this public key
    let wallets: Vec<_> = identity.associated_wallets.values().cloned().collect();
    let private_key = identity
        .private_keys
        .get_resolve(
            &(
                crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity,
                root_key.id(),
            ),
            &wallets,
            identity.network,
        )
        .map_err(|e| format!("Error resolving private key: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or("Private key not found")?;

    // Derive two keys using HMAC-SHA256
    // Key 1 for encToUserId (offset 2^16)
    let mut hasher = Sha256::new();
    hasher.update(&private_key);
    hasher.update(&(65536u32 + derivation_index).to_le_bytes());
    let key1 = hasher.finalize();

    // Key 2 for privateData (offset 2^16 + 1)
    let mut hasher2 = Sha256::new();
    hasher2.update(&private_key);
    hasher2.update(&(65537u32 + derivation_index).to_le_bytes());
    let key2 = hasher2.finalize();

    Ok((key1.into(), key2.into()))
}

// Helper function to decrypt toUserId using AES-256-ECB
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aes::Aes256;
    use aes_gcm::aes::cipher::generic_array::GenericArray;
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
                if let Some(Value::Bytes(enc_user_id)) = props.get("encToUserId") {
                    if let Ok(decrypted_id) = decrypt_to_user_id(enc_user_id, &enc_user_id_key) {
                        let contact_id = Identifier::from_bytes(&decrypted_id).unwrap();

                        // Decrypt private data if available
                        let mut nickname = None;
                        let mut note = None;
                        let mut is_hidden = false;
                        let mut account_reference = 0u32;

                        if let Some(Value::Bytes(encrypted_private)) = props.get("privateData") {
                            if let Ok(decrypted_data) =
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
                                        if pos + alias_len <= decrypted_data.len() && alias_len > 0
                                        {
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
                        }

                        contact_info_map.insert(
                            contact_id,
                            ContactData {
                                identity_id: contact_id,
                                nickname,
                                note,
                                is_hidden,
                                account_reference,
                            },
                        );
                    }
                }
            }
        }
    }

    // Build enriched contact list
    let contact_list: Vec<ContactData> = contacts
        .into_iter()
        .map(|contact_id| {
            contact_info_map
                .get(&contact_id)
                .cloned()
                .unwrap_or_else(|| ContactData {
                    identity_id: contact_id,
                    nickname: None,
                    note: None,
                    is_hidden: false,
                    account_reference: 0,
                })
        })
        .collect();

    Ok(BackendTaskSuccessResult::DashPayContactsWithInfo(
        contact_list,
    ))
}

pub async fn add_contact(
    _app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_username: String,
    account_label: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
    // TODO: Steps to implement:
    // 1. Resolve username to identity ID via DPNS
    // 2. Generate encryption keys for this contact relationship
    // 3. Create the contactRequest document with encrypted fields
    // 4. Broadcast the state transition

    // For now, return a placeholder message
    Ok(BackendTaskSuccessResult::Message(format!(
        "Contact request to {} from {} (label: {:?}) - Not yet implemented",
        contact_username,
        identity.identity.id().to_string(Encoding::Base58),
        account_label
    )))
}

pub async fn remove_contact(
    _app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_id: Identifier,
) -> Result<BackendTaskSuccessResult, String> {
    // TODO: Implement contact removal
    // This would involve deleting the contactInfo document if it exists

    Ok(BackendTaskSuccessResult::Message(format!(
        "Removed contact {} for identity {} - Not yet implemented",
        contact_id,
        identity.identity.id().to_string(Encoding::Base58)
    )))
}
