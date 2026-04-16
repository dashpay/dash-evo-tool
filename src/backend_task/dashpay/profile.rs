//! DashPay profile operations.
//!
//! Load/create/update operations are now handled by
//! platform-wallet's DashPayWallet (sync_profiles, create_profile,
//! update_profile). This module retains only:
//! - fetch_contact_profile (single-contact profile query)
//! - search_profiles (DPNS + profile search)
//! - load_payment_history (local DB read)

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use std::sync::Arc;

pub async fn load_payment_history(
    app_context: &Arc<AppContext>,
    _sdk: &Sdk,
    identity: QualifiedIdentity,
    contact_id: Option<Identifier>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let history = super::payments::load_payment_history(
        app_context,
        &identity.identity.id(),
        contact_id.as_ref(),
    )
    .await
    .map_err(|e| DashPayError::Internal { message: e })?;

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
        Ok(BackendTaskSuccessResult::Message(format!(
            "Found {} payment records",
            history.len()
        )))
    }
}

/// Fetch a contact's public profile from Platform
///
/// TODO: Move to DashPayWallet — this does its own Platform query
/// instead of delegating to the wallet. Should be part of the
/// contact sync or a DashPayWallet::fetch_contact_profile() method.
pub async fn fetch_contact_profile(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    _identity: QualifiedIdentity,
    contact_id: Identifier,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let dashpay_contract = app_context.dashpay_contract.clone();

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

/// Search for users by DPNS username prefix (DIP-12/DIP-15)
///
/// TODO: Move to DashPayWallet::search_profiles() — this does its
/// own DPNS + DashPay Platform queries. The wallet should own all
/// Platform interactions; evo-tool just triggers and displays.
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
        });
    dpns_query.limit = 20;

    let dpns_results = Document::fetch_many(sdk, dpns_query).await?;

    let mut identity_usernames: Vec<(Identifier, String)> = Vec::new();
    for (_, doc) in dpns_results {
        if let Some(document) = doc {
            let identity_id = crate::model::dpns::extract_identity_id_from_dpns_document(&document);
            let Some(identity_id) = identity_id else {
                continue;
            };
            let username = document
                .get("label")
                .and_then(|v| v.as_text())
                .map(|s| format!("{s}.dash"))
                .unwrap_or_else(|| format!("{}.dash", identity_id.to_string(Encoding::Base58)));
            identity_usernames.push((identity_id, username));
        }
    }

    for (identity_id, username) in identity_usernames {
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
        let profile_doc = match profile_results {
            Ok(docs) => docs.into_iter().next().and_then(|(_, doc)| doc),
            Err(_) => None,
        };

        results.push((identity_id, profile_doc, username));
    }

    Ok(BackendTaskSuccessResult::DashPayProfileSearchResults(
        results,
    ))
}
