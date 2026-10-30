use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::{ContactInfoReadError, TaskError};
use crate::context::AppContext;
use crate::model::dashpay::{
    AcceptedAccounts, ContactInfoField, ContactInfoUpdate, UnreadableContactInfoPolicy,
    validate_contact_note,
};
use crate::model::document::new_document_for_creation;
use crate::model::qualified_identity::QualifiedIdentity;
use aes_gcm::aes::Aes256;
use aes_gcm::aes::cipher::{BlockCipherEncrypt, KeyInit};
use bip39::rand::{SeedableRng, rngs::StdRng};
use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::{Bytes32, Value};
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::documents::transitions::DocumentCreateTransitionBuilder;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use dash_sdk::query_types::Documents;
use platform_wallet::wallet::identity::crypto::ContactInfoPrivateData as UpstreamPrivateData;
use platform_wallet::wallet::identity::crypto::contact_info::encode_private_data_bounded;
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use zeroize::Zeroizing;

// ContactInfo private data structure
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    /// The plaintext floor platform-wallet pads to with zero bytes, so that
    /// IV (16) + AES-CBC ciphertext reaches the contract's 48-byte minimum.
    const MIN_PLAINTEXT_SIZE: usize = 17;

    /// The plaintext size older versions of this client (up to
    /// v1.0.0-weekly.20261020) padded to: a `0x00` sentinel, then random bytes.
    const LEGACY_PLAINTEXT_SIZE: usize = 16;

    /// Encode for encryption with platform-wallet's DIP-15 codec, so every
    /// client that uses it reads what this one writes.
    pub fn serialize(&self) -> Result<Vec<u8>, TaskError> {
        let alias_len = self.alias_name.as_deref().map_or(0, str::len);
        if alias_len > u8::MAX as usize {
            return Err(DashPayError::ContactInfoValidationFailed {
                errors: vec![format!("Nickname too long ({alias_len} bytes, max 255)")],
            }
            .into());
        }
        validate_contact_note(self.note.as_deref().unwrap_or_default())?;
        encode_private_data_bounded(&UpstreamPrivateData {
            alias_name: self.alias_name.clone(),
            note: self.note.clone(),
            display_hidden: self.display_hidden,
            accepted_accounts: self.accepted_accounts.clone(),
        })
        .map_err(|source| TaskError::DashPayContactInfoEncoding {
            source: Box::new(source),
        })
    }

    /// Parse the plaintext produced by [`serialize`](Self::serialize), or by
    /// an older version of this client.
    ///
    /// Returns `None` for a truncated or non-canonical v0 payload, invalid
    /// UTF-8, or an unsupported version, and for trailing bytes that are not
    /// padding (see [`Self::is_padding`]).
    /// Stricter than platform-wallet's reader: a write must never drop fields
    /// it cannot read.
    ///
    /// Also returns `None` for bytes that read completely both ways with
    /// different values (see [`PrivateDataReading::Ambiguous`]), so a save
    /// asks before overwriting them instead of persisting a guess.
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        match Self::read(bytes) {
            PrivateDataReading::Read(data) => Some(data),
            PrivateDataReading::Ambiguous | PrivateDataReading::Unreadable => None,
        }
    }

    /// Read `bytes` both as CompactSize and as the single-byte layout older
    /// versions wrote, and keep a result only when it is unambiguous.
    fn read(bytes: &[u8]) -> PrivateDataReading {
        match (
            Self::parse(bytes, LengthPrefix::SingleByte),
            Self::parse(bytes, LengthPrefix::CompactSize),
        ) {
            (Some(legacy), Some(compact)) if legacy != compact => PrivateDataReading::Ambiguous,
            (Some(data), _) | (None, Some(data)) => PrivateDataReading::Read(data),
            (None, None) => PrivateDataReading::Unreadable,
        }
    }

    fn parse(bytes: &[u8], prefix: LengthPrefix) -> Option<Self> {
        let version = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
        if version != 0 {
            return None;
        }
        let mut pos = 4;
        // Whether a single-byte length took a value CompactSize reserves.
        let mut reserved_length = false;

        let mut take_length = |pos: &mut usize| -> Option<usize> {
            let first = *bytes.get(*pos)?;
            if prefix == LengthPrefix::SingleByte {
                reserved_length |= first >= 0xFD;
                *pos += 1;
                return Some(first.into());
            }
            // A Bitcoin CompactSize length, in its shortest encoding only.
            let field = |width: usize| bytes.get(*pos + 1..*pos + 1 + width);
            let (value, width): (u64, usize) = match first {
                0xFD => (u16::from_le_bytes(field(2)?.try_into().ok()?).into(), 3),
                0xFE => (u32::from_le_bytes(field(4)?.try_into().ok()?).into(), 5),
                0xFF => (u64::from_le_bytes(field(8)?.try_into().ok()?), 9),
                n => (n.into(), 1),
            };
            let shortest = match width {
                3 => 0xFD,
                5 => 0x1_0000,
                9 => 0x1_0000_0000,
                _ => 0,
            };
            if value < shortest {
                return None;
            }
            *pos += width;
            usize::try_from(value).ok()
        };
        let mut take_string = |pos: &mut usize| -> Option<Option<String>> {
            let len = take_length(pos)?;
            let raw = bytes.get(*pos..pos.checked_add(len)?)?;
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

        let count = take_length(&mut pos)?;
        let accepted_accounts = (0..count)
            .map(|_| {
                let raw = bytes.get(pos..pos + 4)?;
                pos += 4;
                Some(u32::from_le_bytes(raw.try_into().ok()?))
            })
            .collect::<Option<Vec<u32>>>()?;

        let complete = match prefix {
            LengthPrefix::CompactSize => Self::is_padding(bytes, pos),
            // A payload without a reserved length reads the same both ways, so
            // it is left to the CompactSize reader. One with such a length is
            // over 255 bytes, which older versions never padded.
            LengthPrefix::SingleByte => reserved_length && pos == bytes.len(),
        };
        if !complete {
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

    /// Whether `bytes[end..]`, after the last v0 field, is padding a known
    /// writer adds: none, platform-wallet's zeros up to
    /// [`Self::MIN_PLAINTEXT_SIZE`], or this client's former `0x00` sentinel
    /// and random bytes up to exactly [`Self::LEGACY_PLAINTEXT_SIZE`].
    fn is_padding(bytes: &[u8], end: usize) -> bool {
        match bytes[end..].split_first() {
            None => true,
            Some((0, _)) if bytes.len() == Self::LEGACY_PLAINTEXT_SIZE => true,
            Some(_) => {
                bytes.len() <= Self::MIN_PLAINTEXT_SIZE && bytes[end..].iter().all(|b| *b == 0)
            }
        }
    }
}

/// How a payload encodes string lengths and the account count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LengthPrefix {
    /// DIP-15 Bitcoin CompactSize, in its shortest encoding.
    CompactSize,
    /// One byte, which older versions of this client wrote for lengths up to
    /// 255. It differs from CompactSize only for 253 to 255, the CompactSize
    /// prefix markers, so it is accepted only when such a length is present
    /// and the payload ends exactly after the last field.
    ///
    /// Some bytes read completely both ways, with different values: a legacy
    /// payload can also parse as CompactSize, and a 256+ byte field from
    /// another client can also form a legacy payload. Such bytes are
    /// [`PrivateDataReading::Ambiguous`]. What this client writes never is:
    /// a 253 to 255-byte field is `FD xx 00` with `xx` in `FD..=FF`, never
    /// valid UTF-8 as the first text byte, and a 253+ account count never
    /// totals the same bytes both ways.
    SingleByte,
}

/// The outcome of reading a decrypted `privateData` payload.
#[derive(Debug)]
enum PrivateDataReading {
    /// Exactly one reading succeeded, or both agree.
    Read(ContactInfoPrivateData),
    /// The CompactSize and single-byte readings both succeed with different
    /// values, so neither can be trusted.
    Ambiguous,
    /// Neither reading succeeds.
    Unreadable,
}

/// Decode a decrypted `privateData` payload for display.
///
/// This client's reader comes first, so the contact list shows every payload
/// a save can read with the same values, including the long nicknames and
/// notes older versions of this client wrote, which platform-wallet can
/// misread. An ambiguous payload is shown as unreadable, never as a guess.
/// platform-wallet's tolerant reader then covers what neither reading
/// accepts and a save will not rewrite, such as a newer minor version.
pub(super) fn decode_contact_info_details(plaintext: &[u8]) -> Option<UpstreamPrivateData> {
    match ContactInfoPrivateData::read(plaintext) {
        PrivateDataReading::Read(data) => Some(UpstreamPrivateData {
            alias_name: data.alias_name,
            note: data.note,
            display_hidden: data.display_hidden,
            accepted_accounts: data.accepted_accounts,
        }),
        PrivateDataReading::Ambiguous => None,
        PrivateDataReading::Unreadable => {
            platform_wallet::wallet::identity::crypto::decode_private_data(plaintext).ok()
        }
    }
}

/// Apply an explicit whole-document update to the payload already on Platform.
fn apply_contact_info_update(
    update: ContactInfoUpdate,
    existing: Option<&Document>,
    private_data_key: &[u8; 32],
) -> Result<ContactInfoPrivateData, TaskError> {
    let preserves_existing = matches!(update.display_hidden, ContactInfoField::Preserve)
        || matches!(update.nickname, ContactInfoField::Preserve)
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
    if let ContactInfoField::Replace(hidden) = update.display_hidden {
        data.display_hidden = hidden;
    }
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
fn encrypt_to_user_id(user_id: &[u8; 32], key: &[u8; 32]) -> Result<[u8; 32], String> {
    let cipher = Aes256::new(key.into());

    // The 32-byte ID is exactly two 16-byte AES blocks; encrypt each in place (ECB).
    let mut encrypted = *user_id;
    let (blocks, _) = encrypted.as_chunks_mut::<16>();
    for block in blocks {
        cipher.encrypt_block(block.into());
    }

    Ok(encrypted)
}

/// Decrypt toUserId using AES-256-ECB as specified by DIP-0015.
///
/// See `encrypt_to_user_id` for the rationale behind ECB mode usage per DIP-0015.
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aes::cipher::BlockCipherDecrypt;

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
        .encrypt_padded::<Pkcs7>(&mut buffer, data.len())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    // Combine IV and encrypted data
    let mut result = Vec::with_capacity(16 + encrypted.len());
    result.extend_from_slice(&iv);
    result.extend_from_slice(encrypted);

    Ok(result)
}

struct ExistingContactInfo {
    document: Document,
    derivation_index: u32,
    enc_user_id_key: Zeroizing<[u8; 32]>,
    private_data_key: Zeroizing<[u8; 32]>,
}

struct ContactInfoLookup {
    existing: Option<ExistingContactInfo>,
    next_derivation_index: u32,
}

type ContactInfoKeys = (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>);

const CONTACT_INFO_PAGE_SIZE: usize = 100;

fn next_contact_info_page(docs: &Documents) -> Option<Start> {
    (docs.len() == CONTACT_INFO_PAGE_SIZE)
        .then(|| docs.keys().last())
        .flatten()
        .map(|last_id| Start::StartAfter(last_id.to_buffer().to_vec()))
}

pub(super) async fn fetch_all_contact_info_documents<F, Fut>(
    mut query: DocumentQuery,
    mut fetch_page: F,
) -> Result<Documents, TaskError>
where
    F: FnMut(DocumentQuery) -> Fut,
    Fut: Future<Output = Result<Documents, TaskError>>,
{
    query.limit = CONTACT_INFO_PAGE_SIZE as u32;
    let mut documents = Documents::new();

    loop {
        let page = fetch_page(query.clone()).await?;
        let next_page = next_contact_info_page(&page);
        documents.extend(page);

        match next_page {
            Some(start) => query.start = Some(start),
            None => break,
        }
    }

    Ok(documents)
}

fn contact_info_derivation_index(document: &Document) -> Option<u32> {
    document
        .properties()
        .get("derivationEncryptionKeyIndex")
        .and_then(|value| value.to_integer::<u32>().ok())
}

fn has_contact_info_key_material(document: &Document) -> bool {
    contact_info_derivation_index(document).is_some()
        && document
            .properties()
            .get("rootEncryptionKeyIndex")
            .and_then(|value| value.to_integer::<u32>().ok())
            .is_some()
        && document
            .properties()
            .get("encToUserId")
            .is_some_and(|value| value.as_bytes_slice().is_ok())
}

fn find_existing_contact_info<F>(
    documents: Vec<Document>,
    contact_user_id: Identifier,
    mut derive_keys: F,
) -> Option<ExistingContactInfo>
where
    F: FnMut(u32) -> Result<ContactInfoKeys, TaskError>,
{
    // Documents come oldest first; the newest match is the one the contacts
    // load shows, and a rewrite keeps it the newest.
    for document in documents.into_iter().rev() {
        let Some(derivation_index) = contact_info_derivation_index(&document) else {
            continue;
        };
        if !has_contact_info_key_material(&document) {
            continue;
        }

        let Ok((enc_user_id_key, private_data_key)) = derive_keys(derivation_index) else {
            continue;
        };
        // Platform returns the fixed 32-byte field as `Value::Bytes32`.
        let Some(enc_user_id) = document
            .properties()
            .get("encToUserId")
            .and_then(|value| value.as_bytes_slice().ok())
        else {
            continue;
        };
        if decrypt_to_user_id(enc_user_id, &enc_user_id_key).ok()
            == Some(contact_user_id.to_buffer())
        {
            return Some(ExistingContactInfo {
                document,
                derivation_index,
                enc_user_id_key,
                private_data_key,
            });
        }
    }

    None
}

async fn scan_contact_info_documents<F, Fut>(
    documents: Vec<Document>,
    contact_user_id: Identifier,
    with_key_session: F,
) -> Result<ContactInfoLookup, TaskError>
where
    F: FnOnce(Vec<Document>, Identifier) -> Fut,
    Fut: Future<Output = Result<Option<ExistingContactInfo>, TaskError>>,
{
    let next_derivation_index = documents
        .iter()
        .filter_map(contact_info_derivation_index)
        .fold(0u32, |next, index| next.max(index.saturating_add(1)));
    let existing = if documents.iter().any(has_contact_info_key_material) {
        with_key_session(documents, contact_user_id).await?
    } else {
        None
    };

    Ok(ContactInfoLookup {
        existing,
        next_derivation_index,
    })
}

async fn lookup_contact_info(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
    contact_user_id: Identifier,
) -> Result<ContactInfoLookup, TaskError> {
    let dashpay_contract = app_context.dashpay_contract();
    let identity_id = identity.identity.id();
    let mut query = DocumentQuery::new(dashpay_contract, "contactInfo").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay contactInfo",
            source: Box::new(e.into()),
        }
    })?;
    query = query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    query = query.with_order_by(OrderClause {
        field: "$updatedAt".to_string(),
        ascending: true,
    });
    let existing_docs = fetch_all_contact_info_documents(query, |page_query| async move {
        Document::fetch_many(sdk, page_query)
            .await
            .map_err(TaskError::from)
    })
    .await?;
    let documents = existing_docs.into_values().flatten().collect();
    scan_contact_info_documents(
        documents,
        contact_user_id,
        |documents, contact_user_id| async move {
            let seed_hash = identity
                .dashpay_wallet_seed_hash()
                .ok_or(TaskError::ContactWalletSeedUnavailable)?;
            let network = identity.network;

            app_context
                .wallet_backend()?
                .secret_access()
                .with_secret_session(
                    &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                    async |session| {
                        let plaintext = session.plaintext();
                        let seed = plaintext
                            .expose_hd_seed()
                            .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                        Ok(find_existing_contact_info(
                            documents,
                            contact_user_id,
                            |derivation_index| {
                                crate::wallet_backend::derive_contact_info_encryption_keys(
                                    seed,
                                    network,
                                    derivation_index,
                                )
                            },
                        ))
                    },
                )
                .await
        },
    )
    .await
}

pub(super) async fn contact_info_is_hidden(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
    contact_user_id: Identifier,
    unreadable: UnreadableContactInfoPolicy,
) -> Result<bool, TaskError> {
    let lookup = lookup_contact_info(app_context, sdk, identity, contact_user_id).await?;
    let Some(existing) = lookup.existing else {
        return Ok(false);
    };
    Ok(read_contact_info_private_data(
        Some(&existing.document),
        &existing.private_data_key,
        unreadable,
    )?
    .display_hidden)
}

/// Write the `contactInfo` document for `contact_user_id`, creating it when the
/// identity has none yet and replacing it otherwise.
///
/// The document is written whole, so `update` explicitly chooses which fields
/// are preserved and which fields are replaced. Nicknames and notes are limited
/// to 255 UTF-8 bytes; the encoded payload must also fit the contract size limit.
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
    let write_guard = app_context
        .contact_info_writes
        .acquire(identity.identity.id())
        .await;
    create_or_update_contact_info_locked(
        app_context,
        sdk,
        identity,
        contact_user_id,
        update,
        &write_guard,
    )
    .await
}

/// Write while the caller holds the owner's lock across its reads and decisions.
/// Request actions acquire their request lock before this owner lock.
pub(super) async fn create_or_update_contact_info_locked(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_user_id: Identifier,
    update: ContactInfoUpdate,
    _write_guard: &tokio::sync::OwnedMutexGuard<()>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let dashpay_contract = app_context.dashpay_contract();
    let identity_id = identity.identity.id();
    let lookup = lookup_contact_info(app_context, sdk, &identity, contact_user_id).await?;
    let (found_existing_doc, derivation_index, enc_user_id_key, private_data_key) =
        match lookup.existing {
            Some(existing) => (
                Some(existing.document),
                existing.derivation_index,
                existing.enc_user_id_key,
                existing.private_data_key,
            ),
            None => {
                let derivation_index = lookup.next_derivation_index;
                let (enc_user_id_key, private_data_key) =
                    derive_contact_info_keys(app_context, &identity, derivation_index).await?;
                (None, derivation_index, enc_user_id_key, private_data_key)
            }
        };

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
        let entropy = Bytes32::random_with_rng(&mut rng).to_buffer();
        let document_type = dashpay_contract
            .document_type_for_name("contactInfo")
            .map_err(|e| TaskError::DocumentPreparationFailed {
                source: Box::new(e.into()),
            })?;
        let document = new_document_for_creation(
            document_type,
            identity_id,
            properties,
            &entropy,
            sdk.version(),
        )
        .map_err(|source| TaskError::DocumentPreparationFailed { source })?;

        let mut builder = DocumentCreateTransitionBuilder::new(
            dashpay_contract,
            "contactInfo".to_string(),
            document,
            entropy,
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
    use dash_sdk::dpp::document::{Document as DppDocument, DocumentV0};
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
    use dash_sdk::dpp::version::PlatformVersion;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::future::ready;

    const KEY: [u8; 32] = [7u8; 32];
    const OTHER_KEY: [u8; 32] = [9u8; 32];

    /// DIP-15 `encToUserId` known-answer vector: AES-256-ECB(key = 00..1f,
    /// pt = 40..5f), generated independently with Python `cryptography` 46
    /// (OpenSSL). Pins interoperability across `aes` / `aes-gcm` bumps.
    #[test]
    fn to_user_id_ecb_matches_golden_vector() {
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let user_id: [u8; 32] = std::array::from_fn(|i| 0x40 + i as u8);
        let expected = "a37edf3f975abaef937b62c78d5bb157974b412738e50f45c7f9db25413f274b";

        let encrypted = encrypt_to_user_id(&user_id, &key).expect("encrypt");
        assert_eq!(hex::encode(encrypted), expected);
        let decrypted = decrypt_to_user_id(&encrypted, &key).expect("decrypt");
        assert_eq!(decrypted, user_id);
    }

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    #[test]
    fn a_full_contact_info_page_produces_a_cursor_for_reconciliation() {
        let mut docs = Documents::new();
        for byte in 0..CONTACT_INFO_PAGE_SIZE as u8 {
            docs.insert(id(byte), None);
        }

        assert_eq!(
            next_contact_info_page(&docs),
            Some(Start::StartAfter(id(99).to_buffer().to_vec()))
        );
        docs.shift_remove(&id(99));
        assert_eq!(next_contact_info_page(&docs), None);
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
            moderated_at: None,
            moderated_by: None,
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
            contract_version: None,
        })
    }

    fn lookup_contact_info_document(
        document_id: u8,
        derivation_index: u32,
        encrypted_user_id: [u8; 32],
    ) -> Document {
        let properties = BTreeMap::from([
            (
                "derivationEncryptionKeyIndex".to_string(),
                Value::U32(derivation_index),
            ),
            ("rootEncryptionKeyIndex".to_string(), Value::U32(0)),
            (
                "encToUserId".to_string(),
                Value::Bytes(encrypted_user_id.to_vec()),
            ),
        ]);
        DppDocument::V0(DocumentV0 {
            moderated_at: None,
            moderated_by: None,
            id: id(document_id),
            owner_id: id(254),
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
            contract_version: None,
        })
    }

    #[tokio::test]
    async fn contact_info_scan_uses_one_secret_session_for_many_documents() {
        let contact_user_id = id(200);
        let encrypted_other_id = encrypt_to_user_id(&id(201).to_buffer(), &KEY).expect("encrypt");
        let documents = (0..=CONTACT_INFO_PAGE_SIZE)
            .map(|index| {
                lookup_contact_info_document(index as u8, index as u32, encrypted_other_id)
            })
            .collect();
        let sessions = Cell::new(0);
        let derivations = Cell::new(0);

        let lookup =
            scan_contact_info_documents(documents, contact_user_id, |documents, target| {
                sessions.set(sessions.get() + 1);
                ready(Ok(find_existing_contact_info(documents, target, |_| {
                    derivations.set(derivations.get() + 1);
                    Ok((Zeroizing::new(KEY), Zeroizing::new(OTHER_KEY)))
                })))
            })
            .await
            .expect("scan");

        assert!(lookup.existing.is_none());
        assert_eq!(lookup.next_derivation_index, 101);
        assert_eq!(derivations.get(), 101);
        assert_eq!(sessions.get(), 1, "the entire scan uses one unlock session");
    }

    #[test]
    fn contact_info_scan_stops_deriving_after_target_match() {
        let contact_user_id = id(210);
        let encrypted_target =
            encrypt_to_user_id(&contact_user_id.to_buffer(), &KEY).expect("encrypt target");
        let encrypted_other =
            encrypt_to_user_id(&id(211).to_buffer(), &KEY).expect("encrypt other");
        // Newest last: the scan starts from the newest document.
        let documents = vec![
            lookup_contact_info_document(2, 4, encrypted_other),
            lookup_contact_info_document(1, 3, encrypted_target),
        ];
        let mut derived_indices = Vec::new();

        let existing = find_existing_contact_info(documents, contact_user_id, |index| {
            derived_indices.push(index);
            Ok((Zeroizing::new(KEY), Zeroizing::new(OTHER_KEY)))
        });

        assert!(existing.is_some());
        assert_eq!(derived_indices, vec![3]);
    }

    #[tokio::test]
    async fn contact_info_scan_skips_late_hardened_index_and_keeps_high_water_mark() {
        const SEED: [u8; 64] = [42; 64];
        const MATCH_INDEX: u32 = 7;
        const UNDERIVABLE_INDEX: u32 = 1 << 31;

        let contact_user_id = id(220);
        let (target_key, _) = crate::wallet_backend::derive_contact_info_encryption_keys(
            &SEED,
            dash_sdk::dpp::dashcore::Network::Testnet,
            MATCH_INDEX,
        )
        .expect("derive target key");
        let encrypted_target =
            encrypt_to_user_id(&contact_user_id.to_buffer(), &target_key).expect("encrypt target");
        let documents = vec![
            lookup_contact_info_document(1, MATCH_INDEX, encrypted_target),
            lookup_contact_info_document(2, UNDERIVABLE_INDEX, [0; 32]),
        ];
        let sessions = Cell::new(0);

        let lookup =
            scan_contact_info_documents(documents, contact_user_id, |documents, target| {
                sessions.set(sessions.get() + 1);
                ready(Ok(find_existing_contact_info(documents, target, |index| {
                    crate::wallet_backend::derive_contact_info_encryption_keys(
                        &SEED,
                        dash_sdk::dpp::dashcore::Network::Testnet,
                        index,
                    )
                })))
            })
            .await
            .expect("the late underivable document must not abort the matched lookup");

        assert_eq!(
            lookup.existing.map(|existing| existing.derivation_index),
            Some(MATCH_INDEX)
        );
        assert_eq!(
            lookup.next_derivation_index,
            UNDERIVABLE_INDEX + 1,
            "the on-chain coordinate still advances the public high-water mark"
        );
        assert_eq!(sessions.get(), 1);
    }

    #[tokio::test]
    async fn contact_info_fetch_walks_every_page() {
        let contract =
            load_system_data_contract(SystemDataContract::Dashpay, PlatformVersion::latest())
                .expect("DashPay contract fixture");
        let query = DocumentQuery::new(contract, "contactInfo").expect("contactInfo query");
        let first_page_last_id = id(100);
        let first_page = (1..=100)
            .map(|byte| (id(byte), None))
            .collect::<Documents>();
        let second_page = [(id(101), None)].into_iter().collect::<Documents>();
        let mut pages = VecDeque::from([first_page, second_page]);
        let mut calls = 0;

        let documents = fetch_all_contact_info_documents(query, |query| {
            calls += 1;
            match calls {
                1 => assert!(query.start.is_none()),
                2 => assert!(matches!(
                    query.start,
                    Some(Start::StartAfter(ref cursor))
                        if cursor == &first_page_last_id.to_buffer().to_vec()
                )),
                _ => panic!("the short second page must end pagination"),
            }
            ready(Ok::<_, TaskError>(
                pages.pop_front().expect("a page for every fetch"),
            ))
        })
        .await
        .expect("all pages fetched");

        assert_eq!(calls, 2);
        assert_eq!(documents.len(), 101);
        assert_eq!(documents.keys().last().copied(), Some(id(101)));
    }

    /// DIP-15 writes string lengths as CompactSize, where 253 and longer take a
    /// three-byte prefix. Other platform-wallet clients must read what this
    /// client writes, and this client what they write.
    #[test]
    fn contact_details_round_trip_with_the_platform_wallet_codec() {
        use platform_wallet::wallet::identity::crypto as upstream;
        for len in [0, 252, 253, 255] {
            let text = (len > 0).then(|| "x".repeat(len));
            let ours = ContactInfoPrivateData {
                version: 0,
                alias_name: text.clone(),
                note: text.clone(),
                display_hidden: true,
                accepted_accounts: vec![0, 3],
            };
            let theirs = upstream::ContactInfoPrivateData {
                alias_name: text.clone(),
                note: text.clone(),
                display_hidden: true,
                accepted_accounts: vec![0, 3],
            };

            let decoded = upstream::decode_private_data(&ours.serialize().expect("serialize"));
            assert_eq!(
                decoded.ok(),
                Some(theirs.clone()),
                "platform-wallet reads {len} bytes"
            );

            let parsed =
                ContactInfoPrivateData::deserialize(&upstream::encode_private_data(&theirs))
                    .unwrap_or_else(|| {
                        panic!("this client reads {len} bytes from platform-wallet")
                    });
            assert_eq!(parsed.alias_name, text);
            assert_eq!(parsed.note, text);
            assert!(parsed.display_hidden);
            assert_eq!(parsed.accepted_accounts, vec![0, 3]);
        }
    }

    #[test]
    fn contact_info_fee_estimate_covers_preserved_accounts() {
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimator = PlatformFeeEstimator::new();
        for len in [0, 252, 253, 255] {
            for accounts in [0, 1, 16, 252, 253, 400, 500] {
                let text = (len > 0).then(|| "x".repeat(len));
                let data = ContactInfoPrivateData {
                    alias_name: text.clone(),
                    note: text,
                    accepted_accounts: (0..accounts).collect(),
                    ..Default::default()
                };
                let Ok(encoded) = data.serialize() else {
                    continue;
                };
                let encrypted = encrypt_private_data(&encoded, &KEY).unwrap();
                assert!(
                    estimator.estimate_contact_info_write()
                        >= estimator.estimate_document_create_with_size(112 + encrypted.len()),
                    "{len}-byte fields and {accounts} accounts"
                );
            }
        }
    }

    fn contact_details_page(documents: impl IntoIterator<Item = Document>) -> Documents {
        documents
            .into_iter()
            .map(|document| (document.id(), Some(document)))
            .collect()
    }

    /// A stored contactInfo for `contact` whose note platform-wallet wrote.
    fn saved_note_document(document_id: u8, contact: Identifier, note: &str) -> Document {
        use platform_wallet::wallet::identity::crypto as upstream;
        let user_id = encrypt_to_user_id(&contact.to_buffer(), &KEY).expect("encrypt");
        let mut document = lookup_contact_info_document(document_id, document_id.into(), user_id);
        let plaintext = upstream::encode_private_data(&upstream::ContactInfoPrivateData {
            note: Some(note.to_string()),
            ..Default::default()
        });
        let private_data = encrypt_private_data(&plaintext, &KEY).expect("encrypt");
        document.set("privateData", Value::Bytes(private_data));
        document
    }

    /// The details loaded from `pages`, and whether every page was asked for
    /// in the order the write lookup uses.
    async fn load_details(
        pages: Vec<Documents>,
    ) -> (
        std::collections::HashMap<Identifier, super::super::contacts::ContactData>,
        bool,
    ) {
        let contract =
            load_system_data_contract(SystemDataContract::Dashpay, PlatformVersion::latest())
                .expect("DashPay contract fixture");
        let query = DocumentQuery::new(contract, "contactInfo").expect("contactInfo query");
        let mut pages = VecDeque::from(pages);
        let mut ordered = true;
        let details = super::super::contacts::load_contact_details(
            query,
            |query| {
                ordered &= query
                    .order_by_clauses
                    .iter()
                    .any(|clause| clause.field == "$updatedAt" && clause.ascending);
                ready(Ok::<_, TaskError>(pages.pop_front().unwrap_or_default()))
            },
            |_| ready(Ok((Zeroizing::new(KEY), Zeroizing::new(KEY)))),
        )
        .await
        .expect("contact details load");
        (details, ordered)
    }

    /// A saved note can sit beyond the first page of the owner's contactInfo
    /// documents; it must not be read as "no note".
    #[tokio::test]
    async fn the_contact_details_load_reads_every_page() {
        let contact = id(201);
        let other = encrypt_to_user_id(&id(202).to_buffer(), &KEY).expect("encrypt");
        let first_page = contact_details_page(
            (1..=100).map(|byte| lookup_contact_info_document(byte, byte.into(), other)),
        );
        let note = "n".repeat(253);
        let second_page = contact_details_page([saved_note_document(101, contact, &note)]);

        let (details, ordered) = load_details(vec![first_page, second_page]).await;

        assert_eq!(
            details.get(&contact).and_then(|c| c.note.clone()),
            Some(note)
        );
        assert!(ordered, "pages follow the order the write lookup uses");
    }

    /// A contactInfo document as Platform returns it: DPP decodes the fixed
    /// 32-byte `encToUserId` as `Value::Bytes32`, whoever wrote the document.
    /// Its saved note must still load.
    #[tokio::test]
    async fn the_contact_details_load_reads_a_document_as_platform_returns_it() {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::document::serialization_traits::DocumentPlatformConversionMethodsV0;
        let platform_version = PlatformVersion::latest();
        let contract = load_system_data_contract(SystemDataContract::Dashpay, platform_version)
            .expect("DashPay contract fixture");
        let document_type = contract
            .document_type_for_name("contactInfo")
            .expect("contactInfo type");
        let contact = id(201);
        let mut written = saved_note_document(1, contact, "kept");
        written.set_created_at(Some(1));
        written.set_updated_at(Some(1));
        let bytes = written
            .serialize(document_type, &contract, platform_version)
            .expect("serialize");
        let returned =
            Document::from_bytes(&bytes, document_type, platform_version).expect("deserialize");
        assert!(
            matches!(
                returned.properties().get("encToUserId"),
                Some(Value::Bytes32(_))
            ),
            "precondition: Platform returns encToUserId as Bytes32"
        );

        let (details, _) = load_details(vec![contact_details_page([returned])]).await;

        assert_eq!(
            details.get(&contact).and_then(|c| c.note.clone()),
            Some("kept".to_string())
        );
    }

    /// Stored details that cannot be decrypted or decoded are not "no
    /// details": the contact is marked unreadable and kept hidden, since its
    /// hidden flag cannot be read.
    #[tokio::test]
    async fn unreadable_contact_details_keep_the_contact_hidden() {
        let undecryptable = id(201);
        let undecodable = id(202);
        let mut first = lookup_contact_info_document(
            1,
            1,
            encrypt_to_user_id(&undecryptable.to_buffer(), &KEY).expect("encrypt"),
        );
        first.set("privateData", Value::Bytes(vec![0xA5; 48]));
        let mut second = lookup_contact_info_document(
            2,
            2,
            encrypt_to_user_id(&undecodable.to_buffer(), &KEY).expect("encrypt"),
        );
        second.set(
            "privateData",
            Value::Bytes(encrypt_private_data(&[0xFF; 40], &KEY).expect("encrypt")),
        );

        let (details, _) = load_details(vec![contact_details_page([first, second])]).await;

        for contact in [undecryptable, undecodable] {
            let shown = details.get(&contact).expect("the contact is listed");
            assert!(shown.details_unreadable, "{contact} is marked unreadable");
            assert!(shown.is_hidden, "{contact} stays hidden");
            assert_eq!(shown.note, None);
        }
    }

    /// Readable details are not marked unreadable.
    #[tokio::test]
    async fn readable_contact_details_are_not_marked_unreadable() {
        let contact = id(201);
        let (details, _) = load_details(vec![contact_details_page([saved_note_document(
            1, contact, "n",
        )])])
        .await;

        assert!(!details.get(&contact).expect("listed").details_unreadable);
    }

    /// With two documents for one contact, the load shows the newest, the
    /// one a save rewrites. A rewrite keeps it the newest, so the load and
    /// the save stay on the same document; with the oldest, a rewrite would
    /// move it to the end and the load would show the untouched one.
    #[tokio::test]
    async fn the_contact_details_load_shows_the_document_a_save_rewrites() {
        let contact = id(201);
        let page = contact_details_page([
            saved_note_document(1, contact, "stale"),
            saved_note_document(2, contact, "kept"),
        ]);

        let (details, _) = load_details(vec![page]).await;

        assert_eq!(
            details.get(&contact).and_then(|c| c.note.as_deref()),
            Some("kept")
        );

        let documents = vec![
            saved_note_document(1, contact, "stale"),
            saved_note_document(2, contact, "kept"),
        ];
        let existing = find_existing_contact_info(documents, contact, |_| {
            Ok((Zeroizing::new(KEY), Zeroizing::new(KEY)))
        });
        assert_eq!(existing.map(|existing| existing.derivation_index), Some(2));
    }

    /// Platform returns the fixed 32-byte `encToUserId` as `Value::Bytes32`;
    /// a save must still find the contact's document instead of creating a
    /// second one.
    #[tokio::test]
    async fn a_save_finds_a_document_as_platform_returns_it() {
        let contact = id(230);
        let encrypted = encrypt_to_user_id(&contact.to_buffer(), &KEY).expect("encrypt");
        let mut document = lookup_contact_info_document(1, 5, encrypted);
        document.set("encToUserId", Value::Bytes32(encrypted));

        let lookup = scan_contact_info_documents(vec![document], contact, |documents, target| {
            ready(Ok(find_existing_contact_info(documents, target, |_| {
                Ok((Zeroizing::new(KEY), Zeroizing::new(OTHER_KEY)))
            })))
        })
        .await
        .expect("scan");

        assert_eq!(
            lookup.existing.map(|existing| existing.derivation_index),
            Some(5)
        );
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
            alias_name: Some("123456789".to_string()),
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

    /// Only the 16-byte layout older versions of this client wrote may hold
    /// random bytes after the `0x00` sentinel. Padding of any other length
    /// must be all zeros.
    #[test]
    fn contact_info_padding_rejects_nonzero_bytes_after_the_sentinel() {
        let canonical = ContactInfoPrivateData::new()
            .serialize()
            .expect("serialize");
        for length in (9..=ContactInfoPrivateData::MIN_PLAINTEXT_SIZE)
            .filter(|length| *length != ContactInfoPrivateData::LEGACY_PLAINTEXT_SIZE)
        {
            assert!(ContactInfoPrivateData::deserialize(&canonical[..length]).is_some());
            for offset in 9..length {
                let mut malformed = canonical[..length].to_vec();
                malformed[offset] = 42;
                assert!(
                    ContactInfoPrivateData::deserialize(&malformed).is_none(),
                    "nonzero padding at {offset} in {length} bytes must require confirmation"
                );
            }
        }
    }

    /// Byte-exact copy of the serializer older versions of this client used
    /// (v1.0.0-weekly.20261020 and earlier, back to #810): single-byte
    /// lengths, and a payload under 16 bytes padded with `0x00` and then
    /// random bytes up to 16. `fill` stands in for the random bytes.
    fn legacy_det_plaintext(data: &ContactInfoPrivateData, fill: u8) -> Vec<u8> {
        let mut bytes = data.version.to_le_bytes().to_vec();
        for text in [&data.alias_name, &data.note] {
            let text = text.as_deref().unwrap_or_default().as_bytes();
            bytes.push(u8::try_from(text.len()).expect("the old writer capped at 255"));
            bytes.extend_from_slice(text);
        }
        bytes.push(u8::from(data.display_hidden));
        bytes.push(u8::try_from(data.accepted_accounts.len()).expect("at most 255"));
        for account in &data.accepted_accounts {
            bytes.extend_from_slice(&account.to_le_bytes());
        }
        if bytes.len() < ContactInfoPrivateData::LEGACY_PLAINTEXT_SIZE {
            bytes.push(0x00);
            bytes.resize(ContactInfoPrivateData::LEGACY_PLAINTEXT_SIZE, fill);
        }
        bytes
    }

    fn legacy_contact(
        alias_name: Option<&str>,
        note: Option<&str>,
        display_hidden: bool,
        accepted_accounts: Vec<u32>,
    ) -> ContactInfoPrivateData {
        ContactInfoPrivateData {
            version: 0,
            alias_name: alias_name.map(str::to_owned),
            note: note.map(str::to_owned),
            display_hidden,
            accepted_accounts,
        }
    }

    fn assert_same_details(parsed: &ContactInfoPrivateData, expected: &ContactInfoPrivateData) {
        assert_eq!(parsed.alias_name, expected.alias_name);
        assert_eq!(parsed.note, expected.note);
        assert_eq!(parsed.display_hidden, expected.display_hidden);
        assert_eq!(parsed.accepted_accounts, expected.accepted_accounts);
    }

    /// Details an older version of this client saved for a hidden contact, or
    /// with a short nickname or note, are padded with random bytes. A save
    /// must read them, not report them unreadable and offer to wipe them.
    #[test]
    fn a_save_reads_details_padded_by_older_versions() {
        let contacts = [
            legacy_contact(None, None, true, Vec::new()),
            legacy_contact(Some("Bob"), None, false, Vec::new()),
            legacy_contact(None, Some("hi"), true, Vec::new()),
            legacy_contact(Some("Al"), None, true, vec![0]),
            legacy_contact(Some("Bao"), Some("Met"), false, Vec::new()),
        ];
        for expected in &contacts {
            for fill in [0x01, 0x5A, 0xFF] {
                let plaintext = legacy_det_plaintext(expected, fill);
                assert_eq!(
                    plaintext.len(),
                    ContactInfoPrivateData::LEGACY_PLAINTEXT_SIZE
                );
                let stored = contact_info_document(Some(Value::Bytes(
                    encrypt_private_data(&plaintext, &KEY).expect("encrypt"),
                )));

                let parsed = read_contact_info_private_data(
                    Some(&stored),
                    &KEY,
                    UnreadableContactInfoPolicy::Abort,
                )
                .unwrap_or_else(|error| panic!("{expected:?} padded with {fill:#x}: {error:?}"));
                assert_same_details(&parsed, expected);

                let unhidden = apply_contact_info_update(
                    ContactInfoUpdate::visibility(false),
                    Some(&stored),
                    &KEY,
                )
                .expect("unhiding keeps the saved details");
                assert!(!unhidden.display_hidden);
                assert_eq!(unhidden.alias_name, expected.alias_name);
                assert_eq!(unhidden.note, expected.note);
                assert_eq!(unhidden.accepted_accounts, expected.accepted_accounts);
            }
        }
    }

    /// Older versions of this client wrote every length as one byte, up to
    /// 255. In CompactSize, 253 to 255 are prefix markers instead.
    #[test]
    fn a_save_reads_long_details_written_with_single_byte_lengths() {
        for len in [253, 254, 255] {
            let text = "x".repeat(len);
            for expected in [
                legacy_contact(Some(&text), None, false, vec![2]),
                legacy_contact(None, Some(&text), true, Vec::new()),
                legacy_contact(Some(&text), Some(&text), true, vec![0, 3]),
            ] {
                let parsed =
                    ContactInfoPrivateData::deserialize(&legacy_det_plaintext(&expected, 0xAA))
                        .unwrap_or_else(|| panic!("{len}-byte details must stay readable"));
                assert_same_details(&parsed, &expected);
            }
        }
    }

    /// The contact list reads the same older layouts a save reads, so such a
    /// contact is shown with its details instead of hidden as unreadable.
    #[test]
    fn the_contact_list_reads_details_written_by_older_versions() {
        let long = "y".repeat(254);
        for expected in [
            legacy_contact(None, None, true, Vec::new()),
            legacy_contact(Some("Bob"), Some("hi"), false, Vec::new()),
            legacy_contact(Some(&long), Some("Met"), false, vec![1]),
        ] {
            let shown = decode_contact_info_details(&legacy_det_plaintext(&expected, 0xC3))
                .unwrap_or_else(|| panic!("{expected:?} must be shown"));
            assert_eq!(shown.alias_name, expected.alias_name);
            assert_eq!(shown.note, expected.note);
            assert_eq!(shown.display_hidden, expected.display_hidden);
            assert_eq!(shown.accepted_accounts, expected.accepted_accounts);
        }
    }

    /// Tolerating the older layouts must not let corrupt payloads through.
    #[test]
    fn legacy_layouts_do_not_hide_corruption() {
        let hidden = legacy_contact(None, None, true, Vec::new());
        let legacy = legacy_det_plaintext(&hidden, 0x5A);

        let mut no_sentinel = legacy.clone();
        no_sentinel[8] = 0x5A;
        assert!(ContactInfoPrivateData::deserialize(&no_sentinel).is_none());

        for length in (10..legacy.len()).chain([legacy.len() + 1]) {
            let mut resized = legacy.clone();
            resized.resize(length, 0x5A);
            assert!(
                ContactInfoPrivateData::deserialize(&resized).is_none(),
                "random padding over {length} bytes is not a layout any client wrote"
            );
        }

        // A single-byte length that runs past the end of the payload.
        let mut overrun = legacy_det_plaintext(
            &legacy_contact(Some(&"x".repeat(253)), None, false, Vec::new()),
            0,
        );
        overrun.truncate(overrun.len() - 1);
        assert!(ContactInfoPrivateData::deserialize(&overrun).is_none());
        assert!(decode_contact_info_details(&overrun).is_none());

        // Single-byte lengths still need valid text and no trailing data.
        let long = legacy_det_plaintext(
            &legacy_contact(Some(&"x".repeat(253)), None, false, Vec::new()),
            0,
        );
        let mut invalid_utf8 = long.clone();
        invalid_utf8[5] = 0xFF;
        let mut extended = long.clone();
        extended.push(0);
        for corrupt in [invalid_utf8, extended] {
            assert!(ContactInfoPrivateData::deserialize(&corrupt).is_none());
            assert!(decode_contact_info_details(&corrupt).is_none());
        }
    }

    /// A 253-byte note older versions wrote, starting with bytes that a
    /// CompactSize reader takes as an in-range 8-byte length followed by a
    /// complete, shorter payload.
    fn control_character_note() -> String {
        let note = format!("\u{8}\u{0}abcdefgh\u{1}\u{0}{}", "x".repeat(241));
        assert_eq!(note.len(), 253);
        note
    }

    /// Whatever a save reads, the contact list shows with the same values,
    /// including legacy text with control characters, which platform-wallet
    /// reads as different fields.
    #[test]
    fn the_contact_list_shows_what_a_save_reads() {
        let mut payloads = Vec::new();
        for len in [0, 3, 252, 253, 254, 255] {
            let text = "x".repeat(len);
            let text = (len > 0).then_some(text.as_str());
            for hidden in [false, true] {
                let contact = legacy_contact(text, text, hidden, vec![0, 7]);
                payloads.push(legacy_det_plaintext(&contact, 0x9C));
                payloads.push(contact.serialize().expect("serialize"));
            }
        }
        let control = control_character_note();
        for contact in [
            legacy_contact(None, Some(&control), false, Vec::new()),
            legacy_contact(Some(&control), Some("Met"), true, vec![4]),
            legacy_contact(Some("\u{0}\u{1}"), Some(&control), false, vec![1]),
        ] {
            payloads.push(legacy_det_plaintext(&contact, 0));
            payloads.push(contact.serialize().expect("serialize"));
        }

        for payload in &payloads {
            let saved = ContactInfoPrivateData::deserialize(payload)
                .unwrap_or_else(|| panic!("a save must read {payload:02x?}"));
            let shown = decode_contact_info_details(payload)
                .unwrap_or_else(|| panic!("the contact list must show {payload:02x?}"));
            assert_eq!(shown.alias_name, saved.alias_name, "{payload:02x?}");
            assert_eq!(shown.note, saved.note, "{payload:02x?}");
            assert_eq!(shown.display_hidden, saved.display_hidden, "{payload:02x?}");
            assert_eq!(
                shown.accepted_accounts, saved.accepted_accounts,
                "{payload:02x?}"
            );
        }
    }

    /// Control characters in a long field must not let a damaged payload
    /// through the single-byte reading.
    #[test]
    fn control_characters_in_long_fields_do_not_hide_corruption() {
        let control = control_character_note();
        let long_control = format!("\u{0}\u{1}{}", "x".repeat(252));
        for contact in [
            legacy_contact(None, Some(&control), true, Vec::new()),
            legacy_contact(Some(&long_control), None, false, vec![2]),
            legacy_contact(Some(&control), Some(&long_control), false, Vec::new()),
        ] {
            for payload in [
                legacy_det_plaintext(&contact, 0),
                contact.serialize().expect("serialize"),
            ] {
                let mut appended = payload.clone();
                appended.push(0);
                let mut truncated = payload.clone();
                truncated.pop();
                for corrupt in [appended, truncated] {
                    assert!(
                        ContactInfoPrivateData::deserialize(&corrupt).is_none(),
                        "a save must not read {corrupt:02x?}"
                    );
                }
            }
        }
    }

    /// The single-byte reading only applies to a length CompactSize reserves;
    /// a malformed CompactSize prefix is not reinterpreted as a short length.
    #[test]
    fn single_byte_lengths_apply_only_to_reserved_values() {
        // A non-shortest CompactSize: 0xFD then 5, for 5 bytes of text.
        let mut bytes = vec![0, 0, 0, 0, 0xFD, 5, 0];
        bytes.extend_from_slice(b"hello");
        bytes.extend_from_slice(&[0, 0, 0]);
        assert!(ContactInfoPrivateData::deserialize(&bytes).is_none());

        // A truncated payload without reserved lengths stays rejected.
        let short = legacy_det_plaintext(&legacy_contact(Some("Bob"), None, false, vec![1]), 0);
        assert!(ContactInfoPrivateData::deserialize(&short[..short.len() - 2]).is_none());
    }

    /// An older version's 253-byte note starting with `03 01`, which a
    /// CompactSize reader takes as `FD 03 01`, a 259-byte length, followed by
    /// a complete payload with other values.
    fn legacy_note_that_also_reads_as_compact_size() -> Vec<u8> {
        let note = format!("\u{3}\u{1}{}", "x".repeat(251));
        let contact = legacy_contact(None, Some(&note), true, vec![0, 0]);
        let mut payload = vec![0, 0, 0, 0, 0x00, 0xFD];
        payload.extend_from_slice(note.as_bytes());
        payload.extend_from_slice(&[0x01, 0x02]);
        payload.extend_from_slice(&[0; 8]);
        assert_eq!(payload, legacy_det_plaintext(&contact, 0));
        payload
    }

    /// platform-wallet's encoding of a plain 300-byte nickname whose byte 251
    /// is `1` (49). The legacy reading takes 253 bytes of nickname, then a
    /// 49-byte note that ends exactly at the real visibility byte.
    fn upstream_alias_that_also_reads_as_legacy() -> Vec<u8> {
        use platform_wallet::wallet::identity::crypto as upstream;
        let mut alias = "x".repeat(300);
        alias.replace_range(251..252, "1");
        upstream::encode_private_data(&upstream::ContactInfoPrivateData {
            alias_name: Some(alias),
            note: None,
            display_hidden: true,
            accepted_accounts: vec![7],
        })
    }

    /// Bytes that read completely both as an older version's layout and as
    /// CompactSize, with different values, are unreadable. A save prompts
    /// instead of guessing and persisting the guess, and the contact list
    /// shows no guess either.
    #[test]
    fn details_that_read_two_ways_are_unreadable() {
        use platform_wallet::wallet::identity::crypto as upstream;
        for payload in [
            legacy_note_that_also_reads_as_compact_size(),
            upstream_alias_that_also_reads_as_legacy(),
        ] {
            let legacy = ContactInfoPrivateData::parse(&payload, LengthPrefix::SingleByte)
                .expect("a complete legacy reading");
            let compact = ContactInfoPrivateData::parse(&payload, LengthPrefix::CompactSize)
                .expect("a complete CompactSize reading");
            assert_ne!(legacy, compact);
            assert!(upstream::decode_private_data(&payload).is_ok());

            assert!(ContactInfoPrivateData::deserialize(&payload).is_none());
            assert!(
                decode_contact_info_details(&payload).is_none(),
                "the contact list must not show a guess"
            );

            let stored = contact_info_document(Some(Value::Bytes(
                encrypt_private_data(&payload, &KEY).expect("encrypt"),
            )));
            assert!(matches!(
                read_contact_info_private_data(
                    Some(&stored),
                    &KEY,
                    UnreadableContactInfoPolicy::Abort,
                ),
                Err(TaskError::DashPayContactInfoRead {
                    source: ContactInfoReadError::DeserializeFailed,
                })
            ));
        }
    }

    /// What this client writes now still reads back when a length is 253 to
    /// 255: its `FD xx 00` prefix has `xx` in `FD..=FF`, which the legacy
    /// reading takes as text that is not valid UTF-8.
    #[test]
    fn current_details_with_reserved_lengths_read_back_unchanged() {
        let reviewer_shaped = |len: usize| format!("\u{3}\u{1}{}", "x".repeat(len - 2));
        for len in [253, 254, 255] {
            for text in ["x".repeat(len), reviewer_shaped(len)] {
                for expected in [
                    legacy_contact(Some(&text), None, false, vec![2]),
                    legacy_contact(None, Some(&text), true, vec![0, 0]),
                    legacy_contact(Some(&text), Some(&text), true, vec![0, 3]),
                ] {
                    let payload = expected.serialize().expect("serialize");
                    let parsed = ContactInfoPrivateData::deserialize(&payload)
                        .unwrap_or_else(|| panic!("{len}-byte details must read back"));
                    assert_same_details(&parsed, &expected);
                }
            }
        }
    }

    /// Another client may write a nickname or note of 256 bytes or more,
    /// which this client never writes. Unless its bytes also form a complete
    /// older layout, it reads back.
    #[test]
    fn upstream_details_over_255_bytes_read_back() {
        use platform_wallet::wallet::identity::crypto as upstream;
        for len in [256, 259, 300, 1000] {
            let text = Some("x".repeat(len));
            for (alias_name, note) in [
                (None, text.clone()),
                (text.clone(), None),
                (text.clone(), text.clone()),
            ] {
                for hidden in [false, true] {
                    let theirs = upstream::ContactInfoPrivateData {
                        alias_name: alias_name.clone(),
                        note: note.clone(),
                        display_hidden: hidden,
                        accepted_accounts: vec![0, 0],
                    };
                    let payload = upstream::encode_private_data(&theirs);
                    let parsed = ContactInfoPrivateData::deserialize(&payload)
                        .unwrap_or_else(|| panic!("{len}-byte details must read back"));
                    assert_eq!(parsed.alias_name, theirs.alias_name);
                    assert_eq!(parsed.note, theirs.note);
                    assert_eq!(parsed.display_hidden, hidden);
                    assert_eq!(parsed.accepted_accounts, theirs.accepted_accounts);
                    assert_eq!(decode_contact_info_details(&payload), Some(theirs));
                }
            }
        }
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
    fn contact_info_note_preserves_a_concurrent_visibility_change() {
        let existing = stored_contact_info(vec![0, 4, 9], &KEY);
        let data = apply_contact_info_update(
            ContactInfoUpdate::note(Some("New note".into())),
            Some(&existing),
            &KEY,
        )
        .unwrap();
        assert!(
            data.display_hidden,
            "note edits must keep the latest stored visibility"
        );
        assert_eq!(data.note.as_deref(), Some("New note"));
        assert_eq!(data.alias_name.as_deref(), Some("Bao"));
        assert_eq!(data.accepted_accounts, vec![0, 4, 9]);
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

        // A wrong-key AES-CBC decrypt is unauthenticated: PKCS7 unpadding usually
        // rejects it (DecryptFailed), but ~1/256 the random IV yields valid
        // padding and the garbage plaintext fails to parse (DeserializeFailed).
        // Both are the "present but unreadable" abort this test asserts.
        assert!(
            matches!(
                apply_contact_info_update(
                    ContactInfoUpdate::visibility(false),
                    Some(&existing),
                    &KEY,
                ),
                Err(TaskError::DashPayContactInfoRead {
                    source: ContactInfoReadError::DecryptFailed
                        | ContactInfoReadError::DeserializeFailed,
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

        // Wrong-key decrypt aborts as either DecryptFailed or (≈1/256, on a
        // random IV that passes PKCS7 unpadding) DeserializeFailed.
        assert!(matches!(
            apply_contact_info_update(update.clone(), Some(&existing), &KEY),
            Err(TaskError::DashPayContactInfoRead {
                source: ContactInfoReadError::DecryptFailed
                    | ContactInfoReadError::DeserializeFailed,
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
