//! Contact details (DIP-0015 `contactInfo`).
//!
//! platform-wallet writes and syncs the identity's `contactInfo` documents.
//! This module keeps the paginated document fetch the read of older,
//! legacy-key documents uses, and turns a field-level update into the full
//! set of details platform-wallet publishes.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{ContactInfoUpdate, validate_contact_note};
use crate::model::qualified_identity::QualifiedIdentity;
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
    let _write_guard = app_context
        .contact_info_writes
        .acquire(identity.identity.id())
        .await;
    let backend = app_context.wallet_backend()?;
    let owner = identity.identity.id();

    backend.dashpay_sync_contact_infos(&owner).await?;
    let local = backend
        .dashpay_contact_details(&owner)
        .await
        .remove(&contact_user_id);
    let mut stored = super::contacts::stored_contact_details(app_context, sdk, &identity).await?;
    let published = stored.published.remove(&contact_user_id);
    let synced_before = backend.dashpay_details_synced(&owner, &contact_user_id)
        || super::contacts::holds_details(published.as_ref(), local.as_ref());
    let legacy = stored
        .legacy
        .get(&contact_user_id)
        .map(super::contacts::ContactData::details);
    let current = super::contacts::shown_contact_details(published, local, synced_before, legacy);
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

    let outcome = backend
        .dashpay_set_contact_info(&identity, &contact_user_id, details)
        .await?;
    // platform-wallet holds these details now, published or not.
    if let Err(error) = backend.dashpay_mark_details_synced(&owner, &contact_user_id) {
        tracing::debug!(
            ?error,
            "could not record that the contact details are synced"
        );
    }
    let published = match outcome {
        ContactInfoPublishOutcome::Published => true,
        ContactInfoPublishOutcome::DeferredUntilTwoContacts => false,
        // The write checks for a wallet identity index before platform-wallet
        // runs, so this is reached only if the two disagree.
        ContactInfoPublishOutcome::SkippedWatchOnly => {
            return Err(TaskError::ContactInfoIdentityNotFromWallet);
        }
    };
    Ok(BackendTaskSuccessResult::DashPayContactInfoUpdated {
        identity: owner,
        contact_id: contact_user_id,
        published,
    })
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
