use crate::backend_task::error::TaskError;
use crate::backend_task::{CacheDomain, CacheUpdate, TaskResult};
use crate::context::AppContext;
use crate::model::identity_balance::BalanceProvenance;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::dapi_grpc::platform::v0::ResponseMetadata;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{Fetch, Identifier, Identity};
use dash_sdk::query_types::IdentityBalance;
use dash_sdk::{Error as SdkError, Sdk};
use platform_wallet::{BlockTime, ManagedIdentity};
use std::sync::Arc;

use super::BackendTaskSuccessResult;

/// How long the background read of a paying identity's new balance may take
/// before it gives up, leaving the balance to the next identity refresh.
const FEE_PAYER_BALANCE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl AppContext {
    pub(super) async fn refresh_identity(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let refreshed_identity_id = qualified_identity.identity.id();
        // Fetch the latest state of the identity from Platform
        let (maybe_refreshed_identity, metadata) =
            Identity::fetch_with_metadata(sdk, refreshed_identity_id, None).await?;

        let provenance = match maybe_refreshed_identity {
            Some(_) => BalanceProvenance::ObservedAt(metadata.height),
            None => BalanceProvenance::Unobserved,
        };
        // Apply only what was fetched, to the record as stored now: a stale
        // snapshot written back would undo whatever changed while the fetch
        // ran, such as a removed key whose vault secret is already gone.
        // Upstream's fold keeps raised key limits and, below the stored
        // revision, the stored keys plus any permanent disable; DET's watermark
        // still orders the balance.
        // TODO(platform-wallet): a fetch at the stored revision still takes
        // reversible key states whole, so a lagging proof can undo one
        // (dashpay/platform#5542 item 6).
        let qualified_identity_to_update = self.edit_local_qualified_identity_observed(
            &refreshed_identity_id,
            provenance,
            |stored| {
                match maybe_refreshed_identity {
                    Some(fetched) => {
                        let mut managed =
                            ManagedIdentity::new_out_of_wallet(stored.identity.clone());
                        managed.take_fetched_identity(
                            &fetched,
                            BlockTime::new(
                                metadata.height,
                                metadata.core_chain_locked_height,
                                metadata.time_ms,
                            ),
                        );
                        stored.identity = managed.identity;
                        stored.status.update(IdentityStatus::Active);
                    }
                    None => stored.status.update(IdentityStatus::NotFound),
                }
                Ok(())
            },
        )?;

        self.notify_cache_changed(crate::backend_task::CacheDomain::Identities, &sender)
            .await;

        Ok(BackendTaskSuccessResult::RefreshedIdentity(
            qualified_identity_to_update,
        ))
    }

    /// After a successful task paid by `payer`, read the payer's balance back
    /// from Platform and store it, so screens showing the identity reflect the
    /// fee, then announce the identity cache change.
    ///
    /// Runs as a tracked subtask, never awaited by the task: call it once the
    /// task's result is final and its guards are released. Shutdown cancels it.
    /// Best effort: the task is already paid, so a failed read or save is only
    /// logged.
    pub(crate) fn spawn_fee_payer_balance_refresh(
        self: &Arc<Self>,
        sdk: Arc<Sdk>,
        payer: Identifier,
        sender: SenderAsync<TaskResult>,
    ) {
        let context = Arc::clone(self);
        let shutdown = self.subtasks.cancellation_token.clone();
        let refresh = async move {
            #[cfg(test)]
            drop(context.test_fee_payer_refresh_gate.lock().await);
            let stored = tokio::select! {
                biased;
                () = shutdown.cancelled() => return,
                stored = context.store_identity_balance_from_platform(&sdk, payer) => stored,
            };
            if stored
                && sender
                    .send(TaskResult::CacheUpdated(CacheUpdate {
                        network: context.network,
                        domain: CacheDomain::Identities,
                    }))
                    .await
                    .is_err()
            {
                tracing::debug!(%payer, "Paying identity's new balance stored; no screen is listening for it");
            }
        };
        if self
            .subtasks
            .spawn_sync("fee_payer_balance_refresh", refresh)
            .is_err()
        {
            tracing::debug!(%payer, "Shutting down; the paying identity's balance is not read back");
        }
    }

    /// Store `identity_id`'s balance read from Platform; whether it was stored.
    async fn store_identity_balance_from_platform(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
    ) -> bool {
        self.store_identity_balance_read(
            identity_id,
            IdentityBalance::fetch_with_metadata(sdk, identity_id, None),
        )
        .await
    }

    /// [`Self::store_identity_balance_from_platform`] with the read passed in
    /// as `read`, so what it does with a response's height runs without a
    /// network.
    async fn store_identity_balance_read(
        &self,
        identity_id: Identifier,
        read: impl Future<Output = Result<(Option<IdentityBalance>, ResponseMetadata), SdkError>>,
    ) -> bool {
        let read = tokio::time::timeout(FEE_PAYER_BALANCE_READ_TIMEOUT, read).await;
        let (balance, height) = match read {
            Ok(Ok((Some(balance), metadata))) => (balance, metadata.height),
            Ok(Ok((None, _))) => {
                tracing::debug!(%identity_id, "Paying identity not found on Platform; its stored balance is kept");
                return false;
            }
            Ok(Err(error)) => {
                tracing::warn!(%identity_id, ?error, "Balance after a paid task could not be read; it updates on the next identity refresh");
                return false;
            }
            Err(_) => {
                tracing::warn!(%identity_id, "Balance after a paid task took too long to read; it updates on the next identity refresh");
                return false;
            }
        };
        self.store_observed_balance(&identity_id, balance, BalanceProvenance::ObservedAt(height))
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::accessors::IdentitySettersV0;

    fn context_with_payer(
        dir: &std::path::Path,
        balance: u64,
    ) -> (std::sync::Arc<AppContext>, Identifier) {
        let ctx = crate::context::test_support::test_app_context(dir);
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let id = Identifier::from([0x5A; 32]);
        let mut payer = crate::context::test_support::bare_user_identity(id, Network::Testnet);
        payer.identity.set_balance(balance);
        ctx.update_local_qualified_identity(&payer)
            .expect("store payer");
        (ctx, id)
    }

    fn stored_balance(ctx: &AppContext, id: &Identifier) -> u64 {
        ctx.get_local_qualified_identity(id)
            .expect("read payer")
            .expect("payer stored")
            .identity
            .balance()
    }

    async fn sdk_reporting_balance(id: Identifier, balance: u64) -> Sdk {
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch::<IdentityBalance, _>(id, Some(balance))
            .await
            .expect("balance expectation");
        sdk
    }

    /// Regression: a paid operation (token registration, claim, profile
    /// save) left the stored balance at its pre-fee value until restart.
    #[tokio::test]
    async fn the_payers_balance_read_after_a_paid_task_is_stored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 43_600_388_314);
        let sdk = sdk_reporting_balance(id, 3_400_000_000).await;

        assert!(ctx.store_identity_balance_from_platform(&sdk, id).await);
        assert_eq!(stored_balance(&ctx, &id), 3_400_000_000);
    }

    /// Store `balance` for `id` as observed at `height`.
    fn observe_balance(ctx: &AppContext, id: &Identifier, balance: u64, height: u64) {
        ctx.edit_local_qualified_identity_observed(
            id,
            BalanceProvenance::ObservedAt(height),
            |stored| {
                stored.identity.set_balance(balance);
                Ok(())
            },
        )
        .expect("store the observed balance");
    }

    /// The metadata of a response made at `height`.
    fn response_at(height: u64) -> ResponseMetadata {
        ResponseMetadata {
            height,
            ..Default::default()
        }
    }

    /// A read answered by a node behind the block of a balance already stored
    /// is older than that balance, so it must not replace it.
    #[tokio::test]
    async fn a_read_from_an_earlier_block_keeps_the_later_balance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 1_000);
        observe_balance(&ctx, &id, 700, 100);

        ctx.store_identity_balance_read(id, async { Ok((Some(900), response_at(99))) })
            .await;

        assert_eq!(stored_balance(&ctx, &id), 700);
    }

    /// Regression: with a height on record, a read stored without its own
    /// height was pinned to the old balance, so the fee never showed.
    #[tokio::test]
    async fn a_read_from_a_later_block_updates_a_balance_with_a_height() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 1_000);
        observe_balance(&ctx, &id, 700, 100);

        assert!(
            ctx.store_identity_balance_read(id, async { Ok((Some(600), response_at(101))) })
                .await
        );

        assert_eq!(stored_balance(&ctx, &id), 600);
    }

    /// The task already succeeded and was paid for: an unreadable balance
    /// leaves the stored one and announces nothing.
    #[tokio::test]
    async fn an_unreadable_balance_keeps_the_stored_balance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 1_000);

        assert!(
            !ctx.store_identity_balance_from_platform(&Sdk::new_mock(), id)
                .await
        );
        assert_eq!(stored_balance(&ctx, &id), 1_000);
    }

    /// Shutdown cancels a pending balance read: nothing is stored or announced.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_cancels_a_pending_balance_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 1_000);
        let sdk = Arc::new(sdk_reporting_balance(id, 600).await);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TaskResult>(4);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());

        ctx.subtasks.cancellation_token.cancel();
        ctx.spawn_fee_payer_balance_refresh(sdk, id, sender);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while ctx.subtasks.is_active("fee_payer_balance_refresh") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled read ends");

        assert_eq!(stored_balance(&ctx, &id), 1_000);
        assert!(rx.try_recv().is_err(), "a cancelled read announces nothing");
    }
}

#[cfg(test)]
mod concurrent_refresh_tests {
    use super::*;
    use crate::context::lock_probe::{LockProbe, LockSite};
    use crate::context::test_staging::{stage_identity_with_vaulted_keys, unlist_identity};
    use crate::model::qualified_identity::PrivateKeyTarget;
    use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
    use std::thread::JoinHandle;

    const REACH_LOCK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

    /// A mock SDK whose identity fetch returns `identity`, answered at `height`.
    async fn sdk_returning(identity: &Identity, height: u64) -> Sdk {
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .set_response_height(height)
            .expect_fetch(identity.id(), Some(identity.clone()))
            .await
            .expect("identity expectation");
        sdk
    }

    /// A task sender nobody listens to; a lost cache notice never fails a task.
    fn sender_for(ctx: &AppContext) -> SenderAsync<TaskResult> {
        let (tx, _) = tokio::sync::mpsc::channel::<TaskResult>(1);
        SenderAsync::new(tx, ctx.egui_ctx().clone())
    }

    /// Run `refresh_identity` on its own thread, attached to `probe`.
    fn spawn_refresh(
        probe: &LockProbe,
        ctx: &Arc<AppContext>,
        sdk: Sdk,
        snapshot: QualifiedIdentity,
    ) -> JoinHandle<Result<BackendTaskSuccessResult, TaskError>> {
        let probe = probe.clone();
        let ctx = Arc::clone(ctx);
        std::thread::spawn(move || {
            let _attached = probe.attach();
            let sender = sender_for(&ctx);
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("refresh runtime")
                .block_on(ctx.refresh_identity(&sdk, snapshot, sender))
        })
    }

    /// A refresh must not write back the copy it read before another writer
    /// changed the record: a key removed in Key Info, or a new alias, stays.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_keeps_a_change_made_while_it_was_running() {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let read = || {
            staged
                .ctx
                .get_local_qualified_identity(&staged.id)
                .unwrap()
                .unwrap()
        };
        let before = read();
        let mut fetched = before.identity.clone();
        fetched.set_balance(9_999);
        let sdk = sdk_returning(&fetched, 10).await;

        let lock = staged.ctx.identity_record_lock(staged.id);
        let guard = lock.lock().unwrap();
        let probe = LockProbe::new();
        let refresh = spawn_refresh(&probe, &staged.ctx, sdk, before.clone());
        let refresh = probe.wait_for(LockSite::RecordRequest, refresh, REACH_LOCK_DEADLINE);
        let removed_key = (PrivateKeyTarget::PrivateKeyOnMainIdentity, 1);
        let mut changed = before.clone();
        changed.private_keys.remove_at(&removed_key);
        changed.alias = Some("renamed".to_string());
        staged
            .ctx
            .write_local_qualified_identity_locked(&changed)
            .unwrap();
        drop(guard);
        refresh.join().unwrap().expect("the refresh succeeds");

        let after = read();
        assert!(
            !after.private_keys.has(&removed_key),
            "the refresh must not bring back a key removed while it ran"
        );
        assert_eq!(after.alias.as_deref(), Some("renamed"));
        assert_eq!(after.status, IdentityStatus::Active);
        assert_eq!(after.identity.balance(), 9_999);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// An identity off the roster with no unload marker (an insert that
    /// stopped before its roster add) is not re-listed by a refresh that was
    /// waiting for its record: the refresh reports it as not loaded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_does_not_relist_an_unloaded_identity() {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let before = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let sdk = sdk_returning(&before.identity, 10).await;

        let lock = staged.ctx.identity_record_lock(staged.id);
        let guard = lock.lock().unwrap();
        let probe = LockProbe::new();
        let refresh = spawn_refresh(&probe, &staged.ctx, sdk, before);
        let refresh = probe.wait_for(LockSite::RecordRequest, refresh, REACH_LOCK_DEADLINE);
        unlist_identity(&staged.ctx, &staged.id);
        drop(guard);
        let result = refresh.join().unwrap();

        assert!(
            matches!(result, Err(TaskError::IdentityNotFoundLocally)),
            "{result:?}"
        );
        assert!(!staged.ctx.is_identity_listed(&staged.id).unwrap());
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// A key added while the refresh waited, with its revision bump, is kept
    /// when the fetch answered from before that add; the balance it read is
    /// still taken.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_keeps_a_key_added_while_it_was_running() {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let read = || {
            staged
                .ctx
                .get_local_qualified_identity(&staged.id)
                .unwrap()
                .unwrap()
        };
        let before = read();
        let mut fetched = before.identity.clone();
        fetched.set_balance(9_999);
        let sdk = sdk_returning(&fetched, 10).await;

        let lock = staged.ctx.identity_record_lock(staged.id);
        let guard = lock.lock().unwrap();
        let probe = LockProbe::new();
        let refresh = spawn_refresh(&probe, &staged.ctx, sdk, before.clone());
        let refresh = probe.wait_for(LockSite::RecordRequest, refresh, REACH_LOCK_DEADLINE);
        let mut added = before.clone();
        added.identity.add_public_key(random_key(9));
        added.identity.bump_revision();
        staged
            .ctx
            .write_local_qualified_identity_locked(&added)
            .unwrap();
        drop(guard);
        refresh.join().unwrap().expect("the refresh succeeds");

        let after = read();
        assert_eq!(after.identity.public_keys(), added.identity.public_keys());
        assert_eq!(after.identity.revision(), added.identity.revision());
        assert_eq!(after.identity.balance(), 9_999);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    fn random_key(id: dash_sdk::dpp::identity::KeyID) -> dash_sdk::platform::IdentityPublicKey {
        dash_sdk::platform::IdentityPublicKey::random_key(
            id,
            Some(u64::from(id)),
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
    }

    /// Two refreshes at one revision can finish out of order: the later one
    /// to land, answered before a key's limit was raised, keeps the raise.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_older_same_revision_refresh_keeps_a_raised_key_limit() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;

        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let ctx = &staged.ctx;
        let mut stored = ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let mut fetched = stored.identity.clone();
        fetched.add_public_key(random_key(9).with_limits(Some(1_000), None));
        stored
            .identity
            .add_public_key(random_key(9).with_limits(Some(5_000), None));
        ctx.update_local_qualified_identity(&stored)
            .expect("the newer refresh is stored");
        let sdk = sdk_returning(&fetched, 10).await;

        ctx.refresh_identity(&sdk, stored, sender_for(ctx))
            .await
            .expect("the refresh succeeds");

        let after = ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let key = after.identity.get_public_key_by_id(9).expect("key 9 kept");
        assert_eq!(key.total_budget(), Some(5_000));
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// A refresh answered at an earlier block than a paid task's balance
    /// keeps that newer balance.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_newer_paid_balance_is_not_replaced_by_an_older_refresh() {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let ctx = &staged.ctx;
        let paid = ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let mut fetched = paid.identity.clone();
        fetched.set_balance(100);
        let sdk = sdk_returning(&fetched, 10).await;

        ctx.store_observed_balance(&staged.id, 500, BalanceProvenance::ObservedAt(11))
            .expect("the paid balance is stored");
        ctx.refresh_identity(&sdk, paid, sender_for(ctx))
            .await
            .expect("the refresh succeeds");

        let after = ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.identity.balance(), 500);
        ctx.wallet_backend().unwrap().shutdown().await;
    }
}
