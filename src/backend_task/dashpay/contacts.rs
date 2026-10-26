use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::contact_request_query;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{ContactDetails, contact_request_recipient};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use dash_sdk::query_types::Documents;
use futures::future::join_all;
use platform_wallet::wallet::identity::crypto::{ContactInfoPrivateData, decode_private_data};
use std::collections::{HashMap, HashSet};
use std::future::Future;
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
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aes::Aes256;
    use aes_gcm::aes::cipher::{BlockCipherDecrypt, KeyInit};

    let mut decrypted: [u8; 32] = encrypted
        .try_into()
        .map_err(|_| "Invalid encrypted user ID length".to_string())?;

    let cipher = Aes256::new(key.into());

    // The 32-byte ciphertext is exactly two 16-byte AES blocks; decrypt each in place (ECB).
    let (blocks, _) = decrypted.as_chunks_mut::<16>();
    for block in blocks {
        cipher.decrypt_block(block.into());
    }

    Ok(decrypted)
}

// Helper function to decrypt private data using AES-256-CBC
pub(super) fn decrypt_private_data(
    encrypted_data: &[u8],
    key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use cbc::cipher::BlockModeDecrypt;
    use cbc::cipher::KeyIvInit;
    use cbc::cipher::block_padding::Pkcs7;
    type Aes256CbcDec = cbc::Decryptor<aes_gcm::aes::Aes256>;

    // Extract IV and ciphertext
    let (iv, ciphertext) = encrypted_data
        .split_first_chunk::<16>()
        .ok_or_else(|| "Encrypted data too short (no IV)".to_string())?;

    // Decrypt
    let cipher = Aes256CbcDec::new(key.into(), iv.into());

    let mut buffer = ciphertext.to_vec();
    let decrypted = cipher
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .map_err(|e| format!("Decryption failed: {:?}", e))?;

    Ok(decrypted.to_vec())
}

/// The nickname, note and visibility the owner stored for each contact in
/// `contactInfo` documents sealed under the legacy key root, read from every
/// page of the owner's documents.
///
/// Pages are ordered by `$updatedAt`, oldest first, and a later document for
/// a contact replaces an earlier one, so the newest document wins. The test
/// seam over [`stored_contact_details`]'s legacy half.
#[cfg(test)]
pub(super) async fn load_contact_details<P, PF, D, DF>(
    query: DocumentQuery,
    fetch_page: P,
    derive_keys: D,
) -> Result<HashMap<Identifier, ContactData>, TaskError>
where
    P: FnMut(DocumentQuery) -> PF,
    PF: Future<Output = Result<Documents, TaskError>>,
    D: FnMut(u32) -> DF,
    DF: Future<Output = Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError>>,
{
    let documents = fetch_contact_info_documents(query, fetch_page).await?;
    legacy_details_from(&documents, derive_keys).await
}

/// Every page of the owner's `contactInfo` documents, oldest first by
/// `$updatedAt`.
async fn fetch_contact_info_documents<P, PF>(
    query: DocumentQuery,
    fetch_page: P,
) -> Result<Vec<Document>, TaskError>
where
    P: FnMut(DocumentQuery) -> PF,
    PF: Future<Output = Result<Documents, TaskError>>,
{
    let query = query.with_order_by(OrderClause {
        field: "$updatedAt".to_string(),
        ascending: true,
    });
    let documents =
        super::contact_info::fetch_all_contact_info_documents(query, fetch_page).await?;
    Ok(documents.into_values().flatten().collect())
}

/// The details `documents` (oldest first) hold under the legacy key root;
/// a later document for a contact replaces an earlier one. A document whose
/// keys or contact id do not open is skipped, since nothing tells whose it
/// is. One whose details do not decrypt or decode marks its contact
/// unreadable and hidden.
///
/// # Errors
///
/// The keys could not be derived for a reason that holds for every
/// document, such as a cancelled password prompt.
async fn legacy_details_from<D, DF>(
    documents: &[Document],
    mut derive_keys: D,
) -> Result<HashMap<Identifier, ContactData>, TaskError>
where
    D: FnMut(u32) -> DF,
    DF: Future<Output = Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError>>,
{
    let mut contact_info_map: HashMap<Identifier, ContactData> = HashMap::new();
    for doc in documents {
        let props = doc.properties();
        let Some(deriv_idx) = props
            .get("derivationEncryptionKeyIndex")
            .and_then(|value| value.to_integer::<u32>().ok())
        else {
            continue;
        };
        let (enc_user_id_key, private_data_key) = match derive_keys(deriv_idx).await {
            Ok(keys) => keys,
            // The document's own index cannot be derived.
            Err(TaskError::ContactKeyDerivationFailed { .. }) => continue,
            Err(error) => return Err(error),
        };
        // Platform returns the fixed 32-byte field as `Value::Bytes32`; a
        // document built locally carries `Value::Bytes`.
        let Some(enc_user_id) = props
            .get("encToUserId")
            .and_then(|value| value.as_bytes_slice().ok())
        else {
            continue;
        };
        let Ok(decrypted_id) = decrypt_to_user_id(enc_user_id, &enc_user_id_key) else {
            continue;
        };
        let Ok(contact_id) = Identifier::from_bytes(&decrypted_id) else {
            tracing::warn!(
                "Failed to parse decrypted contact ID (length {}), skipping contact info entry",
                decrypted_id.len()
            );
            continue;
        };
        // The DIP-15 codec platform-wallet writes with. A payload that is
        // present but cannot be opened is not "no details": its hidden flag
        // is unknown, so the contact stays hidden.
        let private_data = match props.get("privateData") {
            None => Some(ContactInfoPrivateData::default()),
            Some(value) => value
                .as_bytes_slice()
                .ok()
                .and_then(|encrypted| decrypt_private_data(encrypted, &private_data_key).ok())
                .and_then(|plaintext| decode_private_data(&plaintext).ok()),
        };
        let details_unreadable = private_data.is_none();
        let private_data = private_data.unwrap_or(ContactInfoPrivateData {
            display_hidden: true,
            ..Default::default()
        });

        contact_info_map.insert(
            contact_id,
            ContactData {
                identity_id: contact_id,
                account_reference: private_data.accepted_accounts.first().copied().unwrap_or(0),
                nickname: private_data.alias_name,
                note: private_data.note,
                is_hidden: private_data.display_hidden,
                details_unreadable,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            },
        );
    }

    Ok(contact_info_map)
}

/// Run `sync` once no contact details save of `owner` is in flight.
///
/// A save stores its details on this device before it publishes them; a
/// sync in between would put the older published details back over them.
async fn after_details_saves<T>(
    writes: &crate::context::ContactInfoWrites,
    owner: Identifier,
    sync: impl Future<Output = T>,
) -> T {
    let _write_guard = writes.acquire(owner).await;
    sync.await
}

/// The details the owner's stored `contactInfo` documents hold.
pub(super) struct StoredContactDetails {
    /// From DIP-0015 documents platform-wallet can read, newest per contact;
    /// an error when the seed could not be used to read them. A save must not
    /// guess past that error; the contacts list may.
    pub(super) published: Result<crate::wallet_backend::PublishedContactInfos, TaskError>,
    /// From documents older versions of this app wrote under the legacy
    /// key root, newest per contact; an error as for `published`.
    pub(super) legacy: Result<HashMap<Identifier, ContactData>, TaskError>,
}

/// Read every `contactInfo` document the owner stored, both as DIP-0015
/// documents and as legacy-key ones.
///
/// platform-wallet does not read DIP-0015 documents back for a wallet loaded
/// without its seed, which is how wallets load after a restart, so they are
/// opened here with platform-wallet's keys and codec.
pub(super) async fn stored_contact_details(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
) -> Result<StoredContactDetails, TaskError> {
    let query = DocumentQuery::new(app_context.dashpay_contract(), "contactInfo")
        .map_err(|e| DashPayError::QueryCreation {
            query_target: "DashPay contactInfo",
            source: Box::new(e.into()),
        })?
        .with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(identity.identity.id().to_buffer()),
        });
    let documents = fetch_contact_info_documents(query, |query| async move {
        Document::fetch_many(sdk, query)
            .await
            .map_err(TaskError::from)
    })
    .await?;
    let published = match app_context.wallet_backend() {
        Ok(backend) => {
            backend
                .dashpay_open_contact_infos(identity, &documents)
                .await
        }
        Err(_) => Ok(crate::wallet_backend::PublishedContactInfos::default()),
    };
    // Without a wallet seed no legacy key exists, so no document opens.
    let legacy = if identity.dashpay_wallet_seed_hash().is_some() {
        legacy_details_from(&documents, |derivation_index| {
            derive_contact_info_keys(app_context, identity, derivation_index)
        })
        .await
    } else {
        Ok(HashMap::new())
    };
    Ok(StoredContactDetails { published, legacy })
}

/// The details the contacts list shows for one contact, and that a save
/// keeps for every field it does not replace.
///
/// A published DIP-0015 document wins: Platform holds the latest saved state
/// from any device. Without one, platform-wallet's details on this device
/// win; a legacy-key document stands in only while platform-wallet has never
/// held details for the contact (`synced_before` false). Once it has, its
/// details win even when empty, so a cleared note or an unhide is never
/// undone by an old legacy document.
pub(super) fn shown_contact_details(
    published: Option<ContactDetails>,
    local: Option<ContactDetails>,
    synced_before: bool,
    legacy: Option<ContactDetails>,
) -> ContactDetails {
    if let Some(published) = published {
        return published;
    }
    if synced_before {
        return local.unwrap_or_default();
    }
    local
        .filter(|details| !details.is_empty())
        .or(legacy)
        .unwrap_or_default()
}

/// Whether [`shown_contact_details`] shows the legacy document's details:
/// only then can unreadable legacy details mark the contact unreadable.
pub(super) fn legacy_details_shown(
    published: Option<&ContactDetails>,
    local: Option<&ContactDetails>,
    synced_before: bool,
) -> bool {
    published.is_none() && !synced_before && local.is_none_or(ContactDetails::is_empty)
}

/// Whether platform-wallet's side holds details worth recording as its own:
/// a published DIP-0015 document (even an empty one), or non-empty details on
/// this device.
pub(super) fn holds_details(
    published: Option<&ContactDetails>,
    local: Option<&ContactDetails>,
) -> bool {
    published.is_some() || local.is_some_and(|details| !details.is_empty())
}

/// The people both sides have sent a contact request to, minus anyone the
/// owner withdrew their request from: a reply to a withdrawn request is a new
/// request for the owner to answer, not an established contact.
pub(super) fn mutual_contacts(
    incoming: &[(Identifier, Document)],
    outgoing: &[(Identifier, Document)],
    is_withdrawn: impl Fn(&Identifier) -> bool,
) -> HashSet<Identifier> {
    incoming
        .iter()
        .map(|(_, doc)| doc.owner_id())
        .filter(|from_id| !is_withdrawn(from_id))
        .filter(|from_id| {
            outgoing
                .iter()
                .any(|(_, doc)| contact_request_recipient(doc).as_ref() == Some(from_id))
        })
        .collect()
}

impl ContactData {
    pub(super) fn details(&self) -> ContactDetails {
        ContactDetails {
            nickname: self.nickname.clone(),
            note: self.note.clone(),
            hidden: self.is_hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactData {
    pub identity_id: Identifier,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub is_hidden: bool,
    /// The stored details exist but could not be decrypted or decoded. Such a
    /// contact is kept hidden, since its hidden flag cannot be read.
    pub details_unreadable: bool,
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
    let dashpay_contract = app_context.dashpay_contract();

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
    let backend = app_context.wallet_backend().ok();
    let contacts = mutual_contacts(&incoming, &outgoing, |peer| {
        backend
            .as_ref()
            .is_some_and(|backend| backend.dashpay_is_withdrawn(&identity_id, peer))
    });

    // platform-wallet's synced details first; a refresh failure leaves the
    // last synced details in place.
    let mut synced = match &backend {
        Some(backend) => {
            if let Err(error) = backend.dashpay_sync(&identity_id).await {
                tracing::debug!(
                    ?error,
                    "LoadContacts: DashPay sync degraded; using cached state"
                );
            }
            let synced = after_details_saves(
                &app_context.contact_info_writes,
                identity_id,
                backend.dashpay_sync_contact_infos(&identity_id),
            )
            .await;
            if let Err(error) = synced {
                tracing::debug!(
                    ?error,
                    "LoadContacts: contact details sync degraded; using cached state"
                );
            }
            backend.dashpay_contact_details(&identity_id).await
        }
        None => {
            tracing::debug!("LoadContacts: no wallet backend; showing legacy details");
            Default::default()
        }
    };
    let StoredContactDetails { published, legacy } =
        stored_contact_details(app_context, sdk, &identity).await?;
    let mut published = published.unwrap_or_else(|error| {
        tracing::debug!(?error, "LoadContacts: published contact details not read");
        crate::wallet_backend::PublishedContactInfos::default()
    });
    let legacy = legacy.unwrap_or_else(|error| {
        tracing::debug!(?error, "LoadContacts: legacy contact details not read");
        HashMap::new()
    });

    // Build enriched contact list with basic data
    let mut contact_list: Vec<ContactData> = contacts
        .into_iter()
        .map(|contact_id| {
            let legacy = legacy.get(&contact_id);
            let synced = synced.remove(&contact_id);
            let (published, published_unreadable) = match published.take(&contact_id) {
                Some((details, unreadable)) => (Some(details), unreadable),
                None => (None, false),
            };
            let synced_before = backend.as_ref().is_some_and(|backend| {
                let synced_before = backend
                    .dashpay_details_synced(&identity_id, &contact_id)
                    .unwrap_or_else(|error| {
                        tracing::debug!(?error, "LoadContacts: synced record not read");
                        false
                    });
                if !synced_before
                    && holds_details(published.as_ref(), synced.as_ref())
                    && let Err(error) =
                        backend.dashpay_mark_details_synced(&identity_id, &contact_id)
                {
                    tracing::debug!(?error, "LoadContacts: could not record synced details");
                }
                synced_before
            });
            let details_unreadable = published_unreadable
                || legacy_details_shown(published.as_ref(), synced.as_ref(), synced_before)
                    && legacy.is_some_and(|legacy| legacy.details_unreadable);
            let details = shown_contact_details(
                published,
                synced,
                synced_before,
                legacy.map(ContactData::details),
            );
            ContactData {
                identity_id: contact_id,
                nickname: details.nickname,
                note: details.note,
                is_hidden: details.hidden,
                details_unreadable,
                account_reference: legacy.map_or(0, |legacy| legacy.account_reference),
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            }
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
                    if let Ok(names) = sdk
                        .get_dpns_usernames_by_identity(contact_id, Some(1))
                        .await
                    {
                        username = names.into_iter().next().map(|name| name.label);
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
/// relationships and contact details come from the upstream-rehydrated
/// `ManagedIdentity` (via [`crate::wallet_backend::DashpayView`]), with this
/// device's private-info copy as a fallback (see [`shown_contact_details`]),
/// and each contact's display profile is served from the DET contact-profile
/// cache.
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
    // The details platform-wallet holds win, as in the network list; this
    // device's private-info copy only stands in where it never held any.
    let mut synced = backend.dashpay_contact_details(&owner_id).await;

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
        // A withdrawn request stays hidden, as in the network list
        // (`mutual_contacts`). Filtered here rather than in
        // `DashpayView::contacts`, whose other callers register payment
        // addresses and must keep every contact.
        if backend.dashpay_is_withdrawn(&owner_id, &contact_id) {
            continue;
        }

        let cached = backend.contact_profile_cache().get(&contact_id);
        let memo = backend
            .dashpay_get_private_info(&owner_id, &contact_id)
            .unwrap_or_else(|error| {
                tracing::debug!(?error, "LoadContactsOffline: private info not read");
                None
            })
            .map(|memo| ContactDetails {
                nickname: Some(memo.nickname).filter(|s| !s.is_empty()),
                note: Some(memo.notes).filter(|s| !s.is_empty()),
                hidden: memo.is_hidden,
            });
        let synced_before = backend
            .dashpay_details_synced(&owner_id, &contact_id)
            .unwrap_or_else(|error| {
                tracing::debug!(?error, "LoadContactsOffline: synced record not read");
                false
            });
        let details = shown_contact_details(None, synced.remove(&contact_id), synced_before, memo);

        contact_list.push(ContactData {
            identity_id: contact_id,
            nickname: details.nickname,
            note: details.note,
            is_hidden: details.hidden,
            details_unreadable: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The contacts list's details sync waits for a save of the same owner,
    /// so it cannot put older published details over the save's own.
    #[tokio::test]
    async fn the_list_details_sync_waits_for_a_save() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let writes = crate::context::ContactInfoWrites::default();
        let owner = Identifier::from([1; 32]);
        let ran = AtomicBool::new(false);
        let save = writes.acquire(owner).await;

        let mut sync = Box::pin(after_details_saves(&writes, owner, async {
            ran.store(true, Ordering::SeqCst);
        }));
        assert!(futures::poll!(&mut sync).is_pending());
        assert!(!ran.load(Ordering::SeqCst), "the sync waits for the save");

        drop(save);
        sync.await;
        assert!(ran.load(Ordering::SeqCst));
    }

    /// After a restart the offline list shows the details platform-wallet
    /// holds (synced from Platform) rather than this device's older copy, so
    /// a contact hidden on another device is not listed as visible here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_offline_list_shows_platform_wallets_details() {
        use crate::model::wallet::Wallet;
        use crate::model::wallet::birth_height::WalletOrigin;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;

        let dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = context.wallet_backend().expect("backend wired");
        let seed = [0x4Du8; 64];
        let wallet =
            Wallet::new_from_seed(seed, Network::Testnet, Some("w".into()), None).expect("wallet");
        let (seed_hash, wallet) = context
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("register upstream");
        let owner = Identifier::from([0x11; 32]);
        let contact = Identifier::from([0x22; 32]);
        let identity =
            Identity::create_basic_identity(owner, PlatformVersion::latest()).expect("identity");
        backend
            .ensure_identity_managed(&seed_hash, &identity, 0)
            .await
            .expect("manage identity");
        let request = |from, to| {
            platform_wallet::ContactRequest::new(from, to, 0, 0, 0, vec![0u8; 96], 100_000, 1)
        };
        backend
            .record_sent_contact_request(&seed_hash, &owner, request(owner, contact))
            .await
            .expect("record sent request");
        backend
            .record_incoming_contact_request(&seed_hash, &owner, request(contact, owner))
            .await
            .expect("record incoming request");
        // This device's older copy.
        backend
            .dashpay_set_private_info(
                &owner,
                &contact,
                &crate::model::dashpay::ContactPrivateInfo {
                    nickname: "Old".to_string(),
                    notes: "old note".to_string(),
                    is_hidden: false,
                },
            )
            .expect("sidecar written");
        let synced = ContactDetails {
            nickname: Some("Bao".into()),
            note: None,
            hidden: true,
        };
        backend
            .set_contact_details_for_tests(&seed_hash, &owner, &contact, synced.clone())
            .await;
        let mut qualified =
            crate::context::test_support::bare_user_identity(owner, Network::Testnet);
        qualified.associated_wallets = BTreeMap::from([(seed_hash, wallet)]);

        let result = load_contacts_offline(&context, qualified)
            .await
            .expect("offline list");

        let BackendTaskSuccessResult::DashPayContactsWithInfo { contacts, .. } = result else {
            panic!("expected DashPayContactsWithInfo, got {result:?}");
        };
        let [row] = contacts.as_slice() else {
            panic!("expected one contact, got {contacts:?}");
        };
        assert_eq!(row.details(), synced);
    }

    /// After a restart the contacts list is built offline; a request the
    /// owner withdrew stays hidden there, as it does in the network list.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_offline_list_hides_a_withdrawn_request() {
        use crate::model::wallet::Wallet;
        use crate::model::wallet::birth_height::WalletOrigin;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;

        let dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = context.wallet_backend().expect("backend wired");
        let seed = [0x3Cu8; 64];
        let wallet =
            Wallet::new_from_seed(seed, Network::Testnet, Some("w".into()), None).expect("wallet");
        let (seed_hash, wallet) = context
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("register upstream");
        let owner = Identifier::from([0x11; 32]);
        let withdrawn = Identifier::from([0x22; 32]);
        let pending = Identifier::from([0x33; 32]);
        let identity =
            Identity::create_basic_identity(owner, PlatformVersion::latest()).expect("identity");
        backend
            .ensure_identity_managed(&seed_hash, &identity, 0)
            .await
            .expect("manage identity");
        for recipient in [withdrawn, pending] {
            backend
                .record_sent_contact_request(
                    &seed_hash,
                    &owner,
                    platform_wallet::ContactRequest::new(
                        owner,
                        recipient,
                        0,
                        0,
                        0,
                        vec![0u8; 96],
                        100_000,
                        1,
                    ),
                )
                .await
                .expect("record sent request");
        }
        backend
            .dashpay_mark_withdrawn(&owner, &withdrawn)
            .expect("withdraw");
        let mut qualified =
            crate::context::test_support::bare_user_identity(owner, Network::Testnet);
        qualified.associated_wallets = BTreeMap::from([(seed_hash, wallet)]);

        let result = load_contacts_offline(&context, qualified)
            .await
            .expect("offline list");

        let BackendTaskSuccessResult::DashPayContactsWithInfo { contacts, .. } = result else {
            panic!("expected DashPayContactsWithInfo, got {result:?}");
        };
        assert_eq!(
            contacts.iter().map(|c| c.identity_id).collect::<Vec<_>>(),
            vec![pending]
        );
    }

    fn details(note: &str) -> ContactDetails {
        ContactDetails {
            nickname: None,
            note: Some(note.to_string()),
            hidden: false,
        }
    }

    #[test]
    fn synced_contact_details_win_over_legacy_ones() {
        assert_eq!(
            shown_contact_details(
                None,
                Some(details("synced")),
                false,
                Some(details("legacy"))
            ),
            details("synced")
        );
    }

    #[test]
    fn legacy_contact_details_show_when_none_were_ever_synced() {
        assert_eq!(
            shown_contact_details(None, None, false, Some(details("legacy"))),
            details("legacy")
        );
        assert_eq!(
            shown_contact_details(
                None,
                Some(ContactDetails::default()),
                false,
                Some(details("legacy"))
            ),
            details("legacy"),
            "a contact platform-wallet never held details for shows the legacy ones"
        );
        assert_eq!(
            shown_contact_details(None, None, false, None),
            ContactDetails::default()
        );
    }

    /// SEC-002: a note the user cleared, or a contact they unhid, must not
    /// come back from an old legacy document — nor be saved again by the
    /// next edit, which keeps these details for every field it leaves alone.
    #[test]
    fn cleared_details_do_not_come_back_from_a_legacy_document() {
        let legacy = ContactDetails {
            nickname: None,
            note: Some("old".into()),
            hidden: true,
        };
        assert_eq!(
            shown_contact_details(
                None,
                Some(ContactDetails::default()),
                true,
                Some(legacy.clone())
            ),
            ContactDetails::default()
        );
        assert_eq!(
            shown_contact_details(None, None, true, Some(legacy)),
            ContactDetails::default()
        );
    }

    /// Unreadable legacy details mark a contact unreadable only while they
    /// stand in for its details, exactly when `shown_contact_details` shows
    /// them.
    #[test]
    fn legacy_details_are_shown_only_when_nothing_newer_holds_details() {
        let empty = ContactDetails::default();
        let note = details("note");
        for (published, local, synced_before) in [
            (None, None, false),
            (None, Some(&empty), false),
            (None, Some(&note), false),
            (None, None, true),
            (None, Some(&empty), true),
            (None, Some(&note), true),
            (Some(&empty), None, false),
            (Some(&note), None, false),
            (Some(&note), Some(&note), true),
        ] {
            let legacy = details("legacy");
            assert_eq!(
                legacy_details_shown(published, local, synced_before),
                shown_contact_details(
                    published.cloned(),
                    local.cloned(),
                    synced_before,
                    Some(legacy.clone())
                ) == legacy,
                "{published:?} {local:?} {synced_before}"
            );
        }
    }

    #[test]
    fn only_non_empty_synced_details_are_recorded_as_synced() {
        assert!(holds_details(None, Some(&details("note"))));
        assert!(!holds_details(None, Some(&ContactDetails::default())));
        assert!(!holds_details(None, None));
    }

    /// SEC-003: Platform's DIP-0015 document is the latest saved state from
    /// any device, so it wins over this device's details and legacy ones —
    /// and a save keeps it for the fields it does not replace.
    #[test]
    fn a_published_document_wins_over_local_and_legacy_details() {
        assert_eq!(
            shown_contact_details(
                Some(details("published")),
                Some(details("local")),
                true,
                Some(details("legacy"))
            ),
            details("published")
        );
        assert_eq!(
            shown_contact_details(
                Some(ContactDetails::default()),
                None,
                false,
                Some(details("legacy"))
            ),
            ContactDetails::default(),
            "details cleared on another device stay cleared"
        );
    }

    #[test]
    fn an_empty_published_document_counts_as_held_details() {
        assert!(holds_details(Some(&ContactDetails::default()), None));
    }

    fn request(document_id: u8, owner: Identifier, to: Identifier) -> (Identifier, Document) {
        use dash_sdk::dpp::document::DocumentV0;
        let id = Identifier::from([document_id; 32]);
        let document = Document::V0(DocumentV0 {
            id,
            owner_id: owner,
            properties: std::collections::BTreeMap::from([(
                "toUserId".to_string(),
                Value::Identifier(to.to_buffer()),
            )]),
            ..Default::default()
        });
        (id, document)
    }

    /// SEC-001: after the owner withdraws their request, a reply from the
    /// other person does not make them a contact.
    #[test]
    fn a_reply_to_a_withdrawn_request_is_not_a_contact() {
        let owner = Identifier::from([1; 32]);
        let bob = Identifier::from([2; 32]);
        let carol = Identifier::from([3; 32]);
        let incoming = [request(10, bob, owner), request(11, carol, owner)];
        let outgoing = [request(20, owner, bob), request(21, owner, carol)];

        let contacts = mutual_contacts(&incoming, &outgoing, |peer| *peer == bob);

        assert_eq!(contacts, HashSet::from([carol]));
    }

    /// Known-answer vectors generated independently with Python
    /// `cryptography` 46 (OpenSSL). They pin DIP-15 interoperability with other
    /// DashPay clients across `cbc` / `aes` / `aes-gcm` bumps.
    const GOLDEN_KEY: [u8; 32] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f,
    ];

    #[test]
    fn decrypt_private_data_opens_golden_blob() {
        // IV (10..1f) ‖ AES-256-CBC-PKCS7(GOLDEN_KEY, IV, "DashPay private data!").
        let blob = hex::decode(
            "101112131415161718191a1b1c1d1e1f\
             faea994a146288d4cdeb1833de096de0dbc32bac4771e7d99338f76072f8700f",
        )
        .expect("blob hex");
        let plaintext = decrypt_private_data(&blob, &GOLDEN_KEY).expect("golden blob must decrypt");
        assert_eq!(plaintext, b"DashPay private data!");
    }

    #[test]
    fn decrypt_to_user_id_opens_golden_ecb_vector() {
        // AES-256-ECB(GOLDEN_KEY, 40..5f).
        let encrypted =
            hex::decode("a37edf3f975abaef937b62c78d5bb157974b412738e50f45c7f9db25413f274b")
                .expect("ciphertext hex");
        let expected: [u8; 32] = std::array::from_fn(|i| 0x40 + i as u8);
        assert_eq!(
            decrypt_to_user_id(&encrypted, &GOLDEN_KEY).expect("decrypt"),
            expected
        );
    }
}
