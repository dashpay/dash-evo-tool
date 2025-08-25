use super::encryption::{
    encrypt_account_label, encrypt_extended_public_key, generate_ecdh_shared_key,
};
use super::hd_derivation::generate_contact_xpub_data;
use super::validation::validate_contact_request_before_send;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use bip39::rand::{SeedableRng, rngs::StdRng};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{Document as DppDocument, DocumentV0, DocumentV0Getters};
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
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub async fn load_contact_requests(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, String> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for incoming contact requests (where toUserId == our identity)
    let mut incoming_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    let query_value = Value::Identifier(identity_id.to_buffer());

    incoming_query = incoming_query.with_where(WhereClause {
        field: "toUserId".to_string(),
        operator: WhereOperator::Equal,
        value: query_value.clone(),
    });
    // WORKAROUND for Platform bug: Add orderBy to trigger proper index usage
    // Without this orderBy, the query returns 0 results even when documents exist
    incoming_query = incoming_query.with_order_by(OrderClause {
        field: "$createdAt".to_string(),
        ascending: true,
    });
    incoming_query.limit = 50;

    // Query for outgoing contact requests (where $ownerId == our identity)
    let mut outgoing_query = DocumentQuery::new(dashpay_contract, "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    outgoing_query = outgoing_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });
    outgoing_query.limit = 50;

    // Fetch both types of requests
    let incoming_docs = Document::fetch_many(sdk, incoming_query)
        .await
        .map_err(|e| format!("Error fetching incoming requests: {}", e))?;

    let outgoing_docs = Document::fetch_many(sdk, outgoing_query)
        .await
        .map_err(|e| format!("Error fetching outgoing requests: {}", e))?;

    // Convert to vec of tuples (id, document)
    // TODO: Process autoAcceptProof for incoming requests
    // When an incoming request has a valid autoAcceptProof, we should:
    // 1. Verify the proof signature
    // 2. Automatically send a contact request back if valid
    // 3. Mark the contact as auto-accepted
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
                let to_id = Identifier::from_bytes(to_id_bytes.as_slice()).unwrap();
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
            let to_id = Identifier::from_bytes(to_id_bytes.as_slice()).unwrap();
            !contacts_established.contains(&to_id)
        } else {
            true
        }
    });

    Ok(BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing })
}

pub async fn send_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    signing_key: IdentityPublicKey,
    to_username_or_id: String,
    account_label: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
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
    auto_accept_proof: Option<Vec<u8>>,
) -> Result<BackendTaskSuccessResult, String> {
    // Step 1: Resolve the recipient identity
    let to_identity = if to_username_or_id.contains('.') {
        // It's a username, resolve via DPNS
        resolve_username_to_identity(sdk, &to_username_or_id).await?
    } else {
        // It's an identity ID
        let to_id = Identifier::from_string_try_encodings(
            &to_username_or_id,
            &[Encoding::Base58, Encoding::Hex],
        )
        .map_err(|e| format!("Invalid identity ID: {}", e))?;

        Identity::fetch(sdk, to_id)
            .await
            .map_err(|e| format!("Failed to fetch identity: {}", e))?
            .ok_or_else(|| format!("Identity {} not found", to_username_or_id))?
    };

    let to_identity_id = to_identity.id();

    // Step 2: Check if a contact request already exists
    let dashpay_contract = app_context.dashpay_contract.clone();
    let mut existing_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

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

    let existing = Document::fetch_many(sdk, existing_query)
        .await
        .map_err(|e| format!("Error checking existing requests: {}", e))?;

    if !existing.is_empty() {
        return Err(format!(
            "Contact request already sent to {}",
            to_username_or_id
        ));
    }

    // Step 3: Get key indices for ECDH
    let sender_key = &signing_key; // Use the selected key

    // Find a recipient key that supports ECDH (must be ECDSA_SECP256K1)
    let recipient_key = to_identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL, SecurityLevel::HIGH, SecurityLevel::MEDIUM]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or_else(|| {
            "Recipient does not have a compatible ECDSA_SECP256K1 authentication key for ECDH encryption".to_string()
        })?;

    // Step 4: Generate ECDH shared key and encrypt data
    let wallets: Vec<_> = identity.associated_wallets.values().cloned().collect();
    let sender_private_key = identity
        .private_keys
        .get_resolve(
            &(
                crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity,
                sender_key.id(),
            ),
            &wallets,
            identity.network,
        )
        .map_err(|e| format!("Error resolving private key: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or_else(|| "Sender private key not found".to_string())?;

    let shared_key = generate_ecdh_shared_key(&sender_private_key, recipient_key)
        .map_err(|e| format!("Failed to generate ECDH shared key: {}", e))?;

    // Generate extended public key for this contact using proper HD derivation
    // For now, use the sender's private key as seed material
    // In production, this would derive from the wallet's HD seed/mnemonic
    let wallet_seed = sender_private_key;

    // Get the network from app context
    let network = app_context.network;

    // Use account 0 for now (could be made configurable)
    let account_index = 0u32;

    // Generate the extended public key data for this contact relationship
    let (parent_fingerprint, chain_code, contact_public_key) = generate_contact_xpub_data(
        &wallet_seed,
        network,
        account_index,
        &identity.identity.id(),
        &to_identity_id,
    )
    .map_err(|e| format!("Failed to generate contact extended public key: {}", e))?;

    let encrypted_public_key = encrypt_extended_public_key(
        parent_fingerprint,
        chain_code,
        contact_public_key,
        &shared_key,
    )
    .map_err(|e| format!("Failed to encrypt extended public key: {}", e))?;

    // Step 5: Get the current core chain height for synchronization
    let (core_height, current_height_for_validation) =
        match CurrentQuorumsInfo::fetch_unproved(sdk, NoParamQuery {}).await {
            Ok(Some(quorum_info)) => {
                eprintln!(
                    "DEBUG: Got core height: {}",
                    quorum_info.last_core_block_height
                );
                (
                    quorum_info.last_core_block_height,
                    Some(quorum_info.last_core_block_height),
                )
            }
            Ok(None) => {
                (0u32, None) // Fallback if no quorum info available
            }
            Err(_e) => {
                (0u32, None) // Fallback on error
            }
        };

    // Step 5.5: Validate the contact request before proceeding
    let validation = validate_contact_request_before_send(
        sdk,
        &identity,
        sender_key.id(),
        to_identity.id(),
        recipient_key.id(),
        0, // account_reference - using 0 for now
        core_height,
        current_height_for_validation,
    )
    .await
    .map_err(|e| format!("Validation failed: {}", e))?;

    // Check if validation passed
    if !validation.is_valid {
        let error_msg = format!(
            "Contact request validation failed: {}",
            validation.errors.join("; ")
        );
        return Err(error_msg);
    }

    // Log any warnings
    for _warning in &validation.warnings {}

    // Step 6: Create contact request document
    let mut properties = BTreeMap::new();
    properties.insert(
        "toUserId".to_string(),
        Value::Identifier(to_identity_id.to_buffer()),
    );
    properties.insert("senderKeyIndex".to_string(), Value::U32(sender_key.id()));
    properties.insert(
        "recipientKeyIndex".to_string(),
        Value::U32(recipient_key.id()),
    );
    // Calculate account reference
    // For now, use the account index directly
    // In production, this would use the full calculation from DIP-0015
    properties.insert("accountReference".to_string(), Value::U32(account_index));
    properties.insert(
        "encryptedPublicKey".to_string(),
        Value::Bytes(encrypted_public_key),
    );

    // Add $coreHeightCreatedAt as required by DIP-0015
    properties.insert("$coreHeightCreatedAt".to_string(), Value::U32(core_height));

    // Add encrypted account label if provided
    if let Some(label) = account_label {
        let encrypted_label = encrypt_account_label(&label, &shared_key)
            .map_err(|e| format!("Failed to encrypt account label: {}", e))?;
        properties.insert(
            "encryptedAccountLabel".to_string(),
            Value::Bytes(encrypted_label),
        );
    }

    // Add autoAcceptProof if provided (from QR code scanning)
    if let Some(proof) = auto_accept_proof {
        eprintln!(
            "DEBUG: Including autoAcceptProof in contact request ({} bytes)",
            proof.len()
        );
        properties.insert("autoAcceptProof".to_string(), Value::Bytes(proof));
    } else {
        // Empty proof for normal requests
        properties.insert("autoAcceptProof".to_string(), Value::Bytes(vec![]));
    }

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

    let _result = sdk
        .document_create(builder, identity_key, &identity)
        .await
        .map_err(|e| format!("Error creating contact request: {}", e))?;

    Ok(BackendTaskSuccessResult::DashPayContactRequestSent(
        to_username_or_id.to_string(),
    ))
}

async fn resolve_username_to_identity(sdk: &Sdk, username: &str) -> Result<Identity, String> {
    // Parse username (e.g., "alice.dash" -> "alice")
    let name = username
        .split('.')
        .next()
        .ok_or_else(|| format!("Invalid username format: {}", username))?;

    // Query DPNS for the username
    let dpns_contract_id = Identifier::from_string(
        "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
        Encoding::Base58,
    )
    .map_err(|e| format!("Failed to parse DPNS contract ID: {}", e))?;

    let dpns_contract = dash_sdk::platform::DataContract::fetch(sdk, dpns_contract_id)
        .await
        .map_err(|e| format!("Failed to fetch DPNS contract: {}", e))?
        .ok_or("DPNS contract not found")?;

    let mut query = DocumentQuery::new(Arc::new(dpns_contract), "domain")
        .map_err(|e| format!("Failed to create DPNS query: {}", e))?;

    query = query.with_where(WhereClause {
        field: "normalizedLabel".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Text(name.to_lowercase()),
    });
    query.limit = 1;

    let results = Document::fetch_many(sdk, query)
        .await
        .map_err(|e| format!("Failed to query DPNS: {}", e))?;

    let (_, document) = results
        .into_iter()
        .next()
        .ok_or_else(|| format!("Username '{}' not found", username))?;

    let document = document.ok_or_else(|| format!("Invalid DPNS document for '{}'", username))?;

    // Get the identity ID from the DPNS document
    let identity_id = document.owner_id();

    // Fetch the identity
    Identity::fetch(sdk, identity_id)
        .await
        .map_err(|e| format!("Failed to fetch identity for '{}': {}", username, e))?
        .ok_or_else(|| format!("Identity not found for username '{}'", username))
}

pub async fn accept_contact_request(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    request_id: Identifier,
) -> Result<BackendTaskSuccessResult, String> {
    // According to DashPay DIP, accepting means sending a contact request back
    // First, we need to fetch the incoming contact request to get the sender's identity

    eprintln!(
        "DEBUG: Accepting contact request {} for identity {}",
        request_id.to_string(Encoding::Base58),
        identity.identity.id().to_string(Encoding::Base58)
    );

    let dashpay_contract = app_context.dashpay_contract.clone();

    // Fetch the specific contact request document by creating a query with its ID
    let query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;
    let query_with_id = DocumentQuery::with_document_id(query, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await
        .map_err(|e| format!("Failed to fetch contact request: {}", e))?
        .ok_or_else(|| format!("Contact request {} not found", request_id))?;

    // Get the sender's identity (the owner of the incoming request)
    let from_identity_id = doc.owner_id();
    eprintln!(
        "DEBUG: Sender identity ID: {}",
        from_identity_id.to_string(Encoding::Base58)
    );

    // Check if we already sent a contact request to this identity
    let mut existing_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

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

    let existing = Document::fetch_many(sdk, existing_query)
        .await
        .map_err(|e| format!("Error checking existing requests: {}", e))?;

    if !existing.is_empty() {
        return Ok(BackendTaskSuccessResult::DashPayContactAlreadyEstablished(
            from_identity_id,
        ));
    }

    // Get a signing key for the identity
    let signing_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]),
            HashSet::from([KeyType::ECDSA_SECP256K1]),
            false,
        )
        .ok_or("No suitable signing key found for identity")?
        .clone();

    // Now send a contact request back to establish the friendship
    eprintln!(
        "DEBUG: Sending contact request back to {}...",
        from_identity_id.to_string(Encoding::Base58)
    );
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
) -> Result<BackendTaskSuccessResult, String> {
    // According to DashPay DIP, rejecting doesn't delete the request (they're immutable)
    // Instead, we should update our contactInfo document to mark this contact as hidden

    // First, fetch the contact request to get the sender's identity
    let dashpay_contract = app_context.dashpay_contract.clone();

    let query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;
    let query_with_id = DocumentQuery::with_document_id(query, &request_id);

    let doc = Document::fetch(sdk, query_with_id)
        .await
        .map_err(|e| format!("Failed to fetch contact request: {}", e))?
        .ok_or_else(|| format!("Contact request {} not found", request_id))?;

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
