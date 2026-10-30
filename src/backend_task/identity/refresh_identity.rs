use crate::backend_task::error::TaskError;
use crate::backend_task::{CacheDomain, CacheUpdate, TaskResult};
use crate::context::AppContext;
use crate::model::identity_balance::BalanceProvenance;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::dapi_grpc::platform::v0::ResponseMetadata;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::{Fetch, Identifier, Identity};
use dash_sdk::query_types::IdentityBalance;
use dash_sdk::{Error as SdkError, Sdk};
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

        // Get local identities
        let mut local_qualified_identities = self.load_local_qualified_identities()?;

        // Find the local identity to update
        let outdated_identity_index = local_qualified_identities
            .iter()
            .position(|qi| qi.identity.id() == refreshed_identity_id)
            .ok_or(TaskError::IdentityNotFoundLocally)?;

        // Remove the outdated identity from local state
        let mut qualified_identity_to_update =
            local_qualified_identities.remove(outdated_identity_index);

        // Update the identity
        let mut provenance = BalanceProvenance::Unobserved;
        match maybe_refreshed_identity {
            Some(refreshed_identity) => {
                provenance = BalanceProvenance::ObservedAt(metadata.height);
                qualified_identity_to_update.identity = refreshed_identity;
                qualified_identity_to_update
                    .status
                    .update(IdentityStatus::Active);
            }
            None => {
                qualified_identity_to_update
                    .status
                    .update(IdentityStatus::NotFound);
            }
        }

        // Insert the updated identity into local state
        self.update_local_qualified_identity_observed(
            &mut qualified_identity_to_update,
            provenance,
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
        let provenance = BalanceProvenance::ObservedAt(height);
        match self.edit_local_qualified_identity_observed(&identity_id, provenance, |stored| {
            stored.identity.set_balance(balance);
            Ok(())
        }) {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%identity_id, ?error, "Balance after a paid task could not be saved locally");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;

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
