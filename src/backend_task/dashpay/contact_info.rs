//! Contact details (DIP-0015 `contactInfo`).
//!
//! platform-wallet writes and syncs the identity's `contactInfo` documents.
//! This module keeps the paginated document fetch the read of older,
//! legacy-key documents uses, the reader for those documents' payloads, and
//! turns a field-level update into the full set of details platform-wallet
//! publishes.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{ContactDetails, ContactInfoUpdate, validate_contact_note};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::WalletBackend;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{DocumentQuery, Identifier};
use dash_sdk::query_types::Documents;
use platform_wallet::wallet::identity::crypto::ContactInfoPrivateData;
use platform_wallet::wallet::identity::crypto::contact_info::encode_private_data_bounded;
use platform_wallet::wallet::identity::network::ContactInfoPublishOutcome;
use std::future::Future;
use std::sync::Arc;

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

/// Save `update` for `contact_user_id` through platform-wallet.
///
/// Every field the update preserves keeps the value the contacts list shows;
/// see [`super::contacts::shown_contact_details`]. Notes are limited to 255 UTF-8
/// bytes, and the details must fit the document size limit.
///
/// # Errors
///
/// Fails when the details are too large, when the identity was not loaded
/// from its wallet, when the contact is not established, or when the sync or
/// the publish fails.
pub async fn create_or_update_contact_info(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_user_id: Identifier,
    update: ContactInfoUpdate,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let owner = identity.identity.id();
    let _write_guard = app_context.contact_info_writes.acquire(owner).await;
    let ops = PlatformSaveOps {
        app_context,
        sdk,
        backend: app_context.wallet_backend()?,
        identity: &identity,
        contact_id: contact_user_id,
    };
    let published = save_contact_details(&ops, &update).await?;
    Ok(BackendTaskSuccessResult::DashPayContactInfoUpdated {
        identity: owner,
        contact_id: contact_user_id,
        published,
    })
}

/// Read the details shown for `contact_id`, the ones a contact details
/// editor starts from. While a save of the owner runs, the details are not
/// synced first; see [`super::contacts::unless_saving`].
///
/// # Errors
///
/// The details sync fails, or as for [`shown_details_for`].
pub async fn load_contact_details(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let backend = app_context.wallet_backend()?;
    let owner = identity.identity.id();
    if let Some(synced) = super::contacts::unless_saving(
        &app_context.contact_info_writes,
        owner,
        backend.dashpay_sync_contact_infos(&owner),
    )
    .await
    {
        synced?;
    }
    let (details, _) =
        shown_details_for(app_context, sdk, &backend, &identity, &contact_id).await?;
    Ok(BackendTaskSuccessResult::DashPayContactDetails {
        identity: identity.identity.id(),
        contact_id,
        details,
    })
}

/// The details shown for `contact_id`, with the precedence of
/// [`super::contacts::shown_contact_details`]: published, then
/// platform-wallet's on this device, then a legacy-key document; and whether
/// they stand in for saved details that could not be read.
///
/// # Errors
///
/// Fails when the document fetch, or reading the published or legacy
/// documents fails (a cancelled password prompt included), rather than
/// guessing: a save keeps every field it does not replace from this. Callers
/// sync platform-wallet's details first, under the owner's write lock.
async fn shown_details_for(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    backend: &WalletBackend,
    identity: &QualifiedIdentity,
    contact_id: &Identifier,
) -> Result<(ContactDetails, bool), TaskError> {
    let owner = identity.identity.id();
    let local = backend
        .dashpay_contact_details(&owner)
        .await
        .remove(contact_id);
    let stored = super::contacts::stored_contact_details(app_context, sdk, identity).await?;
    let (published, published_unreadable) = match stored.published?.take(contact_id) {
        Some((details, unreadable)) => (Some(details), unreadable),
        None => (None, false),
    };
    let legacy = stored.legacy?;
    let synced_before = backend.dashpay_details_synced(&owner, contact_id)?
        || super::contacts::holds_details(published.as_ref(), local.as_ref());
    let legacy = legacy.get(contact_id);
    let unreadable = baseline_unreadable(
        published_unreadable,
        published.as_ref(),
        local.as_ref(),
        synced_before,
        legacy,
    );
    let legacy = legacy.map(super::contacts::ContactData::details);
    Ok((
        super::contacts::shown_contact_details(published, local, synced_before, legacy),
        unreadable,
    ))
}

/// Whether the shown details stand in for saved ones that could not be read:
/// a published document that did not open, or an unreadable legacy document
/// while it is the one shown.
fn baseline_unreadable(
    published_unreadable: bool,
    published: Option<&ContactDetails>,
    local: Option<&ContactDetails>,
    synced_before: bool,
    legacy: Option<&super::contacts::ContactData>,
) -> bool {
    published_unreadable
        || super::contacts::legacy_details_shown(published, local, synced_before)
            && legacy.is_some_and(|legacy| legacy.details_unreadable)
}

/// The reads and writes one contact details save performs, behind a trait so
/// their order and failures can be driven in tests.
trait SaveOps {
    /// The details shown for the contact now, the fields an update keeps, and
    /// whether they are a stand-in for saved details that could not be read.
    fn current(&self) -> impl Future<Output = Result<(ContactDetails, bool), TaskError>> + Send;
    fn publish(
        &self,
        details: ContactDetails,
    ) -> impl Future<Output = Result<ContactInfoPublishOutcome, TaskError>> + Send;
    /// Record that platform-wallet holds the contact's details, so a legacy
    /// document no longer stands in for them.
    fn mark_synced(&self) -> Result<(), TaskError>;
}

/// Save `update` and return whether the details reached Platform.
///
/// # Errors
///
/// Nothing is written when the current details cannot be read (a cancelled
/// password prompt included): a field the update keeps would otherwise be
/// saved from a guess. Over saved details that could not be read, a save that
/// keeps a text field, other than unhiding, is refused with
/// [`TaskError::DashPayContactDetailsUnreadable`]. A save whose synced record
/// fails is reported as failed, since the old legacy details would show
/// again on reload.
async fn save_contact_details<O: SaveOps>(
    ops: &O,
    update: &ContactInfoUpdate,
) -> Result<bool, TaskError> {
    let (current, unreadable) = ops.current().await?;
    // Unreadable saved details are not empty ones: a text field the update
    // keeps would be published as empty, and the saved one lost. Unhiding
    // replaces them on purpose, as the hidden list says.
    if unreadable && update.keeps_text() && !update.unhides_only() {
        return Err(TaskError::DashPayContactDetailsUnreadable);
    }
    let details = update.apply_to(current);

    validate_contact_note(details.note.as_deref().unwrap_or_default())?;
    // The bound platform-wallet enforces, checked before it saves anything.
    encode_private_data_bounded(&ContactInfoPrivateData {
        alias_name: details.nickname.clone(),
        note: details.note.clone(),
        display_hidden: details.hidden,
        accepted_accounts: Vec::new(),
    })
    .map_err(|source| TaskError::DashPayContactInfoEncoding {
        source: Box::new(source),
    })?;

    // A failed publish records nothing. platform-wallet stores the details
    // here before it publishes, but its error does not say whether it got
    // that far, and an attempt can equal details that were never stored.
    // TODO(upstream): https://github.com/dashpay/platform/issues/5646 —
    // platform-wallet keeps its local change when the publish fails, so this
    // device can show details Platform never got (or, for a contact with only
    // a legacy document, the legacy details again). Once platform-wallet rolls
    // the local change back or reports it in a typed outcome, nothing here
    // needs to change for the rollback; a typed outcome gets its own arm.
    let outcome = ops.publish(details).await?;
    match ops.mark_synced() {
        Ok(()) => {}
        // A deferred save published nothing, so the storage error's claim
        // that the change reached the network would be false.
        Err(TaskError::DashpaySidecarStorage { source })
            if outcome == ContactInfoPublishOutcome::DeferredUntilTwoContacts =>
        {
            return Err(TaskError::DashPayContactDetailsNotRecorded { source });
        }
        Err(error) => return Err(error),
    }
    match outcome {
        ContactInfoPublishOutcome::Published => Ok(true),
        ContactInfoPublishOutcome::DeferredUntilTwoContacts => Ok(false),
        // The write checks for a wallet identity index before platform-wallet
        // runs, so this is reached only if the two disagree.
        ContactInfoPublishOutcome::SkippedWatchOnly => {
            Err(TaskError::ContactInfoIdentityNotFromWallet)
        }
    }
}

struct PlatformSaveOps<'a> {
    app_context: &'a Arc<AppContext>,
    sdk: &'a Sdk,
    backend: Arc<WalletBackend>,
    identity: &'a QualifiedIdentity,
    contact_id: Identifier,
}

impl SaveOps for PlatformSaveOps<'_> {
    async fn current(&self) -> Result<(ContactDetails, bool), TaskError> {
        // The save holds the owner's write lock, so this sync cannot race one.
        self.backend
            .dashpay_sync_contact_infos(&self.identity.identity.id())
            .await?;
        shown_details_for(
            self.app_context,
            self.sdk,
            &self.backend,
            self.identity,
            &self.contact_id,
        )
        .await
    }

    async fn publish(
        &self,
        details: ContactDetails,
    ) -> Result<ContactInfoPublishOutcome, TaskError> {
        // TODO: preserved fields come from `current()`'s read; upstream set_contact_info has no expected-revision check, so a newer edit from another device in between is overwritten.
        self.backend
            .dashpay_set_contact_info(self.identity, &self.contact_id, details)
            .await
    }

    fn mark_synced(&self) -> Result<(), TaskError> {
        self.backend
            .dashpay_mark_details_synced(&self.identity.identity.id(), &self.contact_id)
    }
}

/// The plaintext floor platform-wallet pads to with zero bytes, so that
/// IV (16) + AES-CBC ciphertext reaches the contract's 48-byte minimum.
const MIN_PLAINTEXT_SIZE: usize = 17;

/// The plaintext size older versions of this client (up to
/// v1.0.0-weekly.20261020) padded to: a `0x00` sentinel, then random bytes.
const LEGACY_PLAINTEXT_SIZE: usize = 16;

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
    /// [`PrivateDataReading::Ambiguous`]. What platform-wallet writes never
    /// is: a 253 to 255-byte field is `FD xx 00` with `xx` in `FD..=FF`, never
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

/// Decode a decrypted legacy-key `privateData` payload.
///
/// Older versions of this client wrote single-byte lengths, which
/// platform-wallet misreads for fields of 253 to 255 bytes. So this client's
/// strict reader comes first, and an ambiguous payload is unreadable, never a
/// guess. platform-wallet's tolerant reader then covers what neither reading
/// accepts, such as a newer minor version.
pub(super) fn decode_contact_info_details(plaintext: &[u8]) -> Option<ContactInfoPrivateData> {
    match read_private_data(plaintext) {
        PrivateDataReading::Read(data) => Some(data),
        PrivateDataReading::Ambiguous => None,
        PrivateDataReading::Unreadable => {
            platform_wallet::wallet::identity::crypto::decode_private_data(plaintext).ok()
        }
    }
}

/// Read `bytes` both as CompactSize and as the single-byte layout older
/// versions wrote, and keep a result only when it is unambiguous.
///
/// A reading fails for a truncated or non-canonical v0 payload, invalid
/// UTF-8, an unsupported version, or trailing bytes that are not padding
/// (see [`is_padding`]).
fn read_private_data(bytes: &[u8]) -> PrivateDataReading {
    match (
        parse_private_data(bytes, LengthPrefix::SingleByte),
        parse_private_data(bytes, LengthPrefix::CompactSize),
    ) {
        (Some(legacy), Some(compact)) if legacy != compact => PrivateDataReading::Ambiguous,
        (Some(data), _) | (None, Some(data)) => PrivateDataReading::Read(data),
        (None, None) => PrivateDataReading::Unreadable,
    }
}

fn parse_private_data(bytes: &[u8], prefix: LengthPrefix) -> Option<ContactInfoPrivateData> {
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
        LengthPrefix::CompactSize => is_padding(bytes, pos),
        // A payload without a reserved length reads the same both ways, so
        // it is left to the CompactSize reader. One with such a length is
        // over 255 bytes, which older versions never padded.
        LengthPrefix::SingleByte => reserved_length && pos == bytes.len(),
    };
    if !complete {
        return None;
    }

    Some(ContactInfoPrivateData {
        alias_name,
        note,
        display_hidden,
        accepted_accounts,
    })
}

/// Whether `bytes[end..]`, after the last v0 field, is padding a known
/// writer adds: none, platform-wallet's zeros up to [`MIN_PLAINTEXT_SIZE`],
/// or this client's former `0x00` sentinel and random bytes up to exactly
/// [`LEGACY_PLAINTEXT_SIZE`].
fn is_padding(bytes: &[u8], end: usize) -> bool {
    match bytes[end..].split_first() {
        None => true,
        Some((0, _)) if bytes.len() == LEGACY_PLAINTEXT_SIZE => true,
        Some(_) => bytes.len() <= MIN_PLAINTEXT_SIZE && bytes[end..].iter().all(|b| *b == 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::document::{
        Document as DppDocument, DocumentV0, DocumentV0Getters, DocumentV0Setters,
    };
    use dash_sdk::dpp::platform_value::Value;
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Document;
    use std::collections::{BTreeMap, VecDeque};
    use std::future::ready;
    use zeroize::Zeroizing;

    const KEY: [u8; 32] = [7u8; 32];

    fn encrypt_to_user_id(user_id: &[u8; 32], key: &[u8; 32]) -> [u8; 32] {
        platform_encryption::encrypt_enc_to_user_id(key, user_id)
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

    /// The fee estimate covers the largest details platform-wallet accepts.
    #[test]
    fn contact_info_fee_estimate_covers_the_largest_details() {
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimator = PlatformFeeEstimator::new();
        for len in [0, 252, 253, 255] {
            let text = (len > 0).then(|| "x".repeat(len));
            let encoded = encode_private_data_bounded(&ContactInfoPrivateData {
                alias_name: text.clone(),
                note: text,
                display_hidden: true,
                accepted_accounts: Vec::new(),
            })
            .expect("within the bound");
            let encrypted = platform_encryption::encrypt_private_data(&KEY, &[0; 16], &encoded);
            assert!(
                estimator.estimate_contact_info_write()
                    >= estimator.estimate_document_create_with_size(112 + encrypted.len()),
                "{len}-byte fields"
            );
        }
    }

    struct ScriptedSave {
        current: std::sync::Mutex<Option<Result<ContactDetails, TaskError>>>,
        unreadable: bool,
        mark_fails: bool,
        marked: std::sync::Mutex<bool>,
        publish_fails: bool,
        outcome: ContactInfoPublishOutcome,
        published: std::sync::Mutex<Vec<ContactDetails>>,
    }

    impl ScriptedSave {
        fn new(current: Result<ContactDetails, TaskError>, mark_fails: bool) -> Self {
            Self {
                current: std::sync::Mutex::new(Some(current)),
                unreadable: false,
                mark_fails,
                marked: std::sync::Mutex::new(false),
                publish_fails: false,
                outcome: ContactInfoPublishOutcome::DeferredUntilTwoContacts,
                published: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn with_outcome(mut self, outcome: ContactInfoPublishOutcome) -> Self {
            self.outcome = outcome;
            self
        }
    }

    impl SaveOps for ScriptedSave {
        async fn current(&self) -> Result<(ContactDetails, bool), TaskError> {
            self.current
                .lock()
                .unwrap()
                .take()
                .expect("read once")
                .map(|details| (details, self.unreadable))
        }

        async fn publish(
            &self,
            details: ContactDetails,
        ) -> Result<ContactInfoPublishOutcome, TaskError> {
            if self.publish_fails {
                return Err(TaskError::WalletStateInconsistent);
            }
            self.published.lock().unwrap().push(details);
            Ok(self.outcome)
        }

        fn mark_synced(&self) -> Result<(), TaskError> {
            if self.mark_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            *self.marked.lock().unwrap() = true;
            Ok(())
        }
    }

    /// A cancelled password prompt while reading the current details stops
    /// the save: a note-only save must not overwrite the nickname or the
    /// hidden flag with empty values it never read.
    #[tokio::test]
    async fn an_unreadable_baseline_writes_nothing() {
        let ops = ScriptedSave::new(Err(TaskError::SecretPromptCancelled), false);

        let result = save_contact_details(&ops, &ContactInfoUpdate::note(Some("n".into()))).await;

        assert!(matches!(result, Err(TaskError::SecretPromptCancelled)));
        assert!(ops.published.lock().unwrap().is_empty());
    }

    /// The baseline is unreadable when the published document did not open,
    /// or when an unreadable legacy document is the one shown; not when
    /// readable details shown ahead of it replace it.
    #[test]
    fn a_baseline_is_unreadable_only_when_unreadable_details_are_shown() {
        let legacy = |details_unreadable| super::super::contacts::ContactData {
            identity_id: Identifier::default(),
            nickname: None,
            note: None,
            is_hidden: true,
            details_unreadable,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        };
        let saved = ContactDetails {
            note: Some("n".into()),
            ..Default::default()
        };

        assert!(baseline_unreadable(true, Some(&saved), None, true, None));
        assert!(baseline_unreadable(
            false,
            None,
            None,
            false,
            Some(&legacy(true))
        ));
        assert!(!baseline_unreadable(
            false,
            None,
            None,
            false,
            Some(&legacy(false))
        ));
        assert!(!baseline_unreadable(
            false,
            Some(&saved),
            None,
            false,
            Some(&legacy(true))
        ));
        assert!(!baseline_unreadable(
            false,
            None,
            Some(&saved),
            false,
            Some(&legacy(true))
        ));
        assert!(!baseline_unreadable(
            false,
            None,
            None,
            true,
            Some(&legacy(true))
        ));
    }

    /// A failed publish records nothing, even when this device's details
    /// already equal the attempt: platform-wallet's error does not say
    /// whether it stored them first, and an empty attempt equals empty
    /// details that were never stored.
    #[tokio::test]
    async fn a_failed_publish_records_nothing() {
        let ops = ScriptedSave {
            publish_fails: true,
            ..ScriptedSave::new(Ok(ContactDetails::default()), false)
        };

        let result =
            save_contact_details(&ops, &ContactInfoUpdate::replace_all(None, None, false)).await;

        assert!(result.is_err());
        assert!(!*ops.marked.lock().unwrap());
    }

    /// Saved details that could not be read are not empty ones: a save that
    /// keeps a text field would publish it as empty and lose it. Unhiding is
    /// the one such save allowed, since the hidden list says it clears them;
    /// the backend decides from the details it reads, not the list's flag.
    #[tokio::test]
    async fn unreadable_saved_details_are_replaced_only_by_unhiding() {
        let unreadable = || ScriptedSave {
            unreadable: true,
            ..ScriptedSave::new(
                Ok(ContactDetails {
                    hidden: true,
                    ..Default::default()
                }),
                false,
            )
        };

        for update in [
            ContactInfoUpdate::note(Some("n".into())),
            ContactInfoUpdate::visibility(true),
        ] {
            let ops = unreadable();
            let result = save_contact_details(&ops, &update).await;
            assert!(
                matches!(result, Err(TaskError::DashPayContactDetailsUnreadable)),
                "{update:?}"
            );
            assert!(ops.published.lock().unwrap().is_empty());
        }

        let ops = unreadable();
        save_contact_details(&ops, &ContactInfoUpdate::visibility(false))
            .await
            .expect("unhiding replaces them");
        assert_eq!(
            *ops.published.lock().unwrap(),
            vec![ContactDetails::default()]
        );
    }

    /// A published save whose synced record fails is not reported as saved:
    /// on reload the old legacy details would show again. The details did
    /// reach the network, and the error says so.
    #[tokio::test]
    async fn a_failed_synced_record_fails_the_save() {
        let ops = ScriptedSave::new(Ok(ContactDetails::default()), true)
            .with_outcome(ContactInfoPublishOutcome::Published);

        let result =
            save_contact_details(&ops, &ContactInfoUpdate::replace_all(None, None, false)).await;

        assert!(matches!(
            result,
            Err(TaskError::DashpaySidecarStorage { .. })
        ));
    }

    /// A save kept on this device until a second contact exists published
    /// nothing, so a failed synced record must not claim the change reached
    /// the network; the user is asked to save again.
    #[tokio::test]
    async fn a_failed_synced_record_of_a_deferred_save_claims_no_publish() {
        let ops = ScriptedSave::new(Ok(ContactDetails::default()), true);

        let result =
            save_contact_details(&ops, &ContactInfoUpdate::replace_all(None, None, false)).await;

        let Err(error) = result else {
            panic!("a failed synced record fails the save");
        };
        assert!(
            matches!(error, TaskError::DashPayContactDetailsNotRecorded { .. }),
            "got {error:?}"
        );
        assert!(!error.to_string().contains("reached the network"));
    }

    #[tokio::test]
    async fn a_save_keeps_the_fields_it_does_not_replace() {
        let current = ContactDetails {
            nickname: Some("Bao".into()),
            note: None,
            hidden: true,
        };
        let ops = ScriptedSave::new(Ok(current.clone()), false);

        let published = save_contact_details(&ops, &ContactInfoUpdate::note(Some("n".into())))
            .await
            .expect("saved");

        assert!(!published, "deferred until two contacts");
        assert_eq!(
            *ops.published.lock().unwrap(),
            vec![ContactDetails {
                note: Some("n".into()),
                ..current
            }]
        );
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
        let user_id = encrypt_to_user_id(&contact.to_buffer(), &KEY);
        let mut document = lookup_contact_info_document(document_id, document_id.into(), user_id);
        let plaintext = upstream::encode_private_data(&upstream::ContactInfoPrivateData {
            note: Some(note.to_string()),
            ..Default::default()
        });
        let private_data = platform_encryption::encrypt_private_data(&KEY, &[5; 16], &plaintext);
        document.set("privateData", Value::Bytes(private_data));
        document
    }

    /// The legacy details read from `documents` with keys from `derive_keys`.
    async fn legacy_load<D, DF>(
        documents: Vec<Document>,
        derive_keys: D,
    ) -> Result<std::collections::HashMap<Identifier, super::super::contacts::ContactData>, TaskError>
    where
        D: FnMut(u32) -> DF,
        DF: Future<Output = Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError>>,
    {
        let contract =
            load_system_data_contract(SystemDataContract::Dashpay, PlatformVersion::latest())
                .expect("DashPay contract fixture");
        let query = DocumentQuery::new(contract, "contactInfo").expect("contactInfo query");
        let mut page = Some(contact_details_page(documents));
        super::super::contacts::load_contact_details(
            query,
            |_| ready(Ok::<_, TaskError>(page.take().unwrap_or_default())),
            derive_keys,
        )
        .await
    }

    /// A cancelled password prompt for the legacy keys fails the read, so a
    /// save does not keep a legacy-only nickname or hidden flag as empty.
    #[tokio::test]
    async fn a_cancelled_legacy_key_prompt_fails_the_read() {
        let result = legacy_load(vec![saved_note_document(1, id(201), "n")], |_| {
            ready(Err(TaskError::SecretPromptCancelled))
        })
        .await;

        assert!(matches!(result, Err(TaskError::SecretPromptCancelled)));
    }

    /// A document whose own key index cannot be derived is skipped; the rest
    /// still load.
    #[tokio::test]
    async fn an_underivable_legacy_document_is_skipped() {
        let (readable, unreadable) = (id(201), id(202));
        let mut bad = saved_note_document(2, unreadable, "x");
        bad.set("derivationEncryptionKeyIndex", Value::U32(u32::MAX));
        let details = legacy_load(vec![saved_note_document(1, readable, "n"), bad], |index| {
            ready(if index == u32::MAX {
                Err(TaskError::ContactKeyDerivationFailed {
                    source: Box::new(
                        dash_sdk::dpp::key_wallet::bip32::ChildNumber::from_hardened_idx(index)
                            .expect_err("out of range"),
                    ),
                })
            } else {
                Ok((Zeroizing::new(KEY), Zeroizing::new(KEY)))
            })
        })
        .await
        .expect("loaded");

        assert_eq!(
            details.get(&readable).and_then(|c| c.note.as_deref()),
            Some("n")
        );
        assert!(!details.contains_key(&unreadable));
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
        let other = encrypt_to_user_id(&id(202).to_buffer(), &KEY);
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
            encrypt_to_user_id(&undecryptable.to_buffer(), &KEY),
        );
        first.set("privateData", Value::Bytes(vec![0xA5; 48]));
        let mut second =
            lookup_contact_info_document(2, 2, encrypt_to_user_id(&undecodable.to_buffer(), &KEY));
        second.set(
            "privateData",
            Value::Bytes(platform_encryption::encrypt_private_data(
                &KEY,
                &[5; 16],
                &[0xFF; 40],
            )),
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

    /// With two readable documents for one contact, the newer one is shown:
    /// pages are ordered by `$updatedAt`, oldest first.
    #[tokio::test]
    async fn the_contact_details_load_shows_the_newest_document() {
        let contact = id(201);
        let page = contact_details_page([
            saved_note_document(1, contact, "older"),
            saved_note_document(2, contact, "newer"),
        ]);

        let (details, _) = load_details(vec![page]).await;

        assert_eq!(
            details.get(&contact).and_then(|c| c.note.as_deref()),
            Some("newer")
        );
    }

    /// What this client's strict reader accepts, without platform-wallet's
    /// tolerant fallback.
    fn strict_read(bytes: &[u8]) -> Option<ContactInfoPrivateData> {
        match read_private_data(bytes) {
            PrivateDataReading::Read(data) => Some(data),
            PrivateDataReading::Ambiguous | PrivateDataReading::Unreadable => None,
        }
    }

    /// The bytes platform-wallet writes for `data`.
    fn upstream_plaintext(data: &ContactInfoPrivateData) -> Vec<u8> {
        platform_wallet::wallet::identity::crypto::encode_private_data(data)
    }

    fn contact(
        alias_name: Option<&str>,
        note: Option<&str>,
        display_hidden: bool,
        accepted_accounts: Vec<u32>,
    ) -> ContactInfoPrivateData {
        ContactInfoPrivateData {
            alias_name: alias_name.map(str::to_owned),
            note: note.map(str::to_owned),
            display_hidden,
            accepted_accounts,
        }
    }

    /// Byte-exact copy of the serializer older versions of this client used
    /// (v1.0.0-weekly.20261020 and earlier, back to #810): single-byte
    /// lengths, and a payload under 16 bytes padded with `0x00` and then
    /// random bytes up to 16. `fill` stands in for the random bytes.
    fn legacy_det_plaintext(data: &ContactInfoPrivateData, fill: u8) -> Vec<u8> {
        let mut bytes = 0_u32.to_le_bytes().to_vec();
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
        if bytes.len() < LEGACY_PLAINTEXT_SIZE {
            bytes.push(0x00);
            bytes.resize(LEGACY_PLAINTEXT_SIZE, fill);
        }
        bytes
    }

    /// A legacy-key contactInfo for `contact` holding `plaintext`.
    fn legacy_plaintext_document(
        document_id: u8,
        contact: Identifier,
        plaintext: &[u8],
    ) -> Document {
        let user_id = encrypt_to_user_id(&contact.to_buffer(), &KEY);
        let mut document = lookup_contact_info_document(document_id, document_id.into(), user_id);
        let private_data = platform_encryption::encrypt_private_data(&KEY, &[5; 16], plaintext);
        document.set("privateData", Value::Bytes(private_data));
        document
    }

    #[test]
    fn private_data_round_trips_every_accepted_account() {
        let original = contact(Some("Bao"), Some("Met at the meetup"), true, vec![0, 3, 17]);

        assert_eq!(
            strict_read(&upstream_plaintext(&original)),
            Some(original),
            "every accepted account must survive the round trip, not just the first"
        );
    }

    #[test]
    fn private_data_round_trips_through_the_minimum_size_padding() {
        // Empty details are padded up to the contract's minimum size; the
        // padding must not be mistaken for a field.
        let bytes = upstream_plaintext(&ContactInfoPrivateData::default());
        assert_eq!(bytes.len(), MIN_PLAINTEXT_SIZE);
        assert_eq!(strict_read(&bytes), Some(ContactInfoPrivateData::default()));
    }

    #[test]
    fn malformed_private_data_is_not_read_strictly() {
        let accounts = upstream_plaintext(&contact(None, None, false, vec![1, 2, 3]));
        // A list that declares three accounts must not parse as a shorter one.
        assert!(strict_read(&accounts[..accounts.len() - 2]).is_none());

        let empty = upstream_plaintext(&ContactInfoPrivateData::default());
        let mut future_version = empty.clone();
        future_version[..4].copy_from_slice(&1_u32.to_le_bytes());
        let mut invalid_utf8 = empty.clone();
        invalid_utf8[4] = 1;
        invalid_utf8[5] = 0xff;
        let mut unknown_visibility = empty.clone();
        unknown_visibility[6] = 2;
        let mut no_sentinel = empty;
        no_sentinel[8] = 1;
        let mut extended = upstream_plaintext(&contact(Some("123456789"), None, false, Vec::new()));
        assert_eq!(extended.len(), MIN_PLAINTEXT_SIZE);
        extended.push(42);

        for bytes in [
            future_version,
            invalid_utf8,
            unknown_visibility,
            no_sentinel,
            extended,
        ] {
            assert!(strict_read(&bytes).is_none(), "{bytes:02x?}");
        }
    }

    /// Only the 16-byte layout older versions of this client wrote may hold
    /// random bytes after the `0x00` sentinel. Padding of any other length
    /// must be all zeros.
    #[test]
    fn contact_info_padding_rejects_nonzero_bytes_after_the_sentinel() {
        let canonical = upstream_plaintext(&ContactInfoPrivateData::default());
        for length in (9..=MIN_PLAINTEXT_SIZE).filter(|length| *length != LEGACY_PLAINTEXT_SIZE) {
            assert!(strict_read(&canonical[..length]).is_some());
            for offset in 9..length {
                let mut malformed = canonical[..length].to_vec();
                malformed[offset] = 42;
                assert!(
                    strict_read(&malformed).is_none(),
                    "nonzero padding at {offset} in {length} bytes must not be read"
                );
            }
        }
    }

    /// Details an older version of this client saved for a hidden contact, or
    /// with a short nickname or note, are padded with random bytes, and long
    /// ones carry single-byte lengths of 253 to 255, which CompactSize
    /// reserves. Both are read with every field.
    #[test]
    fn details_written_by_older_versions_are_read() {
        let long = "y".repeat(254);
        let mut contacts = vec![
            contact(None, None, true, Vec::new()),
            contact(Some("Bob"), None, false, Vec::new()),
            contact(None, Some("hi"), true, Vec::new()),
            contact(Some("Al"), None, true, vec![0]),
            contact(Some("Bao"), Some("Met"), false, Vec::new()),
            contact(Some(&long), Some("Met"), false, vec![1]),
        ];
        for len in [253, 254, 255] {
            let text = "x".repeat(len);
            contacts.push(contact(Some(&text), None, false, vec![2]));
            contacts.push(contact(None, Some(&text), true, Vec::new()));
            contacts.push(contact(Some(&text), Some(&text), true, vec![0, 3]));
        }
        for expected in &contacts {
            for fill in [0x01, 0x5A, 0xFF] {
                let plaintext = legacy_det_plaintext(expected, fill);
                assert_eq!(strict_read(&plaintext).as_ref(), Some(expected));
                assert_eq!(
                    decode_contact_info_details(&plaintext).as_ref(),
                    Some(expected)
                );
            }
        }
    }

    /// A legacy document with a 254-byte nickname loads with that nickname,
    /// which platform-wallet's reader alone misreads.
    #[tokio::test]
    async fn a_legacy_document_with_a_long_nickname_is_shown() {
        let owner = id(201);
        let nickname = "z".repeat(254);
        let plaintext = legacy_det_plaintext(&contact(Some(&nickname), None, false, vec![]), 0);
        assert_ne!(
            platform_wallet::wallet::identity::crypto::decode_private_data(&plaintext)
                .ok()
                .and_then(|data| data.alias_name),
            Some(nickname.clone())
        );

        let details = legacy_load(
            vec![legacy_plaintext_document(1, owner, &plaintext)],
            |_| ready(Ok((Zeroizing::new(KEY), Zeroizing::new(KEY)))),
        )
        .await
        .expect("loaded");

        let shown = details.get(&owner).expect("listed");
        assert_eq!(shown.nickname.as_deref(), Some(nickname.as_str()));
        assert!(!shown.details_unreadable);
    }

    /// Tolerating the older layouts must not let corrupt payloads through.
    #[test]
    fn legacy_layouts_do_not_hide_corruption() {
        let legacy = legacy_det_plaintext(&contact(None, None, true, Vec::new()), 0x5A);

        let mut no_sentinel = legacy.clone();
        no_sentinel[8] = 0x5A;
        assert!(strict_read(&no_sentinel).is_none());

        for length in (10..legacy.len()).chain([legacy.len() + 1]) {
            let mut resized = legacy.clone();
            resized.resize(length, 0x5A);
            assert!(
                strict_read(&resized).is_none(),
                "random padding over {length} bytes is not a layout any client wrote"
            );
        }

        let long =
            legacy_det_plaintext(&contact(Some(&"x".repeat(253)), None, false, Vec::new()), 0);
        // A single-byte length that runs past the end of the payload.
        let overrun = long[..long.len() - 1].to_vec();
        // Single-byte lengths still need valid text and no trailing data.
        let mut invalid_utf8 = long.clone();
        invalid_utf8[5] = 0xFF;
        let mut extended = long;
        extended.push(0);
        for corrupt in [overrun, invalid_utf8, extended] {
            assert!(strict_read(&corrupt).is_none());
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

    /// The details shown are what the strict reader reads, including legacy
    /// text with control characters, which platform-wallet reads as different
    /// fields.
    #[test]
    fn shown_details_match_the_strict_reading() {
        let mut payloads = Vec::new();
        for len in [0, 3, 252, 253, 254, 255] {
            let text = "x".repeat(len);
            let text = (len > 0).then_some(text.as_str());
            for hidden in [false, true] {
                let details = contact(text, text, hidden, vec![0, 7]);
                payloads.push(legacy_det_plaintext(&details, 0x9C));
                payloads.push(upstream_plaintext(&details));
            }
        }
        let control = control_character_note();
        for details in [
            contact(None, Some(&control), false, Vec::new()),
            contact(Some(&control), Some("Met"), true, vec![4]),
            contact(Some("\u{0}\u{1}"), Some(&control), false, vec![1]),
        ] {
            payloads.push(legacy_det_plaintext(&details, 0));
            payloads.push(upstream_plaintext(&details));
        }

        for payload in &payloads {
            let strict = strict_read(payload)
                .unwrap_or_else(|| panic!("the strict reader must read {payload:02x?}"));
            assert_eq!(
                decode_contact_info_details(payload),
                Some(strict),
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
        for details in [
            contact(None, Some(&control), true, Vec::new()),
            contact(Some(&long_control), None, false, vec![2]),
            contact(Some(&control), Some(&long_control), false, Vec::new()),
        ] {
            for payload in [
                legacy_det_plaintext(&details, 0),
                upstream_plaintext(&details),
            ] {
                let mut appended = payload.clone();
                appended.push(0);
                let mut truncated = payload;
                truncated.pop();
                for corrupt in [appended, truncated] {
                    assert!(strict_read(&corrupt).is_none(), "{corrupt:02x?}");
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
        assert!(strict_read(&bytes).is_none());

        // A truncated payload without reserved lengths stays rejected.
        let short = legacy_det_plaintext(&contact(Some("Bob"), None, false, vec![1]), 0);
        assert!(strict_read(&short[..short.len() - 2]).is_none());
    }

    /// An older version's 253-byte note starting with `03 01`, which a
    /// CompactSize reader takes as `FD 03 01`, a 259-byte length, followed by
    /// a complete payload with other values.
    fn legacy_note_that_also_reads_as_compact_size() -> Vec<u8> {
        let note = format!("\u{3}\u{1}{}", "x".repeat(251));
        let details = contact(None, Some(&note), true, vec![0, 0]);
        let mut payload = vec![0, 0, 0, 0, 0x00, 0xFD];
        payload.extend_from_slice(note.as_bytes());
        payload.extend_from_slice(&[0x01, 0x02]);
        payload.extend_from_slice(&[0; 8]);
        assert_eq!(payload, legacy_det_plaintext(&details, 0));
        payload
    }

    /// platform-wallet's encoding of a plain 300-byte nickname whose byte 251
    /// is `1` (49). The legacy reading takes 253 bytes of nickname, then a
    /// 49-byte note that ends exactly at the real visibility byte.
    fn upstream_alias_that_also_reads_as_legacy() -> Vec<u8> {
        let mut alias = "x".repeat(300);
        alias.replace_range(251..252, "1");
        upstream_plaintext(&contact(Some(&alias), None, true, vec![7]))
    }

    /// Bytes that read completely both as an older version's layout and as
    /// CompactSize, with different values, are unreadable: no guess is shown,
    /// and a legacy document holding them keeps its contact hidden.
    #[tokio::test]
    async fn details_that_read_two_ways_are_unreadable() {
        for payload in [
            legacy_note_that_also_reads_as_compact_size(),
            upstream_alias_that_also_reads_as_legacy(),
        ] {
            let legacy = parse_private_data(&payload, LengthPrefix::SingleByte)
                .expect("a complete legacy reading");
            let compact = parse_private_data(&payload, LengthPrefix::CompactSize)
                .expect("a complete CompactSize reading");
            assert_ne!(legacy, compact);
            assert!(
                platform_wallet::wallet::identity::crypto::decode_private_data(&payload).is_ok()
            );

            assert!(matches!(
                read_private_data(&payload),
                PrivateDataReading::Ambiguous
            ));
            assert!(
                decode_contact_info_details(&payload).is_none(),
                "no guess may be shown"
            );

            let owner = id(202);
            let details = legacy_load(vec![legacy_plaintext_document(1, owner, &payload)], |_| {
                ready(Ok((Zeroizing::new(KEY), Zeroizing::new(KEY))))
            })
            .await
            .expect("loaded");
            let shown = details.get(&owner).expect("listed");
            assert!(shown.details_unreadable);
            assert!(shown.is_hidden);
        }
    }

    /// What platform-wallet writes reads back unchanged when a length is 253
    /// to 255: its `FD xx 00` prefix has `xx` in `FD..=FF`, which the legacy
    /// reading takes as text that is not valid UTF-8.
    #[test]
    fn upstream_details_with_reserved_lengths_read_back_unchanged() {
        let reviewer_shaped = |len: usize| format!("\u{3}\u{1}{}", "x".repeat(len - 2));
        for len in [253, 254, 255] {
            for text in ["x".repeat(len), reviewer_shaped(len)] {
                for expected in [
                    contact(Some(&text), None, false, vec![2]),
                    contact(None, Some(&text), true, vec![0, 0]),
                    contact(Some(&text), Some(&text), true, vec![0, 3]),
                ] {
                    let payload = upstream_plaintext(&expected);
                    assert_eq!(strict_read(&payload).as_ref(), Some(&expected));
                    assert_eq!(decode_contact_info_details(&payload), Some(expected));
                }
            }
        }
    }

    /// Another client may write a nickname or note of 256 bytes or more.
    /// Unless its bytes also form a complete older layout, it reads back.
    #[test]
    fn upstream_details_over_255_bytes_read_back() {
        for len in [256, 259, 300, 1000] {
            let text = "x".repeat(len);
            let text = Some(text.as_str());
            for (alias_name, note) in [(None, text), (text, None), (text, text)] {
                for hidden in [false, true] {
                    let theirs = contact(alias_name, note, hidden, vec![0, 0]);
                    let payload = upstream_plaintext(&theirs);
                    assert_eq!(strict_read(&payload).as_ref(), Some(&theirs));
                    assert_eq!(decode_contact_info_details(&payload), Some(theirs));
                }
            }
        }
    }
}
