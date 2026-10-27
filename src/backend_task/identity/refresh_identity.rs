use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::{Fetch, Identifier, Identity};
use dash_sdk::query_types::IdentityBalance;

use super::BackendTaskSuccessResult;

/// How long a finished paid task waits for its payer's new balance before
/// reporting success without it.
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
        let maybe_refreshed_identity =
            Identity::fetch_by_identifier(sdk, refreshed_identity_id).await?;

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
        match maybe_refreshed_identity {
            Some(refreshed_identity) => {
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
        self.update_local_qualified_identity(&qualified_identity_to_update)?;

        // Send refresh message to refresh the Identities Screen
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        Ok(BackendTaskSuccessResult::RefreshedIdentity(
            qualified_identity_to_update,
        ))
    }

    /// After a successful task paid by `payer`, store the payer's balance read
    /// back from Platform, so screens showing the identity reflect the fee.
    ///
    /// Best effort: the task is already paid, so a failed read or save is only
    /// logged and `result` is returned unchanged.
    pub(crate) async fn refresh_fee_payer_balance(
        &self,
        sdk: &Sdk,
        payer: Option<Identifier>,
        result: Result<BackendTaskSuccessResult, TaskError>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if let (Ok(_), Some(identity_id)) = (&result, payer) {
            self.store_identity_balance_from_platform(sdk, identity_id)
                .await;
        }
        result
    }

    async fn store_identity_balance_from_platform(&self, sdk: &Sdk, identity_id: Identifier) {
        let read = tokio::time::timeout(
            FEE_PAYER_BALANCE_READ_TIMEOUT,
            IdentityBalance::fetch(sdk, identity_id),
        )
        .await;
        let balance = match read {
            Ok(Ok(Some(balance))) => balance,
            Ok(Ok(None)) => {
                tracing::debug!(%identity_id, "Paying identity not found on Platform; its stored balance is kept");
                return;
            }
            Ok(Err(error)) => {
                tracing::warn!(%identity_id, ?error, "Balance after a paid task could not be read; it updates on the next identity refresh");
                return;
            }
            Err(_) => {
                tracing::warn!(%identity_id, "Balance after a paid task took too long to read; it updates on the next identity refresh");
                return;
            }
        };
        if let Err(error) = self.edit_local_qualified_identity(&identity_id, |stored| {
            stored.identity.set_balance(balance);
            Ok(())
        }) {
            tracing::warn!(%identity_id, ?error, "Balance after a paid task could not be saved locally");
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
    async fn a_paid_task_stores_the_payers_balance_read_after_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 43_600_388_314);
        let sdk = sdk_reporting_balance(id, 3_400_000_000).await;

        let result = ctx
            .refresh_fee_payer_balance(&sdk, Some(id), Ok(BackendTaskSuccessResult::None))
            .await;

        assert!(matches!(result, Ok(BackendTaskSuccessResult::None)));
        assert_eq!(stored_balance(&ctx, &id), 3_400_000_000);
    }

    #[tokio::test]
    async fn a_failed_task_leaves_the_stored_balance_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 1_000);
        let sdk = sdk_reporting_balance(id, 600).await;

        let result = ctx
            .refresh_fee_payer_balance(&sdk, Some(id), Err(TaskError::IdentityNotFoundLocally))
            .await;

        assert!(matches!(result, Err(TaskError::IdentityNotFoundLocally)));
        assert_eq!(stored_balance(&ctx, &id), 1_000);
    }

    /// The task already succeeded and was paid for: an unreadable balance
    /// must not turn it into a failure.
    #[tokio::test]
    async fn an_unreadable_balance_keeps_the_success_and_the_stored_balance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ctx, id) = context_with_payer(dir.path(), 1_000);

        let result = ctx
            .refresh_fee_payer_balance(
                &Sdk::new_mock(),
                Some(id),
                Ok(BackendTaskSuccessResult::None),
            )
            .await;

        assert!(matches!(result, Ok(BackendTaskSuccessResult::None)));
        assert_eq!(stored_balance(&ctx, &id), 1_000);
    }
}
