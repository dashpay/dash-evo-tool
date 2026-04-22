use dash_sdk::{
    Sdk, dpp::state_transition::StateTransition,
    platform::transition::broadcast::BroadcastStateTransition,
};

use crate::backend_task::error::TaskError;
use crate::context::AppContext;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub async fn broadcast_state_transition(
        &self,
        state_transition: StateTransition,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        state_transition
            .broadcast(sdk, None)
            .await
            .map(|_| BackendTaskSuccessResult::BroadcastedStateTransition)
            .map_err(TaskError::from)
    }
}
