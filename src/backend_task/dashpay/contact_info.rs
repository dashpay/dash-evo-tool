use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use aes_gcm::aes::Aes256;
use aes_gcm::aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use bip39::rand::{SeedableRng, rngs::StdRng};
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{
    Document as DppDocument, DocumentV0, DocumentV0Getters, DocumentV0Setters,
};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::{Bytes32, Value};
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::documents::transitions::{
    DocumentCreateTransitionBuilder, DocumentReplaceTransitionBuilder,
};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

// ContactInfo private data structure
#[derive(Debug, Clone)]
pub struct ContactInfoPrivateData {
    pub version: u32,
    pub alias_name: Option<String>,
    pub note: Option<String>,
    pub display_hidden: bool,
    pub accepted_accounts: Vec<u32>,
}

impl ContactInfoPrivateData {
    pub fn new() -> Self {
        Self {
            version: 0,
            alias_name: None,
            note: None,
            display_hidden: false,
            accepted_accounts: Vec::new(),
        }
    }

    // Serialize to bytes for encryption
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Version (4 bytes)
        bytes.extend_from_slice(&self.version.to_le_bytes());

        // Alias name (length + string)
        if let Some(alias) = &self.alias_name {
            let alias_bytes = alias.as_bytes();
            bytes.push(alias_bytes.len() as u8);
            bytes.extend_from_slice(alias_bytes);
        } else {
            bytes.push(0u8);
        }

        // Note (length + string)
        if let Some(note) = &self.note {
            let note_bytes = note.as_bytes();
            bytes.push(note_bytes.len() as u8);
            bytes.extend_from_slice(note_bytes);
        } else {
            bytes.push(0u8);
        }

        // Display hidden (1 byte)
        bytes.push(if self.display_hidden { 1 } else { 0 });

        // Accepted accounts (length + array)
        bytes.push(self.accepted_accounts.len() as u8);
        for account in &self.accepted_accounts {
            bytes.extend_from_slice(&account.to_le_bytes());
        }

        bytes
    }
}

// Derive encryption keys for contactInfo
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

// Encrypt toUserId using AES-256-ECB
fn encrypt_to_user_id(user_id: &[u8; 32], key: &[u8; 32]) -> Result<[u8; 32], String> {
    let cipher = Aes256::new(GenericArray::from_slice(key));

    // Split the 32-byte ID into two 16-byte blocks for ECB mode
    let mut encrypted = [0u8; 32];

    let mut block1 = GenericArray::clone_from_slice(&user_id[0..16]);
    let mut block2 = GenericArray::clone_from_slice(&user_id[16..32]);

    cipher.encrypt_block(&mut block1);
    cipher.encrypt_block(&mut block2);

    encrypted[0..16].copy_from_slice(&block1);
    encrypted[16..32].copy_from_slice(&block2);

    Ok(encrypted)
}

// Decrypt toUserId using AES-256-ECB
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aes::cipher::BlockDecrypt;

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

// Encrypt private data using AES-256-CBC
fn encrypt_private_data(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use cbc::cipher::block_padding::Pkcs7;
    type Aes256CbcEnc = cbc::Encryptor<aes_gcm::aes::Aes256>;

    // Generate random IV (16 bytes)
    let mut rng = StdRng::from_entropy();
    let mut iv = [0u8; 16];
    use bip39::rand::RngCore;
    rng.fill_bytes(&mut iv);

    // Pad data to multiple of 16 bytes and encrypt
    let cipher = Aes256CbcEnc::new(key.into(), &iv.into());

    // Allocate buffer with padding
    let mut buffer = vec![0u8; data.len() + 16]; // Extra space for padding
    buffer[..data.len()].copy_from_slice(data);

    let encrypted = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, data.len())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    // Combine IV and encrypted data
    let mut result = Vec::with_capacity(16 + encrypted.len());
    result.extend_from_slice(&iv);
    result.extend_from_slice(encrypted);

    Ok(result)
}

// Decrypt private data using AES-256-CBC
fn decrypt_private_data(encrypted_data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use cbc::cipher::BlockDecryptMut;
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

pub async fn create_or_update_contact_info(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_user_id: Identifier,
    nickname: Option<String>,
    note: Option<String>,
    display_hidden: bool,
    accepted_accounts: Vec<u32>,
) -> Result<BackendTaskSuccessResult, String> {
    let dashpay_contract = app_context.dashpay_contract.clone();
    let identity_id = identity.identity.id();

    // Query for existing contactInfo document
    let mut query = DocumentQuery::new(dashpay_contract.clone(), "contactInfo")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    query = query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    query.limit = 100; // Get all contact info documents

    let existing_docs = Document::fetch_many(sdk, query)
        .await
        .map_err(|e| format!("Error fetching contact info: {}", e))?;

    // Check if we already have a contactInfo for this contact
    let mut found_existing_doc = None;
    let mut next_derivation_index = 0u32;

    // Try to find existing contactInfo for this contact
    for (_doc_id, doc) in existing_docs.iter() {
        if let Some(doc) = doc {
            let props = doc.properties();

            // Get the derivation index used for this document
            if let Some(Value::U32(deriv_idx)) = props.get("derivationEncryptionKeyIndex") {
                // Track the highest derivation index
                if *deriv_idx >= next_derivation_index {
                    next_derivation_index = deriv_idx + 1;
                }

                // Get the root key index to derive keys
                if let Some(Value::U32(_root_idx)) = props.get("rootEncryptionKeyIndex") {
                    // Derive keys for this document
                    let (enc_user_id_key, _) = derive_contact_info_keys(&identity, *deriv_idx)?;

                    // Decrypt encToUserId to check if it matches
                    if let Some(Value::Bytes(enc_user_id)) = props.get("encToUserId") {
                        match decrypt_to_user_id(enc_user_id, &enc_user_id_key) {
                            Ok(decrypted_id) if decrypted_id == contact_user_id.to_buffer() => {
                                // Found existing contactInfo for this contact
                                found_existing_doc = Some(doc.clone());
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Use the found derivation index or the next available one
    let derivation_index = if found_existing_doc.is_some() {
        // Use the same derivation index for updates
        found_existing_doc
            .as_ref()
            .and_then(|doc| doc.properties().get("derivationEncryptionKeyIndex"))
            .and_then(|v| {
                if let Value::U32(idx) = v {
                    Some(*idx)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    } else {
        next_derivation_index
    };

    // Derive encryption keys
    let (enc_user_id_key, private_data_key) =
        derive_contact_info_keys(&identity, derivation_index)?;

    // Encrypt toUserId
    let encrypted_user_id = encrypt_to_user_id(&contact_user_id.to_buffer(), &enc_user_id_key)?;

    // Create private data
    let mut private_data = ContactInfoPrivateData::new();
    private_data.alias_name = nickname;
    private_data.note = note;
    private_data.display_hidden = display_hidden;
    private_data.accepted_accounts = accepted_accounts;

    // Encrypt private data
    let encrypted_private_data =
        encrypt_private_data(&private_data.serialize(), &private_data_key)?;

    // Get signing key
    let signing_key = identity
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
        .ok_or("No suitable signing key found")?;

    // Create document properties
    let mut properties = BTreeMap::new();
    properties.insert(
        "encToUserId".to_string(),
        Value::Bytes(encrypted_user_id.to_vec()),
    );
    properties.insert(
        "rootEncryptionKeyIndex".to_string(),
        Value::U32(signing_key.id()),
    );
    properties.insert(
        "derivationEncryptionKeyIndex".to_string(),
        Value::U32(derivation_index),
    );
    properties.insert(
        "privateData".to_string(),
        Value::Bytes(encrypted_private_data),
    );

    if found_existing_doc.is_none() {
        // Create new contactInfo document
        let mut rng = StdRng::from_entropy();
        let entropy = Bytes32::random_with_rng(&mut rng);

        let document_id = Document::generate_document_id_v0(
            &dashpay_contract.id(),
            &identity_id,
            "contactInfo",
            entropy.as_slice(),
        );

        let document = DppDocument::V0(DocumentV0 {
            id: document_id,
            owner_id: identity_id,
            properties,
            revision: Some(1),
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        });

        let mut builder = DocumentCreateTransitionBuilder::new(
            dashpay_contract,
            "contactInfo".to_string(),
            document,
            entropy
                .as_slice()
                .try_into()
                .expect("entropy should be 32 bytes"),
        );

        // Add state transition options if available
        let maybe_options = app_context.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let _result = sdk
            .document_create(builder, &signing_key, &identity)
            .await
            .map_err(|e| format!("Error creating contact info: {}", e))?;
    } else {
        // Update existing document
        let existing_doc = found_existing_doc.unwrap();
        let mut updated_doc = existing_doc.clone();

        // Update properties
        for (key, value) in properties {
            updated_doc.set(&key, value);
        }

        // Bump revision
        updated_doc.bump_revision();

        // Create replacement transition
        use dash_sdk::platform::documents::transitions::DocumentReplaceTransitionBuilder;
        let mut builder = DocumentReplaceTransitionBuilder::new(
            dashpay_contract,
            "contactInfo".to_string(),
            updated_doc,
        );

        // Add state transition options if available
        let maybe_options = app_context.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let _result = sdk
            .document_replace(builder, &signing_key, &identity)
            .await
            .map_err(|e| format!("Error updating contact info: {}", e))?;
    }

    Ok(BackendTaskSuccessResult::Message(
        "Contact information updated successfully".to_string(),
    ))
}
