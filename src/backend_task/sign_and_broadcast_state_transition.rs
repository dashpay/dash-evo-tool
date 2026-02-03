//! Sign and broadcast state transitions requested via dash-st: URIs

use dash_sdk::Sdk;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::platform::IdentityPublicKey;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;

use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;

use super::BackendTaskSuccessResult;

impl AppContext {
    /// Sign and broadcast a state transition on behalf of an external application
    ///
    /// This is used for dash-st: URI requests where an external app provides
    /// an unsigned state transition that needs signing and broadcasting.
    pub async fn sign_and_broadcast_state_transition(
        &self,
        sdk: &Sdk,
        identity: &QualifiedIdentity,
        signing_key: &IdentityPublicKey,
        mut state_transition: StateTransition,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Sign the state transition using the identity's key
        state_transition
            .sign_external(signing_key, identity, None::<fn(_, _) -> _>)
            .map_err(|e| format!("Failed to sign state transition: {}", e))?;

        // Broadcast the signed state transition
        match state_transition.broadcast(sdk, None).await {
            Ok(_) => Ok(BackendTaskSuccessResult::BroadcastedStateTransition),
            Err(e) => Err(format!("Error broadcasting state transition: {}", e)),
        }
    }
}
