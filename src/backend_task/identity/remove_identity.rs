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

        let mut primary_cleanup_deferred = false;
        match self.delete_local_qualified_identity(&identity_id) {
            Ok(()) => {}
            Err(error) => {
                // `delete_local_qualified_identity` removes the identity from
                // the Global index before its irreversible vault delete, so a
                // failure can land strictly after the identity is already
                // gone from every screen — a durable vault-cleanup manifest
                // (see that method's doc comment) guarantees the next boot's
                // sweep finishes the job regardless. Reporting that as an
                // outright removal failure would tell the user their
                // already-gone identity is still there and safe to retry,
                // when neither is true and no UI control remains to reach it
                // with anyway.
                if self.is_identity_listed(&identity_id).unwrap_or(true) {
                    return Err(error);
                }
                primary_cleanup_deferred = true;
                tracing::warn!(
                    ?error,
                    %identity_id,
                    "Identity removed but its vault cleanup is still pending; the next boot's sweep will finish it"
                );
            }
        }

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
            primary_cleanup_deferred,
        })
    }
}
