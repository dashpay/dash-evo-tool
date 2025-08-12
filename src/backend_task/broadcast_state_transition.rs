use dash_sdk::{
    Sdk, dpp::state_transition::StateTransition,
    platform::transition::broadcast::BroadcastStateTransition,
};

use crate::context::AppContext;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub async fn broadcast_state_transition(
        &self,
        state_transition: StateTransition,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match state_transition.broadcast(sdk, None).await {
            Ok(_) => Ok(BackendTaskSuccessResult::Message(
                "State transition broadcasted successfully".to_string(),
            )),
            Err(e) => Err(format!("Error broadcasting state transition: {}", e)),
        }
    }
}
