use super::errors::DashPayError;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::auto_accept_proof::{
    AutoAcceptProofData, create_auto_accept_proof_bytes_with_key,
};
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{
    Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use platform_wallet::changeset::changeset::{
    ContactChangeSet, ContactRequestEntry, PlatformWalletChangeSet,
};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub async fn load_contact_requests(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    tracing::info!(
        "Loading contact requests for identity: {}",
        identity_id.to_string(Encoding::Base58)
    );

    // Query for incoming contact requests (where toUserId == our identity)
    let mut incoming_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e),
        })?;

    let query_value = Value::Identifier(identity_id.to_buffer());

    incoming_query = incoming_query.with_where(WhereClause {
        field: "toUserId".to_string(),
        operator: WhereOperator::Equal,
        value: query_value.clone(),
    });

    // Without this orderBy, the query returns 0 results even when documents exist
    incoming_query = incoming_query.with_order_by(OrderClause {
        field: "$createdAt".to_string(),
        ascending: true,
    });
    incoming_query.limit = 50;

    // Query for outgoing contact requests (where $ownerId == our identity)
    let mut outgoing_query =
        DocumentQuery::new(dashpay_contract, "contactRequest").map_err(|e| {
            DashPayError::QueryCreation {
                query_target: "DashPay contactRequest",
                source: Box::new(e),
            }
        })?;

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
    let mut contacts_established = HashSet::new();

    // Check each incoming request
    for (_, incoming_doc) in incoming.iter() {
        let from_id = incoming_doc.owner_id();

        // Check if we also sent a request to this person
        for (_, outgoing_doc) in outgoing.iter() {
            if let Some(Value::Identifier(to_id_bytes)) = outgoing_doc.properties().get("toUserId")
            {
                // Parse the identifier, skip if invalid
                let Ok(to_id) = Identifier::from_bytes(to_id_bytes.as_slice()) else {
                    tracing::warn!("Invalid toUserId in contact request document, skipping");
                    continue;
                };
                if to_id == from_id {
                    // Mutual request found - they are now contacts
                    contacts_established.insert(from_id);
                }
            }
        }
    }

    // Filter out established contacts from both lists
    incoming.retain(|(_, doc)| !contacts_established.contains(&doc.owner_id()));

    outgoing.retain(|(_, doc)| {
        if let Some(Value::Identifier(to_id_bytes)) = doc.properties().get("toUserId") {
            // Parse the identifier, keep the document if we can't parse (defensive)
            let Ok(to_id) = Identifier::from_bytes(to_id_bytes.as_slice()) else {
                tracing::warn!("Invalid toUserId in outgoing contact request, keeping in list");
                return true;
            };
            !contacts_established.contains(&to_id)
        } else {
            true
        }
    });

    tracing::info!(
        "After filtering: {} incoming, {} outgoing contact requests",
        incoming.len(),
        outgoing.len()
    );

    Ok(BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing })
}

/// Send a contact request, delegating ECDH, xpub encryption, document
/// construction and broadcast to the platform-wallet's `DashPayWallet`.
///
/// The caller is still responsible for:
/// - Resolving usernames / identity IDs
/// - Checking for duplicate requests on Platform
/// - Returning the appropriate `BackendTaskSuccessResult`
pub async fn send_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    _signing_key: IdentityPublicKey,
    to_username_or_id: String,
    account_label: Option<String>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    send_contact_request_with_proof(
        app_context,
        sdk,
        identity,
        _signing_key,
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
    _signing_key: IdentityPublicKey,
    to_username_or_id: String,
    account_label: Option<String>,
    qr_auto_accept: Option<AutoAcceptProofData>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // Step 1: Resolve the recipient identity
    let to_username_or_id = to_username_or_id.trim().to_string();

    if let Err(input) = crate::model::dpns::validate_dpns_input(&to_username_or_id) {
        return Err(TaskError::DashPay(DashPayError::InvalidUsername {
            username: input,
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
    let mut existing_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e),
        })?;

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

    // Step 3: Build auto-accept proof bytes if QR data was provided
    let auto_accept_proof = if let Some(qr) = qr_auto_accept {
        // Ensure the QR target matches the resolved recipient
        if qr.identity_id != to_identity_id {
            return Err(DashPayError::InvalidQrCode {
                reason: "QR code target identity does not match recipient".to_string(),
            }
            .into());
        }
        // NOTE: Behavior change from old code. Previously account_reference was
        // calculated via DIP-15 calculate_account_reference(). Now platform-wallet's
        // send_contact_request() uses account_index (0) as account_reference in
        // the document. The auto-accept proof must use the same value to match.
        // QR auto-accept codes are session-scoped, so old QR codes from previous
        // app versions won't be valid anyway (they expire).
        let proof = create_auto_accept_proof_bytes_with_key(
            qr.expires_at,
            &qr.proof_key,
            &identity.identity.id(),
            &to_identity_id,
            0, // Must match platform-wallet's account_reference (= account_index = 0)
        )
        .map_err(|e| TaskError::EncryptionError { detail: e })?;
        tracing::debug!(
            "Including autoAcceptProof in contact request ({} bytes)",
            proof.len()
        );
        Some(proof)
    } else {
        None
    };

    // Step 4: Delegate to platform-wallet's DashPayWallet
    let platform_wallet = app_context.platform_wallet_for_identity(&identity)?;
    let sender_id = identity.identity.id();

    let contact_request = platform_wallet
        .dashpay()
        .send_contact_request(
            &sender_id,
            &to_identity_id,
            account_label,
            auto_accept_proof,
        )
        .await
        .map_err(|e| TaskError::PlatformWallet {
            source: Box::new(e),
        })?;

    // Step 5: Stage a PlatformWalletChangeSet capturing the sent contact request.
    // TODO: re-wire persistence after changeset migration
    // key_wallet::changeset (AccountChangeSet, WalletChangeSet) was removed from dashcore.
    // The `wallet` field no longer exists on PlatformWalletChangeSet.
    let changeset = PlatformWalletChangeSet {
        contacts: Some(ContactChangeSet {
            sent_requests: BTreeMap::from([(
                platform_wallet::changeset::SentContactRequestKey {
                    owner_id: sender_id,
                    recipient_id: to_identity_id,
                },
                ContactRequestEntry {
                    request: contact_request,
                },
            )]),
            ..Default::default()
        }),
        ..Default::default()
    };
    platform_wallet.queue_persist(changeset);

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
        order_by_clauses: vec![],
        limit: 1,
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

/// Accept an incoming contact request by sending a reciprocal request via
/// platform-wallet's `DashPayWallet`.
pub async fn accept_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // Fetch the incoming contact request document to identify the sender
    let dashpay_contract = app_context.dashpay_contract.clone();

    let query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e),
        }
    })?;
    let query_with_id = DocumentQuery::with_document_id(query, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await?
        .ok_or(TaskError::DocumentNotFound)?;

    // Get the sender's identity (the owner of the incoming request)
    let from_identity_id = doc.owner_id();

    // Check if we already sent a contact request to this identity
    let mut existing_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e),
        })?;

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
        return Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished(
            from_identity_id,
        ));
    }

    // Delegate to platform-wallet: send a reciprocal contact request
    let platform_wallet = app_context.platform_wallet_for_identity(&identity)?;
    let our_identity_id = identity.identity.id();

    let contact_request = platform_wallet
        .dashpay()
        .send_contact_request(
            &our_identity_id,
            &from_identity_id,
            Some("Accepted contact".to_string()),
            None,
        )
        .await
        .map_err(|e| TaskError::PlatformWallet {
            source: Box::new(e),
        })?;

    // Contact persistence is owned by the platform-wallet persister:
    // the flush of a `ContactChangeSet` writes both
    // `dashpay_contact_requests` and `dashpay_contacts` rows. The
    // direct-write helpers (`Database::save_contact_request`,
    // `Database::save_dashpay_contact`) are `#[cfg(test)]` only.
    let _ = (platform_wallet, contact_request);

    Ok(BackendTaskSuccessResult::DashPayContactRequestAccepted(
        request_id,
    ))
}

pub async fn reject_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // According to DashPay DIP, rejecting doesn't delete the request (they're immutable)
    // Instead, we should update our contactInfo document to mark this contact as hidden

    // First, fetch the contact request to get the sender's identity
    let dashpay_contract = app_context.dashpay_contract.clone();

    let query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay contactRequest",
            source: Box::new(e),
        }
    })?;
    let query_with_id = DocumentQuery::with_document_id(query, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await?
        .ok_or(TaskError::DocumentNotFound)?;

    let from_identity_id = doc.owner_id();

    // Create or update contactInfo to mark this contact as hidden
    use super::contact_info::create_or_update_contact_info;

    let _ = create_or_update_contact_info(
        app_context,
        sdk,
        identity,
        from_identity_id,
        None,       // No nickname
        None,       // No note
        true,       // display_hidden = true for rejected contacts
        Vec::new(), // No accepted accounts
    )
    .await?;

    Ok(BackendTaskSuccessResult::DashPayContactRequestRejected(
        request_id,
    ))
}
