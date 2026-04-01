use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::DPNSNameInfo;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

impl AppContext {
    pub(super) async fn refresh_loaded_identities_dpns_names(
        &self,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let qualified_identities = self.load_local_qualified_identities()?;

        let sdk = self.sdk.load().as_ref().clone();

        for mut qualified_identity in qualified_identities {
            let identity_id = qualified_identity.identity.id();

            let dpns_usernames = sdk
                .get_dpns_usernames_by_identity(identity_id, None)
                .await
                .map_err(|e| TaskError::DpnsFetchError {
                    source: Box::new(e),
                })?;

            qualified_identity.dpns_names = dpns_usernames
                .into_iter()
                .map(|u| DPNSNameInfo {
                    name: u.label,
                    acquired_at: 0, // Timestamp not available from SDK query
                })
                .collect();

            if qualified_identity.alias.is_none() && !qualified_identity.dpns_names.is_empty() {
                let dpns_name = &qualified_identity.dpns_names[0].name;
                qualified_identity.alias = Some(format!("{}.dash", dpns_name));
            }

            self.update_local_qualified_identity(&qualified_identity)
                .map_err(|e| TaskError::Database { source: e })?;
        }

        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        Ok(BackendTaskSuccessResult::RefreshedOwnedDpnsNames)
    }
}
