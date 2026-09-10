use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::{Value, string_encoding::Encoding};
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use platform_wallet::ProfileUpdate;
use std::sync::Arc;

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
            source: Box::new(e.into()),
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

        // Mirror to upstream so DashpayView::profile observes the loaded state.
        mirror_profile_to_backend(
            app_context,
            &identity_id,
            Some(BackendProfileFields {
                display_name: non_empty(display_name),
                bio: non_empty(bio),
                avatar_url: non_empty(avatar_url),
                avatar_hash: doc
                    .get("avatarHash")
                    .and_then(|value| value.as_bytes_slice().ok())
                    .and_then(|bytes| bytes.try_into().ok()),
                avatar_fingerprint: doc
                    .get("avatarFingerprint")
                    .and_then(|value| value.as_bytes_slice().ok())
                    .and_then(|bytes| bytes.try_into().ok()),
            }),
        )
        .await;

        Ok(BackendTaskSuccessResult::DashPayProfile(Some((
            display_name.to_string(),
            bio.to_string(),
            avatar_url.to_string(),
        ))))
    } else {
        // No profile found — clear any stale upstream entry for this owner.
        mirror_profile_to_backend(app_context, &identity_id, None).await;

        Ok(BackendTaskSuccessResult::DashPayProfile(None))
    }
}

/// Profile fields suitable for [`DashPayProfile`] construction. Used by the
/// thin mirror that pushes profile state down to upstream `WalletBackend`.
struct BackendProfileFields {
    display_name: Option<String>,
    bio: Option<String>,
    avatar_url: Option<String>,
    avatar_hash: Option<[u8; 32]>,
    avatar_fingerprint: Option<[u8; 8]>,
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Cache a fetched profile; a mirror failure leaves the next refresh to retry.
async fn mirror_profile_to_backend(
    app_context: &Arc<AppContext>,
    owner: &Identifier,
    fields: Option<BackendProfileFields>,
) {
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    use platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile;

    let Ok(backend) = app_context.wallet_backend() else {
        // Pre-init / headless: nothing to mirror. View paths fall back to
        // the upstream-empty default until the backend is wired.
        return;
    };

    let now_ms = chrono::Utc::now().timestamp_millis().max(0);

    let profile = fields.map(|f| DashPayProfile {
        display_name: f.display_name,
        public_message: f.bio.clone(),
        bio: f.bio,
        avatar_url: f.avatar_url,
        avatar_hash: f.avatar_hash,
        avatar_fingerprint: f.avatar_fingerprint,
    });

    if let Err(e) = backend.dashpay_set_profile(owner, profile).await {
        tracing::debug!(
            owner = %owner.to_string(Encoding::Base58),
            error = ?e,
            "DashPay profile mirror to WalletBackend failed; view will re-fetch on next refresh"
        );
        return;
    }

    if let Err(e) = backend.dashpay_set_timestamps(owner, now_ms, now_ms) {
        tracing::debug!(
            owner = %owner.to_string(Encoding::Base58),
            error = ?e,
            "DashPay profile timestamp sidecar write failed; created_at/updated_at will read 0"
        );
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
    let mut input = profile_update_input(display_name, bio, avatar_url)?;
    let backend = app_context.wallet_backend()?;
    let identity_id = identity.identity.id();
    let mut query =
        DocumentQuery::new(app_context.dashpay_contract.clone(), "profile").map_err(|e| {
            DashPayError::QueryCreation {
                query_target: "DashPay profile",
                source: Box::new(e.into()),
            }
        })?;
    query = query.with_where(WhereClause {
        field: "$ownerId".to_string(),
        operator: WhereOperator::Equal,
        value: identity_id.to_buffer().into(),
    });
    query.limit = 1;
    let profiles = Document::fetch_many(sdk, query).await?;
    let existing = profiles.values().flatten().next();
    ensure_profile_fields_preserved(existing, &input)?;

    if let Some(url) = &input.avatar_url {
        match super::avatar_processing::fetch_image_bytes(url).await {
            Ok(bytes) => input.avatar_bytes = Some(bytes),
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "Profile avatar could not be downloaded; saving its URL only"
                );
            }
        }
    }

    backend
        .dashpay_write_profile(&identity, input, existing.is_none())
        .await?;
    Ok(BackendTaskSuccessResult::DashPayProfileUpdated(identity_id))
}

fn profile_update_input(
    display_name: Option<String>,
    bio: Option<String>,
    avatar_url: Option<String>,
) -> Result<ProfileUpdate, TaskError> {
    let display_name = display_name.unwrap_or_default();
    let bio = bio.unwrap_or_default();
    let avatar_url = avatar_url.unwrap_or_default();
    let errors = crate::model::dashpay::validate_profile_fields(
        display_name.trim(),
        bio.trim(),
        avatar_url.trim(),
    );
    if !errors.is_empty() {
        return Err(DashPayError::ProfileValidationFailed { errors }.into());
    }
    Ok(ProfileUpdate {
        display_name: non_empty(display_name.trim()),
        public_message: non_empty(bio.trim()),
        avatar_url: non_empty(avatar_url.trim()),
        avatar_bytes: None,
    })
}

fn ensure_profile_fields_preserved(
    existing: Option<&Document>,
    input: &ProfileUpdate,
) -> Result<(), TaskError> {
    let Some(existing) = existing else {
        return Ok(());
    };
    // Upstream treats None as unchanged, so it cannot fulfill a request to clear a field.
    for (field, replacement) in [
        ("displayName", &input.display_name),
        ("publicMessage", &input.public_message),
        ("avatarUrl", &input.avatar_url),
    ] {
        if replacement.is_none()
            && existing
                .get(field)
                .and_then(Value::as_text)
                .is_some_and(|value| !value.is_empty())
        {
            return Err(DashPayError::ProfileFieldRemovalUnsupported.into());
        }
    }
    Ok(())
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
            source: Box::new(e.into()),
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
            source: Box::new(e.into()),
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
                    source: Box::new(e.into()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_identity() -> QualifiedIdentity {
        QualifiedIdentity {
            identity: dash_sdk::platform::Identity::default_versioned(
                dash_sdk::dpp::version::LATEST_PLATFORM_VERSION,
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: Default::default(),
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
    }

    #[tokio::test]
    async fn profile_write_validates_fields_before_network_or_signing() {
        let dir = tempfile::tempdir().expect("temporary app data");
        let context = crate::context::test_support::test_app_context(dir.path());
        let sdk = context.sdk.load_full();
        let error = update_profile(
            &context,
            &sdk,
            empty_identity(),
            Some("x".repeat(256)),
            None,
            None,
        )
        .await
        .expect_err("invalid input must not reach the network or signer");
        assert!(
            matches!(
                error,
                TaskError::DashPay(DashPayError::ProfileValidationFailed { .. })
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn profile_input_omits_blank_fields_and_maps_bio_to_public_message() {
        let input = profile_update_input(
            Some("  Alice  ".into()),
            Some(" About me ".into()),
            Some(" \n ".into()),
        )
        .expect("valid input");
        assert_eq!(input.display_name.as_deref(), Some("Alice"));
        assert_eq!(input.public_message.as_deref(), Some("About me"));
        assert!(input.avatar_url.is_none());
        assert!(input.avatar_bytes.is_none());
        let blank = profile_update_input(Some(" ".into()), None, None).expect("empty profile");
        assert!(blank.display_name.is_none());
        assert!(blank.public_message.is_none());
        assert!(ensure_profile_fields_preserved(None, &blank).is_ok());
    }

    #[test]
    fn profile_removal_is_rejected_instead_of_silently_retaining_fields() {
        for field in ["displayName", "publicMessage", "avatarUrl"] {
            let existing = Document::V0(dash_sdk::dpp::document::DocumentV0 {
                properties: [(field.to_string(), Value::Text("existing value".into()))].into(),
                ..Default::default()
            });
            let input = profile_update_input(None, None, None).expect("blank input");
            assert!(
                matches!(
                    ensure_profile_fields_preserved(Some(&existing), &input),
                    Err(TaskError::DashPay(
                        DashPayError::ProfileFieldRemovalUnsupported
                    ))
                ),
                "clearing {field} must not report a successful save"
            );
        }
    }

    #[test]
    fn profile_replacement_is_allowed_with_unchanged_optional_fields() {
        let existing = Document::V0(dash_sdk::dpp::document::DocumentV0 {
            properties: [("displayName".to_string(), Value::Text("Alice".into()))].into(),
            ..Default::default()
        });
        let input = profile_update_input(Some("Bob".into()), None, None).expect("replacement");
        assert!(ensure_profile_fields_preserved(Some(&existing), &input).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_write_without_managing_wallet_is_an_error() {
        let dir = tempfile::tempdir().expect("temporary app data");
        let context = crate::context::test_support::test_app_context(dir.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel(32);
        context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                sender,
                context.egui_ctx().clone(),
            ))
            .await
            .expect("offline wallet backend");
        let backend = context.wallet_backend().expect("backend");
        for create in [true, false] {
            let error = backend
                .dashpay_write_profile(
                    &empty_identity(),
                    profile_update_input(Some("Alice".into()), None, None).expect("input"),
                    create,
                )
                .await
                .expect_err("a write cannot silently skip an unowned identity");
            assert!(matches!(
                error,
                TaskError::DashPay(DashPayError::ProfileWalletRequired)
            ));
        }
    }
}
