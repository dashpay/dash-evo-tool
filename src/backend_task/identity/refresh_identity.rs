use crate::backend_task::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
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
        let maybe_refreshed_identity =
            Identity::fetch_by_identifier(sdk, refreshed_identity_id).await?;

        // Apply only what was fetched, to the record as stored now: a stale
        // snapshot written back would undo whatever changed while the fetch
        // ran, such as a removed key whose vault secret is already gone.
        let qualified_identity_to_update =
            self.edit_local_qualified_identity(&refreshed_identity_id, |stored| {
                // Refresh only a loaded identity; never re-list one.
                if !self.is_identity_listed(&refreshed_identity_id)? {
                    return Err(TaskError::IdentityNotFoundLocally);
                }
                match maybe_refreshed_identity {
                    Some(refreshed_identity) => {
                        stored.identity = refreshed_identity;
                        stored.status.update(IdentityStatus::Active);
                    }
                    None => stored.status.update(IdentityStatus::NotFound),
                }
                Ok(())
            })?;

        // Send refresh message to refresh the Identities Screen
        sender
            .send(TaskResult::CacheUpdated(crate::backend_task::CacheUpdate {
                network: self.network,
                domain: crate::backend_task::CacheDomain::Identities,
            }))
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        Ok(BackendTaskSuccessResult::RefreshedIdentity(
            qualified_identity_to_update,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::lock_probe::{LockProbe, LockSite};
    use crate::context::test_staging::stage_identity_with_voter_twin;
    use std::sync::Arc;

    /// A refresh that read the node before another writer changed it must not
    /// write its stale copy back: the removal of an old voting key (or any
    /// key) would otherwise come back, pointing at vault secrets already gone.
    /// The refresh changes only what it fetched.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_keeps_a_change_made_while_it_was_running() {
        let staged = stage_identity_with_voter_twin(rand::random(), rand::random()).await;
        let before = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        assert!(before.associated_voter_identity.is_some());
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch(staged.id, Some(before.identity.clone()))
            .await
            .unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(8);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, staged.ctx.egui_ctx().clone());

        let probe = LockProbe::new();
        let refresh_probe = probe.clone();
        let lock = staged.ctx.identity_record_lock(staged.id);
        let guard = lock.lock().unwrap();
        let ctx = Arc::clone(&staged.ctx);
        let snapshot = before.clone();
        let refresh = std::thread::spawn(move || {
            let _attached = refresh_probe.attach();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(ctx.refresh_identity(&sdk, snapshot, sender))
        });
        while probe.count(LockSite::RecordRequest) == 0 {
            std::thread::yield_now();
        }
        let mut removed = before.clone();
        removed.associated_voter_identity = None;
        staged
            .ctx
            .write_local_qualified_identity_locked(&removed)
            .unwrap();
        drop(guard);
        refresh.join().unwrap().unwrap();

        let after = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        assert!(
            after.associated_voter_identity.is_none(),
            "the refresh must not bring back what was removed while it ran"
        );
        assert_eq!(after.status, IdentityStatus::Active);
    }
}
