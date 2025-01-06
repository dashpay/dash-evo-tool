use dash_sdk::{
    dpp::state_transition::StateTransition,
    platform::transition::broadcast::BroadcastStateTransition, Sdk,
};

use crate::context::AppContext;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub async fn broadcast_state_transition(
        &self,
        state_transition: StateTransition,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match state_transition.broadcast_and_wait(sdk, None).await {
            Ok(_) => Ok(BackendTaskSuccessResult::Message(
                "State transition broadcasted successfully".to_string(),
            )),
            Err(e) => Err(format!("Error broadcasting state transition: {}", e)),
        }
    }
}
