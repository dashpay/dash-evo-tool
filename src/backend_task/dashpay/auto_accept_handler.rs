use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::dashpay::auto_accept_proof::{StoredProof, verify_auto_accept_proof};
use crate::backend_task::dashpay::contact_requests::{
    accept_contact_request, send_contact_request,
};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use std::collections::HashSet;
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

    // Load stored proofs from database (mock for now)
    let stored_proofs = load_stored_proofs(&identity)?;

    let mut auto_accepted_requests = Vec::new();

    for (request_id, doc) in incoming_docs {
        if let Some(doc) = doc {
            let from_id = doc.owner_id();
            let props = doc.properties();

            // Check if this request has an autoAcceptProof
            if let Some(Value::Bytes(proof_data)) = props.get("autoAcceptProof") {
                eprintln!(
                    "DEBUG: Found contact request with autoAcceptProof from {}",
                    from_id.to_string(Encoding::Base58)
                );

                // Verify the proof
                match verify_auto_accept_proof(proof_data, from_id, &identity, &stored_proofs) {
                    Ok(true) => {
                        eprintln!(
                            "DEBUG: Valid autoAcceptProof! Auto-accepting contact request from {}",
                            from_id.to_string(Encoding::Base58)
                        );

                        // Accept the request (which sends a reciprocal request)
                        match accept_contact_request(app_context, sdk, identity.clone(), request_id)
                            .await
                        {
                            Ok(_) => {
                                auto_accepted_requests.push((from_id, true));

                                // Mark the proof as used
                                mark_proof_as_used(proof_data, &identity)?;
                            }
                            Err(e) => {
                                eprintln!("ERROR: Failed to auto-accept contact request: {}", e);
                                auto_accepted_requests.push((from_id, false));
                            }
                        }
                    }
                    Ok(false) => {
                        eprintln!(
                            "DEBUG: Invalid or expired autoAcceptProof from {}",
                            from_id.to_string(Encoding::Base58)
                        );
                    }
                    Err(e) => {
                        eprintln!("ERROR: Failed to verify autoAcceptProof: {}", e);
                    }
                }
            }
        }
    }

    Ok(auto_accepted_requests)
}

/// Load stored proofs from database
///
/// In production, this would load from SQLite database
fn load_stored_proofs(_identity: &QualifiedIdentity) -> Result<Vec<StoredProof>, String> {
    // TODO: Implement database loading
    // For now, return empty list
    Ok(Vec::new())
}

/// Mark a proof as used so it can't be reused
fn mark_proof_as_used(_proof_data: &[u8], _identity: &QualifiedIdentity) -> Result<(), String> {
    // TODO: Update database to mark proof as used
    Ok(())
}

/// Generate autoAcceptProof data to include in a contact request
///
/// This is called when sending a contact request after scanning someone's QR code
pub fn generate_proof_for_request(
    scanned_qr_data: &str,
    our_identity: &QualifiedIdentity,
) -> Result<Vec<u8>, String> {
    // Parse the QR code data
    let proof_data =
        crate::backend_task::dashpay::auto_accept_proof::AutoAcceptProofData::from_qr_string(
            scanned_qr_data,
        )?;

    // The proof to include is simply the proof key from the QR code
    // This proves we received it from them
    Ok(proof_data.proof_key.to_vec())
}
