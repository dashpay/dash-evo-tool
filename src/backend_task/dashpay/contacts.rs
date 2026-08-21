use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::contact_request_query;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::contact_request_recipient;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use futures::future::join_all;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use zeroize::Zeroizing;

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
pub(super) fn decrypt_private_data(
    encrypted_data: &[u8],
    key: &[u8; 32],
) -> Result<Vec<u8>, String> {
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
) -> Result<BackendTaskSuccessResult, TaskError> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for contact requests where we are the sender (ownerId)
    let mut outgoing_query = contact_request_query(app_context)?;

    outgoing_query = outgoing_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    outgoing_query.limit = 100;

    // Query for contact requests where we are the recipient (toUserId)
    let mut incoming_query = contact_request_query(app_context)?;

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
    let outgoing_docs = Document::fetch_many(sdk, outgoing_query).await?;

    let incoming_docs = Document::fetch_many(sdk, incoming_query).await?;

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
    let contacts: HashSet<Identifier> = incoming
        .iter()
        .map(|(_, doc)| doc.owner_id())
        .filter(|from_id| {
            outgoing
                .iter()
                .any(|(_, doc)| contact_request_recipient(doc).as_ref() == Some(from_id))
        })
        .collect();

    // Now query for contact info documents
    let mut contact_info_query = DocumentQuery::new(dashpay_contract.clone(), "contactInfo")
        .map_err(|e| DashPayError::QueryCreation {
            query_target: "DashPay contactInfo",
            source: Box::new(e.into()),
        })?;

    contact_info_query = contact_info_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    contact_info_query.limit = 100;

    let contact_info_docs = Document::fetch_many(sdk, contact_info_query).await?;

    // Build a map of contact ID to contact info
    let mut contact_info_map: HashMap<Identifier, ContactData> = HashMap::new();

    for (_doc_id, doc) in contact_info_docs.iter() {
        if let Some(doc) = doc {
            let props = doc.properties();

            // Get the derivation index used for this document
            if let Some(deriv_idx) = props
                .get("derivationEncryptionKeyIndex")
                .and_then(|value| value.to_integer::<u32>().ok())
            {
                // Derive keys for this document
                let (enc_user_id_key, private_data_key) =
                    match derive_contact_info_keys(app_context, &identity, deriv_idx).await {
                        Ok(keys) => keys,
                        Err(_) => continue,
                    };

                // Decrypt encToUserId to find which contact this is for
                if let Some(Value::Bytes(enc_user_id)) = props.get("encToUserId")
                    && let Ok(decrypted_id) = decrypt_to_user_id(enc_user_id, &enc_user_id_key)
                {
                    let Ok(contact_id) = Identifier::from_bytes(&decrypted_id) else {
                        tracing::warn!(
                            "Failed to parse decrypted contact ID (length {}), skipping contact info entry",
                            decrypted_id.len()
                        );
                        continue;
                    };

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
                                .get("publicMessage")
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

    // Mirror the freshly-fetched contact profiles into the DET-side cache so
    // the next view can paint names and avatars offline. Best-
    // effort: a cache write miss only costs the offline optimisation.
    cache_contact_profiles(app_context, &contact_list);

    Ok(BackendTaskSuccessResult::DashPayContactsWithInfo {
        identity: identity_id,
        contacts: contact_list,
    })
}

/// Read the contact list for `identity` entirely from offline state: contact
/// relationships and private memos come from the upstream-rehydrated
/// `ManagedIdentity` (via [`crate::wallet_backend::DashpayView`]), and each
/// contact's display profile is served from the DET contact-profile cache.
///
/// No network round-trip — a view rendered from this result does not require
/// connectivity. Profiles a contact has not yet been fetched for are returned
/// without display fields; an explicit refresh (network `load_contacts`) fills
/// and re-caches them.
pub async fn load_contacts_offline(
    app_context: &Arc<AppContext>,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let owner_id = identity.identity.id();
    let backend = app_context.wallet_backend()?;

    let stored = backend.dashpay_view().contacts(&owner_id).await;

    let mut contact_list = Vec::with_capacity(stored.len());
    for sc in stored {
        // Skip the owner's own row defensively (contacts() does not emit it,
        // but the network path filters it, so mirror that here).
        let Ok(contact_id) = Identifier::from_bytes(&sc.contact_identity_id) else {
            continue;
        };
        if contact_id == owner_id {
            continue;
        }

        let cached = backend.contact_profile_cache().get(&contact_id);
        let memo = backend
            .dashpay_get_private_info(&owner_id, &contact_id)
            .unwrap_or(None);

        contact_list.push(ContactData {
            identity_id: contact_id,
            nickname: memo
                .as_ref()
                .map(|m| m.nickname.clone())
                .filter(|s| !s.is_empty()),
            note: memo
                .as_ref()
                .map(|m| m.notes.clone())
                .filter(|s| !s.is_empty()),
            is_hidden: memo.as_ref().map(|m| m.is_hidden).unwrap_or(false),
            // Carry the cached account reference so the "Account #N" badge shows
            // and offline ordering matches the post-refresh view. Falls back to
            // 0 ("default account") when no network read has cached one yet.
            account_reference: cached
                .as_ref()
                .and_then(|c| c.account_reference)
                .unwrap_or(0),
            username: cached.as_ref().and_then(|c| c.username.clone()),
            display_name: cached
                .as_ref()
                .and_then(|c| c.display_name.clone())
                .or_else(|| sc.display_name.clone()),
            avatar_url: cached.as_ref().and_then(|c| c.avatar_url.clone()),
            bio: cached.as_ref().and_then(|c| c.bio.clone()),
        });
    }

    Ok(BackendTaskSuccessResult::DashPayContactsWithInfo {
        identity: owner_id,
        contacts: contact_list,
    })
}

/// Write each contact's fetched display profile into the DET contact-profile
/// cache so later offline reads can serve it. Skips contacts with no
/// displayable field. Best-effort: write misses are logged and ignored.
fn cache_contact_profiles(app_context: &Arc<AppContext>, contacts: &[ContactData]) {
    use crate::wallet_backend::CachedContactProfile;

    let Ok(backend) = app_context.wallet_backend() else {
        return;
    };
    let cache = backend.contact_profile_cache();
    for contact in contacts {
        let profile = CachedContactProfile {
            username: contact.username.clone(),
            display_name: contact.display_name.clone(),
            avatar_url: contact.avatar_url.clone(),
            bio: contact.bio.clone(),
            // Carry the account reference so the offline read can show the
            // "Account #N" badge and sort consistently. 0 means "default
            // account / not set", so it is not worth caching on its own.
            account_reference: (contact.account_reference != 0)
                .then_some(contact.account_reference),
        };
        if profile.is_empty() {
            continue;
        }
        if let Err(e) = cache.put(&contact.identity_id, &profile) {
            tracing::debug!(
                contact = %contact.identity_id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                error = ?e,
                "Failed to cache contact profile for offline use"
            );
        }
    }
}
