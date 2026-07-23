use crate::backend_task::{BackendTaskSuccessResult, TaskError};
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

impl AppContext {
    pub(super) fn remove_identity(
        &self,
        identity_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let associated_voter_identity_id = self
            .get_local_qualified_identity(&identity_id)?
            .and_then(|identity| {
                identity
                    .associated_voter_identity
                    .map(|(voter_identity, _)| voter_identity.id())
            });

        self.delete_local_qualified_identity(&identity_id)?;

        let mut removed_identity_ids = vec![identity_id];
        let mut associated_cleanup_failed = false;
        if let Some(voter_id) = associated_voter_identity_id.filter(|id| *id != identity_id) {
            match self.delete_local_qualified_identity(&voter_id) {
                Ok(()) => removed_identity_ids.push(voter_id),
                Err(error) => {
                    associated_cleanup_failed = true;
                    tracing::warn!(
                        ?error,
                        voter_identity_id = %voter_id,
                        "Associated voter identity cleanup failed"
                    );
                }
            }
        }

        Ok(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: removed_identity_ids,
            associated_cleanup_failed,
        })
    }
}
