use crate::backend_task::dashpay::auto_accept_proof::verify_auto_accept_proof;
use crate::backend_task::dashpay::contact_requests::accept_contact_request;
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

/// Process incoming contact requests and check for autoAcceptProof
///
/// This function checks all incoming contact requests for valid autoAcceptProof
/// and automatically accepts and reciprocates if the proof is valid.
pub async fn process_auto_accept_requests(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: QualifiedIdentity,
) -> Result<Vec<(Identifier, bool)>, String> {
    let identity_id = identity.identity.id();
    let dashpay_contract = app_context.dashpay_contract.clone();

    // Query for incoming contact requests
    let mut incoming_query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    incoming_query = incoming_query.with_where(WhereClause {
        field: "toUserId".to_string(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(identity_id.to_buffer()),
    });

    // Add orderBy to avoid platform bug
    incoming_query = incoming_query.with_order_by(OrderClause {
        field: "$createdAt".to_string(),
        ascending: true,
    });
    incoming_query.limit = 100;

    let incoming_docs = Document::fetch_many(sdk, incoming_query)
        .await
        .map_err(|e| format!("Error fetching incoming contact requests: {}", e))?;

    // Stateless verification; no stored proofs needed

    let mut auto_accepted_requests = Vec::new();

    for (request_id, doc) in incoming_docs {
        if let Some(doc) = doc {
            let from_id = doc.owner_id();
            let props = doc.properties();

            // Check if this request has an autoAcceptProof
            if let Some(Value::Bytes(proof_data)) = props.get("autoAcceptProof") {
                tracing::debug!(
                    "Found contact request with autoAcceptProof from {}",
                    from_id.to_string(Encoding::Base58)
                );

                // Extract accountReference for message construction (default to 0 if missing)
                let account_reference = match props.get("accountReference") {
                    Some(Value::U32(v)) => *v,
                    Some(Value::U64(v)) => *v as u32,
                    Some(Value::I64(v)) => *v as u32,
                    Some(Value::U128(v)) => *v as u32,
                    Some(Value::I128(v)) => *v as u32,
                    _ => 0u32,
                };

                // Verify the proof per DIP-0015
                match verify_auto_accept_proof(
                    proof_data,
                    from_id,
                    identity.identity.id(),
                    &identity,
                    account_reference,
                ) {
                    Ok(true) => {
                        tracing::debug!(
                            "Valid autoAcceptProof, auto-accepting contact request from {}",
                            from_id.to_string(Encoding::Base58)
                        );

                        // Accept the request (which sends a reciprocal request)
                        match accept_contact_request(app_context, sdk, identity.clone(), request_id)
                            .await
                        {
                            Ok(_) => {
                                auto_accepted_requests.push((from_id, true));

                                // Stateless: no persistence required
                            }
                            Err(e) => {
                                tracing::error!("Failed to auto-accept contact request: {}", e);
                                auto_accepted_requests.push((from_id, false));
                            }
                        }
                    }
                    Ok(false) => {
                        tracing::warn!(
                            "Invalid or expired autoAcceptProof from {}",
                            from_id.to_string(Encoding::Base58)
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to verify autoAcceptProof: {}", e);
                    }
                }
            }
        }
    }

    Ok(auto_accepted_requests)
}

// No DB persistence required

// Proof creation moved to contact_requests::send_contact_request_with_proof
