use crate::backend_task::{BackendTaskSuccessResult, TaskError};
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

/// Fail-closed roster check for the post-delisting-failure distinction
/// below: an unreadable index cannot prove `id` is gone, so treat it as
/// still listed (the safer of the two wrong answers — a real failure stays
/// a real failure) rather than silently swallowing the read error.
fn still_listed_or_assume_so(app_context: &AppContext, id: &Identifier) -> bool {
    match app_context.is_identity_listed(id) {
        Ok(listed) => listed,
        Err(error) => {
            tracing::warn!(
                ?error,
                identity_id = %id,
                "Could not read the identity index to check removal outcome; assuming still listed"
            );
            true
        }
    }
}

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

        // `delete_local_qualified_identity` removes an identity from the
        // Global index before its irreversible vault delete, so a failure
        // can land strictly after that identity is already gone from every
        // screen — a durable vault-cleanup manifest (see that method's doc
        // comment on its `Err` postcondition) guarantees the next boot's
        // sweep finishes the job regardless. `is_identity_listed` tells that
        // case apart from a real failure: reporting a post-delisting failure
        // as an outright removal failure would tell the user their
        // already-gone identity is still there and safe to retry, when
        // neither is true and no UI control remains to reach it with anyway.
        // Applied to both identities this call can touch — the primary and,
        // below, its associated voter twin — since the same failure window
        // exists on both `delete_local_qualified_identity` calls.
        let mut cleanup_deferred = false;
        match self.delete_local_qualified_identity(&identity_id) {
            Ok(()) => {}
            Err(error) => {
                if still_listed_or_assume_so(self, &identity_id) {
                    return Err(error);
                }
                cleanup_deferred = true;
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
                    if still_listed_or_assume_so(self, &voter_id) {
                        associated_cleanup_failed = true;
                        tracing::warn!(
                            ?error,
                            voter_identity_id = %voter_id,
                            "Associated voter identity cleanup failed"
                        );
                    } else {
                        removed_identity_ids.push(voter_id);
                        cleanup_deferred = true;
                        tracing::warn!(
                            ?error,
                            voter_identity_id = %voter_id,
                            "Associated voter identity removed but its vault cleanup is still pending; the next boot's sweep will finish it"
                        );
                    }
                }
            }
        }

        Ok(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: removed_identity_ids,
            associated_cleanup_failed,
            cleanup_deferred,
        })
    }
}
