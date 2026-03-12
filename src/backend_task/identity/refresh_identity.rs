use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Fetch, Identity};

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn refresh_identity(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let refreshed_identity_id = qualified_identity.identity.id();
        // Fetch the latest state of the identity from Platform
        let maybe_refreshed_identity = Identity::fetch_by_identifier(sdk, refreshed_identity_id)
            .await
            .map_err(|e| TaskError::Generic(format!("Failed to fetch identity: {}", e)))?;

        // Get local identities
        let mut local_qualified_identities = self.load_local_qualified_identities()?;

        // Find the local identity to update
        let outdated_identity_index = local_qualified_identities
            .iter()
            .position(|qi| qi.identity.id() == refreshed_identity_id)
            .ok_or_else(|| {
                TaskError::Generic(format!(
                    "Identity with id {} not found in local identities",
                    refreshed_identity_id.to_string(Encoding::Base58)
                ))
            })?;

        // Remove the outdated identity from local state
        let mut qualified_identity_to_update =
            local_qualified_identities.remove(outdated_identity_index);

        // Update the identity
        match maybe_refreshed_identity {
            Some(refreshed_identity) => {
                qualified_identity_to_update.identity = refreshed_identity;
                qualified_identity_to_update
                    .status
                    .update(IdentityStatus::Active);
            }
            None => {
                qualified_identity_to_update
                    .status
                    .update(IdentityStatus::NotFound);
            }
        }

        // Insert the updated identity into local state
        self.update_local_qualified_identity(&qualified_identity_to_update)?;

        // Send refresh message to refresh the Identities Screen
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|e| TaskError::Generic(e.to_string()))?;

        Ok(BackendTaskSuccessResult::RefreshedIdentity(
            qualified_identity,
        ))
    }
}
