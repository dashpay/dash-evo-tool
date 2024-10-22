use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Fetch, Identity};
use dash_sdk::Sdk;
use tokio::sync::mpsc;

impl AppContext {
    pub(super) async fn refresh_identity(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        // Fetch the latest state of the identity from Platform
        let refreshed_identity =
            Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| {
                    format!(
                        "Identity with id {} not found in Platform state",
                        qualified_identity.identity.id().to_string(Encoding::Base58)
                    )
                })?;

        // Get local identities
        let mut local_qualified_identities = self
            .load_local_qualified_identities()
            .map_err(|e| e.to_string())?;

        // Find the local identity to update
        let outdated_identity_index = local_qualified_identities
            .iter()
            .position(|qi| qi.identity.id() == refreshed_identity.id())
            .ok_or_else(|| {
                format!(
                    "Identity with id {} not found in local identities",
                    refreshed_identity.id().to_string(Encoding::Base58)
                )
            })?;

        // Remove the outdated identity from local state
        let mut qualified_identity_to_update =
            local_qualified_identities.remove(outdated_identity_index);

        // Update the identity
        qualified_identity_to_update.identity = refreshed_identity;

        // Insert the updated identity into local state
        self.insert_local_qualified_identity(&qualified_identity_to_update)
            .map_err(|e| e.to_string())?;

        // Send refresh message to refresh the Identities Screen
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}
