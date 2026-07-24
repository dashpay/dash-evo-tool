use std::collections::HashMap;

use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::wallet_backend::poison::{MutexRecover, RwLockRecover};

fn retain_other_identities(identities: &mut HashMap<u32, Identity>, identity_id: &Identifier) {
    identities.retain(|_, identity| identity.id() != *identity_id);
}

impl AppContext {
    pub(crate) fn reconcile_unloaded_identity_memory(&self, identity_id: &Identifier) {
        let wallets = self.wallets.read_recover();
        for wallet in wallets.values() {
            retain_other_identities(&mut wallet.write_recover().identities, identity_id);
        }
        drop(wallets);

        if self.selected_identity_id() == Some(*identity_id) {
            self.set_selected_identity(None);
        }
        let mut pending = self.pending_identity_selection.lock_recover();
        if *pending == Some(*identity_id) {
            *pending = None;
        }
    }

    pub(super) fn unload_identity(
        &self,
        identity_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let cleanup_error = match self.unload_local_qualified_identity(&identity_id) {
            Ok(()) => None,
            Err(error) if error.identity_was_removed() => Some(error),
            Err(error) => return Err(error),
        };
        self.reconcile_unloaded_identity_memory(&identity_id);

        tracing::info!(
            target = "backend_task::identity::unload_identity",
            identity = %identity_id,
            "Unloaded identity and its local device state",
        );
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        Ok(BackendTaskSuccessResult::UnloadedIdentity(identity_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use crate::model::dashpay::ContactPrivateInfo;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    use crate::model::wallet::Wallet;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};

    #[test]
    fn identity_unload_evicts_only_target_from_wallet_cache() {
        let platform_version = PlatformVersion::latest();
        let target_id = Identifier::from([0x11; 32]);
        let sibling_id = Identifier::from([0x22; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let sibling = Identity::create_basic_identity(sibling_id, platform_version)
            .expect("create sibling identity");
        let mut identities = HashMap::from([(3, target), (7, sibling)]);

        retain_other_identities(&mut identities, &target_id);

        assert_eq!(identities.len(), 1, "only the target must be evicted");
        assert_eq!(
            identities.get(&7).map(IdentityGettersV0::id),
            Some(sibling_id),
            "the sibling identity must remain cached"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_unload_handler_clears_wallet_cache_and_identity_selection() {
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
        let target_id = Identifier::from([0x31; 32]);
        let sibling_id = Identifier::from([0x32; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let sibling = Identity::create_basic_identity(sibling_id, platform_version)
            .expect("create sibling identity");
        let mut wallet = Wallet::new_from_seed([0x33; 64], Network::Testnet, None, None)
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
            .unload_identity(target_id)
            .expect("unload identity through the real handler");

        assert!(matches!(
            result,
            BackendTaskSuccessResult::UnloadedIdentity(identity_id) if identity_id == target_id
        ));
        // Snapshot the cache state under the wallet guards, releasing them at the
        // block's end so no guard is live across `backend.shutdown().await` below
        // (clippy::await_holding_lock).
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
            "the target identity must be evicted from the wallet cache"
        );
        assert_eq!(
            cached_sibling,
            Some(sibling_id),
            "the sibling identity must remain cached"
        );
        assert_eq!(
            ctx.selected_identity_id(),
            None,
            "the unloaded identity must no longer be selected"
        );
        assert_eq!(
            ctx.take_pending_identity_selection(),
            None,
            "a pending selection for the unloaded identity must be cleared"
        );
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_unload_reconciles_memory_after_committed_cleanup_failure() {
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
        let target_id = Identifier::from([0x51; 32]);
        let contact_id = Identifier::from([0x52; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let mut wallet = Wallet::new_from_seed([0x53; 64], Network::Testnet, None, None)
            .expect("build test wallet");
        wallet.identities.insert(3, target.clone());
        let wallet_seed_hash = wallet.seed_hash();
        let qualified_identity = QualifiedIdentity {
            identity: target,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: BTreeMap::from([(
                wallet_seed_hash,
                Arc::new(RwLock::new(wallet.clone())),
            )]),
            secret_access: Some(backend.secret_access()),
            wallet_index: Some(3),
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        ctx.insert_local_qualified_identity(&qualified_identity, &Some((wallet_seed_hash, 3)))
            .expect("insert target identity");
        ctx.wallets()
            .write()
            .expect("write wallets")
            .insert(wallet_seed_hash, Arc::new(RwLock::new(wallet)));
        ctx.set_selected_identity(Some(target_id));
        ctx.set_pending_identity_selection(target_id);

        backend
            .dashpay_set_private_info(
                &target_id,
                &contact_id,
                &ContactPrivateInfo {
                    nickname: "target contact".into(),
                    notes: "target note".into(),
                    is_hidden: false,
                },
            )
            .expect("seed target owner overlay");
        let target_buf = target_id.to_buffer();
        let overlay_key = backend
            .kv()
            .list(
                crate::wallet_backend::DetScope::Identity(&target_buf),
                Some("det:dashpay:private:"),
            )
            .expect("list target overlays")
            .into_iter()
            .next()
            .expect("target overlay key");
        let persister_path = backend.spv_storage_dir().join("platform-wallet.sqlite");
        let fault_connection =
            rusqlite::Connection::open(&persister_path).expect("open persister second handle");
        fault_connection
            .execute_batch(&format!(
                "CREATE TRIGGER fail_unload_overlay_delete
                 BEFORE DELETE ON meta_identity
                 WHEN OLD.identity_id = X'{}' AND OLD.key = '{}'
                 BEGIN
                     SELECT RAISE(FAIL, 'injected owner overlay delete failure');
                 END;",
                hex::encode(target_buf),
                overlay_key.replace('\'', "''"),
            ))
            .expect("install owner-overlay delete trigger");

        let error = ctx
            .unload_identity(target_id)
            .expect_err("cleanup failure must still reach the caller");
        assert!(matches!(
            error,
            TaskError::IdentityUnloadCleanupFailed { identity_id, .. }
                if identity_id == target_id
        ));

        let target_is_cached = {
            let wallets = ctx.wallets().read().expect("read wallets");
            let wallet = wallets
                .get(&wallet_seed_hash)
                .expect("wallet remains")
                .read()
                .expect("read wallet");
            wallet
                .identities
                .values()
                .any(|identity| identity.id() == target_id)
        };
        assert!(
            !target_is_cached,
            "post-commit cleanup errors must still evict the wallet cache"
        );
        assert_eq!(ctx.selected_identity_id(), None);
        assert_eq!(ctx.take_pending_identity_selection(), None);

        fault_connection
            .execute_batch("DROP TRIGGER fail_unload_overlay_delete;")
            .expect("remove owner-overlay delete trigger");
        backend.shutdown().await;
    }
}
