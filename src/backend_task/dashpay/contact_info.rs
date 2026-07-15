use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::{ContactInfoReadError, TaskError};
use crate::context::AppContext;
use crate::model::dashpay::{
    AcceptedAccounts, ContactInfoField, ContactInfoUpdate, UnreadableContactInfoPolicy,
};
use crate::model::qualified_identity::QualifiedIdentity;
use aes_gcm::aes::Aes256;
use aes_gcm::aes::cipher::{BlockEncrypt, KeyInit};
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
use dash_sdk::platform::documents::transitions::DocumentCreateTransitionBuilder;
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use zeroize::Zeroizing;

// ContactInfo private data structure
#[derive(Debug, Clone, Default)]
pub struct ContactInfoPrivateData {
    pub version: u32,
    pub alias_name: Option<String>,
    pub note: Option<String>,
    pub display_hidden: bool,
    pub accepted_accounts: Vec<u32>,
}

impl ContactInfoPrivateData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Minimum plaintext size so that IV (16) + AES-CBC ciphertext ≥ 48 bytes
    /// (the `privateData` field's `minItems` in the DashPay contract).
    /// PKCS7 pads 16 bytes to 32 (adds a full padding block when input is
    /// block-aligned), so 16 plaintext → 32 ciphertext → 48 with IV.
    const MIN_PLAINTEXT_SIZE: usize = 16;

    // Serialize to bytes for encryption
    pub fn serialize(&self) -> Result<Vec<u8>, DashPayError> {
        let mut bytes = Vec::new();

        // Version (4 bytes)
        bytes.extend_from_slice(&self.version.to_le_bytes());

        // Alias name (length + string)
        if let Some(alias) = &self.alias_name {
            let alias_bytes = alias.as_bytes();
            let alias_len = alias_bytes.len();
            if alias_len > u8::MAX as usize {
                return Err(DashPayError::ContactInfoValidationFailed {
                    errors: vec![format!("Nickname too long ({alias_len} bytes, max 255)")],
                });
            }
            bytes.push(alias_len as u8);
            bytes.extend_from_slice(alias_bytes);
        } else {
            bytes.push(0u8);
        }

        // Note (length + string)
        if let Some(note) = &self.note {
            let note_bytes = note.as_bytes();
            let note_len = note_bytes.len();
            if note_len > u8::MAX as usize {
                return Err(DashPayError::ContactInfoValidationFailed {
                    errors: vec![format!("Note too long ({note_len} bytes, max 255)")],
                });
            }
            bytes.push(note_len as u8);
            bytes.extend_from_slice(note_bytes);
        } else {
            bytes.push(0u8);
        }

        // Display hidden (1 byte)
        bytes.push(if self.display_hidden { 1 } else { 0 });

        // Accepted accounts (length + array)
        let accounts_len = self.accepted_accounts.len();
        if accounts_len > u8::MAX as usize {
            return Err(DashPayError::ContactInfoValidationFailed {
                errors: vec![format!(
                    "Too many accepted accounts ({accounts_len}, max 255)"
                )],
            });
        }
        bytes.push(accounts_len as u8);
        for account in &self.accepted_accounts {
            bytes.extend_from_slice(&account.to_le_bytes());
        }

        // Pad to minimum plaintext size so the encrypted output (IV + ciphertext)
        // meets the DashPay contract's privateData minItems (48 bytes).
        // First padding byte is 0x00 as a sentinel so deserializers can
        // distinguish real data from padding. Remaining bytes are random.
        if bytes.len() < Self::MIN_PLAINTEXT_SIZE {
            use bip39::rand::RngCore;
            bytes.push(0x00); // sentinel: marks start of padding
            let remaining = Self::MIN_PLAINTEXT_SIZE - bytes.len();
            if remaining > 0 {
                let mut pad = vec![0u8; remaining];
                StdRng::from_entropy().fill_bytes(&mut pad);
                bytes.extend_from_slice(&pad);
            }
        }

        Ok(bytes)
    }

    /// Parse the plaintext produced by [`serialize`](Self::serialize).
    ///
    /// Returns `None` for a truncated or non-canonical v0 payload, invalid
    /// UTF-8, or an unsupported version. The only accepted trailing bytes are
    /// this client's sentinel padding up to [`Self::MIN_PLAINTEXT_SIZE`].
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        let version = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
        if version != 0 {
            return None;
        }
        let mut pos = 4;

        let take_string = |pos: &mut usize| -> Option<Option<String>> {
            let len = *bytes.get(*pos)? as usize;
            *pos += 1;
            let raw = bytes.get(*pos..*pos + len)?;
            *pos += len;
            if len == 0 {
                Some(None)
            } else {
                Some(Some(std::str::from_utf8(raw).ok()?.to_owned()))
            }
        };
        let alias_name = take_string(&mut pos)?;
        let note = take_string(&mut pos)?;

        let display_hidden = match *bytes.get(pos)? {
            0 => false,
            1 => true,
            _ => return None,
        };
        pos += 1;

        let count = *bytes.get(pos)? as usize;
        pos += 1;
        let accepted_accounts = (0..count)
            .map(|_| {
                let raw = bytes.get(pos..pos + 4)?;
                pos += 4;
                Some(u32::from_le_bytes(raw.try_into().ok()?))
            })
            .collect::<Option<Vec<u32>>>()?;

        if pos < bytes.len()
            && (bytes.len() != Self::MIN_PLAINTEXT_SIZE || bytes.get(pos) != Some(&0))
        {
            return None;
        }

        Some(Self {
            version,
            alias_name,
            note,
            display_hidden,
            accepted_accounts,
        })
    }
}

/// Apply an explicit whole-document update to the payload already on Platform.
fn apply_contact_info_update(
    update: ContactInfoUpdate,
    existing: Option<&Document>,
    private_data_key: &[u8; 32],
) -> Result<ContactInfoPrivateData, TaskError> {
    let preserves_existing = matches!(update.nickname, ContactInfoField::Preserve)
        || matches!(update.note, ContactInfoField::Preserve)
        || matches!(update.accepted_accounts, AcceptedAccounts::Preserve);

    let mut data = if preserves_existing {
        read_contact_info_private_data(existing, private_data_key, update.unreadable)?
    } else {
        ContactInfoPrivateData::new()
    };

    if let ContactInfoField::Replace(nickname) = update.nickname {
        data.alias_name = nickname;
    }
    if let ContactInfoField::Replace(note) = update.note {
        data.note = note;
    }
    data.display_hidden = update.display_hidden;
    if let AcceptedAccounts::Replace(accounts) = update.accepted_accounts {
        data.accepted_accounts = accounts;
    }
    Ok(data)
}

fn read_contact_info_private_data(
    existing: Option<&Document>,
    private_data_key: &[u8; 32],
    unreadable: UnreadableContactInfoPolicy,
) -> Result<ContactInfoPrivateData, TaskError> {
    let Some(existing) = existing else {
        return Ok(ContactInfoPrivateData::new());
    };
    let Some(value) = existing.properties().get("privateData") else {
        return Ok(ContactInfoPrivateData::new());
    };
    let result = match value {
        Value::Bytes(encrypted) => {
            super::contacts::decrypt_private_data(encrypted, private_data_key)
                .map_err(|_| ContactInfoReadError::DecryptFailed)
                .and_then(|plaintext| {
                    ContactInfoPrivateData::deserialize(&plaintext)
                        .ok_or(ContactInfoReadError::DeserializeFailed)
                })
        }
        _ => Err(ContactInfoReadError::UnexpectedPrivateDataType),
    };

    match (result, unreadable) {
        (Ok(data), _) => Ok(data),
        (Err(_), UnreadableContactInfoPolicy::Overwrite) => Ok(ContactInfoPrivateData::new()),
        (Err(source), UnreadableContactInfoPolicy::Abort) => {
            Err(TaskError::DashPayContactInfoRead { source })
        }
    }
}

/// Derive the DIP-0015 contactInfo encryption keys for `identity`, fetching
/// the wallet's HD seed just-in-time through the [`SecretAccess`] chokepoint.
///
/// The seed is obtained per-operation via
/// [`SecretAccess::with_secret`](crate::wallet_backend::SecretAccess::with_secret)
/// keyed by the identity's DashPay wallet seed hash, and the BIP-32
/// derivation runs inside the closure through the shared
/// [`derive_contact_info_encryption_keys`](crate::wallet_backend::derive_contact_info_encryption_keys)
/// helper — the raw seed never enters this layer.
#[allow(clippy::type_complexity)]
async fn derive_contact_info_keys(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    derivation_index: u32,
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError> {
    let seed_hash = identity
        .dashpay_wallet_seed_hash()
        .ok_or(TaskError::ContactWalletSeedUnavailable)?;
    let network = identity.network;

    app_context
        .wallet_backend()?
        .secret_access()
        .with_secret(
            &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
            |plaintext| {
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                crate::wallet_backend::derive_contact_info_encryption_keys(
                    seed,
                    network,
                    derivation_index,
                )
            },
        )
        .await
}

/// Encrypt toUserId using AES-256-ECB as specified by DIP-0015.
///
/// DIP-0015 mandates ECB mode for encToUserId encryption because:
/// 1. The toUserId is derived from SHA256, making it appear random (no patterns)
/// 2. Keys are never reused (unique per contact via hardened BIP32 derivation)
/// 3. The data is fixed-size (32 bytes = exactly 2 AES blocks)
///
/// These properties eliminate typical ECB vulnerabilities (pattern leakage).
/// See: https://github.com/dashpay/dips/blob/master/dip-0015.md
#[allow(deprecated)]
fn encrypt_to_user_id(user_id: &[u8; 32], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aead::generic_array::GenericArray;
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

/// Decrypt toUserId using AES-256-ECB as specified by DIP-0015.
///
/// See `encrypt_to_user_id` for the rationale behind ECB mode usage per DIP-0015.
#[allow(deprecated)]
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aead::generic_array::GenericArray;
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

/// Write the `contactInfo` document for `contact_user_id`, creating it when the
/// identity has none yet and replacing it otherwise.
///
/// The document is written whole, so `update` explicitly chooses which fields
/// are preserved and which fields are replaced. Nicknames, notes, and account
/// lists are each limited to 255 bytes/items by the v0 encoding.
///
/// # Errors
///
/// Fails when a preserved payload is present but unreadable, when the contact's
/// encryption keys cannot be derived, when an encoded field exceeds its v0 or
/// contract limit, when the identity has no usable authentication key, or when
/// the state transition is rejected.
pub async fn create_or_update_contact_info(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_user_id: Identifier,
    update: ContactInfoUpdate,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let dashpay_contract = app_context.dashpay_contract.clone();
    let identity_id = identity.identity.id();

    // Query for existing contactInfo document
    let mut query = DocumentQuery::new(dashpay_contract.clone(), "contactInfo").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay contactInfo",
            source: Box::new(e),
        }
    })?;

    query = query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    query.limit = 100; // Get all contact info documents

    let existing_docs = Document::fetch_many(sdk, query).await?;

    // Check if we already have a contactInfo for this contact
    let mut found_existing_doc = None;
    let mut next_derivation_index = 0u32;

    // Try to find existing contactInfo for this contact
    for (_doc_id, doc) in existing_docs.iter() {
        if let Some(doc) = doc {
            let props = doc.properties();

            // Get the derivation index used for this document
            if let Some(deriv_idx) = props
                .get("derivationEncryptionKeyIndex")
                .and_then(|value| value.to_integer::<u32>().ok())
            {
                // Track the highest derivation index
                if deriv_idx >= next_derivation_index {
                    next_derivation_index = deriv_idx + 1;
                }

                // Get the root key index to derive keys
                if let Some(_root_idx) = props
                    .get("rootEncryptionKeyIndex")
                    .and_then(|value| value.to_integer::<u32>().ok())
                {
                    // Derive keys for this document
                    let (enc_user_id_key, _) =
                        derive_contact_info_keys(app_context, &identity, deriv_idx).await?;

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
            .and_then(|value| value.to_integer::<u32>().ok())
            .unwrap_or(0)
    } else {
        next_derivation_index
    };

    // Derive encryption keys
    let (enc_user_id_key, private_data_key) =
        derive_contact_info_keys(app_context, &identity, derivation_index).await?;

    // Encrypt toUserId
    let encrypted_user_id = encrypt_to_user_id(&contact_user_id.to_buffer(), &enc_user_id_key)
        .map_err(|e| TaskError::EncryptionError { detail: e })?;

    let private_data =
        apply_contact_info_update(update, found_existing_doc.as_ref(), &private_data_key)?;

    // Encrypt private data
    let encrypted_private_data =
        encrypt_private_data(&private_data.serialize()?, &private_data_key)
            .map_err(|e| TaskError::EncryptionError { detail: e })?;

    let validation = crate::backend_task::dashpay::validation::validate_contact_info_field_sizes(
        &encrypted_user_id,
        &encrypted_private_data,
    );
    if !validation.is_valid {
        return Err(TaskError::DashPay(
            DashPayError::ContactInfoValidationFailed {
                errors: validation.errors,
            },
        ));
    }

    // Get signing key — accept any key type (BLS, ECDSA, EDDSA) since
    // Platform accepts all for document state transitions.
    let signing_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]),
            KeyType::all_key_types().into(),
            false,
        )
        .ok_or_else(|| TaskError::DashPay(DashPayError::MissingAuthenticationKey))?;

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

    if let Some(existing_doc) = found_existing_doc {
        // Update existing document
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

        let result = sdk
            .document_replace(builder, signing_key, &identity)
            .await?;

        // Log the proof-verified document for audit trail
        match result {
            dash_sdk::platform::documents::transitions::DocumentReplaceResult::Document(doc) => {
                tracing::info!(
                    "Contact info updated: doc_id={}, revision={:?}",
                    doc.id(),
                    doc.revision()
                );
            }
        }
    } else {
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
            creator_id: None,
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

        let result = sdk.document_create(builder, signing_key, &identity).await?;

        // Log the proof-verified document for audit trail
        match result {
            dash_sdk::platform::documents::transitions::DocumentCreateResult::Document(doc) => {
                tracing::info!(
                    "Contact info created: doc_id={}, revision={:?}",
                    doc.id(),
                    doc.revision()
                );
            }
        }
    }

    Ok(BackendTaskSuccessResult::DashPayContactInfoUpdated {
        identity: identity_id,
        contact_id: contact_user_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];
    const OTHER_KEY: [u8; 32] = [9u8; 32];

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    /// A stored `contactInfo` document whose `privateData` holds `accounts`,
    /// encrypted exactly the way [`create_or_update_contact_info`] writes it.
    fn stored_contact_info(accounts: Vec<u32>, key: &[u8; 32]) -> Document {
        let private_data = ContactInfoPrivateData {
            version: 0,
            alias_name: Some("Bao".to_string()),
            note: None,
            display_hidden: true,
            accepted_accounts: accounts,
        };
        let encrypted = encrypt_private_data(&private_data.serialize().expect("serialize"), key)
            .expect("encrypt");

        contact_info_document(Some(Value::Bytes(encrypted)))
    }

    fn contact_info_document(private_data: Option<Value>) -> Document {
        let mut properties = BTreeMap::new();
        if let Some(private_data) = private_data {
            properties.insert("privateData".to_string(), private_data);
        }
        DppDocument::V0(DocumentV0 {
            id: id(1),
            owner_id: id(2),
            creator_id: None,
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
        })
    }

    #[test]
    fn private_data_round_trips_every_accepted_account() {
        let original = ContactInfoPrivateData {
            version: 0,
            alias_name: Some("Bao".to_string()),
            note: Some("Met at the meetup".to_string()),
            display_hidden: true,
            accepted_accounts: vec![0, 3, 17],
        };

        let parsed = ContactInfoPrivateData::deserialize(&original.serialize().expect("serialize"))
            .expect("a document this client wrote must parse back");

        assert_eq!(parsed.alias_name.as_deref(), Some("Bao"));
        assert_eq!(parsed.note.as_deref(), Some("Met at the meetup"));
        assert!(parsed.display_hidden);
        assert_eq!(
            parsed.accepted_accounts,
            vec![0, 3, 17],
            "every accepted account must survive the round trip, not just the first"
        );
    }

    #[test]
    fn private_data_round_trips_through_the_minimum_size_padding() {
        // An empty private data is padded up to the contract's minimum size —
        // the padding must not be mistaken for a field.
        let original = ContactInfoPrivateData::new();
        let parsed = ContactInfoPrivateData::deserialize(&original.serialize().expect("serialize"))
            .expect("padded plaintext must parse");

        assert_eq!(parsed.alias_name, None);
        assert_eq!(parsed.note, None);
        assert!(!parsed.display_hidden);
        assert!(parsed.accepted_accounts.is_empty());
    }

    #[test]
    fn truncated_private_data_is_not_parsed_as_a_shorter_list() {
        let bytes = ContactInfoPrivateData {
            version: 0,
            alias_name: None,
            note: None,
            display_hidden: false,
            accepted_accounts: vec![1, 2, 3],
        }
        .serialize()
        .expect("serialize");

        // Chop the final account in half: the list declares three, so a parser
        // that returned the two it could read would silently drop an account.
        assert!(
            ContactInfoPrivateData::deserialize(&bytes[..bytes.len() - 2]).is_none(),
            "a truncated account list must not parse as a shorter one"
        );
    }

    #[test]
    fn unsupported_private_data_version_requires_confirmation() {
        let mut bytes = ContactInfoPrivateData::new()
            .serialize()
            .expect("serialize");
        bytes[..4].copy_from_slice(&1_u32.to_le_bytes());

        assert!(
            ContactInfoPrivateData::deserialize(&bytes).is_none(),
            "a future payload version must not be rewritten as though it were version zero"
        );
    }

    #[test]
    fn invalid_utf8_is_not_treated_as_an_absent_contact_detail() {
        let mut bytes = ContactInfoPrivateData::new()
            .serialize()
            .expect("serialize");
        bytes[4] = 1;
        bytes[5] = 0xff;

        assert!(
            ContactInfoPrivateData::deserialize(&bytes).is_none(),
            "an unreadable nickname must not silently become an empty nickname"
        );
    }

    #[test]
    fn unknown_visibility_encoding_requires_confirmation() {
        let mut bytes = ContactInfoPrivateData::new()
            .serialize()
            .expect("serialize");
        bytes[6] = 2;

        assert!(ContactInfoPrivateData::deserialize(&bytes).is_none());
    }

    #[test]
    fn trailing_extension_bytes_require_confirmation() {
        let mut bytes = ContactInfoPrivateData {
            version: 0,
            alias_name: Some("12345678".to_string()),
            note: None,
            display_hidden: false,
            accepted_accounts: Vec::new(),
        }
        .serialize()
        .expect("serialize without minimum-size padding");
        assert_eq!(bytes.len(), ContactInfoPrivateData::MIN_PLAINTEXT_SIZE);
        bytes.push(42);

        assert!(ContactInfoPrivateData::deserialize(&bytes).is_none());
    }

    #[test]
    fn minimum_size_padding_requires_the_sentinel() {
        let mut bytes = ContactInfoPrivateData::new()
            .serialize()
            .expect("serialize");
        bytes[8] = 1;

        assert!(ContactInfoPrivateData::deserialize(&bytes).is_none());
    }

    #[test]
    fn visibility_update_preserves_every_unrelated_stored_field() {
        let existing = stored_contact_info(vec![0, 4, 9], &KEY);
        let data =
            apply_contact_info_update(ContactInfoUpdate::visibility(false), Some(&existing), &KEY)
                .expect("stored details must be readable");

        assert_eq!(data.alias_name.as_deref(), Some("Bao"));
        assert_eq!(data.note, None);
        assert!(!data.display_hidden);
        assert_eq!(data.accepted_accounts, vec![0, 4, 9]);
    }

    #[test]
    fn replace_overwrites_whatever_the_document_stored() {
        let existing = stored_contact_info(vec![0, 4, 9], &KEY);

        let data = apply_contact_info_update(
            ContactInfoUpdate::replace_all(None, None, false, vec![2]),
            Some(&existing),
            &OTHER_KEY,
        )
        .expect("an explicit full replacement does not read stored data");

        assert_eq!(data.accepted_accounts, vec![2]);
        assert_eq!(data.alias_name, None);
    }

    #[test]
    fn preserving_a_contact_with_no_stored_document_yields_no_accounts() {
        let data = apply_contact_info_update(ContactInfoUpdate::visibility(false), None, &KEY)
            .expect("a first-ever contactInfo has nothing to preserve");

        assert!(data.accepted_accounts.is_empty());
    }

    #[test]
    fn unreadable_private_data_aborts_the_write() {
        let existing = stored_contact_info(vec![0, 4, 9], &OTHER_KEY);

        assert!(
            matches!(
                apply_contact_info_update(
                    ContactInfoUpdate::visibility(false),
                    Some(&existing),
                    &KEY,
                ),
                Err(TaskError::DashPayContactInfoRead {
                    source: ContactInfoReadError::DecryptFailed,
                })
            ),
            "an undecryptable present payload must abort before it can be overwritten"
        );
    }

    #[test]
    fn confirmed_overwrite_is_the_only_escape_hatch_for_unreadable_private_data() {
        use crate::model::dashpay::ContactInfoUpdate;

        let existing = stored_contact_info(vec![0, 4, 9], &OTHER_KEY);
        let update = ContactInfoUpdate::visibility(false);

        assert!(matches!(
            apply_contact_info_update(update.clone(), Some(&existing), &KEY),
            Err(TaskError::DashPayContactInfoRead {
                source: ContactInfoReadError::DecryptFailed,
            })
        ));

        let replaced =
            apply_contact_info_update(update.overwrite_unreadable(), Some(&existing), &KEY)
                .expect("a confirmed overwrite must let the contact be unhidden");
        assert_eq!(replaced.alias_name, None);
        assert_eq!(replaced.note, None);
        assert!(!replaced.display_hidden);
        assert!(replaced.accepted_accounts.is_empty());
    }

    #[test]
    fn missing_private_data_is_safe_to_treat_as_empty() {
        let existing = contact_info_document(None);
        let data =
            apply_contact_info_update(ContactInfoUpdate::visibility(false), Some(&existing), &KEY)
                .expect("an absent payload contains no details to lose");

        assert!(data.accepted_accounts.is_empty());
    }

    #[test]
    fn unexpected_private_data_type_requires_confirmation() {
        let existing = contact_info_document(Some(Value::Text("unknown".to_string())));

        assert!(matches!(
            apply_contact_info_update(ContactInfoUpdate::visibility(false), Some(&existing), &KEY,),
            Err(TaskError::DashPayContactInfoRead {
                source: ContactInfoReadError::UnexpectedPrivateDataType,
            })
        ));
    }

    #[test]
    fn undecodable_private_data_aborts_the_write() {
        let encrypted = encrypt_private_data(&[0, 0, 0, 0], &KEY).expect("encrypt malformed data");
        let existing = contact_info_document(Some(Value::Bytes(encrypted)));

        assert!(matches!(
            apply_contact_info_update(ContactInfoUpdate::visibility(false), Some(&existing), &KEY,),
            Err(TaskError::DashPayContactInfoRead {
                source: ContactInfoReadError::DeserializeFailed,
            })
        ));
    }
}
