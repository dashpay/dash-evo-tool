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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTaskSuccessResult;
    use crate::context::test_staging::{
        StagedIdentity, fail_removal_before_delisting, fail_removals_after_delisting,
        stage_identity_with_vaulted_keys, stage_identity_with_voter_twin,
    };
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::wallet_backend::IdentityKeyView;

    const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const HIGH: [u8; 32] = [0xA1; 32];
    const MEDIUM: [u8; 32] = [0xB2; 32];

    /// Unpack the only success shape this task produces.
    fn removal_outcome(result: BackendTaskSuccessResult) -> (Vec<Identifier>, bool, bool) {
        match result {
            BackendTaskSuccessResult::RemovedIdentities {
                identity_ids,
                associated_cleanup_failed,
                cleanup_deferred,
            } => (identity_ids, associated_cleanup_failed, cleanup_deferred),
            other => panic!("removing an identity must report a removal, got {other:?}"),
        }
    }

    /// Whether any of the identity's keys are still in the vault.
    fn keys_remain(staged: &StagedIdentity, id: Identifier) -> bool {
        let view = IdentityKeyView::new(&staged.store, id.to_buffer());
        [1, 2]
            .iter()
            .any(|key_id| view.get(&MAIN, *key_id).unwrap().is_some())
    }

    /// A removal that fails before the identity is delisted did not happen:
    /// the identity is still on every screen, its keys are still on the
    /// device, and the button that started this still works. Reporting it as a
    /// success would strand a user who can plainly see the identity is still
    /// there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_removal_that_fails_before_delisting_is_reported_as_a_failure() {
        let staged = stage_identity_with_vaulted_keys(HIGH, MEDIUM).await;
        fail_removal_before_delisting(&staged.ctx, &staged.id);

        staged
            .ctx
            .remove_identity(staged.id)
            .expect_err("a removal that never delisted the identity is a failure");

        assert!(
            staged
                .ctx
                .is_identity_listed(&staged.id)
                .expect("read listed state"),
            "the identity must still be listed, so the user's retry is reachable"
        );
        assert!(
            keys_remain(&staged, staged.id),
            "nothing was deleted, so every key must still be in the vault"
        );
    }

    /// The distinction this classifier exists for. Once `index_remove_identity`
    /// has run the identity is off every screen and no UI control can reach it
    /// again, so a later failure is not something the user can retry — it is a
    /// completed removal whose vault cleanup the next boot's sweep finishes.
    /// Reporting it as an outright failure would tell them an identity they can
    /// no longer see is still there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_removal_that_fails_after_delisting_is_a_removal_with_a_deferred_cleanup() {
        let staged = stage_identity_with_vaulted_keys(HIGH, MEDIUM).await;
        fail_removals_after_delisting(&staged.ctx);

        let (identity_ids, associated_cleanup_failed, cleanup_deferred) = removal_outcome(
            staged
                .ctx
                .remove_identity(staged.id)
                .expect("a delisted identity is removed, whatever failed afterwards"),
        );

        assert_eq!(identity_ids, vec![staged.id]);
        assert!(
            cleanup_deferred,
            "the vault cleanup is outstanding and must be reported as such"
        );
        assert!(
            !associated_cleanup_failed,
            "there is no associated identity here, so nothing may be blamed on one"
        );
        assert!(
            !staged
                .ctx
                .is_identity_listed(&staged.id)
                .expect("read listed state"),
            "the identity really is delisted; that is what makes this not a failure"
        );
        assert!(
            keys_remain(&staged, staged.id),
            "the deferral must be real: the keys the sweep will collect are still there"
        );
    }

    /// A voter twin that fails *after* its own delisting is in the same
    /// position as the primary: gone from storage, cleanup pending. It belongs
    /// in the removed set — counting it as an associated-cleanup failure would
    /// invite the user to retry an identity that no longer exists.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_voter_twin_that_fails_after_delisting_still_counts_as_removed() {
        let staged = stage_identity_with_voter_twin(HIGH, MEDIUM).await;
        let voter_id = staged.voter_id.expect("the fixture stages a voter twin");
        fail_removals_after_delisting(&staged.ctx);

        let (identity_ids, associated_cleanup_failed, cleanup_deferred) = removal_outcome(
            staged
                .ctx
                .remove_identity(staged.id)
                .expect("both identities are delisted, whatever failed afterwards"),
        );

        assert_eq!(
            identity_ids,
            vec![staged.id, voter_id],
            "both identities were delisted, so both count as removed"
        );
        assert!(
            !associated_cleanup_failed,
            "the voter identity was removed; only its vault cleanup is pending"
        );
        assert!(cleanup_deferred, "both cleanups are outstanding");
        assert!(
            !staged
                .ctx
                .is_identity_listed(&voter_id)
                .expect("read listed state"),
            "the voter identity really is delisted"
        );
    }

    /// The two outcomes are independent and can land in the same removal: the
    /// primary delisted with its cleanup pending, the voter twin never removed
    /// at all. Neither may mask the other — the user has one identity to retry
    /// and one they will never see again, and needs to be told both.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_voter_cleanup_and_a_deferred_primary_cleanup_are_reported_together() {
        let staged = stage_identity_with_voter_twin(HIGH, MEDIUM).await;
        let voter_id = staged.voter_id.expect("the fixture stages a voter twin");
        fail_removals_after_delisting(&staged.ctx);
        fail_removal_before_delisting(&staged.ctx, &voter_id);

        let (identity_ids, associated_cleanup_failed, cleanup_deferred) = removal_outcome(
            staged
                .ctx
                .remove_identity(staged.id)
                .expect("the primary is delisted, so this is still a removal"),
        );

        assert_eq!(
            identity_ids,
            vec![staged.id],
            "the voter identity was never removed, so it must not be listed as removed"
        );
        assert!(
            associated_cleanup_failed,
            "the voter identity is still on file and its failure must be named"
        );
        assert!(
            cleanup_deferred,
            "the primary's pending vault cleanup must survive the voter's failure"
        );
        assert!(
            staged
                .ctx
                .is_identity_listed(&voter_id)
                .expect("read listed state"),
            "the voter identity must still be listed, so its retry is reachable"
        );
    }
}
