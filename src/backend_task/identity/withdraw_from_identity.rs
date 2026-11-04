use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::identity_balance::BalanceProvenance;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::MasternodeWithdrawal;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::transition::withdraw_from_identity::WithdrawFromIdentityWithMetadata;
use dash_sdk::platform::{Fetch, Identity};

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn withdraw_from_identity(
        &self,
        mut qualified_identity: QualifiedIdentity,
        to_address: Option<Address>,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = self.sdk.load().as_ref().clone();

        tracing::info!(
            identity_id = %qualified_identity.identity.id().to_string(Encoding::Base58),
            local_revision = qualified_identity.identity.revision(),
            "Refreshing identity from Platform before withdrawal"
        );

        let refreshed_identity =
            Identity::fetch_by_identifier(&sdk, qualified_identity.identity.id())
                .await?
                .ok_or(TaskError::IdentityNotFound)?;

        tracing::info!(
            platform_revision = refreshed_identity.revision(),
            platform_balance = refreshed_identity.balance(),
            "Fetched identity from Platform"
        );

        qualified_identity.identity = refreshed_identity;

        tracing::info!(
            identity_id = %qualified_identity.identity.id().to_string(Encoding::Base58),
            to_address = ?to_address,
            credits = credits,
            key_id = ?id,
            identity_balance = qualified_identity.identity.balance(),
            identity_revision = qualified_identity.identity.revision(),
            "Starting withdrawal from identity"
        );

        // Resolve one active TRANSFER-or-OWNER key the local signer can use,
        // against the freshly fetched identity. Passing an explicit key to the
        // SDK below (rather than `None`) stops it from running its own
        // `TransferPreferred` selection, which would accept a disabled key or
        // fall back to an OWNER key and bypass the owner-address policy.
        let signing_key_id = qualified_identity
            .resolve_withdrawal_signing_key(id)
            .map_err(|_| TaskError::NoUsableWithdrawalKey)?;
        let signing_key = qualified_identity
            .identity
            .get_public_key_by_id(signing_key_id);
        if let Some(key) = &signing_key {
            tracing::info!(
                key_id = key.id(),
                key_purpose = ?key.purpose(),
                key_type = ?key.key_type(),
                key_security_level = ?key.security_level(),
                "Using signing key for withdrawal"
            );
        }

        // Platform rejects an output script when signing with an OWNER key,
        // routing the withdrawal to the registered payout address instead.
        let to_address = qualified_identity
            .resolve_withdrawal_output(
                signing_key.as_ref().map(|key| key.purpose()),
                to_address,
                self.network(),
            )
            .map_err(|_| TaskError::OwnerKeyWithdrawalNotAllowed)?;

        tracing::debug!(
            num_private_keys = qualified_identity.private_keys.len(),
            num_wallets = qualified_identity.associated_wallets.len(),
            "Qualified identity key info"
        );

        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = self.fee_estimator().estimate_credit_withdrawal();

        // A masternode key held by one of this device's wallets is signed by
        // that wallet's masternode withdrawal, which re-checks the key against
        // the node and enforces the payout-address rule itself. Keys entered
        // by hand keep the identity's own signer.
        let (remaining_balance, provenance) = match qualified_identity
            .wallet_masternode_withdrawal(signing_key_id)
        {
            Some(route) => {
                tracing::info!(role = ?route.role, "Withdrawing through the wallet that holds the masternode key");
                let remaining = self
                    .wallet_backend()?
                    .masternode_withdraw(
                        &route.wallet_seed_hash,
                        MasternodeWithdrawal {
                            identity_id: qualified_identity.identity.id(),
                            key_hashes: route.key_hashes,
                            role: route.role,
                            amount_credits: credits,
                            destination: to_address,
                            owner_key_index_hint: route.owner_key_index_hint,
                        },
                    )
                    .await?;
                // TODO(platform-wallet): `masternode_withdraw` returns only the
                // credits, without a height, so a late refresh can bring back
                // the balance from before this withdrawal (same gap as top-up).
                (remaining, BalanceProvenance::ConfirmedUnordered)
            }
            None => {
                let (remaining, metadata) = qualified_identity
                    .identity
                    .clone()
                    .withdraw_with_metadata(
                        &sdk,
                        to_address,
                        credits,
                        Some(1),
                        signing_key,
                        qualified_identity.clone(),
                        None,
                    )
                    .await?;
                (remaining, BalanceProvenance::ObservedAt(metadata.height))
            }
        };

        let actual_fee = balance_before
            .saturating_sub(remaining_balance)
            .saturating_sub(credits);
        tracing::info!(
            "Withdrawal complete: withdrew {} credits, estimated fee {} credits, actual fee {} credits",
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

        qualified_identity.identity.set_balance(remaining_balance);

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        self.persist_withdrawn_identity(&qualified_identity, provenance);
        Ok(BackendTaskSuccessResult::WithdrewFromIdentity(fee_result))
    }

    /// Store the identity as fetched for a finished withdrawal, carrying its
    /// remaining balance, into the record as stored now: a concurrent change
    /// such as a removed voting key stays removed. The withdrawal already
    /// happened, so a failed save is only logged; it never reads as a failed
    /// withdrawal that the user might repeat.
    fn persist_withdrawn_identity(
        &self,
        withdrawn: &QualifiedIdentity,
        provenance: BalanceProvenance,
    ) {
        let id = withdrawn.identity.id();
        if let Err(error) = self.edit_local_qualified_identity_observed(&id, provenance, |stored| {
            stored.identity = withdrawn.identity.clone();
            Ok(())
        }) {
            tracing::warn!(identity_id = %id, ?error, "Identity could not be saved locally after a withdrawal");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::context::test_staging::stage_identity_with_voter_twin;
    use crate::model::identity_balance::BalanceProvenance;
    use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};

    /// A withdrawal's network work can outlast the removal of an old voting
    /// key; its final write must not bring that key back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finished_withdrawal_keeps_a_change_made_while_it_ran() {
        let staged = stage_identity_with_voter_twin(rand::random(), rand::random()).await;
        let read = || {
            staged
                .ctx
                .get_local_qualified_identity(&staged.id)
                .unwrap()
                .unwrap()
        };
        let mut withdrawn = read();
        assert!(withdrawn.associated_voter_identity.is_some());
        let mut removed = withdrawn.clone();
        removed.associated_voter_identity = None;
        staged
            .ctx
            .update_local_qualified_identity(&removed)
            .unwrap();

        withdrawn.identity.set_balance(4_321);
        staged
            .ctx
            .persist_withdrawn_identity(&withdrawn, BalanceProvenance::ConfirmedUnordered);

        let after = read();
        assert!(
            after.associated_voter_identity.is_none(),
            "the withdrawal must not bring back what was removed while it ran"
        );
        assert_eq!(after.identity.balance(), 4_321);
    }
}
