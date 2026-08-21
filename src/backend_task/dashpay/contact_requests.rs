use super::contact_request_query;
use super::encryption::{
    encrypt_account_label, encrypt_extended_public_key, generate_ecdh_shared_key,
};
use super::errors::DashPayError;
use super::validation::validate_contact_request_before_send;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::auto_accept_proof::{
    AutoAcceptProofData, create_auto_accept_proof_bytes_with_key,
};
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{
    ContactInfoUpdate, UnreadableContactInfoPolicy, contact_request_recipient,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::{ContactRequestActionKind, ContactRequestActionPhase};
// Upstream contact-request type: used to record the sent request in the
// local wallet-manager so dashpay_sync can auto-establish the contact.
use bip39::rand::{SeedableRng, rngs::StdRng};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{Document as DppDocument, DocumentV0, DocumentV0Getters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::{Bytes32, Value};
use dash_sdk::drive::query::{OrderClause, SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::documents::transitions::DocumentCreateTransitionBuilder;
use dash_sdk::platform::{
    Document, DocumentQuery, Fetch, FetchMany, FetchUnproved, Identifier, IdentityPublicKey,
};
use dash_sdk::query_types::{CurrentQuorumsInfo, NoParamQuery};
use platform_wallet::ContactRequest as UpstreamContactRequest;
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::sync::Arc;

pub async fn load_contact_requests(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let identity_id = identity.identity.id();

    tracing::info!(
        "Loading contact requests for identity: {}",
        identity_id.to_string(Encoding::Base58)
    );

    // Query for incoming contact requests (where toUserId == our identity)
    let mut incoming_query = contact_request_query(app_context)?;

    incoming_query = incoming_query.with_where(WhereClause {
        field: "toUserId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });

    // Without this orderBy, the query returns 0 results even when documents exist
    incoming_query = incoming_query.with_order_by(OrderClause {
        field: "$createdAt".to_string(),
        ascending: true,
    });
    incoming_query.limit = 50;

    // Query for outgoing contact requests (where $ownerId == our identity)
    let mut outgoing_query = contact_request_query(app_context)?;

    outgoing_query = outgoing_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });

    // Without this orderBy, the query may return 0 results even when documents exist
    outgoing_query = outgoing_query.with_order_by(OrderClause {
        field: "$createdAt".to_string(),
        ascending: true,
    });
    outgoing_query.limit = 50;

    // Fetch both types of requests
    tracing::info!("Fetching incoming contact requests...");
    let incoming_docs = Document::fetch_many(sdk, incoming_query).await?;
    tracing::info!("Fetched {} incoming documents", incoming_docs.len());

    tracing::info!("Fetching outgoing contact requests...");
    let outgoing_docs = Document::fetch_many(sdk, outgoing_query).await?;
    tracing::info!("Fetched {} outgoing documents", outgoing_docs.len());

    // Convert to vec of tuples (id, document)
    let mut incoming: Vec<(Identifier, Document)> = incoming_docs
        .into_iter()
        .filter_map(|(id, doc)| doc.map(|d| (id, d)))
        .collect();

    let mut outgoing: Vec<(Identifier, Document)> = outgoing_docs
        .into_iter()
        .filter_map(|(id, doc)| doc.map(|d| (id, d)))
        .collect();

    // Filter out mutual requests (where both parties have sent requests to each other)
    // These are now contacts, not pending requests
    let contacts_established: HashSet<Identifier> = incoming
        .iter()
        .map(|(_, doc)| doc.owner_id())
        .filter(|from_id| {
            outgoing
                .iter()
                .any(|(_, doc)| contact_request_recipient(doc).as_ref() == Some(from_id))
        })
        .collect();

    // Filter out established contacts from both lists. An outgoing document
    // with an unreadable recipient is kept: it cannot be attributed, and hiding
    // a request the user may still be waiting on is the worse failure.
    incoming.retain(|(_, doc)| !contacts_established.contains(&doc.owner_id()));

    outgoing.retain(|(_, doc)| match contact_request_recipient(doc) {
        Some(to_id) => !contacts_established.contains(&to_id),
        None => true,
    });

    // Drop requests the user has already resolved locally (declined an incoming
    // one, or withdrawn a sent one). Platform keeps contactRequest documents
    // forever, so without this the resolved row reappears on every reload.
    let backend = app_context.wallet_backend().ok();
    retain_unresolved(
        &mut incoming,
        &mut outgoing,
        |sender| match &backend {
            Some(backend) => backend.dashpay_is_declined(&identity_id, sender),
            None => false,
        },
        |recipient| match &backend {
            Some(backend) => backend.dashpay_is_withdrawn(&identity_id, recipient),
            None => false,
        },
    );

    tracing::info!(
        "After filtering: {} incoming, {} outgoing contact requests",
        incoming.len(),
        outgoing.len()
    );

    Ok(BackendTaskSuccessResult::DashPayContactRequests {
        identity: identity_id,
        incoming,
        outgoing,
    })
}

/// Drop every request the user has already resolved: an incoming one they
/// declined (per `is_declined`, keyed on the sender) or a sent one they withdrew
/// (per `is_withdrawn`, keyed on the recipient).
///
/// The two directions are asked separately on purpose — withdrawing our request
/// to Bob resolves nothing about the request Bob sends us afterwards, and a
/// shared marker would silently hide it.
///
/// An outgoing document with an unreadable `toUserId` is kept: we cannot prove
/// it was resolved, and hiding a request the user may still be waiting on is the
/// worse failure.
fn retain_unresolved(
    incoming: &mut Vec<(Identifier, Document)>,
    outgoing: &mut Vec<(Identifier, Document)>,
    is_declined: impl Fn(&Identifier) -> bool,
    is_withdrawn: impl Fn(&Identifier) -> bool,
) {
    incoming.retain(|(_, doc)| !is_declined(&doc.owner_id()));
    outgoing.retain(|(_, doc)| match contact_request_recipient(doc) {
        Some(to) => !is_withdrawn(&to),
        None => true,
    });
}

/// Verify that `doc` is a contact request `owner` actually sent, and return its
/// recipient. Guards the cancel path against acting on a stale UI row that
/// belongs to a different identity.
fn recipient_of_sent_request(
    doc: &Document,
    owner: &Identifier,
) -> Result<Identifier, DashPayError> {
    if doc.owner_id() != *owner {
        return Err(DashPayError::ContactRequestNotSentByYou);
    }
    contact_request_recipient(doc).ok_or_else(|| DashPayError::InvalidDocument {
        reason: "contact request document is missing its toUserId field".to_string(),
    })
}

/// Verify that `doc` is a contact request addressed to `recipient`, and return
/// its sender.
///
/// Accepting or declining reads the counterparty off the fetched document, so
/// without this check a stale row — one the user clicked after switching
/// identity — would sign a real state transition under the wrong identity's key.
fn sender_of_received_request(
    doc: &Document,
    recipient: &Identifier,
) -> Result<Identifier, DashPayError> {
    match contact_request_recipient(doc) {
        Some(to) if to == *recipient => Ok(doc.owner_id()),
        Some(_) => Err(DashPayError::ContactRequestNotAddressedToYou),
        None => Err(DashPayError::InvalidDocument {
            reason: "contact request document is missing its toUserId field".to_string(),
        }),
    }
}

pub async fn send_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    signing_key: IdentityPublicKey,
    to_username_or_id: String,
    account_label: Option<String>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    send_contact_request_with_proof(
        app_context,
        sdk,
        identity,
        signing_key,
        to_username_or_id,
        account_label,
        None,
    )
    .await
}

pub async fn send_contact_request_with_proof(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    signing_key: IdentityPublicKey,
    to_username_or_id: String,
    account_label: Option<String>,
    qr_auto_accept: Option<AutoAcceptProofData>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    if let Some(label) = account_label.as_deref()
        && let Err(error) = crate::model::dashpay::validate_account_label(label)
    {
        return Err(DashPayError::AccountLabelTooLong {
            length: error.actual,
            max: error.max,
        }
        .into());
    }

    // Step 1: Resolve the recipient identity
    let to_username_or_id = to_username_or_id.trim().to_string();

    if crate::model::dpns::validate_dpns_input(&to_username_or_id).is_err() {
        return Err(TaskError::DashPay(DashPayError::InvalidUsername {
            username: to_username_or_id.clone(),
        }));
    }

    let to_identity = if crate::model::dpns::has_dash_suffix(&to_username_or_id) {
        // It's a complete username, resolve via DPNS
        resolve_username_to_identity(app_context, sdk, &to_username_or_id).await?
    } else {
        // Try to parse as identity ID first
        match Identifier::from_string_try_encodings(
            &to_username_or_id,
            &[Encoding::Base58, Encoding::Hex],
        ) {
            Ok(to_id) => {
                // Successfully parsed as ID, fetch the identity
                Identity::fetch(sdk, to_id)
                    .await?
                    .ok_or(TaskError::IdentityNotFound)?
            }
            Err(_) => {
                // Not a valid ID format, assume it's a username without .dash suffix
                resolve_username_to_identity(app_context, sdk, &to_username_or_id).await?
            }
        }
    };

    let to_identity_id = to_identity.id();

    // Step 2: Reject self-contact (Platform would reject with code 40500 anyway)
    if to_identity_id == identity.identity.id() {
        return Err(TaskError::DashPay(DashPayError::CannotContactSelf));
    }

    // Step 3: Check if a contact request already exists
    let dashpay_contract = app_context.dashpay_contract.clone();
    let mut existing_query = contact_request_query(app_context)?;

    existing_query = existing_query
        .with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(identity.identity.id().to_buffer()),
        })
        .with_where(WhereClause {
            field: "toUserId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(to_identity_id.to_buffer()),
        });
    existing_query.limit = 1;

    let existing = Document::fetch_many(sdk, existing_query).await?;

    if !existing.is_empty() {
        return Err(DashPayError::ContactRequestAlreadySent {
            to: to_username_or_id.to_string(),
        }
        .into());
    }

    // Step 3: Get key indices for ECDH
    // Per DIP-11/DIP-15: Use ENCRYPTION key for sender (to encrypt outgoing),
    // DECRYPTION key for recipient (they will decrypt incoming)
    // Note: signing_key is an AUTHENTICATION key used to sign the state transition
    // We need a separate ENCRYPTION key for ECDH
    let sender_encryption_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::ENCRYPTION,
            HashSet::from([SecurityLevel::MEDIUM]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or_else(|| TaskError::DashPay(DashPayError::MissingEncryptionKey))?;

    // Find a recipient DECRYPTION key that supports ECDH (must be ECDSA_SECP256K1)
    // Platform enforces MEDIUM security level for ENCRYPTION/DECRYPTION keys.
    // This key belongs to the RECIPIENT (`to_identity`); its absence means the
    // recipient is not set up for DashPay contacts — not a sender-side fault.
    let recipient_key = to_identity
        .get_first_public_key_matching(
            Purpose::DECRYPTION,
            HashSet::from([SecurityLevel::MEDIUM]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or_else(|| TaskError::DashPay(DashPayError::RecipientMissingDecryptionKey))?;

    // Step 4: Generate ECDH shared key and encrypt data.
    // Resolve the ENCRYPTION private key through the JIT chokepoint — no
    // parked-seed read.
    let sender_private_key = identity
        .resolve_private_key_bytes(sender_encryption_key)
        .await?
        .map(|(_, private_key)| private_key)
        .ok_or_else(|| {
            TaskError::DashPay(DashPayError::PrivateKeyResolution {
                key_purpose: "ENCRYPTION".to_string(),
                reason: "Private key not loaded into Dash Evo Tool".to_string(),
            })
        })?;

    let shared_key = generate_ecdh_shared_key(&sender_private_key[..], recipient_key)
        .map_err(|e| TaskError::EncryptionError { detail: e })?;

    let network = app_context.network;

    // DIP-15 account index for the relationship. Receive-side derivation
    // (incoming_payments) must use the same value or funds land on
    // addresses we never scan.
    let account_index = 0u32;

    // Derive the contact-relationship xpub from the wallet's real HD seed,
    // obtained just-in-time through the JIT chokepoint. The wallet type and
    // seed stay inside the wallet_backend seam; we receive only the published
    // byte material plus the account reference. The seed determines where the
    // contact's payments land, so it must be the same HD seed the receive-side
    // address derivation uses.
    let seed_hash = identity
        .dashpay_wallet_seed_hash()
        .ok_or(TaskError::ContactWalletSeedUnavailable)?;
    let owner_id = identity.identity.id();
    let contact_material = app_context
        .wallet_backend()?
        .secret_access()
        .with_secret(
            &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
            |plaintext| {
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                crate::wallet_backend::derive_contact_xpub_material(
                    seed,
                    network,
                    account_index,
                    &owner_id,
                    &to_identity_id,
                    &sender_private_key,
                )
            },
        )
        .await?;
    let parent_fingerprint = contact_material.parent_fingerprint;
    let chain_code = contact_material.chain_code;
    let contact_public_key = contact_material.public_key;
    let account_reference = contact_material.account_reference;

    let encrypted_public_key = encrypt_extended_public_key(
        parent_fingerprint,
        chain_code,
        contact_public_key,
        &shared_key,
    )
    .map_err(|e| TaskError::EncryptionError { detail: e })?;

    // Step 5: Get the current core chain height for synchronization
    let (core_height, current_height_for_validation) =
        match CurrentQuorumsInfo::fetch_unproved(sdk, NoParamQuery {}).await {
            Ok(Some(quorum_info)) => (
                quorum_info.last_core_block_height,
                Some(quorum_info.last_core_block_height),
            ),
            Ok(None) => {
                (0u32, None) // Fallback if no quorum info available
            }
            Err(_e) => {
                (0u32, None) // Fallback on error
            }
        };

    // Step 5.5: Validate the contact request before proceeding
    // Note: We validate the ENCRYPTION key (used for ECDH), not the signing key
    let validation = validate_contact_request_before_send(
        sdk,
        &identity,
        sender_encryption_key.id(),
        to_identity.id(),
        recipient_key.id(),
        account_reference,
        core_height,
        current_height_for_validation,
    )
    .await
    .map_err(|e| DashPayError::ValidationFailed {
        errors: vec![e.to_string()],
    })?;

    // Check if validation passed
    if !validation.is_valid {
        return Err(DashPayError::ValidationFailed {
            errors: validation.errors.clone(),
        }
        .into());
    }

    // Log any warnings
    for _warning in &validation.warnings {}

    // Step 6: Create contact request document
    let mut properties = BTreeMap::new();
    properties.insert(
        "toUserId".to_string(),
        Value::Identifier(to_identity_id.to_buffer()),
    );
    properties.insert(
        "senderKeyIndex".to_string(),
        Value::U32(sender_encryption_key.id()),
    );
    properties.insert(
        "recipientKeyIndex".to_string(),
        Value::U32(recipient_key.id()),
    );
    // Account reference calculated per DIP-0015 (ASK-based shortening)
    properties.insert(
        "accountReference".to_string(),
        Value::U32(account_reference),
    );
    // Clone before the move so the record we build after document_create
    // carries the real encrypted xpub (matches upstream behaviour).
    let encrypted_public_key_record = encrypted_public_key.clone();
    properties.insert(
        "encryptedPublicKey".to_string(),
        Value::Bytes(encrypted_public_key),
    );

    // Note: $coreHeightCreatedAt is handled automatically by the platform

    // Add encrypted account label if provided
    if let Some(label) = account_label {
        let encrypted_label = encrypt_account_label(&label, &shared_key)
            .map_err(|e| TaskError::EncryptionError { detail: e })?;
        properties.insert(
            "encryptedAccountLabel".to_string(),
            Value::Bytes(encrypted_label),
        );
    }

    // If QR auto-accept data is provided, create the proof bytes now to match the final accountReference
    if let Some(qr) = qr_auto_accept {
        // Ensure the QR target matches the resolved recipient
        if qr.identity_id != to_identity_id {
            return Err(DashPayError::InvalidQrCode {
                reason: "QR code target identity does not match recipient".to_string(),
            }
            .into());
        }
        let proof = create_auto_accept_proof_bytes_with_key(
            qr.expires_at,
            &qr.proof_key,
            &identity.identity.id(),
            &to_identity_id,
            account_reference,
        )
        .map_err(|e| TaskError::EncryptionError { detail: e })?;
        tracing::debug!(
            "Including autoAcceptProof in contact request ({} bytes)",
            proof.len()
        );
        properties.insert("autoAcceptProof".to_string(), Value::Bytes(proof));
    }
    // If no proof, don't include the field at all (schema requires 38-102 bytes if present)

    // Generate random entropy for the document transition
    let mut rng = StdRng::from_entropy();
    let entropy = Bytes32::random_with_rng(&mut rng);

    // Generate deterministic document ID based on entropy
    let document_id = Document::generate_document_id_v0(
        &dashpay_contract.id(),
        &identity.identity.id(),
        "contactRequest",
        entropy.as_slice(),
    );

    // Create the document
    let document = DppDocument::V0(DocumentV0 {
        id: document_id,
        owner_id: identity.identity.id(),
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

    // Step 7: Submit the contact request
    // Use the selected signing key
    let identity_key = &signing_key;

    let mut builder = DocumentCreateTransitionBuilder::new(
        dashpay_contract,
        "contactRequest".to_string(),
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

    let result = sdk
        .document_create(builder, identity_key, &identity)
        .await?;

    // Log the proof-verified document and capture timestamps for the local
    // wallet-manager record.
    let (core_height_created_at, created_at_ts) = match result {
        dash_sdk::platform::documents::transitions::DocumentCreateResult::Document(doc) => {
            tracing::info!(
                "Contact request created: doc_id={}, revision={:?}",
                doc.id(),
                doc.revision()
            );
            (
                doc.created_at_core_block_height().unwrap_or(0),
                doc.created_at().unwrap_or(0),
            )
        }
    };

    // Mirror the sent request in the local wallet-manager.
    //
    // `dashpay_sync` auto-establishes a contact only when
    // `sent_contact_requests[peer]` already exists locally.
    // DET's custom send path bypasses
    // `IdentityWallet::send_contact_request_with_external_signer`, so we
    // must call `add_sent_contact_request` explicitly after the state
    // transition commits.  Without this, `established_contacts` is never
    // populated and contact receiving-account registration is silently
    // skipped for all users who send a contact request from DET.
    let contact_record = UpstreamContactRequest::new(
        owner_id,
        to_identity_id,
        sender_encryption_key.id(),
        recipient_key.id(),
        account_reference,
        encrypted_public_key_record,
        core_height_created_at,
        created_at_ts,
    );
    if let Ok(backend) = app_context.wallet_backend() {
        if let Err(err) = backend
            .record_sent_contact_request(&seed_hash, &owner_id, contact_record)
            .await
        {
            tracing::warn!(
                %err,
                "record_sent_contact_request failed; contact was sent but \
                 local wallet-manager state not updated",
            );
        }

        // Sending to someone retires an earlier decline of their request and an
        // earlier withdrawal of ours: the user has deliberately re-engaged, so
        // neither direction may stay filtered out of the list.
        // Both markers are cleared unconditionally — a failure on one must not
        // leave the other standing.
        let declined = backend.dashpay_unmark_declined(&owner_id, &to_identity_id);
        let withdrawn = backend.dashpay_unmark_withdrawn(&owner_id, &to_identity_id);
        if let Err(err) = declined.and(withdrawn) {
            tracing::debug!(
                %err,
                "Clearing the stale resolution markers failed; an earlier declined or \
                 withdrawn request involving this person may stay hidden",
            );
        }
    }

    Ok(BackendTaskSuccessResult::DashPayContactRequestSent(
        to_username_or_id.to_string(),
    ))
}

async fn resolve_username_to_identity(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    username: &str,
) -> Result<Identity, TaskError> {
    let normalized = crate::model::dpns::normalize_dpns_label(username);

    // Use the cached DPNS contract from AppContext instead of fetching from network
    let domain_query = DocumentQuery {
        select: SelectProjection::documents(),
        data_contract: app_context.dpns_contract.clone(),
        document_type_name: "domain".to_string(),
        where_clauses: vec![
            WhereClause {
                field: "normalizedParentDomainName".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Text("dash".to_string()),
            },
            WhereClause {
                field: "normalizedLabel".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Text(normalized),
            },
        ],
        group_by: Vec::new(),
        having: Vec::new(),
        order_by_clauses: vec![],
        limit: 1,
        offset: None,
        start: None,
    };

    let results = Document::fetch_many(sdk, domain_query).await?;

    let document = results
        .values()
        .filter_map(|maybe_doc| maybe_doc.as_ref())
        .next()
        .ok_or_else(|| {
            TaskError::DashPay(DashPayError::UsernameResolutionFailed {
                username: username.to_string(),
            })
        })?;

    // Extract the identity ID from records.identity — this is the authoritative
    // identity reference, which may differ from owner_id() after name transfers.
    let identity_id = crate::model::dpns::extract_identity_id_from_dpns_document(document)
        .ok_or_else(|| {
            TaskError::DashPay(DashPayError::InvalidDocument {
                reason: format!(
                    "DPNS document for '{}' is missing records.identity field",
                    username
                ),
            })
        })?;

    // Fetch the identity
    Identity::fetch(sdk, identity_id)
        .await?
        .ok_or(TaskError::IdentityNotFound)
}

pub async fn accept_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let owner_id = identity.identity.id();
    let _action_guard = app_context
        .wallet_backend()?
        .dashpay_lock_contact_request_action(&owner_id, &request_id)
        .await;

    // According to DashPay DIP, accepting means sending a contact request back
    // First, we need to fetch the incoming contact request to get the sender's identity

    // Fetch the specific contact request document by creating a query with its ID
    let query_with_id =
        DocumentQuery::with_document_id(contact_request_query(app_context)?, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await?
        .ok_or(TaskError::DocumentNotFound)?;

    // Verify the request was addressed to us before acting on it, and get the
    // sender's identity (the owner of the incoming request).
    let from_identity_id = sender_of_received_request(&doc, &identity.identity.id())?;

    // Check if we already sent a contact request to this identity
    let mut existing_query = contact_request_query(app_context)?;

    existing_query = existing_query
        .with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(identity.identity.id().to_buffer()),
        })
        .with_where(WhereClause {
            field: "toUserId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(from_identity_id.to_buffer()),
        });
    existing_query.limit = 1;

    let existing = Document::fetch_many(sdk, existing_query).await?;

    if !existing.is_empty() {
        return Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished {
            request_id,
            contact_id: from_identity_id,
        });
    }

    // Get an AUTHENTICATION key for signing the state transition
    // Platform requires CRITICAL or HIGH security level for document creation
    let signing_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL, SecurityLevel::HIGH]),
            KeyType::all_key_types().into(),
            false,
        )
        .ok_or_else(|| TaskError::DashPay(DashPayError::MissingAuthenticationKey))?
        .clone();

    // Option A fix: record A's incoming CR into B's wallet-manager
    // BEFORE sending the reciprocal.
    //
    // After record_sent_contact_request populates sent[A], upstream
    // sync_contact_requests skips A's document (skip guard:
    //   sent[A] || incoming[A] || established[A] → continue)
    // so dashpay_sync can never call add_incoming_contact_request and
    // established_contacts stays empty.
    //
    // By pre-populating incoming[A] here, add_sent_contact_request for
    // B's outgoing CR (fired by record_sent_contact_request at the end of
    // send_contact_request_with_proof) finds incoming[A] and auto-establishes
    // established_contacts[A] in-process — no dashpay_sync round-trip needed.
    if let Some(seed_hash) = identity.dashpay_wallet_seed_hash() {
        let owner_id = identity.identity.id();
        let props = doc.properties();
        let maybe_incoming_cr = (|| -> Option<UpstreamContactRequest> {
            let sender_key_index = props.get("senderKeyIndex")?.to_integer::<u32>().ok()?;
            let recipient_key_index = props.get("recipientKeyIndex")?.to_integer::<u32>().ok()?;
            let account_reference = props.get("accountReference")?.to_integer::<u32>().ok()?;
            let encrypted_public_key = props.get("encryptedPublicKey")?.as_bytes()?.clone();
            Some(UpstreamContactRequest::new(
                from_identity_id, // sender = A
                owner_id,         // recipient = B
                sender_key_index,
                recipient_key_index,
                account_reference,
                encrypted_public_key,
                doc.created_at_core_block_height().unwrap_or(0),
                doc.created_at().unwrap_or(0),
            ))
        })();

        match maybe_incoming_cr {
            Some(incoming_cr) => {
                if let Ok(backend) = app_context.wallet_backend()
                    && let Err(err) = backend
                        .record_incoming_contact_request(&seed_hash, &owner_id, incoming_cr)
                        .await
                {
                    tracing::warn!(
                        %err,
                        "record_incoming_contact_request failed; \
                         auto-establishment will depend on dashpay_sync",
                    );
                }
            }
            None => {
                tracing::warn!(
                    sender_id = %from_identity_id,
                    "accept_contact_request: incoming CR document missing required \
                     fields; auto-establishment will depend on dashpay_sync",
                );
            }
        }
    }

    let result = send_contact_request(
        app_context,
        sdk,
        identity,
        signing_key,
        from_identity_id.to_string(Encoding::Base58),
        Some("Accepted contact".to_string()),
    )
    .await;

    match result {
        Ok(_) => Ok(BackendTaskSuccessResult::DashPayContactRequestAccepted(
            request_id,
        )),
        Err(e) => Err(e),
    }
}

pub async fn reject_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
    unreadable: UnreadableContactInfoPolicy,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // According to DashPay DIP, rejecting doesn't delete the request (they're immutable)
    // Instead, we should update our contactInfo document to mark this contact as hidden

    // First, fetch the contact request to get the sender's identity
    let query_with_id =
        DocumentQuery::with_document_id(contact_request_query(app_context)?, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await?
        .ok_or(TaskError::DocumentNotFound)?;

    // Captured before `identity` is moved into `create_or_update_contact_info`;
    // the decline marker is scoped to this acting identity.
    let owner_id = identity.identity.id();

    // Verify the request was addressed to us before declining it.
    let from_identity_id = sender_of_received_request(&doc, &owner_id)?;

    let ops = PlatformRejectOps {
        app_context,
        sdk,
        identity,
        owner_id,
        from_identity_id,
        request_id,
    };
    reject_flow(&ops, unreadable).await
}

trait RejectOps {
    fn lock_action(
        &self,
    ) -> impl Future<Output = Result<tokio::sync::OwnedMutexGuard<()>, TaskError>> + Send;
    fn phase(&self) -> Result<Option<ContactRequestActionPhase>, TaskError>;
    fn set_phase(&self, phase: ContactRequestActionPhase) -> Result<(), TaskError>;
    fn clear_phase(&self) -> Result<(), TaskError>;
    fn contact_is_hidden(
        &self,
        unreadable: UnreadableContactInfoPolicy,
    ) -> impl Future<Output = Result<bool, TaskError>> + Send;
    fn update_contact_info(
        &self,
        update: ContactInfoUpdate,
    ) -> impl Future<Output = Result<(), TaskError>> + Send;
    fn mark_declined(&self) -> Result<(), TaskError>;
    fn request_id(&self) -> Identifier;
}

async fn reject_flow<O: RejectOps>(
    ops: &O,
    unreadable: UnreadableContactInfoPolicy,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let _action_guard = ops.lock_action().await?;
    let phase = match ops.phase()? {
        Some(phase) => phase,
        None => {
            ops.set_phase(ContactRequestActionPhase::HideIntent)?;
            ContactRequestActionPhase::HideIntent
        }
    };

    if phase == ContactRequestActionPhase::HideIntent {
        if !ops.contact_is_hidden(unreadable).await? {
            ops.update_contact_info(hidden_contact_update(true, unreadable))
                .await?;
        }
        ops.set_phase(ContactRequestActionPhase::MarkerPending)?;
    }

    let result = complete_rejection(ops.request_id(), || ops.mark_declined())?;
    ops.clear_phase()?;
    Ok(result)
}

struct PlatformRejectOps<'a> {
    app_context: &'a Arc<AppContext>,
    sdk: &'a Sdk,
    identity: QualifiedIdentity,
    owner_id: Identifier,
    from_identity_id: Identifier,
    request_id: Identifier,
}

impl RejectOps for PlatformRejectOps<'_> {
    async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
        Ok(self
            .app_context
            .wallet_backend()?
            .dashpay_lock_contact_request_action(&self.owner_id, &self.request_id)
            .await)
    }

    fn phase(&self) -> Result<Option<ContactRequestActionPhase>, TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_contact_request_action_phase(
                &self.owner_id,
                &self.request_id,
                ContactRequestActionKind::Decline,
            )
    }

    fn set_phase(&self, phase: ContactRequestActionPhase) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_set_contact_request_action_phase(
                &self.owner_id,
                &self.request_id,
                ContactRequestActionKind::Decline,
                phase,
            )
    }

    fn clear_phase(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_clear_contact_request_action(
                &self.owner_id,
                &self.request_id,
                ContactRequestActionKind::Decline,
            )
    }

    async fn contact_is_hidden(
        &self,
        unreadable: UnreadableContactInfoPolicy,
    ) -> Result<bool, TaskError> {
        super::contact_info::contact_info_is_hidden(
            self.app_context,
            self.sdk,
            &self.identity,
            self.from_identity_id,
            unreadable,
        )
        .await
    }

    async fn update_contact_info(&self, update: ContactInfoUpdate) -> Result<(), TaskError> {
        super::contact_info::create_or_update_contact_info(
            self.app_context,
            self.sdk,
            self.identity.clone(),
            self.from_identity_id,
            update,
        )
        .await
        .map(|_| ())
    }

    fn mark_declined(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_mark_declined(&self.owner_id, &self.from_identity_id)
    }

    fn request_id(&self) -> Identifier {
        self.request_id
    }
}

/// The visibility flip that declining and withdrawing both write. Every other
/// stored detail is preserved: these actions hide a contact, they do not edit it.
fn hidden_contact_update(
    hidden: bool,
    unreadable: UnreadableContactInfoPolicy,
) -> ContactInfoUpdate {
    let mut update = ContactInfoUpdate::visibility(hidden);
    update.unreadable = unreadable;
    update
}

/// Return rejection success only after storing the local marker that retires the row.
/// The closure seam keeps the failure path deterministic in tests.
///
/// # Errors
///
/// The marker is the only thing that retires the row — Platform keeps the
/// `contactRequest` document forever — so a failed write must surface rather
/// than be reported as a completed rejection.
fn complete_rejection(
    request_id: Identifier,
    mark_declined: impl FnOnce() -> Result<(), TaskError>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    mark_declined()?;
    Ok(BackendTaskSuccessResult::DashPayContactRequestRejected(
        request_id,
    ))
}

/// What a cancellation attempt resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelOutcome {
    /// The request was pending throughout; it is now withdrawn.
    Withdrawn,
    /// The recipient had answered — there was nothing left to cancel.
    AlreadyEstablished,
}

/// The Platform reads and writes a cancellation performs.
///
/// Behind a trait so [`cancel_flow`]'s ordering — and the race window between
/// the reciprocal check and the hide broadcast — can be driven deterministically
/// in tests, which is impossible against a live Platform.
trait CancelOps {
    fn lock_action(
        &self,
    ) -> impl Future<Output = Result<tokio::sync::OwnedMutexGuard<()>, TaskError>> + Send;
    fn phase(&self) -> Result<Option<ContactRequestActionPhase>, TaskError>;
    fn set_phase(&self, phase: ContactRequestActionPhase) -> Result<(), TaskError>;
    fn clear_phase(&self) -> Result<(), TaskError>;

    /// `true` when the recipient has already sent a contact request back, which
    /// makes the pending request an established contact instead.
    fn reciprocal_request_exists(&self) -> impl Future<Output = Result<bool, TaskError>> + Send;

    fn contact_is_hidden(
        &self,
        unreadable: UnreadableContactInfoPolicy,
    ) -> impl Future<Output = Result<bool, TaskError>> + Send;

    /// Broadcast the complete visibility update for the recipient.
    fn update_contact_info(
        &self,
        update: ContactInfoUpdate,
    ) -> impl Future<Output = Result<(), TaskError>> + Send;

    /// Record the withdrawal in the DET sidecar so the request stops being
    /// listed as pending.
    ///
    /// # Errors
    ///
    /// The marker is the only thing that retires the row — Platform keeps the
    /// `contactRequest` document forever — so a failed write must surface
    /// rather than be reported as a completed cancellation.
    fn mark_withdrawn(&self) -> Result<(), TaskError>;
}

/// Check, hide, then re-check and undo the hide if the recipient answered inside
/// the window.
///
/// Platform has no conditional write, so the reciprocal check and the hide
/// broadcast cannot be one atomic step: they are two round-trips against two
/// different documents. The check is therefore the last read before the write,
/// and a second read straight after the write closes the gap retroactively — a
/// reciprocal request that lands mid-flight is detected and the hide is
/// reverted, leaving the freshly-established contact visible. That is the same
/// end state the caller would have reached had the reciprocal arrived a moment
/// earlier.
///
/// Residual risk: a reciprocal request landing *after* the second read still
/// leaves the contact hidden. It is not detectable from here at any window
/// width; recovery is the Contacts tab's hidden-contacts section, which can
/// unhide the contact.
async fn cancel_flow<O: CancelOps>(
    ops: &O,
    unreadable: UnreadableContactInfoPolicy,
) -> Result<CancelOutcome, TaskError> {
    let _action_guard = ops.lock_action().await?;
    let mut phase = match ops.phase()? {
        Some(phase) => phase,
        None => {
            if ops.reciprocal_request_exists().await? {
                return Ok(CancelOutcome::AlreadyEstablished);
            }
            ops.set_phase(ContactRequestActionPhase::HideIntent)?;
            ContactRequestActionPhase::HideIntent
        }
    };

    if phase == ContactRequestActionPhase::HideIntent {
        if !ops.contact_is_hidden(unreadable).await? {
            ops.update_contact_info(hidden_contact_update(true, unreadable))
                .await?;
        }
        ops.set_phase(ContactRequestActionPhase::HideCommitted)?;
        phase = ContactRequestActionPhase::HideCommitted;
    }

    if phase == ContactRequestActionPhase::HideCommitted {
        if ops.reciprocal_request_exists().await? {
            ops.set_phase(ContactRequestActionPhase::CorrectiveUnhideIntent)?;
            phase = ContactRequestActionPhase::CorrectiveUnhideIntent;
        } else {
            ops.set_phase(ContactRequestActionPhase::MarkerPending)?;
            phase = ContactRequestActionPhase::MarkerPending;
        }
    }

    if phase == ContactRequestActionPhase::CorrectiveUnhideIntent {
        if ops.contact_is_hidden(unreadable).await? {
            ops.update_contact_info(hidden_contact_update(false, unreadable))
                .await?;
        }
        ops.set_phase(ContactRequestActionPhase::CorrectiveUnhideComplete)?;
        phase = ContactRequestActionPhase::CorrectiveUnhideComplete;
    }

    if phase == ContactRequestActionPhase::CorrectiveUnhideComplete {
        ops.clear_phase()?;
        return Ok(CancelOutcome::AlreadyEstablished);
    }

    ops.mark_withdrawn()?;
    ops.clear_phase()?;
    Ok(CancelOutcome::Withdrawn)
}

/// [`CancelOps`] against the live Platform, for one sender/recipient pair.
struct PlatformCancelOps<'a> {
    app_context: &'a Arc<AppContext>,
    sdk: &'a Sdk,
    identity: QualifiedIdentity,
    /// The cancelling identity — the sender of the request being withdrawn.
    owner_id: Identifier,
    /// The recipient of the request being withdrawn.
    to_identity_id: Identifier,
    request_id: Identifier,
}

impl CancelOps for PlatformCancelOps<'_> {
    async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
        Ok(self
            .app_context
            .wallet_backend()?
            .dashpay_lock_contact_request_action(&self.owner_id, &self.request_id)
            .await)
    }

    fn phase(&self) -> Result<Option<ContactRequestActionPhase>, TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_contact_request_action_phase(
                &self.owner_id,
                &self.request_id,
                ContactRequestActionKind::Cancel,
            )
    }

    fn set_phase(&self, phase: ContactRequestActionPhase) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_set_contact_request_action_phase(
                &self.owner_id,
                &self.request_id,
                ContactRequestActionKind::Cancel,
                phase,
            )
    }

    fn clear_phase(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_clear_contact_request_action(
                &self.owner_id,
                &self.request_id,
                ContactRequestActionKind::Cancel,
            )
    }

    async fn reciprocal_request_exists(&self) -> Result<bool, TaskError> {
        let mut query = contact_request_query(self.app_context)?;
        query = query
            .with_where(WhereClause {
                field: "$ownerId".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(self.to_identity_id.to_buffer()),
            })
            .with_where(WhereClause {
                field: "toUserId".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(self.owner_id.to_buffer()),
            });
        query.limit = 1;

        Ok(!Document::fetch_many(self.sdk, query).await?.is_empty())
    }

    async fn update_contact_info(&self, update: ContactInfoUpdate) -> Result<(), TaskError> {
        super::contact_info::create_or_update_contact_info(
            self.app_context,
            self.sdk,
            self.identity.clone(),
            self.to_identity_id,
            update,
        )
        .await
        .map(|_| ())
    }

    async fn contact_is_hidden(
        &self,
        unreadable: UnreadableContactInfoPolicy,
    ) -> Result<bool, TaskError> {
        super::contact_info::contact_info_is_hidden(
            self.app_context,
            self.sdk,
            &self.identity,
            self.to_identity_id,
            unreadable,
        )
        .await
    }

    fn mark_withdrawn(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_mark_withdrawn(&self.owner_id, &self.to_identity_id)
    }
}

/// Cancel a contact request this identity sent and that is still pending.
///
/// DashPay `contactRequest` documents are immutable and cannot be deleted
/// (`documentsMutable: false`, `canBeDeleted: false` in the DashPay contract),
/// so the request cannot be un-sent from Platform. Cancelling therefore does
/// what `reject_contact_request` does for the incoming direction: it broadcasts
/// a `contactInfo` document marking the recipient hidden — a real, persisted
/// state transition — and records the withdrawal in the DET sidecar so the
/// request stops being listed as pending.
///
/// State is re-verified against Platform before acting: the request must still
/// exist, must have been sent by `identity`, and must not have been answered in
/// the meantime. See [`cancel_flow`] for how the answer race is handled.
pub async fn cancel_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
    unreadable: UnreadableContactInfoPolicy,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let owner_id = identity.identity.id();

    // Re-fetch the request rather than trusting the row the user clicked — the
    // list may be stale by seconds or by an identity switch.
    let query_with_id =
        DocumentQuery::with_document_id(contact_request_query(app_context)?, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await?
        .ok_or(TaskError::DocumentNotFound)?;

    // Verify we sent it, and to whom.
    let to_identity_id = recipient_of_sent_request(&doc, &owner_id)?;

    let ops = PlatformCancelOps {
        app_context,
        sdk,
        identity,
        owner_id,
        to_identity_id,
        request_id,
    };

    match cancel_flow(&ops, unreadable).await? {
        CancelOutcome::Withdrawn => Ok(BackendTaskSuccessResult::DashPayContactRequestCancelled(
            request_id,
        )),
        // The recipient answered: the pair is a contact now, so report the real
        // state and let the UI refresh into it.
        CancelOutcome::AlreadyEstablished => {
            Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished {
                request_id,
                contact_id: to_identity_id,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::document::DocumentV0;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    /// Build a `contactRequest`-shaped document. `to` of `None` omits the
    /// `toUserId` property entirely, modelling a malformed document.
    fn request_doc(owner: Identifier, to: Option<Identifier>) -> Document {
        let mut properties = BTreeMap::new();
        if let Some(to) = to {
            properties.insert("toUserId".to_string(), Value::Identifier(to.to_buffer()));
        }
        DppDocument::V0(DocumentV0 {
            id: id(99),
            owner_id: owner,
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
    fn recipient_of_sent_request_returns_the_recipient() {
        let doc = request_doc(id(1), Some(id(2)));
        assert_eq!(recipient_of_sent_request(&doc, &id(1)).unwrap(), id(2));
    }

    #[test]
    fn cancelling_someone_elses_request_is_rejected() {
        // A stale row from a previous identity must never cancel through.
        let doc = request_doc(id(1), Some(id(2)));
        assert!(matches!(
            recipient_of_sent_request(&doc, &id(3)),
            Err(DashPayError::ContactRequestNotSentByYou)
        ));
    }

    #[test]
    fn cancelling_a_malformed_request_is_rejected() {
        let doc = request_doc(id(1), None);
        assert!(matches!(
            recipient_of_sent_request(&doc, &id(1)),
            Err(DashPayError::InvalidDocument { .. })
        ));
    }

    #[test]
    fn accepting_a_request_addressed_to_someone_else_is_rejected() {
        // A stale row from a previous identity must never reach a signed
        // acceptance under the identity now in use.
        let doc = request_doc(id(1), Some(id(2)));
        assert!(matches!(
            sender_of_received_request(&doc, &id(3)),
            Err(DashPayError::ContactRequestNotAddressedToYou)
        ));
    }

    #[test]
    fn accepting_a_request_addressed_to_us_returns_its_sender() {
        let doc = request_doc(id(1), Some(id(2)));
        assert_eq!(sender_of_received_request(&doc, &id(2)).unwrap(), id(1));
    }

    #[test]
    fn accepting_a_malformed_request_is_rejected() {
        let doc = request_doc(id(1), None);
        assert!(matches!(
            sender_of_received_request(&doc, &id(2)),
            Err(DashPayError::InvalidDocument { .. })
        ));
    }

    #[test]
    fn retain_unresolved_drops_declined_and_withdrawn_rows() {
        // Incoming from id(1) — declined. Incoming from id(2) — still pending.
        let mut incoming = vec![
            (id(10), request_doc(id(1), Some(id(9)))),
            (id(11), request_doc(id(2), Some(id(9)))),
        ];
        // Outgoing to id(3) — withdrawn. Outgoing to id(4) — still pending.
        let mut outgoing = vec![
            (id(12), request_doc(id(9), Some(id(3)))),
            (id(13), request_doc(id(9), Some(id(4)))),
        ];

        retain_unresolved(
            &mut incoming,
            &mut outgoing,
            |sender| *sender == id(1),
            |recipient| *recipient == id(3),
        );

        assert_eq!(incoming.len(), 1, "the declined request must not be listed");
        assert_eq!(incoming[0].0, id(11));
        assert_eq!(
            outgoing.len(),
            1,
            "the withdrawn request must not be listed"
        );
        assert_eq!(outgoing[0].0, id(13));
    }

    #[test]
    fn retain_unresolved_keeps_rows_with_unreadable_recipients() {
        let mut incoming = Vec::new();
        let mut outgoing = vec![(id(12), request_doc(id(9), None))];

        retain_unresolved(&mut incoming, &mut outgoing, |_| true, |_| true);

        assert_eq!(
            outgoing.len(),
            1,
            "a request we cannot attribute must stay visible rather than be silently hidden"
        );
    }

    #[test]
    fn a_withdrawn_outgoing_request_does_not_hide_a_later_incoming_one() {
        // We (id 9) withdrew a request to id(2), so id(2) carries a withdrawal
        // marker. Later id(2) sends us a genuine request: it is new, unresolved
        // business and must be listed.
        let mut incoming = vec![(id(20), request_doc(id(2), Some(id(9))))];
        let mut outgoing = Vec::new();

        retain_unresolved(
            &mut incoming,
            &mut outgoing,
            /* is_declined = */ |_| false,
            /* is_withdrawn = */ |recipient| *recipient == id(2),
        );

        assert_eq!(
            incoming.len(),
            1,
            "withdrawing our own request must not silence the other side's new request"
        );
    }

    #[test]
    fn a_declined_incoming_request_does_not_hide_a_later_outgoing_one() {
        // The mirror case: we declined id(2)'s request, then changed our mind
        // and sent them one. Ours is pending until they answer it.
        let mut incoming = Vec::new();
        let mut outgoing = vec![(id(21), request_doc(id(9), Some(id(2))))];

        retain_unresolved(
            &mut incoming,
            &mut outgoing,
            /* is_declined = */ |sender| *sender == id(2),
            /* is_withdrawn = */ |_| false,
        );

        assert_eq!(
            outgoing.len(),
            1,
            "declining their earlier request must not silence the request we sent them"
        );
    }

    // -----------------------------------------------------------------
    // Cancellation flow — the reciprocal check and the hide broadcast are
    // separate Platform round-trips, so the window between them is where a
    // reciprocal request can slip in and get a fresh contact hidden.
    // -----------------------------------------------------------------

    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Scriptable [`CancelOps`]. `reciprocal` is consumed one answer per probe,
    /// which is how a reciprocal request is injected *inside* the window:
    /// `[false, true]` means "pending when we checked, established by the time
    /// the hide landed".
    #[derive(Default)]
    struct ScriptedOps {
        action_lock: Arc<tokio::sync::Mutex<()>>,
        reciprocal: Mutex<VecDeque<Result<bool, TaskError>>>,
        /// Every contact-info update, in call order.
        contact_updates: Mutex<Vec<ContactInfoUpdate>>,
        contact_hidden: Mutex<bool>,
        phase: Mutex<Option<ContactRequestActionPhase>>,
        withdrawn: Mutex<bool>,
        /// When set, the first hide broadcast fails.
        hide_fails: bool,
        /// When set, recording the withdrawal in the sidecar fails.
        withdraw_failures: Mutex<usize>,
    }

    impl ScriptedOps {
        fn with_reciprocal(answers: [bool; 2]) -> Self {
            Self {
                reciprocal: Mutex::new(answers.map(Ok).into()),
                ..Default::default()
            }
        }

        fn with_reciprocal_results(
            answers: impl IntoIterator<Item = Result<bool, TaskError>>,
        ) -> Self {
            Self {
                reciprocal: Mutex::new(answers.into_iter().collect()),
                ..Default::default()
            }
        }

        fn hidden_writes(&self) -> Vec<bool> {
            self.contact_updates
                .lock()
                .expect("not poisoned")
                .iter()
                .map(|update| update.display_hidden)
                .collect()
        }

        fn contact_updates(&self) -> Vec<ContactInfoUpdate> {
            self.contact_updates.lock().expect("not poisoned").clone()
        }

        fn was_withdrawn(&self) -> bool {
            *self.withdrawn.lock().expect("not poisoned")
        }
    }

    impl CancelOps for ScriptedOps {
        async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
            Ok(self.action_lock.clone().lock_owned().await)
        }

        fn phase(&self) -> Result<Option<ContactRequestActionPhase>, TaskError> {
            Ok(*self.phase.lock().expect("not poisoned"))
        }

        fn set_phase(&self, phase: ContactRequestActionPhase) -> Result<(), TaskError> {
            *self.phase.lock().expect("not poisoned") = Some(phase);
            Ok(())
        }

        fn clear_phase(&self) -> Result<(), TaskError> {
            *self.phase.lock().expect("not poisoned") = None;
            Ok(())
        }

        async fn reciprocal_request_exists(&self) -> Result<bool, TaskError> {
            self.reciprocal
                .lock()
                .expect("not poisoned")
                .pop_front()
                .unwrap_or(Ok(false))
        }

        async fn contact_is_hidden(
            &self,
            _unreadable: UnreadableContactInfoPolicy,
        ) -> Result<bool, TaskError> {
            let hidden = *self.contact_hidden.lock().expect("not poisoned");
            tokio::task::yield_now().await;
            Ok(hidden)
        }

        async fn update_contact_info(&self, update: ContactInfoUpdate) -> Result<(), TaskError> {
            if self.hide_fails {
                return Err(TaskError::DocumentNotFound);
            }
            self.contact_updates
                .lock()
                .expect("not poisoned")
                .push(update.clone());
            *self.contact_hidden.lock().expect("not poisoned") = update.display_hidden;
            Ok(())
        }

        fn mark_withdrawn(&self) -> Result<(), TaskError> {
            let mut failures = self.withdraw_failures.lock().expect("not poisoned");
            if *failures > 0 {
                *failures -= 1;
                return Err(TaskError::WalletBackendNotYetWired);
            }
            *self.withdrawn.lock().expect("not poisoned") = true;
            Ok(())
        }
    }

    fn assert_preserving_visibility_update(update: &ContactInfoUpdate, hidden: bool) {
        use crate::model::dashpay::{AcceptedAccounts, ContactInfoField};

        assert_eq!(update.nickname, ContactInfoField::Preserve);
        assert_eq!(update.note, ContactInfoField::Preserve);
        assert_eq!(update.accepted_accounts, AcceptedAccounts::Preserve);
        assert_eq!(update.display_hidden, hidden);
    }

    #[tokio::test]
    async fn cancelling_a_pending_request_preserves_unrelated_contact_details() {
        let ops = ScriptedOps::with_reciprocal([false, false]);

        let outcome = cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
            .await
            .expect("cancellation succeeds");

        assert_eq!(outcome, CancelOutcome::Withdrawn);
        assert_eq!(
            ops.hidden_writes(),
            vec![true],
            "a pending request must be hidden exactly once"
        );
        assert_preserving_visibility_update(&ops.contact_updates()[0], true);
        assert!(
            ops.was_withdrawn(),
            "the withdrawal must be recorded so the row stops being listed"
        );
    }

    #[tokio::test]
    async fn cancelling_an_answered_request_hides_nothing() {
        // The recipient answered before the user clicked Cancel.
        let ops = ScriptedOps::with_reciprocal([true, true]);

        let outcome = cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
            .await
            .expect("cancellation succeeds");

        assert_eq!(outcome, CancelOutcome::AlreadyEstablished);
        assert!(
            ops.hidden_writes().is_empty(),
            "an established contact must never be hidden by a cancellation"
        );
        assert!(!ops.was_withdrawn());
    }

    #[tokio::test]
    async fn a_reciprocal_request_landing_inside_the_window_leaves_the_contact_visible() {
        // Pending when checked, established by the time the hide broadcast
        // landed — the exact race the check-then-write ordering cannot prevent.
        let ops = ScriptedOps::with_reciprocal([false, true]);

        let outcome = cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
            .await
            .expect("cancellation succeeds");

        assert_eq!(
            outcome,
            CancelOutcome::AlreadyEstablished,
            "the caller must be told the pair is a contact, not that a request was cancelled"
        );
        assert_eq!(
            ops.hidden_writes(),
            vec![true, false],
            "the hide must be undone once the reciprocal request is detected, so the \
             freshly-established contact does not vanish"
        );
        assert!(
            !ops.was_withdrawn(),
            "an established contact must not be marked as a withdrawn request"
        );
    }

    #[tokio::test]
    async fn a_failed_hide_broadcast_records_no_withdrawal() {
        let ops = ScriptedOps {
            hide_fails: true,
            ..ScriptedOps::with_reciprocal([false, false])
        };

        assert!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_err(),
            "a failed broadcast must surface, not be swallowed"
        );
        assert!(
            !ops.was_withdrawn(),
            "the request is still pending on Platform, so it must keep being listed"
        );
    }

    #[tokio::test]
    async fn a_failed_withdrawal_record_is_not_reported_as_a_cancellation() {
        // The marker is what retires the row from the listing. Without it the
        // request comes back as pending on the next reload, so announcing a
        // successful cancellation would be a lie.
        let ops = ScriptedOps {
            withdraw_failures: Mutex::new(1),
            ..ScriptedOps::with_reciprocal([false, false])
        };

        assert!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_err(),
            "a withdrawal the sidecar refused must surface as an error, not as success"
        );
    }

    #[test]
    fn failed_mark_declined_write_surfaces_error_instead_of_rejected() {
        let result = complete_rejection(id(7), || {
            Err(TaskError::DashpaySidecarStorage {
                source: crate::wallet_backend::KvAdapterError::Truncated,
            })
        });

        assert!(matches!(
            result,
            Err(TaskError::DashpaySidecarStorage { .. })
        ));
    }

    #[derive(Default)]
    struct ScriptedRejectOps {
        action_lock: Arc<tokio::sync::Mutex<()>>,
        phase: Mutex<Option<ContactRequestActionPhase>>,
        hidden: Mutex<bool>,
        hidden_writes: Mutex<usize>,
        mark_attempts: Mutex<usize>,
        mark_failures: Mutex<usize>,
    }

    impl RejectOps for ScriptedRejectOps {
        async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
            Ok(self.action_lock.clone().lock_owned().await)
        }

        fn phase(&self) -> Result<Option<ContactRequestActionPhase>, TaskError> {
            Ok(*self.phase.lock().expect("not poisoned"))
        }

        fn set_phase(&self, phase: ContactRequestActionPhase) -> Result<(), TaskError> {
            *self.phase.lock().expect("not poisoned") = Some(phase);
            Ok(())
        }

        fn clear_phase(&self) -> Result<(), TaskError> {
            *self.phase.lock().expect("not poisoned") = None;
            Ok(())
        }

        async fn contact_is_hidden(
            &self,
            _unreadable: UnreadableContactInfoPolicy,
        ) -> Result<bool, TaskError> {
            let hidden = *self.hidden.lock().expect("not poisoned");
            tokio::task::yield_now().await;
            Ok(hidden)
        }

        async fn update_contact_info(&self, update: ContactInfoUpdate) -> Result<(), TaskError> {
            assert!(update.display_hidden);
            *self.hidden.lock().expect("not poisoned") = true;
            *self.hidden_writes.lock().expect("not poisoned") += 1;
            Ok(())
        }

        fn mark_declined(&self) -> Result<(), TaskError> {
            *self.mark_attempts.lock().expect("not poisoned") += 1;
            let mut failures = self.mark_failures.lock().expect("not poisoned");
            if *failures > 0 {
                *failures -= 1;
                return Err(TaskError::WalletBackendNotYetWired);
            }
            Ok(())
        }

        fn request_id(&self) -> Identifier {
            id(7)
        }
    }

    #[tokio::test]
    async fn decline_retry_after_marker_failure_does_not_rebroadcast_hide() {
        let ops = ScriptedRejectOps {
            mark_failures: Mutex::new(1),
            ..Default::default()
        };

        assert!(
            reject_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_err()
        );
        assert!(
            reject_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_ok()
        );
        assert_eq!(*ops.hidden_writes.lock().expect("not poisoned"), 1);
        assert_eq!(*ops.mark_attempts.lock().expect("not poisoned"), 2);
    }

    #[tokio::test]
    async fn concurrent_declines_share_one_paid_hide() {
        let ops = ScriptedRejectOps::default();

        let (first, second) = tokio::join!(
            reject_flow(&ops, UnreadableContactInfoPolicy::Abort),
            reject_flow(&ops, UnreadableContactInfoPolicy::Abort),
        );

        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(
            *ops.hidden_writes.lock().expect("not poisoned"),
            1,
            "backend serialization must permit only one paid hide"
        );
    }

    #[tokio::test]
    async fn cancel_retry_after_marker_failure_does_not_rebroadcast_hide() {
        let ops = ScriptedOps {
            withdraw_failures: Mutex::new(1),
            ..ScriptedOps::with_reciprocal([false, false])
        };

        assert!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_err()
        );
        assert_eq!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .expect("marker repair succeeds"),
            CancelOutcome::Withdrawn
        );
        assert_eq!(ops.hidden_writes(), vec![true]);
        assert!(ops.reciprocal.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn concurrent_cancellations_share_one_paid_hide() {
        let ops = ScriptedOps::with_reciprocal([false, false]);

        let (first, second) = tokio::join!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort),
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort),
        );

        assert_eq!(first.expect("first cancellation"), CancelOutcome::Withdrawn);
        assert_eq!(
            second.expect("second cancellation"),
            CancelOutcome::Withdrawn
        );
        assert_eq!(
            ops.hidden_writes(),
            vec![true],
            "backend serialization must permit only one paid hide"
        );
    }

    #[tokio::test]
    async fn cancel_retry_after_post_hide_probe_failure_does_not_rehide() {
        let ops = ScriptedOps::with_reciprocal_results([
            Ok(false),
            Err(TaskError::DocumentNotFound),
            Ok(false),
        ]);

        assert!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_err()
        );
        assert_eq!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .expect("post-hide probe resumes"),
            CancelOutcome::Withdrawn
        );
        assert_eq!(ops.hidden_writes(), vec![true]);
    }

    #[tokio::test]
    async fn cancel_retry_that_discovers_reciprocal_unhides_once() {
        let ops = ScriptedOps::with_reciprocal_results([
            Ok(false),
            Err(TaskError::DocumentNotFound),
            Ok(true),
        ]);

        assert!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .is_err()
        );
        assert_eq!(
            cancel_flow(&ops, UnreadableContactInfoPolicy::Abort)
                .await
                .expect("reciprocal retry succeeds"),
            CancelOutcome::AlreadyEstablished
        );
        assert_eq!(ops.hidden_writes(), vec![true, false]);
        assert!(!ops.was_withdrawn());
    }

    #[test]
    fn declining_preserves_unrelated_contact_details() {
        // The write `reject_contact_request` broadcasts. Declining hides the
        // sender; it must not volunteer empty details over what they stored.
        let update = hidden_contact_update(true, UnreadableContactInfoPolicy::Abort);

        assert_preserving_visibility_update(&update, true);
    }

    #[test]
    fn a_confirmed_overwrite_carries_the_users_choice_into_the_write() {
        use crate::model::dashpay::AcceptedAccounts;

        let update = hidden_contact_update(true, UnreadableContactInfoPolicy::Overwrite);

        assert_eq!(update.unreadable, UnreadableContactInfoPolicy::Overwrite);
        assert_eq!(
            update.accepted_accounts,
            AcceptedAccounts::Preserve,
            "confirming an overwrite of unreadable data must not also volunteer an empty account list"
        );
    }
}
