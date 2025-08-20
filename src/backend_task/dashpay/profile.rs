use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{DocumentV0, DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::{string_encoding::Encoding, Value};
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use dash_sdk::platform::documents::transitions::{DocumentCreateTransitionBuilder, DocumentReplaceTransitionBuilder};
use sha2::{Sha256, Digest};
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
                avatar_url.to_string()
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
            
            // TODO: In a production implementation, we would:
            // 1. Fetch the image from the URL
            // 2. Calculate SHA-256 hash of the image bytes
            // 3. Calculate perceptual hash using DHash algorithm
            // For now, we'll add placeholder values to ensure compliance with DIP-0015
            
            // Note: These should be calculated from actual image data
            // avatarHash: SHA-256 hash of image bytes (32 bytes)
            // avatarFingerprint: DHash perceptual fingerprint (8 bytes)
            
            // Placeholder: In production, fetch image and calculate real hash
            // let image_bytes = fetch_image(&url).await?;
            // let mut hasher = Sha256::new();
            // hasher.update(&image_bytes);
            // let hash = hasher.finalize();
            // profile_data.insert("avatarHash".to_string(), Value::Bytes(hash.to_vec()));
            
            // Placeholder: In production, calculate DHash fingerprint
            // let fingerprint = calculate_dhash(&image_bytes)?;
            // profile_data.insert("avatarFingerprint".to_string(), Value::Bytes(fingerprint));
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
    _app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount: u64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
    // TODO: DashPay payments implementation
    // This is complex and requires:
    // 1. Verifying contact relationship exists
    // 2. Getting payment channel keys from contactInfo
    // 3. Deriving unique payment addresses
    // 4. Creating and broadcasting payment transaction
    // 5. Storing payment metadata locally

    Ok(BackendTaskSuccessResult::Message(format!(
        "Payment of {} credits from {} to {} (memo: {:?}) - Not yet implemented",
        amount,
        from_identity.identity.id().to_string(Encoding::Base58),
        to_contact_id,
        memo
    )))
}

pub async fn load_payment_history(
    _app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_id: Option<Identifier>,
) -> Result<BackendTaskSuccessResult, String> {
    // TODO: Payment history would be stored locally in the database
    // as DashPay payments are not stored on-chain

    let filter_msg = if let Some(cid) = contact_id {
        format!(" with contact {}", cid)
    } else {
        String::new()
    };

    Ok(BackendTaskSuccessResult::Message(format!(
        "Payment history for {}{} - Not yet implemented",
        identity.identity.id().to_string(Encoding::Base58),
        filter_msg
    )))
}
