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
    ContactData, ContactDetails, ContactInfoUpdate, shown_contact_details, validate_contact_note,
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
use std::collections::{BTreeMap, BTreeSet};
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
/// see [`crate::model::dashpay::shown_contact_details`]. Notes are limited to 255 UTF-8
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

/// Bring the contact details platform-wallet holds for `identity` up to date
/// with the identity's stored `contactInfo` documents, read with the wallet
/// seed: platform-wallet does not read them back for a wallet loaded without
/// its seed, which is how wallets load after a restart.
///
/// For each established contact the precedence of
/// [`shown_contact_details`] applies: a published DIP-0015 document, then
/// platform-wallet's details on this device, then a document older versions
/// of this app sealed under the legacy key root. A legacy document is taken
/// over once, so a later cleared note is not undone by it.
///
/// # Errors
///
/// The details sync or the document fetch failed, the seed could not be used
/// (a cancelled password prompt included), or the result could not be stored.
pub(crate) async fn refresh_contact_details(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
) -> Result<(), TaskError> {
    let _write_guard = app_context
        .contact_info_writes
        .acquire(identity.identity.id())
        .await;
    let backend = app_context.wallet_backend()?;
    refresh_contact_details_locked(app_context, sdk, &backend, identity).await
}

/// [`refresh_contact_details`] for a caller holding the owner's contact
/// details write lock.
async fn refresh_contact_details_locked(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    backend: &WalletBackend,
    identity: &QualifiedIdentity,
) -> Result<(), TaskError> {
    let owner = identity.identity.id();
    backend.dashpay_sync_contact_infos(&owner).await?;
    let held = backend.dashpay_contact_details(&owner).await;
    if held.is_empty() {
        return Ok(());
    }
    let stored = super::contacts::stored_contact_details(app_context, sdk, identity).await?;
    let published = stored.published?;
    let legacy = stored.legacy?;
    let mut synced = BTreeSet::new();
    for contact_id in held.keys() {
        if backend.dashpay_details_synced(&owner, contact_id)? {
            synced.insert(*contact_id);
        }
    }
    let plan = plan_details_refresh(held, published, &legacy, &synced);
    backend
        .dashpay_apply_contact_details(&owner, plan.updates)
        .await?;
    for contact_id in plan.now_held {
        backend.dashpay_mark_details_synced(&owner, &contact_id)?;
    }
    Ok(())
}

/// What a contact details refresh changes in platform-wallet.
#[derive(Debug, Default, PartialEq)]
struct DetailsRefreshPlan {
    /// The details to store, for contacts whose shown details differ from
    /// the held ones.
    updates: BTreeMap<Identifier, ContactDetails>,
    /// Contacts platform-wallet holds the details of from now on, so a legacy
    /// document no longer stands in for them.
    now_held: Vec<Identifier>,
}

/// Decide a refresh of the `held` details from the `published` and `legacy`
/// documents; `synced` lists the contacts already held before.
fn plan_details_refresh(
    held: BTreeMap<Identifier, ContactDetails>,
    mut published: BTreeMap<Identifier, ContactDetails>,
    legacy: &std::collections::HashMap<Identifier, ContactData>,
    synced: &BTreeSet<Identifier>,
) -> DetailsRefreshPlan {
    let mut plan = DetailsRefreshPlan::default();
    for (contact_id, local) in held {
        let published = published.remove(&contact_id);
        let was_published = published.is_some();
        let synced_before = synced.contains(&contact_id);
        let shown = shown_contact_details(
            published,
            Some(local.clone()),
            synced_before,
            legacy.get(&contact_id).map(ContactData::details),
        );
        // platform-wallet holds what was published, or what it now stores.
        if !synced_before && (was_published || !shown.is_empty()) {
            plan.now_held.push(contact_id);
        }
        if shown != local {
            plan.updates.insert(contact_id, shown);
        }
    }
    plan
}

/// The details shown for `contact_id` once [`refresh_contact_details`] ran.
///
/// # Errors
///
/// As for [`refresh_contact_details`]: a save keeps every field it does not
/// replace from this read, so it must not guess.
async fn shown_details_for(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    backend: &WalletBackend,
    identity: &QualifiedIdentity,
    contact_id: &Identifier,
) -> Result<ContactDetails, TaskError> {
    refresh_contact_details_locked(app_context, sdk, backend, identity).await?;
    Ok(backend
        .dashpay_contact_details(&identity.identity.id())
        .await
        .remove(contact_id)
        .unwrap_or_default())
}

/// The reads and writes one contact details save performs, behind a trait so
/// their order and failures can be driven in tests.
trait SaveOps {
    /// The details shown for the contact now; the fields an update keeps.
    fn current(&self) -> impl Future<Output = Result<ContactDetails, TaskError>> + Send;
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
/// saved from a guess. A save whose synced record fails is reported as
/// failed, since the old legacy details would show again on reload.
async fn save_contact_details<O: SaveOps>(
    ops: &O,
    update: &ContactInfoUpdate,
) -> Result<bool, TaskError> {
    let details = update.apply_to(ops.current().await?);

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

    let outcome = ops.publish(details).await?;
    ops.mark_synced()?;
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
    async fn current(&self) -> Result<ContactDetails, TaskError> {
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
        mark_fails: bool,
        published: std::sync::Mutex<Vec<ContactDetails>>,
    }

    impl ScriptedSave {
        fn new(current: Result<ContactDetails, TaskError>, mark_fails: bool) -> Self {
            Self {
                current: std::sync::Mutex::new(Some(current)),
                mark_fails,
                published: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl SaveOps for ScriptedSave {
        async fn current(&self) -> Result<ContactDetails, TaskError> {
            self.current.lock().unwrap().take().expect("read once")
        }

        async fn publish(
            &self,
            details: ContactDetails,
        ) -> Result<ContactInfoPublishOutcome, TaskError> {
            self.published.lock().unwrap().push(details);
            Ok(ContactInfoPublishOutcome::DeferredUntilTwoContacts)
        }

        fn mark_synced(&self) -> Result<(), TaskError> {
            if self.mark_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
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

    /// A save whose synced record fails is not reported as saved: on reload
    /// the old legacy details would show again.
    #[tokio::test]
    async fn a_failed_synced_record_fails_the_save() {
        let ops = ScriptedSave::new(Ok(ContactDetails::default()), true);

        let result =
            save_contact_details(&ops, &ContactInfoUpdate::replace_all(None, None, false)).await;

        assert!(matches!(
            result,
            Err(TaskError::DashpaySidecarStorage { .. })
        ));
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
    ) -> Result<std::collections::HashMap<Identifier, crate::model::dashpay::ContactData>, TaskError>
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
        std::collections::HashMap<Identifier, crate::model::dashpay::ContactData>,
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

    fn noted(note: &str) -> ContactDetails {
        ContactDetails {
            note: Some(note.to_owned()),
            ..ContactDetails::default()
        }
    }

    fn legacy_noted(contact: Identifier, note: &str) -> ContactData {
        ContactData {
            identity_id: contact,
            nickname: None,
            note: Some(note.to_owned()),
            is_hidden: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }
    }

    #[test]
    fn published_details_replace_the_held_ones_and_are_held_from_then_on() {
        let contact = Identifier::from([1; 32]);
        let plan = plan_details_refresh(
            BTreeMap::from([(contact, noted("old"))]),
            BTreeMap::from([(contact, noted("published"))]),
            &std::collections::HashMap::new(),
            &BTreeSet::new(),
        );
        assert_eq!(
            plan.updates,
            BTreeMap::from([(contact, noted("published"))])
        );
        assert_eq!(plan.now_held, vec![contact]);
    }

    #[test]
    fn a_legacy_document_is_taken_over_once() {
        let contact = Identifier::from([2; 32]);
        let legacy = std::collections::HashMap::from([(contact, legacy_noted(contact, "legacy"))]);
        let first = plan_details_refresh(
            BTreeMap::from([(contact, ContactDetails::default())]),
            BTreeMap::new(),
            &legacy,
            &BTreeSet::new(),
        );
        assert_eq!(first.updates, BTreeMap::from([(contact, noted("legacy"))]));
        assert_eq!(first.now_held, vec![contact]);

        // The user cleared the note afterwards: the legacy document must not
        // bring it back.
        let later = plan_details_refresh(
            BTreeMap::from([(contact, ContactDetails::default())]),
            BTreeMap::new(),
            &legacy,
            &BTreeSet::from([contact]),
        );
        assert_eq!(later, DetailsRefreshPlan::default());
    }

    #[test]
    fn nothing_changes_when_nothing_is_stored() {
        let contact = Identifier::from([3; 32]);
        let plan = plan_details_refresh(
            BTreeMap::from([(contact, ContactDetails::default())]),
            BTreeMap::new(),
            &std::collections::HashMap::new(),
            &BTreeSet::new(),
        );
        assert_eq!(plan, DetailsRefreshPlan::default());
    }

    #[test]
    fn held_details_already_shown_are_not_rewritten() {
        let contact = Identifier::from([4; 32]);
        let plan = plan_details_refresh(
            BTreeMap::from([(contact, noted("mine"))]),
            BTreeMap::new(),
            &std::collections::HashMap::new(),
            &BTreeSet::from([contact]),
        );
        assert_eq!(plan, DetailsRefreshPlan::default());
    }
}
