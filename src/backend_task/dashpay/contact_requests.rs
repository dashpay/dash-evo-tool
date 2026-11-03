use super::contact_request_query;
use super::encryption::{
    encrypt_account_label, encrypt_extended_public_key, generate_ecdh_shared_key,
};
use super::errors::DashPayError;
use super::validation::validate_contact_request_before_send;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::contact_request_recipient;
use crate::model::document::new_document_for_creation;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::{ContactRequestActionKind, HeldPeerLock};
// Upstream contact-request type: used to record the sent request in the
// local wallet-manager so dashpay_sync can auto-establish the contact.
use bip39::rand::{SeedableRng, rngs::StdRng};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::{Bytes32, Value};
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
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

    let backend = app_context.wallet_backend().ok();
    pending_requests(
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

/// Keep only the requests still waiting for an answer: drop the pairs that
/// are contacts (both sides sent a request, unless the owner withdrew theirs
/// or declined the other's — a reply to a withdrawn request is a new request
/// to answer), then every request the user resolved on this device; see
/// [`retain_unresolved`].
fn pending_requests(
    incoming: &mut Vec<(Identifier, Document)>,
    outgoing: &mut Vec<(Identifier, Document)>,
    is_declined: impl Fn(&Identifier) -> bool,
    is_withdrawn: impl Fn(&Identifier) -> bool,
) {
    // A sender the owner declined is not a contact until the owner
    // re-engages, which clears the decline.
    let contacts_established = super::contacts::mutual_contacts(incoming, outgoing, |peer| {
        is_withdrawn(peer) || is_declined(peer)
    });
    // An outgoing document with an unreadable recipient is kept: it cannot be
    // attributed, and hiding a request the user may still be waiting on is
    // the worse failure.
    incoming.retain(|(_, doc)| !contacts_established.contains(&doc.owner_id()));
    outgoing.retain(|(_, doc)| match contact_request_recipient(doc) {
        Some(to_id) => !contacts_established.contains(&to_id),
        None => true,
    });
    retain_unresolved(incoming, outgoing, is_declined, is_withdrawn);
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
    qr_code: Option<crate::model::dashpay_contact_qr::ContactQrCode>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    send_contact_request_inner(
        app_context,
        sdk,
        identity,
        signing_key,
        to_username_or_id,
        account_label,
        qr_code,
        None,
    )
    .await
}

/// Body of [`send_contact_request_with_proof`]; `held` is the request-action
/// lock a caller already holds for the recipient, if any.
#[allow(clippy::too_many_arguments)]
async fn send_contact_request_inner(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    signing_key: IdentityPublicKey,
    to_username_or_id: String,
    account_label: Option<String>,
    qr_code: Option<crate::model::dashpay_contact_qr::ContactQrCode>,
    held: Option<&HeldPeerLock>,
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
        resolve_username_to_identity(sdk, &to_username_or_id).await?
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
                resolve_username_to_identity(sdk, &to_username_or_id).await?
            }
        }
    };

    let to_identity_id = to_identity.id();

    // Step 2: Reject self-contact (Platform would reject with code 40500 anyway)
    if to_identity_id == identity.identity.id() {
        return Err(TaskError::DashPay(DashPayError::CannotContactSelf));
    }

    // Step 3: Check if a contact request already exists
    let dashpay_contract = app_context.dashpay_contract();
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
        // A withdrawn request still stands on Platform: sending again restores
        // it instead of publishing a duplicate. This also finishes a send whose
        // re-engagement failed after it was published.
        if let Ok(backend) = app_context.wallet_backend()
            && restore_under_peer_lock(&backend, identity.identity.id(), to_identity_id, held)
                .await?
        {
            return Ok(BackendTaskSuccessResult::DashPayContactRequestSent(
                to_username_or_id.to_string(),
            ));
        }
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

    // A scanned contact QR code signs the autoAcceptProof over the final
    // accountReference. The recipient was resolved from the code's own
    // username, so the proof is always addressed to the code's owner.
    if let Some(qr) = qr_code {
        let proof = qr.sign_proof(&identity.identity.id(), &to_identity_id, account_reference);
        tracing::debug!(
            "Including autoAcceptProof in contact request ({} bytes)",
            proof.len()
        );
        properties.insert("autoAcceptProof".to_string(), Value::Bytes(proof));
    }
    // If no proof, don't include the field at all (schema requires 38-102 bytes if present)

    // Generate random entropy for the document transition
    let mut rng = StdRng::from_entropy();
    let entropy = Bytes32::random_with_rng(&mut rng).to_buffer();
    let document_type = dashpay_contract
        .document_type_for_name("contactRequest")
        .map_err(|e| TaskError::DocumentPreparationFailed {
            source: Box::new(e.into()),
        })?;
    let document = new_document_for_creation(
        document_type,
        identity.identity.id(),
        properties,
        &entropy,
        sdk.version(),
    )
    .map_err(|source| TaskError::DocumentPreparationFailed { source })?;

    // Step 7: Submit the contact request
    // Use the selected signing key
    let identity_key = &signing_key;

    let mut builder = DocumentCreateTransitionBuilder::new(
        dashpay_contract,
        "contactRequest".to_string(),
        document,
        entropy,
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
        finish_send(
            &backend,
            &seed_hash,
            owner_id,
            to_identity_id,
            contact_record,
            held,
        )
        .await?;
    }

    Ok(BackendTaskSuccessResult::DashPayContactRequestSent(
        to_username_or_id.to_string(),
    ))
}

/// Record a just-published request locally and re-engage its recipient.
async fn finish_send(
    backend: &crate::wallet_backend::WalletBackend,
    seed_hash: &crate::model::wallet::WalletSeedHash,
    owner_id: Identifier,
    to_identity_id: Identifier,
    contact_record: UpstreamContactRequest,
    held: Option<&HeldPeerLock>,
) -> Result<(), TaskError> {
    // A cancel ignores the person and then records the withdrawal; a
    // re-engagement landing in between would leave this fresh request
    // withdrawn, so it waits for the person's lock.
    let _peer_guard = lock_peer_unless_held(backend, owner_id, to_identity_id, held).await;

    if let Err(err) = backend
        .record_sent_contact_request(seed_hash, &owner_id, contact_record)
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
    re_engage_after_send(&PlatformReEngage {
        backend,
        owner: owner_id,
        peer: to_identity_id,
    })
    .await
}

/// `owner`'s request-action lock for `peer`, or `None` when `held` already
/// is that lock: it is not reentrant, so a caller holding it hands it down.
/// Callers take it only around local writes, never across a network call.
async fn lock_peer_unless_held(
    backend: &crate::wallet_backend::WalletBackend,
    owner: Identifier,
    peer: Identifier,
    held: Option<&HeldPeerLock>,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    match held {
        Some(held) if held.covers(&owner, &peer) => None,
        _ => Some(
            backend
                .dashpay_lock_contact_request_action(&owner, &peer)
                .await,
        ),
    }
}

/// [`restore_withdrawn_request`] under `owner`'s lock for `peer`. A cancel
/// ignores the person and then records the withdrawal under that lock; a
/// restore landing in between would lift the ignore and leave the request
/// withdrawn while reporting it sent.
async fn restore_under_peer_lock(
    backend: &crate::wallet_backend::WalletBackend,
    owner: Identifier,
    peer: Identifier,
    held: Option<&HeldPeerLock>,
) -> Result<bool, TaskError> {
    let _peer_guard = lock_peer_unless_held(backend, owner, peer, held).await;
    restore_withdrawn_request(&PlatformReEngage {
        backend,
        owner,
        peer,
    })
    .await
}

async fn resolve_username_to_identity(sdk: &Sdk, username: &str) -> Result<Identity, TaskError> {
    // The name's own record names its identity, which after a transfer is not
    // the document's owner.
    let identity_id = sdk
        .resolve_dpns_name(username.trim())
        .await?
        .ok_or_else(|| {
            TaskError::DashPay(DashPayError::UsernameResolutionFailed {
                username: username.to_string(),
            })
        })?;

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
    // Accept, decline and cancel lock on the person too: a cancel of the
    // owner's request to them must not record a withdrawal while this accept
    // establishes the contact.
    let peer_lock = app_context
        .wallet_backend()?
        .dashpay_lock_contact_peer(&owner_id, &from_identity_id)
        .await;

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
        // Our earlier request still stands on Platform. If it was withdrawn,
        // accepting restores it, and with it the contact.
        let backend = app_context.wallet_backend()?;
        let restored = restore_under_peer_lock(
            &backend,
            identity.identity.id(),
            from_identity_id,
            Some(&peer_lock),
        )
        .await?;
        return Ok(if restored {
            BackendTaskSuccessResult::DashPayContactRequestAccepted(request_id)
        } else {
            BackendTaskSuccessResult::DashPayContactAlreadyEstablished {
                request_id,
                contact_id: from_identity_id,
            }
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

    // The reply is sent under the person's lock taken above, which the send
    // would otherwise wait on forever.
    let result = send_contact_request_inner(
        app_context,
        sdk,
        identity,
        signing_key,
        from_identity_id.to_string(Encoding::Base58),
        Some("Accepted contact".to_string()),
        None,
        Some(&peer_lock),
    )
    .await;

    match result {
        Ok(_) => Ok(BackendTaskSuccessResult::DashPayContactRequestAccepted(
            request_id,
        )),
        Err(e) => Err(e),
    }
}

/// The local state that re-engages someone the owner withdrew a request
/// from, behind a trait so the decision can be driven in tests.
trait ReEngageOps {
    /// # Errors
    ///
    /// The withdrawal record could not be read.
    fn is_withdrawn(&self) -> Result<bool, TaskError>;
    /// # Errors
    ///
    /// The decline record could not be read.
    fn is_declined(&self) -> Result<bool, TaskError>;
    /// Whether platform-wallet still ignores the person. A decline or
    /// withdrawal ignores them before writing its marker, so an ignore can
    /// outlive a marker write that failed (and a rollback that failed too).
    fn is_ignored(&self) -> impl Future<Output = bool> + Send;
    /// Lift the ignore and clear the withdrawal and decline markers.
    fn re_engage(&self) -> impl Future<Output = Result<(), TaskError>> + Send;
}

/// Restore a request the owner sent earlier, which still stands on Platform,
/// when this device still hides the person; returns whether it did. Nothing
/// is published.
///
/// The person is hidden after a withdrawal, or after a decline that a send
/// could not clear (see [`re_engage_after_send`]), so sending again retries.
/// An ignore left in platform-wallet without either marker is lifted the same
/// way; otherwise their reply would never establish the contact.
async fn restore_withdrawn_request<O: ReEngageOps>(ops: &O) -> Result<bool, TaskError> {
    if !(ops.is_withdrawn()? || ops.is_declined()? || ops.is_ignored().await) {
        return Ok(false);
    }
    ops.re_engage().await?;
    Ok(true)
}

/// Re-engage the recipient of a request that was just published.
///
/// # Errors
///
/// [`TaskError::DashPayContactRequestSentNotUpdated`] when this device could
/// not be updated. The ignore is lifted before the markers are cleared, so a
/// failure leaves a marker standing and sending again finishes the update
/// through [`restore_withdrawn_request`].
async fn re_engage_after_send<O: ReEngageOps>(ops: &O) -> Result<(), TaskError> {
    ops.re_engage()
        .await
        .map_err(|source| TaskError::DashPayContactRequestSentNotUpdated {
            source: Box::new(source),
        })
}

struct PlatformReEngage<'a> {
    backend: &'a crate::wallet_backend::WalletBackend,
    owner: Identifier,
    peer: Identifier,
}

impl ReEngageOps for PlatformReEngage<'_> {
    fn is_withdrawn(&self) -> Result<bool, TaskError> {
        self.backend
            .dashpay_withdrawal_recorded(&self.owner, &self.peer)
    }

    fn is_declined(&self) -> Result<bool, TaskError> {
        self.backend
            .dashpay_decline_recorded(&self.owner, &self.peer)
    }

    async fn is_ignored(&self) -> bool {
        self.backend
            .dashpay_sender_ignored(&self.owner, &self.peer)
            .await
    }

    async fn re_engage(&self) -> Result<(), TaskError> {
        // The ignore first: the withdrawal keeps the person out of the lists
        // until it is lifted, so a failure here leaves a retry possible.
        self.backend
            .dashpay_unignore_contact_sender(&self.owner, &self.peer)
            .await?;
        self.backend
            .dashpay_unmark_declined(&self.owner, &self.peer)?;
        self.backend
            .dashpay_unmark_withdrawn(&self.owner, &self.peer)
    }
}

pub async fn reject_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // A contactRequest document is immutable, and a contactInfo about someone
    // who is not a contact would publish whom this identity turned down, so
    // declining is local only: platform-wallet ignores the sender and the DET
    // marker retires the request.
    let query_with_id =
        DocumentQuery::with_document_id(contact_request_query(app_context)?, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await?
        .ok_or(TaskError::DocumentNotFound)?;

    let owner_id = identity.identity.id();

    // Verify the request was addressed to us before declining it.
    let from_identity_id = sender_of_received_request(&doc, &owner_id)?;

    let ops = PlatformRejectOps {
        app_context,
        owner_id,
        from_identity_id,
        request_id,
    };
    reject_flow(&ops).await
}

/// The local writes a decline performs, behind a trait so their order can be
/// driven in tests.
trait RejectOps {
    fn lock_action(
        &self,
    ) -> impl Future<Output = Result<tokio::sync::OwnedMutexGuard<()>, TaskError>> + Send;
    /// Forget the progress record an older version kept for this action.
    fn clear_phase(&self) -> Result<(), TaskError>;
    /// Ignore the sender in platform-wallet (local only), unless the contact
    /// is already established there; returns whether the ignore stands.
    fn ignore_sender(&self) -> impl Future<Output = Result<bool, TaskError>> + Send;
    /// Undo [`Self::ignore_sender`] when the decline cannot be recorded.
    fn unignore_sender(&self) -> impl Future<Output = Result<(), TaskError>> + Send;
    fn mark_declined(&self) -> Result<(), TaskError>;
    fn request_id(&self) -> Identifier;
    fn sender_id(&self) -> Identifier;
}

/// A decline or withdrawal changes only this device, so a failed local
/// record must not be reported as one whose change reached the network.
fn recorded_on_this_device(error: TaskError) -> TaskError {
    match error {
        TaskError::DashpaySidecarStorage { source } => {
            TaskError::DashPayRequestDecisionNotRecorded { source }
        }
        error => error,
    }
}

/// Record a decline or withdrawal whose ignore already stands; if the record
/// cannot be written, lift the ignore again so no half-done decision is left.
///
/// The ignore and the marker live in different stores (platform-wallet's and
/// DET's), so they cannot change together. A rollback that fails too is only
/// logged: the user is told the decision was not recorded, and the next send
/// or accept lifts the leftover ignore (see [`restore_withdrawn_request`]).
async fn record_or_unignore<U>(
    mark: Result<(), TaskError>,
    unignore: impl FnOnce() -> U,
) -> Result<(), TaskError>
where
    U: Future<Output = Result<(), TaskError>>,
{
    let Err(error) = mark else {
        return Ok(());
    };
    if let Err(rollback) = unignore().await {
        tracing::warn!(
            ?rollback,
            "could not lift the ignore of a decision that was not recorded"
        );
    }
    Err(recorded_on_this_device(error))
}

/// The progress record an older version kept is read by nothing once the
/// decision is recorded, so failing to forget it leaves an inert entry and
/// does not fail the action.
fn forget_phase(cleared: Result<(), TaskError>) {
    if let Err(error) = cleared {
        tracing::warn!(
            ?error,
            "could not forget an old contact request progress record"
        );
    }
}

async fn reject_flow<O: RejectOps>(ops: &O) -> Result<BackendTaskSuccessResult, TaskError> {
    let _action_guard = ops.lock_action().await?;
    let result = if ops.ignore_sender().await? {
        let recorded = record_or_unignore(ops.mark_declined(), || ops.unignore_sender()).await;
        complete_rejection(ops.request_id(), || recorded)?
    } else {
        // The contact was established before the ignore took effect.
        BackendTaskSuccessResult::DashPayContactAlreadyEstablished {
            request_id: ops.request_id(),
            contact_id: ops.sender_id(),
        }
    };
    forget_phase(ops.clear_phase());
    Ok(result)
}

struct PlatformRejectOps<'a> {
    app_context: &'a Arc<AppContext>,
    owner_id: Identifier,
    from_identity_id: Identifier,
    request_id: Identifier,
}

impl RejectOps for PlatformRejectOps<'_> {
    async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
        Ok(self
            .app_context
            .wallet_backend()?
            .dashpay_lock_contact_request_action(&self.owner_id, &self.from_identity_id)
            .await)
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

    async fn ignore_sender(&self) -> Result<bool, TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_ignore_unless_established(&self.owner_id, &self.from_identity_id)
            .await
    }

    async fn unignore_sender(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_unignore_contact_sender(&self.owner_id, &self.from_identity_id)
            .await
    }

    fn mark_declined(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_mark_declined(&self.owner_id, &self.from_identity_id)
    }

    fn request_id(&self) -> Identifier {
        self.request_id
    }

    fn sender_id(&self) -> Identifier {
        self.from_identity_id
    }
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
    /// The request was pending; it is now withdrawn.
    Withdrawn,
    /// The recipient had answered — there was nothing left to cancel.
    AlreadyEstablished,
}

/// The reads and local writes a cancellation performs, behind a trait so
/// their order can be driven in tests.
trait CancelOps {
    fn lock_action(
        &self,
    ) -> impl Future<Output = Result<tokio::sync::OwnedMutexGuard<()>, TaskError>> + Send;
    /// Forget the progress record an older version kept for this action.
    fn clear_phase(&self) -> Result<(), TaskError>;

    /// `true` when the recipient has already sent a contact request back, which
    /// makes the pending request an established contact instead.
    fn reciprocal_request_exists(&self) -> impl Future<Output = Result<bool, TaskError>> + Send;

    /// Ignore the recipient in platform-wallet (local only), so a reply to
    /// the withdrawn request cannot establish the contact on its own; returns
    /// whether the ignore stands, which it does not when the reply was taken
    /// in first and the contact is already established.
    fn ignore_recipient(&self) -> impl Future<Output = Result<bool, TaskError>> + Send;

    /// Undo [`Self::ignore_recipient`] when the withdrawal cannot be recorded.
    fn unignore_recipient(&self) -> impl Future<Output = Result<(), TaskError>> + Send;

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

/// Withdraw the request unless the recipient has answered it. Local only:
/// nothing is published.
async fn cancel_flow<O: CancelOps>(ops: &O) -> Result<CancelOutcome, TaskError> {
    let _action_guard = ops.lock_action().await?;
    let outcome = if ops.reciprocal_request_exists().await? {
        CancelOutcome::AlreadyEstablished
    } else if ops.ignore_recipient().await? {
        record_or_unignore(ops.mark_withdrawn(), || ops.unignore_recipient()).await?;
        CancelOutcome::Withdrawn
    } else {
        // The reply was taken in after the query, before the ignore.
        CancelOutcome::AlreadyEstablished
    };
    forget_phase(ops.clear_phase());
    Ok(outcome)
}

/// [`CancelOps`] against the live Platform, for one sender/recipient pair.
struct PlatformCancelOps<'a> {
    app_context: &'a Arc<AppContext>,
    sdk: &'a Sdk,
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
            .dashpay_lock_contact_request_action(&self.owner_id, &self.to_identity_id)
            .await)
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

    fn mark_withdrawn(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_mark_withdrawn(&self.owner_id, &self.to_identity_id)
    }

    async fn ignore_recipient(&self) -> Result<bool, TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_ignore_unless_established(&self.owner_id, &self.to_identity_id)
            .await
    }

    async fn unignore_recipient(&self) -> Result<(), TaskError> {
        self.app_context
            .wallet_backend()?
            .dashpay_unignore_contact_sender(&self.owner_id, &self.to_identity_id)
            .await
    }
}

/// Cancel a contact request this identity sent and that is still pending.
///
/// DashPay `contactRequest` documents are immutable and cannot be deleted
/// (`documentsMutable: false`, `canBeDeleted: false` in the DashPay contract),
/// so the request cannot be un-sent from Platform. Cancelling records the
/// withdrawal on this device only, so the request stops being listed as
/// pending; nothing is published.
///
/// State is re-verified against Platform before acting: the request must still
/// exist, must have been sent by `identity`, and must not have been answered in
/// the meantime.
pub async fn cancel_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
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
        owner_id,
        to_identity_id,
        request_id,
    };

    match cancel_flow(&ops).await? {
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
    use dash_sdk::dpp::document::{Document as DppDocument, DocumentV0};

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
            moderated_at: None,
            moderated_by: None,
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
            contract_version: None,
        })
    }

    /// Mock the registered-name lookup for the normalized label `normalized`.
    async fn expect_name_lookup(
        sdk: &mut Sdk,
        ctx: &AppContext,
        normalized: &str,
        found: Option<Document>,
    ) {
        let equals = |field: &str, value: &str| WhereClause {
            field: field.to_string(),
            operator: WhereOperator::Equal,
            value: Value::Text(value.to_string()),
        };
        let mut query = DocumentQuery::new(ctx.dpns_contract(), "domain")
            .expect("domain query")
            .with_where(equals("normalizedParentDomainName", "dash"))
            .with_where(equals("normalizedLabel", normalized));
        query.limit = 1;
        let documents: dash_sdk::query_types::Documents =
            found.map(|doc| (doc.id(), Some(doc))).into_iter().collect();
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, dash_sdk::query_types::Documents>(
                query,
                Some(documents),
            )
            .await
            .expect("name lookup expectation");
    }

    /// The network may hand the name's identity back as plain bytes; the name
    /// still belongs to that identity.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_username_resolves_when_its_identity_arrives_as_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let target = id(7);
        let mut sdk = crate::context::test_support::mock_sdk_for(&ctx);
        let found = crate::context::test_support::dpns_name_document(
            id(50),
            "Alice",
            Value::Bytes32(target.to_buffer()),
        );
        expect_name_lookup(&mut sdk, &ctx, "a11ce", Some(found)).await;
        let identity = Identity::create_basic_identity(
            target,
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("identity");
        sdk.mock()
            .expect_fetch(target, Some(identity))
            .await
            .expect("identity expectation");

        let resolved = resolve_username_to_identity(&sdk, "Alice.dash")
            .await
            .expect("the name resolves");

        assert_eq!(resolved.id(), target);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unregistered_username_is_reported_as_not_resolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut sdk = crate::context::test_support::mock_sdk_for(&ctx);
        expect_name_lookup(&mut sdk, &ctx, "n0b0dy", None).await;

        let error = resolve_username_to_identity(&sdk, "nobody")
            .await
            .expect_err("no such name");

        assert!(
            matches!(
                error,
                TaskError::DashPay(DashPayError::UsernameResolutionFailed { .. })
            ),
            "got {error:?}"
        );
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

    /// After the owner withdraws their request to
    /// id(3), id(3)'s later request is listed for the owner to answer; the
    /// pair is not taken for a contact and both rows dropped.
    #[test]
    fn a_reply_to_a_withdrawn_request_is_listed_to_answer() {
        let owner = id(9);
        let mut incoming = vec![
            (id(10), request_doc(id(3), Some(owner))),
            (id(11), request_doc(id(4), Some(owner))),
        ];
        let mut outgoing = vec![
            (id(12), request_doc(owner, Some(id(3)))),
            (id(13), request_doc(owner, Some(id(4)))),
        ];

        pending_requests(
            &mut incoming,
            &mut outgoing,
            |_| false,
            |peer| *peer == id(3),
        );

        assert_eq!(
            incoming.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![id(10)],
            "id(3)'s reply waits for an answer; id(4) is a contact"
        );
        assert!(outgoing.is_empty(), "the withdrawn request is not listed");
    }

    /// A declined sender is not a contact until the owner re-engages, even
    /// when the owner's own request to them is still on Platform: that sent
    /// request stays listed, so sending it again can finish the update.
    #[test]
    fn a_declined_sender_with_a_sent_request_is_not_a_contact() {
        let owner = id(9);
        let mut incoming = vec![(id(10), request_doc(id(3), Some(owner)))];
        let mut outgoing = vec![(id(12), request_doc(owner, Some(id(3))))];

        pending_requests(
            &mut incoming,
            &mut outgoing,
            |peer| *peer == id(3),
            |_| false,
        );

        assert!(incoming.is_empty(), "the declined request stays declined");
        assert_eq!(
            outgoing.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![id(12)]
        );
    }

    /// A request record for `finish_send`; this test wallet has no such
    /// identity, so recording it only logs.
    fn sent_record(owner: Identifier, peer: Identifier) -> UpstreamContactRequest {
        UpstreamContactRequest::new(
            owner,
            peer,
            0,
            0,
            0,
            bip39::rand::random::<[u8; 32]>().to_vec(),
            0,
            0,
        )
    }

    /// A cancel holding the person's lock is not undone halfway: the send
    /// waits for it, then retires the withdrawal it recorded, so the fresh
    /// request stays listed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_send_finishing_during_a_cancel_waits_and_then_retires_the_withdrawal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = app_context.wallet_backend().expect("backend");
        let (owner, peer) = (id(1), id(2));
        let cancel_guard = backend
            .dashpay_lock_contact_request_action(&owner, &peer)
            .await;

        // Driven on this task, not spawned, so it has run before the cancel
        // records its withdrawal whether or not it waits.
        let seed_hash = bip39::rand::random::<[u8; 32]>();
        let mut send = Box::pin(finish_send(
            &backend,
            &seed_hash,
            owner,
            peer,
            sent_record(owner, peer),
            None,
        ));
        assert_waits(&mut send).await;

        backend
            .dashpay_mark_withdrawn(&owner, &peer)
            .expect("withdraw");
        drop(cancel_guard);
        send.await.expect("send finished");

        assert!(!backend.dashpay_disengaged_from(&owner, &peer));
    }

    /// Drive `fut` long enough for it to finish if nothing blocks it, and
    /// assert it did not: it waits on a lock held by the test.
    async fn assert_waits<F: Future + Unpin>(fut: &mut F) {
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(300), fut)
                .await
                .is_err(),
            "it must wait for the cancel"
        );
    }

    /// Sending again to someone whose request still stands restores it, and
    /// waits for a cancel holding the person's lock: restoring between the
    /// cancel's ignore and its withdrawal record would report the request
    /// sent while leaving it withdrawn.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_restore_during_a_cancel_waits_and_then_retires_the_withdrawal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = app_context.wallet_backend().expect("backend");
        let (owner, peer) = (id(1), id(2));
        let cancel_guard = backend
            .dashpay_lock_contact_request_action(&owner, &peer)
            .await;

        let mut restore = Box::pin(restore_under_peer_lock(&backend, owner, peer, None));
        assert_waits(&mut restore).await;

        backend
            .dashpay_mark_withdrawn(&owner, &peer)
            .expect("withdraw");
        drop(cancel_guard);

        assert!(restore.await.expect("restore finished"), "restored");
        assert!(!backend.dashpay_disengaged_from(&owner, &peer));
    }

    /// An accept restores its earlier request under the person's lock it
    /// already holds; the restore must not wait on that lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_accepts_restore_under_its_own_peer_lock_finishes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = app_context.wallet_backend().expect("backend");
        let (owner, peer) = (id(1), id(2));
        backend
            .dashpay_mark_withdrawn(&owner, &peer)
            .expect("withdraw");
        let peer_lock = backend.dashpay_lock_contact_peer(&owner, &peer).await;

        let restored = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            restore_under_peer_lock(&backend, owner, peer, Some(&peer_lock)),
        )
        .await
        .expect("the restore must not wait on its caller's lock");

        assert!(restored.expect("restore finished"), "restored");
        assert!(!backend.dashpay_disengaged_from(&owner, &peer));
    }

    /// An accept sends its reply under the person's lock it already holds and
    /// hands that lock down; `finish_send` is the only place the send path
    /// takes the lock, so the reply finishes instead of waiting on itself.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_accepts_reply_under_its_own_peer_lock_finishes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = app_context.wallet_backend().expect("backend");
        let (owner, peer) = (id(1), id(2));
        backend
            .dashpay_mark_withdrawn(&owner, &peer)
            .expect("withdraw");
        let peer_lock = backend.dashpay_lock_contact_peer(&owner, &peer).await;

        let finished = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            finish_send(
                &backend,
                &bip39::rand::random::<[u8; 32]>(),
                owner,
                peer,
                sent_record(owner, peer),
                Some(&peer_lock),
            ),
        )
        .await
        .expect("the send must not wait on its caller's lock");

        finished.expect("send finished");
        assert!(!backend.dashpay_disengaged_from(&owner, &peer));
    }

    /// A lock held for someone else does not stand in for the recipient's:
    /// the send still waits for a cancel holding the recipient's lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_lock_held_for_another_person_does_not_skip_the_wait() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = app_context.wallet_backend().expect("backend");
        let (owner, peer, other) = (id(1), id(2), id(3));
        let other_lock = backend.dashpay_lock_contact_peer(&owner, &other).await;
        let cancel_guard = backend
            .dashpay_lock_contact_request_action(&owner, &peer)
            .await;

        let waited = tokio::time::timeout(
            std::time::Duration::from_millis(300),
            finish_send(
                &backend,
                &bip39::rand::random::<[u8; 32]>(),
                owner,
                peer,
                sent_record(owner, peer),
                Some(&other_lock),
            ),
        )
        .await;

        assert!(waited.is_err(), "the send must wait for the cancel");
        drop(cancel_guard);
    }

    /// Every send re-engages, a first one included: a declined or withdrawn
    /// person is a contact again once the owner sends them a request.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_send_retires_a_decline_and_a_withdrawal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = app_context.wallet_backend().expect("backend");
        let (owner, peer) = (id(1), id(2));
        backend
            .dashpay_mark_declined(&owner, &peer)
            .expect("decline");
        backend
            .dashpay_mark_withdrawn(&owner, &peer)
            .expect("withdraw");
        assert!(backend.dashpay_disengaged_from(&owner, &peer));

        re_engage_after_send(&PlatformReEngage {
            backend: &backend,
            owner,
            peer,
        })
        .await
        .expect("re-engaged");

        assert!(!backend.dashpay_disengaged_from(&owner, &peer));
    }

    /// Actions on requests with the same person run one at a time, whichever
    /// request they act on: a cancel must not record a withdrawal while an
    /// accept of that person's request establishes the contact.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn request_actions_with_one_person_run_one_at_a_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app_context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let sdk = app_context.sdk();
        let (owner, peer) = (id(1), id(2));
        let cancel = PlatformCancelOps {
            app_context: &app_context,
            sdk: &sdk,
            owner_id: owner,
            to_identity_id: peer,
            request_id: id(10),
        };
        let decline = PlatformRejectOps {
            app_context: &app_context,
            owner_id: owner,
            from_identity_id: peer,
            request_id: id(11),
        };

        let held = cancel.lock_action().await.expect("lock");
        let mut waiting = Box::pin(decline.lock_action());
        assert!(futures::poll!(&mut waiting).is_pending());
        drop(held);
        drop(waiting.await.expect("lock"));
    }

    #[derive(Default)]
    struct ScriptedReEngage {
        withdrawn: bool,
        declined: bool,
        /// platform-wallet still ignores the person, with no DET marker.
        ignored: bool,
        unreadable: bool,
        re_engage_fails: bool,
        re_engaged: std::sync::Mutex<bool>,
    }

    impl ReEngageOps for ScriptedReEngage {
        fn is_withdrawn(&self) -> Result<bool, TaskError> {
            if self.unreadable {
                return Err(TaskError::DashPayLocalStateRead {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            Ok(self.withdrawn)
        }

        fn is_declined(&self) -> Result<bool, TaskError> {
            Ok(self.declined)
        }

        async fn is_ignored(&self) -> bool {
            self.ignored
        }

        async fn re_engage(&self) -> Result<(), TaskError> {
            if self.re_engage_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            *self.re_engaged.lock().unwrap() = true;
            Ok(())
        }
    }

    /// A send that reached Platform but could not update this device is
    /// reported, not shown as complete: the person could stay hidden here.
    #[tokio::test]
    async fn a_failed_re_engagement_after_a_send_is_reported() {
        let ops = ScriptedReEngage {
            re_engage_fails: true,
            ..Default::default()
        };

        assert!(matches!(
            re_engage_after_send(&ops).await,
            Err(TaskError::DashPayContactRequestSentNotUpdated { .. })
        ));
        assert!(
            re_engage_after_send(&ScriptedReEngage::default())
                .await
                .is_ok()
        );
    }

    /// Sending again retries a re-engagement a failed send left unfinished:
    /// the decline it did not clear is cleared now, with nothing published.
    #[tokio::test]
    async fn a_decline_left_by_a_failed_send_is_retried_on_the_next_send() {
        let ops = ScriptedReEngage {
            declined: true,
            ..Default::default()
        };

        assert!(restore_withdrawn_request(&ops).await.expect("restored"));
        assert!(*ops.re_engaged.lock().unwrap());
    }

    /// A decline or withdrawal whose marker write failed after the ignore
    /// stood (and whose rollback failed too) leaves only platform-wallet's
    /// ignore. Sending or accepting again still lifts it, rather than
    /// reporting the request as already sent while the person stays ignored.
    #[tokio::test]
    async fn an_ignore_left_without_a_marker_is_lifted_on_re_engagement() {
        let ops = ScriptedReEngage {
            ignored: true,
            ..Default::default()
        };

        assert!(restore_withdrawn_request(&ops).await.expect("restored"));
        assert!(*ops.re_engaged.lock().unwrap());
    }

    /// Accepting (or sending again) where the owner's
    /// withdrawn request still stands restores it without a new publish.
    #[tokio::test]
    async fn an_existing_withdrawn_request_is_restored() {
        let ops = ScriptedReEngage {
            withdrawn: true,
            ..Default::default()
        };
        assert!(restore_withdrawn_request(&ops).await.expect("restored"));
        assert!(*ops.re_engaged.lock().unwrap());

        let pending = ScriptedReEngage::default();
        assert!(
            !restore_withdrawn_request(&pending)
                .await
                .expect("nothing to restore")
        );
        assert!(!*pending.re_engaged.lock().unwrap());
    }

    /// An unreadable withdrawal record fails the
    /// accept or send instead of reporting success with the withdrawal still
    /// in force.
    #[tokio::test]
    async fn an_unreadable_withdrawal_record_fails_the_restore() {
        let ops = ScriptedReEngage {
            unreadable: true,
            ..Default::default()
        };

        assert!(matches!(
            restore_withdrawn_request(&ops).await,
            Err(TaskError::DashPayLocalStateRead { .. })
        ));
        assert!(!*ops.re_engaged.lock().unwrap());
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
    // Decline and withdraw — both are local only: no contact details are
    // written for someone who is not a contact.
    // -----------------------------------------------------------------

    use std::sync::Mutex;

    #[derive(Default)]
    struct ScriptedOps {
        action_lock: Arc<tokio::sync::Mutex<()>>,
        reciprocal: bool,
        withdrawn: Mutex<bool>,
        withdraw_fails: bool,
        phase_cleared: Mutex<bool>,
        phase_fails: bool,
        ignored: Mutex<bool>,
        unignore_fails: bool,
        /// The reply is taken in between the reciprocal query and the ignore.
        established_meanwhile: bool,
    }

    impl CancelOps for ScriptedOps {
        async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
            Ok(self.action_lock.clone().lock_owned().await)
        }

        fn clear_phase(&self) -> Result<(), TaskError> {
            if self.phase_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            *self.phase_cleared.lock().unwrap() = true;
            Ok(())
        }

        async fn reciprocal_request_exists(&self) -> Result<bool, TaskError> {
            Ok(self.reciprocal)
        }

        async fn ignore_recipient(&self) -> Result<bool, TaskError> {
            *self.ignored.lock().unwrap() = !self.established_meanwhile;
            Ok(!self.established_meanwhile)
        }

        async fn unignore_recipient(&self) -> Result<(), TaskError> {
            if self.unignore_fails {
                return Err(TaskError::WalletStateInconsistent);
            }
            *self.ignored.lock().unwrap() = false;
            Ok(())
        }

        fn mark_withdrawn(&self) -> Result<(), TaskError> {
            if self.withdraw_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            *self.withdrawn.lock().unwrap() = true;
            Ok(())
        }
    }

    /// The reply is taken in and the contact
    /// established after the reciprocal query; the cancellation then reports
    /// the contact and records no withdrawal.
    #[tokio::test]
    async fn a_reply_taken_in_during_the_cancel_wins() {
        let ops = ScriptedOps {
            established_meanwhile: true,
            ..Default::default()
        };

        assert_eq!(
            cancel_flow(&ops).await.expect("cancel"),
            CancelOutcome::AlreadyEstablished
        );
        assert!(!*ops.withdrawn.lock().unwrap());
        assert!(!*ops.ignored.lock().unwrap(), "the ignore is lifted again");
    }

    #[tokio::test]
    async fn withdrawing_a_pending_request_records_it_on_this_device() {
        let ops = ScriptedOps::default();

        assert_eq!(
            cancel_flow(&ops).await.expect("cancel"),
            CancelOutcome::Withdrawn
        );
        assert!(*ops.withdrawn.lock().unwrap());
        assert!(
            *ops.ignored.lock().unwrap(),
            "SEC-001: a later reply must not establish the contact on its own"
        );
        assert!(
            *ops.phase_cleared.lock().unwrap(),
            "an older version's record is retired"
        );
    }

    #[tokio::test]
    async fn withdrawing_an_answered_request_changes_nothing() {
        let ops = ScriptedOps {
            reciprocal: true,
            ..Default::default()
        };

        assert_eq!(
            cancel_flow(&ops).await.expect("cancel"),
            CancelOutcome::AlreadyEstablished
        );
        assert!(!*ops.withdrawn.lock().unwrap());
        assert!(
            !*ops.ignored.lock().unwrap(),
            "an established contact is not ignored"
        );
    }

    #[tokio::test]
    async fn a_failed_withdrawal_record_is_not_reported_as_a_cancellation() {
        // The marker is what retires the row from the listing. Without it the
        // request comes back as pending on the next reload, so announcing a
        // successful cancellation would be a lie.
        let ops = ScriptedOps {
            withdraw_fails: true,
            ..Default::default()
        };

        assert!(cancel_flow(&ops).await.is_err());
    }

    /// A withdrawal whose marker cannot be written lifts the ignore it just
    /// set, so nothing is left half-done: no marker means no ignore.
    #[tokio::test]
    async fn a_failed_withdrawal_record_lifts_the_ignore_again() {
        let ops = ScriptedOps {
            withdraw_fails: true,
            ..Default::default()
        };

        assert!(matches!(
            cancel_flow(&ops).await,
            Err(TaskError::DashPayRequestDecisionNotRecorded { .. })
        ));
        assert!(!*ops.ignored.lock().unwrap());
    }

    /// The decline twin of `a_failed_withdrawal_record_lifts_the_ignore_again`.
    #[tokio::test]
    async fn a_failed_decline_record_lifts_the_ignore_again() {
        let ops = ScriptedRejectOps {
            decline_fails: true,
            ..Default::default()
        };

        assert!(matches!(
            reject_flow(&ops).await,
            Err(TaskError::DashPayRequestDecisionNotRecorded { .. })
        ));
        assert!(*ops.unignored.lock().unwrap());
    }

    /// When the rollback fails too, the user still sees why the decision
    /// was not recorded, not the rollback's error; the ignore left behind is
    /// lifted by the next send or accept
    /// (`an_ignore_left_without_a_marker_is_lifted_on_re_engagement`).
    #[tokio::test]
    async fn a_failed_rollback_reports_the_unrecorded_decision() {
        let decline = ScriptedRejectOps {
            decline_fails: true,
            unignore_fails: true,
            ..Default::default()
        };
        let withdraw = ScriptedOps {
            withdraw_fails: true,
            unignore_fails: true,
            ..Default::default()
        };

        assert!(matches!(
            reject_flow(&decline).await,
            Err(TaskError::DashPayRequestDecisionNotRecorded { .. })
        ));
        assert!(matches!(
            cancel_flow(&withdraw).await,
            Err(TaskError::DashPayRequestDecisionNotRecorded { .. })
        ));
        assert!(*withdraw.ignored.lock().unwrap());
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
        ignore_fails: bool,
        decline_fails: bool,
        phase_fails: bool,
        established_meanwhile: bool,
        ignored: Mutex<usize>,
        unignore_fails: bool,
        unignored: Mutex<bool>,
        declined: Mutex<bool>,
    }

    impl RejectOps for ScriptedRejectOps {
        async fn lock_action(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, TaskError> {
            Ok(self.action_lock.clone().lock_owned().await)
        }

        fn clear_phase(&self) -> Result<(), TaskError> {
            if self.phase_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            Ok(())
        }

        async fn ignore_sender(&self) -> Result<bool, TaskError> {
            if self.ignore_fails {
                return Err(TaskError::WalletStateInconsistent);
            }
            if self.established_meanwhile {
                return Ok(false);
            }
            *self.ignored.lock().unwrap() += 1;
            Ok(true)
        }

        async fn unignore_sender(&self) -> Result<(), TaskError> {
            if self.unignore_fails {
                return Err(TaskError::WalletStateInconsistent);
            }
            *self.unignored.lock().unwrap() = true;
            Ok(())
        }

        fn mark_declined(&self) -> Result<(), TaskError> {
            if self.decline_fails {
                return Err(TaskError::DashpaySidecarStorage {
                    source: crate::wallet_backend::KvAdapterError::Truncated,
                });
            }
            *self.declined.lock().unwrap() = true;
            Ok(())
        }

        fn request_id(&self) -> Identifier {
            id(7)
        }

        fn sender_id(&self) -> Identifier {
            id(8)
        }
    }

    /// The decline twin of `a_reply_taken_in_during_the_cancel_wins`: the
    /// owner's own request (sent from another device) is taken in first, so
    /// the contact is established and nothing is declined.
    #[tokio::test]
    async fn a_contact_established_during_the_decline_wins() {
        let ops = ScriptedRejectOps {
            established_meanwhile: true,
            ..Default::default()
        };

        assert!(matches!(
            reject_flow(&ops).await,
            Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished { request_id, contact_id })
                if request_id == id(7) && contact_id == id(8)
        ));
        assert!(!*ops.declined.lock().unwrap());
    }

    #[tokio::test]
    async fn declining_ignores_the_sender_and_records_the_decline() {
        let ops = ScriptedRejectOps::default();

        assert!(matches!(
            reject_flow(&ops).await,
            Ok(BackendTaskSuccessResult::DashPayContactRequestRejected(request)) if request == id(7)
        ));
        assert_eq!(*ops.ignored.lock().unwrap(), 1);
        assert!(*ops.declined.lock().unwrap());
    }

    /// The old progress record is inert once the decision is recorded, so
    /// failing to forget it does not fail a decline or a withdrawal.
    #[tokio::test]
    async fn a_leftover_progress_record_does_not_fail_a_recorded_decision() {
        let decline = ScriptedRejectOps {
            phase_fails: true,
            ..Default::default()
        };
        let withdraw = ScriptedOps {
            phase_fails: true,
            ..Default::default()
        };

        assert!(matches!(
            reject_flow(&decline).await,
            Ok(BackendTaskSuccessResult::DashPayContactRequestRejected(_))
        ));
        assert!(*decline.declined.lock().unwrap());
        assert_eq!(
            cancel_flow(&withdraw).await.expect("withdrawn"),
            CancelOutcome::Withdrawn
        );
    }

    /// Declining and withdrawing publish nothing, so a failed local record
    /// must not tell the user that the change reached the network.
    #[tokio::test]
    async fn a_failed_local_decision_record_does_not_claim_the_network() {
        let decline = ScriptedRejectOps {
            decline_fails: true,
            ..Default::default()
        };
        let withdraw = ScriptedOps {
            withdraw_fails: true,
            ..Default::default()
        };

        for error in [
            reject_flow(&decline)
                .await
                .expect_err("decline record fails"),
            cancel_flow(&withdraw)
                .await
                .expect_err("withdrawal record fails"),
        ] {
            assert!(
                matches!(error, TaskError::DashPayRequestDecisionNotRecorded { .. }),
                "{error:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_ignore_records_no_decline() {
        let ops = ScriptedRejectOps {
            ignore_fails: true,
            ..Default::default()
        };

        assert!(reject_flow(&ops).await.is_err());
        assert!(!*ops.declined.lock().unwrap(), "a retry starts over");
    }
}
