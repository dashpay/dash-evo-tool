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
        let primary_cleanup_failed = cleanup_error.is_some();
        let mut associated_cleanup_failed = false;
        let mut associated_removal_failed = false;
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
                    associated_removal_failed = true;
                    tracing::warn!(
                        ?error,
                        voter_identity_id = %voter_id,
                        "Associated voter identity could not be removed"
                    );
                }
            }
        }

        Ok(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: removed_identity_ids,
            primary_cleanup_failed,
            associated_cleanup_failed,
            associated_removal_failed,
        })
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
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::IdentityPublicKey;
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};

    fn qualified_identity(
        identity: Identity,
        associated_voter_identity: Option<(Identity, IdentityPublicKey)>,
        secret_access: crate::wallet_backend::SecretAccess,
    ) -> QualifiedIdentity {
        QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: BTreeMap::new(),
            secret_access: Some(secret_access),
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    async fn removal_result_with_cleanup_failure(
        fail_associated_cleanup: bool,
    ) -> BackendTaskSuccessResult {
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
        let target_id = Identifier::from([0x81; 32]);
        let voter_id = Identifier::from([0x82; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let voter = Identity::create_basic_identity(voter_id, platform_version)
            .expect("create voter identity");
        let voter_key = IdentityPublicKey::random_key(1, Some(1), platform_version);
        let associated_voter_identity = fail_associated_cleanup.then(|| (voter.clone(), voter_key));
        let target = qualified_identity(target, associated_voter_identity, backend.secret_access());
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        if fail_associated_cleanup {
            let voter = qualified_identity(voter, None, backend.secret_access());
            ctx.insert_local_qualified_identity(&voter, &None)
                .expect("insert voter identity");
        }

        let fault_id = if fail_associated_cleanup {
            voter_id
        } else {
            target_id
        };
        let contact_id = Identifier::from([0x83; 32]);
        backend
            .dashpay_set_private_info(
                &fault_id,
                &contact_id,
                &ContactPrivateInfo {
                    nickname: "cleanup fault".into(),
                    notes: "cleanup fault".into(),
                    is_hidden: false,
                },
            )
            .expect("seed owner overlay");
        let fault_buf = fault_id.to_buffer();
        let overlay_key = backend
            .kv()
            .list(
                crate::wallet_backend::DetScope::Identity(&fault_buf),
                Some("det:dashpay:private:"),
            )
            .expect("list owner overlays")
            .into_iter()
            .next()
            .expect("owner overlay key");
        let persister_path = backend.spv_storage_dir().join("platform-wallet.sqlite");
        let fault_connection =
            rusqlite::Connection::open(&persister_path).expect("open persister second handle");
        fault_connection
            .execute_batch(&format!(
                "CREATE TRIGGER fail_remove_identity_overlay_delete
                 BEFORE DELETE ON meta_identity
                 WHEN OLD.identity_id = X'{}' AND OLD.key = '{}'
                 BEGIN
                     SELECT RAISE(FAIL, 'injected owner overlay delete failure');
                 END;",
                hex::encode(fault_buf),
                overlay_key.replace('\'', "''"),
            ))
            .expect("install owner-overlay delete trigger");

        let result = ctx
            .remove_identity(target_id)
            .expect("committed cleanup faults remain a successful removal result");

        fault_connection
            .execute_batch("DROP TRIGGER fail_remove_identity_overlay_delete;")
            .expect("remove owner-overlay delete trigger");
        backend.shutdown().await;
        result
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_identity_reports_primary_and_associated_cleanup_failures_independently() {
        let primary_failure = removal_result_with_cleanup_failure(false).await;
        assert!(matches!(
            primary_failure,
            BackendTaskSuccessResult::RemovedIdentities {
                primary_cleanup_failed: true,
                associated_cleanup_failed: false,
                associated_removal_failed: false,
                ..
            }
        ));

        let associated_failure = removal_result_with_cleanup_failure(true).await;
        assert!(matches!(
            associated_failure,
            BackendTaskSuccessResult::RemovedIdentities {
                primary_cleanup_failed: false,
                associated_cleanup_failed: true,
                associated_removal_failed: false,
                ..
            }
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_identity_distinguishes_associated_removal_failure_from_cleanup_residue() {
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
        let target_id = Identifier::from([0x84; 32]);
        let voter_id = Identifier::from([0x85; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let voter = Identity::create_basic_identity(voter_id, platform_version)
            .expect("create voter identity");
        let voter_key = IdentityPublicKey::random_key(1, Some(1), platform_version);
        let target = qualified_identity(
            target,
            Some((voter.clone(), voter_key)),
            backend.secret_access(),
        );
        let voter = qualified_identity(voter, None, backend.secret_access());
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        ctx.insert_local_qualified_identity(&voter, &None)
            .expect("insert voter identity");
        let voter_load_guard = ctx
            .begin_identity_load(voter_id, None)
            .expect("hold voter load claim");

        let result = ctx
            .remove_identity(target_id)
            .expect("primary removal remains successful");

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RemovedIdentities {
                identity_ids,
                primary_cleanup_failed: false,
                associated_cleanup_failed: false,
                associated_removal_failed: true,
            } if identity_ids == vec![target_id]
        ));
        assert!(
            ctx.get_local_qualified_identity(&voter_id)
                .expect("read voter identity")
                .is_some(),
            "a genuine associated removal failure must leave the voter present"
        );

        drop(voter_load_guard);
        backend.shutdown().await;
    }

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
            BackendTaskSuccessResult::RemovedIdentities {
                identity_ids,
                primary_cleanup_failed,
                associated_cleanup_failed,
                associated_removal_failed,
            } if identity_ids == &vec![target_id]
                && !primary_cleanup_failed
                && !associated_cleanup_failed
                && !associated_removal_failed
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
