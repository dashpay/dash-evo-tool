use super::avatar_processing::{calculate_avatar_hash, calculate_dhash_fingerprint};
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::{DocumentV0, DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::{Value, string_encoding::Encoding};
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::documents::transitions::{
    DocumentCreateTransitionBuilder, DocumentReplaceTransitionBuilder,
};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use platform_wallet::wallet::dashpay::DashPayProfile;
use rand::RngCore;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use super::platform_wallet_cache::cache_profile;

pub async fn load_profile(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for profile document owned by this identity
    let mut profile_query = DocumentQuery::new(dashpay_contract, "profile").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay profile",
            source: Box::new(e),
        }
    })?;

    profile_query = profile_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: identity_id.to_buffer().into(),
    });
    profile_query.limit = 1;

    let profile_docs = Document::fetch_many(sdk, profile_query).await?;

    if let Some((_, Some(doc))) = profile_docs.iter().next() {
        // Extract profile fields from the document
        let display_name = doc
            .get("displayName")
            .and_then(|v| v.as_text())
            .unwrap_or_default();
        // The "publicMessage" field in the DashPay contract is actually the bio
        let bio = doc
            .get("publicMessage")
            .and_then(|v| v.as_text())
            .unwrap_or_default();
        let avatar_url = doc
            .get("avatarUrl")
            .and_then(|v| v.as_text())
            .unwrap_or_default();
        // `avatarHash` / `avatarFingerprint` are required by the
        // DashPay contract whenever `avatarUrl` is set, and pinned
        // to 32 / 8 bytes. A wrong-length value is a contract
        // violation and propagates as a TaskError so callers can
        // surface it rather than silently swallowing the field.
        let avatar_hash: Option<[u8; 32]> = doc
            .get("avatarHash")
            .and_then(|v| v.as_bytes())
            .map(|b| {
                <[u8; 32]>::try_from(b.as_slice()).map_err(|_| DashPayError::InvalidDocument {
                    reason: format!("avatarHash must be 32 bytes (got {})", b.len()),
                })
            })
            .transpose()?;
        let avatar_fingerprint: Option<[u8; 8]> = doc
            .get("avatarFingerprint")
            .and_then(|v| v.as_bytes())
            .map(|b| {
                <[u8; 8]>::try_from(b.as_slice()).map_err(|_| DashPayError::InvalidDocument {
                    reason: format!("avatarFingerprint must be 8 bytes (got {})", b.len()),
                })
            })
            .transpose()?;

        // Cache the loaded profile via the platform-wallet so the
        // persister catches it on the next flush. The
        // `dashpay_profiles` row gets written by the persister, not
        // by a direct `db.save_dashpay_profile` call (Phase 9b-1).
        cache_profile(
            app_context,
            &identity,
            Some(DashPayProfile {
                display_name: if display_name.is_empty() {
                    None
                } else {
                    Some(display_name.to_string())
                },
                bio: if bio.is_empty() {
                    None
                } else {
                    Some(bio.to_string())
                },
                avatar_url: if avatar_url.is_empty() {
                    None
                } else {
                    Some(avatar_url.to_string())
                },
                avatar_hash,
                avatar_fingerprint,
                avatar_bytes: None,
                public_message: None,
            }),
        )
        .await;

        Ok(BackendTaskSuccessResult::DashPayProfile(Some((
            display_name.to_string(),
            bio.to_string(),
            avatar_url.to_string(),
        ))))
    } else {
        // No profile found — cache the empty state via the platform
        // wallet to avoid repeated network queries.
        cache_profile(app_context, &identity, Some(DashPayProfile::default())).await;

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
) -> Result<BackendTaskSuccessResult, TaskError> {
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
        .ok_or_else(|| TaskError::DashPay(DashPayError::MissingAuthenticationKey))?;

    // Check if profile already exists
    let mut profile_query =
        DocumentQuery::new(dashpay_contract.clone(), "profile").map_err(|e| {
            DashPayError::QueryCreation {
                query_target: "DashPay profile",
                source: Box::new(e),
            }
        })?;

    profile_query = profile_query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: identity_id.to_buffer().into(),
    });
    profile_query.limit = 1;

    let existing_profile = Document::fetch_many(sdk, profile_query).await?;

    // Prepare profile data
    let mut profile_data = BTreeMap::new();

    // Keep copies for database save later
    let display_name_for_db = display_name.clone();
    let bio_for_db = bio.clone();
    let avatar_url_for_db = avatar_url.clone();
    // Computed below when avatar_url is non-empty and fetch succeeds.
    // Kept around so the persister cache mirrors what landed on-chain.
    let mut avatar_hash_for_db: Option<[u8; 32]> = None;
    let mut avatar_fingerprint_for_db: Option<[u8; 8]> = None;

    // Only add non-empty fields according to DashPay DIP
    if let Some(name) = display_name.filter(|name| !name.is_empty()) {
        profile_data.insert("displayName".to_string(), Value::Text(name));
    }
    if let Some(bio_text) = bio.filter(|bio| !bio.is_empty()) {
        profile_data.insert("publicMessage".to_string(), Value::Text(bio_text));
    }
    if let Some(url) = avatar_url.as_ref().filter(|url| !url.is_empty()) {
        profile_data.insert("avatarUrl".to_string(), Value::Text(url.clone()));

        // Try to fetch and process the avatar image
        // Note: This requires an HTTP client which may not be available
        // In production, this should be done asynchronously
        match super::avatar_processing::fetch_image_bytes(url).await {
            Ok(image_bytes) => {
                // Calculate SHA-256 hash of the image
                let avatar_hash = calculate_avatar_hash(&image_bytes);
                profile_data.insert("avatarHash".to_string(), Value::Bytes(avatar_hash.to_vec()));
                avatar_hash_for_db = Some(avatar_hash);

                // Calculate DHash perceptual fingerprint
                match calculate_dhash_fingerprint(&image_bytes) {
                    Ok(fingerprint) => {
                        profile_data.insert(
                            "avatarFingerprint".to_string(),
                            Value::Bytes(fingerprint.to_vec()),
                        );
                        avatar_fingerprint_for_db = Some(fingerprint);
                    }
                    Err(e) => {
                        tracing::warn!("Could not calculate avatar fingerprint: {}", e);
                        // Continue without fingerprint - it's optional
                    }
                }
            }
            Err(e) => {
                // If we can't fetch the image, just set the URL without hash/fingerprint
                // These fields are optional according to DIP-0015
                tracing::warn!("Could not fetch avatar image for processing: {}", e);
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

        // Handle avatar removal: if avatar_url is None or empty, remove avatar-related fields
        if avatar_url.as_ref().is_none_or(|url| url.is_empty()) {
            // Remove avatar-related fields from the document
            let Document::V0(ref mut doc_v0) = updated_document;
            doc_v0.properties_mut().remove("avatarUrl");
            doc_v0.properties_mut().remove("avatarHash");
            doc_v0.properties_mut().remove("avatarFingerprint");
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

        let result = sdk
            .document_replace(builder, identity_key, &identity)
            .await?;

        // Log the proof-verified document for audit trail
        match result {
            dash_sdk::platform::documents::transitions::DocumentReplaceResult::Document(doc) => {
                tracing::info!(
                    "Profile updated: doc_id={}, revision={:?}",
                    doc.id(),
                    doc.revision()
                );
            }
        }

        // Cache the updated profile via the platform-wallet (Phase 9b-1).
        cache_profile(
            app_context,
            &identity,
            Some(DashPayProfile {
                display_name: display_name_for_db.clone(),
                bio: bio_for_db.clone(),
                avatar_url: avatar_url_for_db.clone(),
                avatar_hash: avatar_hash_for_db,
                avatar_fingerprint: avatar_fingerprint_for_db,
                avatar_bytes: None,
                public_message: None,
            }),
        )
        .await;

        Ok(BackendTaskSuccessResult::DashPayProfileUpdated(
            identity.identity.id(),
        ))
    } else {
        // Create new profile using DocumentCreateTransitionBuilder
        // Generate random entropy for document ID (security: prevents predictable IDs)
        let mut entropy = [0u8; 32];
        rand::rng().fill_bytes(&mut entropy);

        let profile_doc_id = Document::generate_document_id_v0(
            &dashpay_contract.id(),
            &identity_id,
            "profile",
            &entropy,
        );

        let document = Document::V0(DocumentV0 {
            id: profile_doc_id,
            owner_id: identity_id,
            creator_id: None,
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
            entropy, // Use same entropy as document ID generation
        );

        // Add state transition options if available
        let maybe_options = app_context.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .document_create(builder, identity_key, &identity)
            .await?;

        // Log the proof-verified document for audit trail
        match result {
            dash_sdk::platform::documents::transitions::DocumentCreateResult::Document(doc) => {
                tracing::info!(
                    "Profile created: doc_id={}, revision={:?}",
                    doc.id(),
                    doc.revision()
                );
            }
        }

        // Cache the newly created profile via the platform-wallet
        // (Phase 9b-1).
        cache_profile(
            app_context,
            &identity,
            Some(DashPayProfile {
                display_name: display_name_for_db.clone(),
                bio: bio_for_db.clone(),
                avatar_url: avatar_url_for_db.clone(),
                avatar_hash: avatar_hash_for_db,
                avatar_fingerprint: avatar_fingerprint_for_db,
                avatar_bytes: None,
                public_message: None,
            }),
        )
        .await;

        Ok(BackendTaskSuccessResult::DashPayProfileUpdated(
            identity.identity.id(),
        ))
    }
}

pub async fn send_payment(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount_dash: f64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    super::payments::send_payment_to_contact(
        app_context,
        sdk,
        from_identity,
        to_contact_id,
        amount_dash,
        memo,
    )
    .await
}

pub async fn load_payment_history(
    app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_id: Option<Identifier>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // Load payment history from local database
    let history = super::payments::load_payment_history(
        app_context,
        &identity.identity.id(),
        contact_id.as_ref(),
    )
    .await
    .map_err(|e| DashPayError::Internal { message: e })?;

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

/// Fetch a contact's public profile from the Platform
pub async fn fetch_contact_profile(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    _identity: QualifiedIdentity, // May be needed for future privacy features
    contact_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for the contact's profile document
    let mut query = DocumentQuery::new(dashpay_contract, "profile").map_err(|e| {
        DashPayError::QueryCreation {
            query_target: "DashPay profile",
            source: Box::new(e),
        }
    })?;

    query = query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(contact_id.to_buffer()),
    });
    query.limit = 1;

    let results = Document::fetch_many(sdk, query).await?;
    let profile_doc = results.into_iter().next().and_then(|(_, doc)| doc);
    Ok(BackendTaskSuccessResult::DashPayContactProfile(profile_doc))
}

/// Search for users on the Platform by DPNS username (per DIP-12/DIP-15)
///
/// Per the DIPs, search should:
/// 1. Query DPNS for username prefix matches
/// 2. Get the identity IDs from those results
/// 3. Fetch profiles for display info (avatar, displayName)
/// 4. Return the DPNS username prominently (it's the verified identifier)
pub async fn search_profiles(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    search_query: String,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let dpns_contract = app_context.dpns_contract.clone();
    let dashpay_contract = app_context.dashpay_contract.clone();
    let mut results: Vec<(Identifier, Option<Document>, String)> = Vec::new();

    let query_trimmed = search_query.trim();
    if query_trimmed.is_empty() {
        return Ok(BackendTaskSuccessResult::DashPayProfileSearchResults(
            results,
        ));
    }

    let normalized_query = crate::model::dpns::normalize_dpns_label(query_trimmed);

    // Search DPNS for usernames starting with the query
    let mut dpns_query =
        DocumentQuery::new(dpns_contract, "domain").map_err(|e| DashPayError::QueryCreation {
            query_target: "DPNS domain",
            source: Box::new(e),
        })?;

    dpns_query = dpns_query
        .with_where(WhereClause {
            field: "normalizedParentDomainName".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Text("dash".to_string()),
        })
        .with_where(WhereClause {
            field: "normalizedLabel".to_string(),
            operator: WhereOperator::StartsWith,
            value: Value::Text(normalized_query.clone()),
        })
        .with_order_by(OrderClause {
            field: "normalizedLabel".to_string(),
            ascending: true,
        }); // Required for StartsWith range query
    dpns_query.limit = 20; // Limit results

    let dpns_results = Document::fetch_many(sdk, dpns_query).await?;

    // Collect identity IDs and usernames from DPNS results
    let mut identity_usernames: Vec<(Identifier, String)> = Vec::new();
    for (_, doc) in dpns_results {
        if let Some(document) = doc {
            // Extract identity ID from records.identity — the authoritative
            // reference, which may differ from owner_id() after name transfers.
            let identity_id = crate::model::dpns::extract_identity_id_from_dpns_document(&document);

            let Some(identity_id) = identity_id else {
                continue;
            };

            let username = document
                .get("label")
                .and_then(|v| v.as_text())
                .map(|s| format!("{}.dash", s))
                .unwrap_or_else(|| format!("{}.dash", identity_id.to_string(Encoding::Base58)));

            identity_usernames.push((identity_id, username));
        }
    }

    // Fetch profiles for each identity
    for (identity_id, username) in identity_usernames {
        // Query for profile document owned by this identity
        let mut profile_query =
            DocumentQuery::new(dashpay_contract.clone(), "profile").map_err(|e| {
                DashPayError::QueryCreation {
                    query_target: "DashPay profile",
                    source: Box::new(e),
                }
            })?;

        profile_query = profile_query.with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(identity_id.to_buffer()),
        });
        profile_query.limit = 1;

        let profile_results = Document::fetch_many(sdk, profile_query).await;

        // Get the profile document if it exists (profile is optional)
        let profile_doc = match profile_results {
            Ok(docs) => docs.into_iter().next().and_then(|(_, doc)| doc),
            Err(_) => None, // Profile fetch failed, but user exists
        };

        results.push((identity_id, profile_doc, username));
    }

    Ok(BackendTaskSuccessResult::DashPayProfileSearchResults(
        results,
    ))
}
