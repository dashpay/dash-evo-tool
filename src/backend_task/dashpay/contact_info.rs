//! Contact details (DIP-0015 `contactInfo`).
//!
//! platform-wallet writes and syncs the identity's `contactInfo` documents.
//! This module keeps the paginated document fetch the read of older,
//! legacy-key documents uses, and turns a field-level update into the full
//! set of details platform-wallet publishes.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{
    ContactDetails, ContactInfoField, ContactInfoUpdate, validate_contact_note,
};
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
    let unreadable = published_unreadable
        || super::contacts::legacy_details_shown(published.as_ref(), local.as_ref(), synced_before)
            && legacy.is_some_and(|legacy| legacy.details_unreadable);
    let legacy = legacy.map(super::contacts::ContactData::details);
    Ok((
        super::contacts::shown_contact_details(published, local, synced_before, legacy),
        unreadable,
    ))
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
    /// Whether platform-wallet holds `details` for the contact on this device.
    fn holds_locally(&self, details: &ContactDetails) -> impl Future<Output = bool> + Send;
}

/// Save `update` and return whether the details reached Platform.
///
/// # Errors
///
/// Nothing is written when the current details cannot be read (a cancelled
/// password prompt included): a field the update keeps would otherwise be
/// saved from a guess. A save whose synced record fails is reported as
/// failed, since the old legacy details would show again on reload.
async fn save_contact_details<O: SaveOps>(
    ops: &O,
    update: &ContactInfoUpdate,
) -> Result<bool, TaskError> {
    let (current, unreadable) = ops.current().await?;
    // Unreadable saved details are not empty ones: a text field the update
    // keeps would be published as empty, and the saved one lost.
    if unreadable
        && (matches!(update.nickname, ContactInfoField::Preserve)
            || matches!(update.note, ContactInfoField::Preserve))
    {
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

    let outcome = match ops.publish(details.clone()).await {
        Ok(outcome) => outcome,
        Err(error) => {
            // platform-wallet stores the details here before it publishes
            // them. Once it has, a legacy document must not stand in for them
            // on reload: this device stays ahead of Platform until next save.
            if ops.holds_locally(&details).await
                && let Err(mark) = ops.mark_synced()
            {
                tracing::debug!(error = ?mark, "contact details held here but not recorded");
            }
            return Err(error);
        }
    };
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

    async fn holds_locally(&self, details: &ContactDetails) -> bool {
        self.backend
            .dashpay_contact_details(&self.identity.identity.id())
            .await
            .get(&self.contact_id)
            == Some(details)
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
        holds_locally: bool,
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
                holds_locally: false,
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

        async fn holds_locally(&self, _details: &ContactDetails) -> bool {
            self.holds_locally
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

    /// A publish that fails after platform-wallet stored the details here
    /// still records them as held, so a cleared field does not fall back to
    /// an older legacy document on reload: this device stays ahead of
    /// Platform until the next save. The save is still reported as failed.
    #[tokio::test]
    async fn a_failed_publish_of_details_held_here_records_them() {
        for holds_locally in [true, false] {
            let ops = ScriptedSave {
                publish_fails: true,
                holds_locally,
                ..ScriptedSave::new(Ok(ContactDetails::default()), false)
            };

            let result =
                save_contact_details(&ops, &ContactInfoUpdate::replace_all(None, None, false))
                    .await;

            assert!(result.is_err());
            assert_eq!(*ops.marked.lock().unwrap(), holds_locally);
        }
    }

    /// Saved details that could not be read are not empty ones: a save that
    /// keeps a text field would publish it as empty and lose it. Only an
    /// update that replaces every text field may write over them.
    #[tokio::test]
    async fn unreadable_saved_details_are_replaced_only_on_purpose() {
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

        let ops = unreadable();
        let result = save_contact_details(&ops, &ContactInfoUpdate::visibility(false)).await;
        assert!(matches!(
            result,
            Err(TaskError::DashPayContactDetailsUnreadable)
        ));
        assert!(ops.published.lock().unwrap().is_empty());

        let ops = unreadable();
        save_contact_details(&ops, &ContactInfoUpdate::replace_all(None, None, false))
            .await
            .expect("an explicit replacement");
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
}
