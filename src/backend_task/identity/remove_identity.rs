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

        // `unload_local_qualified_identity` (not the bare `delete_...`) so a
        // wallet-derived identity's forgotten-marker is recorded — otherwise
        // discovery would silently resurrect it after this "Remove" action,
        // same as any other unload. Discriminate a committed-but-cleanup-only
        // failure (`identity_was_removed()`) from a genuine removal failure so
        // in-memory state still gets reconciled in the former case, matching
        // `unload_identity()`'s contract instead of aborting via a bare `?`.
        let cleanup_error = match self.unload_local_qualified_identity(&identity_id) {
            Ok(()) => None,
            Err(error) if error.identity_was_removed() => Some(error),
            Err(error) => return Err(error),
        };
        self.reconcile_unloaded_identity_memory(&identity_id);

        let mut removed_identity_ids = vec![identity_id];
        let mut associated_cleanup_failed = cleanup_error.is_some();
        if let Some(voter_id) = associated_voter_identity_id.filter(|id| *id != identity_id) {
            match self.unload_local_qualified_identity(&voter_id) {
                Ok(()) => {
                    self.reconcile_unloaded_identity_memory(&voter_id);
                    removed_identity_ids.push(voter_id);
                }
                Err(error) if error.identity_was_removed() => {
                    self.reconcile_unloaded_identity_memory(&voter_id);
                    removed_identity_ids.push(voter_id);
                    associated_cleanup_failed = true;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use crate::model::wallet::Wallet;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::sync::{Arc, RwLock};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_identity_reconciles_wallet_cache_and_selection() {
        use crate::app::TaskResult;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let platform_version = PlatformVersion::latest();
        let target_id = Identifier::from([0x61; 32]);
        let sibling_id = Identifier::from([0x62; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let sibling = Identity::create_basic_identity(sibling_id, platform_version)
            .expect("create sibling identity");
        let mut wallet = Wallet::new_from_seed([0x63; 64], Network::Testnet, None, None)
            .expect("build test wallet");
        wallet.identities.insert(3, target);
        wallet.identities.insert(7, sibling);
        let wallet_seed_hash = wallet.seed_hash();
        ctx.wallets()
            .write()
            .expect("write wallets")
            .insert(wallet_seed_hash, Arc::new(RwLock::new(wallet)));
        ctx.set_selected_identity(Some(target_id));
        ctx.set_pending_identity_selection(target_id);

        let result = ctx
            .remove_identity(target_id)
            .expect("remove identity through the real handler");

        assert!(matches!(
            &result,
            BackendTaskSuccessResult::RemovedIdentities { identity_ids, associated_cleanup_failed }
                if identity_ids == &vec![target_id] && !associated_cleanup_failed
        ));
        let (target_evicted, cached_sibling) = {
            let wallets = ctx.wallets().read().expect("read wallets");
            let wallet = wallets
                .get(&wallet_seed_hash)
                .expect("test wallet remains")
                .read()
                .expect("read test wallet");
            (
                wallet
                    .identities
                    .values()
                    .all(|identity| identity.id() != target_id),
                wallet.identities.get(&7).map(IdentityGettersV0::id),
            )
        };
        assert!(
            target_evicted,
            "the removed identity must be evicted from the wallet cache"
        );
        assert_eq!(
            cached_sibling,
            Some(sibling_id),
            "the sibling identity must remain cached"
        );
        assert_eq!(
            ctx.selected_identity_id(),
            None,
            "the removed identity must no longer be selected"
        );
        assert_eq!(
            ctx.take_pending_identity_selection(),
            None,
            "a pending selection for the removed identity must be cleared"
        );
        backend.shutdown().await;
    }
}
