use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Fetch, Identity};
use dash_sdk::Sdk;
use tokio::sync::mpsc;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn refresh_identity(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let refreshed_identity_id = qualified_identity.identity.id();
        // Fetch the latest state of the identity from Platform
        let maybe_refreshed_identity = Identity::fetch_by_identifier(sdk, refreshed_identity_id)
            .await
            .map_err(|e| e.to_string())?;

        // Get local identities
        let mut local_qualified_identities = self
            .load_local_qualified_identities()
            .map_err(|e| e.to_string())?;

        // Find the local identity to update
        let outdated_identity_index = local_qualified_identities
            .iter()
            .position(|qi| qi.identity.id() == refreshed_identity_id)
            .ok_or_else(|| {
                format!(
                    "Identity with id {} not found in local identities",
                    refreshed_identity_id.to_string(Encoding::Base58)
                )
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
                // it is not found and the status allows refresh, update status to NotFound
                qualified_identity_to_update
                    .status
                    .update(IdentityStatus::NotFound);
            }
        }

        // Insert the updated identity into local state
        self.update_local_qualified_identity(&qualified_identity_to_update)
            .map_err(|e| e.to_string())?;

        // Send refresh message to refresh the Identities Screen
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|e| e.to_string())?;

        Ok(BackendTaskSuccessResult::Message(
            "Successfully refreshed identity".to_string(),
        ))
    }
}
