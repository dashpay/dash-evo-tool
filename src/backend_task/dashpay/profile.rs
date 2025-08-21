use super::avatar_processing::{calculate_avatar_hash, calculate_dhash_fingerprint};
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{DocumentV0, DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::{Value, string_encoding::Encoding};
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::documents::transitions::{
    DocumentCreateTransitionBuilder, DocumentReplaceTransitionBuilder,
};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub async fn load_profile(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, String> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for profile document owned by this identity
    let mut profile_query = DocumentQuery::new(dashpay_contract, "profile")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    profile_query = profile_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: identity_id.to_buffer().into(),
    });
    profile_query.limit = 1;

    let profile_docs = Document::fetch_many(sdk, profile_query)
        .await
        .map_err(|e| format!("Error fetching profile: {}", e))?;

    if let Some((_, doc_opt)) = profile_docs.iter().next() {
        if let Some(doc) = doc_opt {
            // Extract profile fields from the document
            let display_name = doc
                .get("displayName")
                .and_then(|v| v.as_text())
                .unwrap_or_default();
            let public_message = doc
                .get("publicMessage")
                .and_then(|v| v.as_text())
                .unwrap_or_default();
            let avatar_url = doc
                .get("avatarUrl")
                .and_then(|v| v.as_text())
                .unwrap_or_default();

            Ok(BackendTaskSuccessResult::DashPayProfile(Some((
                display_name.to_string(),
                public_message.to_string(),
                avatar_url.to_string(),
            ))))
        } else {
            Ok(BackendTaskSuccessResult::DashPayProfile(None))
        }
    } else {
        Ok(BackendTaskSuccessResult::DashPayProfile(None))
    }
}

pub async fn update_profile(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
    display_name: Option<String>,
    bio: Option<String>,
    avatar_url: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Get the appropriate identity key for signing
    let identity_key = identity
        .identity
        .get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL]),
            KeyType::all_key_types().into(),
            false,
        )
        .ok_or("No suitable authentication key found for identity")?;

    // Check if profile already exists
    let mut profile_query = DocumentQuery::new(dashpay_contract.clone(), "profile")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    profile_query = profile_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: identity_id.to_buffer().into(),
    });
    profile_query.limit = 1;

    let existing_profile = Document::fetch_many(sdk, profile_query)
        .await
        .map_err(|e| format!("Error checking for existing profile: {}", e))?;

    // Prepare profile data
    let mut profile_data = BTreeMap::new();

    // Only add non-empty fields according to DashPay DIP
    if let Some(name) = display_name {
        if !name.is_empty() {
            profile_data.insert("displayName".to_string(), Value::Text(name));
        }
    }
    if let Some(bio_text) = bio {
        if !bio_text.is_empty() {
            profile_data.insert("publicMessage".to_string(), Value::Text(bio_text));
        }
    }
    if let Some(url) = avatar_url {
        if !url.is_empty() {
            profile_data.insert("avatarUrl".to_string(), Value::Text(url.clone()));

            // Try to fetch and process the avatar image
            // Note: This requires an HTTP client which may not be available
            // In production, this should be done asynchronously
            match super::avatar_processing::fetch_image_bytes(&url).await {
                Ok(image_bytes) => {
                    // Calculate SHA-256 hash of the image
                    let avatar_hash = calculate_avatar_hash(&image_bytes);
                    profile_data
                        .insert("avatarHash".to_string(), Value::Bytes(avatar_hash.to_vec()));

                    // Calculate DHash perceptual fingerprint
                    match calculate_dhash_fingerprint(&image_bytes) {
                        Ok(fingerprint) => {
                            profile_data.insert(
                                "avatarFingerprint".to_string(),
                                Value::Bytes(fingerprint.to_vec()),
                            );
                        }
                        Err(e) => {
                            eprintln!("Warning: Could not calculate avatar fingerprint: {}", e);
                            // Continue without fingerprint - it's optional
                        }
                    }
                }
                Err(e) => {
                    // If we can't fetch the image, just set the URL without hash/fingerprint
                    // These fields are optional according to DIP-0015
                    eprintln!(
                        "Warning: Could not fetch avatar image for processing: {}",
                        e
                    );
                }
            }
        }
    }

    if let Some((_, Some(existing_doc))) = existing_profile.iter().next() {
        // Update existing profile using DocumentReplaceTransitionBuilder
        let mut updated_document = existing_doc.clone();

        // Update the document's properties
        for (key, value) in profile_data {
            updated_document.set(&key, value);
        }

        // Bump revision for replacement
        updated_document.bump_revision();

        let mut builder = DocumentReplaceTransitionBuilder::new(
            dashpay_contract,
            "profile".to_string(),
            updated_document,
        );

        // Add state transition options if available
        let maybe_options = app_context.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let _result = sdk
            .document_replace(builder, &identity_key, &identity)
            .await
            .map_err(|e| format!("Error replacing profile: {}", e))?;

        Ok(BackendTaskSuccessResult::Message(format!(
            "Profile updated successfully for identity {}",
            identity.identity.id().to_string(Encoding::Base58)
        )))
    } else {
        // Create new profile using DocumentCreateTransitionBuilder
        // Generate document ID
        let profile_doc_id = Document::generate_document_id_v0(
            &dashpay_contract.id(),
            &identity_id,
            "profile",
            &[0u8; 32], // entropy
        );

        let document = Document::V0(DocumentV0 {
            id: profile_doc_id,
            owner_id: identity_id,
            properties: profile_data,
            revision: None,
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

        let mut builder = DocumentCreateTransitionBuilder::new(
            dashpay_contract,
            "profile".to_string(),
            document,
            [0u8; 32], // entropy - using zero for deterministic behavior
        );

        // Add state transition options if available
        let maybe_options = app_context.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let _result = sdk
            .document_create(builder, &identity_key, &identity)
            .await
            .map_err(|e| format!("Error creating profile: {}", e))?;

        Ok(BackendTaskSuccessResult::Message(format!(
            "Profile created successfully for identity {}",
            identity.identity.id().to_string(Encoding::Base58)
        )))
    }
}

pub async fn send_payment(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount: u64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
    // Use the new payments module to send payment
    super::payments::send_payment_to_contact(
        app_context,
        sdk,
        from_identity,
        to_contact_id,
        amount,
        memo,
    )
    .await
}

pub async fn load_payment_history(
    app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_id: Option<Identifier>,
) -> Result<BackendTaskSuccessResult, String> {
    // Load payment history from local database
    let history = super::payments::load_payment_history(
        app_context,
        &identity.identity.id(),
        contact_id.as_ref(),
    )
    .await?;

    // Format the results
    if history.is_empty() {
        let filter_msg = if let Some(cid) = contact_id {
            format!(" with contact {}", cid.to_string(Encoding::Base58))
        } else {
            String::new()
        };

        Ok(BackendTaskSuccessResult::Message(format!(
            "No payment history found for {}{}",
            identity.identity.id().to_string(Encoding::Base58),
            filter_msg
        )))
    } else {
        // In production, this would return a structured result
        Ok(BackendTaskSuccessResult::Message(format!(
            "Found {} payment records",
            history.len()
        )))
    }
}
