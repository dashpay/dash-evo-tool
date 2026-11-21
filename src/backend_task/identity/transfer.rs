use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::identity_balance::BalanceProvenance;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::transition::transfer::TransferToIdentityWithMetadata;
use dash_sdk::platform::{Identifier, IdentityPublicKey};

use super::BackendTaskSuccessResult;

/// Resolves the key that signs an identity credit transfer, before any network
/// call. Handing the SDK an explicit key stops it from taking the identity's
/// first `TRANSFER` key itself, which may be disabled or not held in this app.
pub(super) fn transfer_signing_key(
    identity: &QualifiedIdentity,
    requested: Option<KeyID>,
) -> Result<IdentityPublicKey, TaskError> {
    let key_id = identity
        .resolve_transfer_signing_key(requested)
        .map_err(|_| TaskError::NoUsableTransferKey)?;
    identity
        .identity
        .get_public_key_by_id(key_id)
        .cloned()
        .ok_or(TaskError::NoUsableTransferKey)
}

impl AppContext {
    pub(super) async fn transfer_to_identity(
        &self,
        qualified_identity: QualifiedIdentity,
        to_identifier: Identifier,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = self.sdk.load().as_ref().clone();
        let signing_key = transfer_signing_key(&qualified_identity, id)?;

        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = self.fee_estimator().estimate_credit_transfer();

        let ((sender_balance, receiver_balance), metadata) = qualified_identity
            .identity
            .clone()
            .transfer_credits_with_metadata(
                &sdk,
                to_identifier,
                credits,
                Some(&signing_key),
                qualified_identity.clone(),
                None,
            )
            .await?;

        let actual_fee = balance_before
            .saturating_sub(sender_balance)
            .saturating_sub(credits);
        tracing::info!(
            "Credit transfer complete: sent {} credits, estimated fee {} credits, actual fee {} credits",
            credits,
            estimated_fee,
            actual_fee
        );
        if actual_fee != estimated_fee {
            tracing::warn!(
                "Fee mismatch: estimated {} vs actual {} (diff: {})",
                estimated_fee,
                actual_fee,
                actual_fee as i64 - estimated_fee as i64
            );
        }

        self.store_transfer_balances(
            (qualified_identity.identity.id(), sender_balance),
            (to_identifier, receiver_balance),
            BalanceProvenance::ObservedAt(metadata.height),
        );

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::TransferredCredits(fee_result))
    }

    /// Store the `(identity, balance)` pairs a finished transfer reports for
    /// its sender and receiver, each only when that identity is loaded here.
    ///
    /// TODO(platform-wallet): `IdentityWallet::transfer_credits_with_signer`
    /// stores both via `persist_confirmed_balance`; route wallet-owned
    /// identities through it once `IdentityManager` holds the identity record.
    fn store_transfer_balances(
        &self,
        (sender_id, sender_balance): (Identifier, Credits),
        (receiver_id, receiver_balance): (Identifier, Credits),
        observed: BalanceProvenance,
    ) {
        self.store_observed_balance(&receiver_id, receiver_balance, observed);
        self.store_observed_balance(&sender_id, sender_balance, observed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::IdentityType;
    use crate::model::qualified_identity::withdrawal_key_tests::{
        build_identity, disabled_key, key,
    };
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

    /// The transfer stores only the two balances: an alias set on either
    /// identity while it ran is kept.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_transfer_stores_both_balances_without_overwriting() {
        let staged = crate::context::test_staging::stage_identity_with_vaulted_keys(
            rand::random(),
            rand::random(),
        )
        .await;
        let ctx = &staged.ctx;
        let receiver_id = Identifier::from([0x5B; 32]);
        ctx.update_local_qualified_identity(&crate::context::test_support::bare_user_identity(
            receiver_id,
            dash_sdk::dpp::dashcore::Network::Testnet,
        ))
        .unwrap();
        ctx.set_identity_alias(&staged.id, Some("sender alias"))
            .unwrap();
        ctx.set_identity_alias(&receiver_id, Some("receiver alias"))
            .unwrap();

        ctx.store_transfer_balances(
            (staged.id, 700),
            (receiver_id, 1_300),
            BalanceProvenance::ObservedAt(7),
        );

        let read = |id| ctx.get_local_qualified_identity(id).unwrap().unwrap();
        let (sender_after, receiver_after) = (read(&staged.id), read(&receiver_id));
        assert_eq!(sender_after.alias.as_deref(), Some("sender alias"));
        assert_eq!(sender_after.identity.balance(), 700);
        assert_eq!(receiver_after.alias.as_deref(), Some("receiver alias"));
        assert_eq!(receiver_after.identity.balance(), 1_300);
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[test]
    fn transfer_signs_with_the_active_key_held_in_this_app() {
        let disabled = disabled_key(0, Purpose::TRANSFER);
        let not_imported = key(1, Purpose::TRANSFER);
        let usable = key(2, Purpose::TRANSFER);
        let identity = build_identity(
            IdentityType::User,
            vec![disabled.clone(), not_imported, usable.clone()],
            vec![disabled, usable],
        );
        let signing_key = transfer_signing_key(&identity, None).expect("a usable key");
        assert_eq!(signing_key.id(), 2);
        assert!(!signing_key.is_disabled());
    }

    /// Both transfer tasks, dispatched the way the app dispatches them, must
    /// refuse before they reach the SDK. The mock SDK answers nothing, so a
    /// task that got that far would fail with a different error.
    #[tokio::test]
    async fn transfer_tasks_refuse_to_start_without_a_usable_key() {
        use crate::backend_task::identity::IdentityTask;
        use dash_sdk::dpp::address_funds::PlatformAddress;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let sdk = dash_sdk::Sdk::new_mock();
        // A transfer between identities uses the SDK the context holds.
        ctx.sdk.store(std::sync::Arc::new(sdk.clone()));
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(sender, ctx.egui_ctx().clone());

        let not_held = build_identity(IdentityType::User, vec![key(1, Purpose::TRANSFER)], vec![]);
        let disabled = disabled_key(1, Purpose::TRANSFER);
        let held_but_disabled =
            build_identity(IdentityType::User, vec![disabled.clone()], vec![disabled]);
        for (identity, requested) in [
            (not_held.clone(), None),
            (not_held, Some(1)),
            (held_but_disabled, Some(1)),
        ] {
            let to_identity = IdentityTask::Transfer(
                identity.clone(),
                Identifier::from([0x4E; 32]),
                1_000,
                requested,
            );
            let to_address = IdentityTask::TransferToAddresses {
                identity,
                outputs: [(PlatformAddress::P2pkh([0x4E; 20]), 1_000)].into(),
                key_id: requested,
            };
            for task in [to_identity, to_address] {
                let outcome = ctx.run_identity_task(task, &sdk, sender.clone()).await;
                assert!(
                    matches!(outcome, Err(TaskError::NoUsableTransferKey)),
                    "requested key {requested:?}: {outcome:?}"
                );
            }
        }
    }

    #[test]
    fn transfer_without_a_usable_key_fails_with_its_own_error() {
        let identity = build_identity(IdentityType::User, vec![key(1, Purpose::TRANSFER)], vec![]);
        for requested in [None, Some(1)] {
            assert!(matches!(
                transfer_signing_key(&identity, requested),
                Err(TaskError::NoUsableTransferKey)
            ));
        }
    }
}
