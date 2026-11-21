use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::identity_balance::BalanceProvenance;
use crate::model::qualified_identity::QualifiedIdentity;
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

        let (remaining_balance, metadata) = qualified_identity
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

        self.store_paid_balance(
            &qualified_identity,
            BalanceProvenance::ObservedAt(metadata.height),
        );

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::WithdrewFromIdentity(fee_result))
    }
}

#[cfg(test)]
mod tests {
    use crate::context::test_staging::stage_identity_with_vaulted_keys;
    use crate::model::identity_balance::BalanceProvenance;
    use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::IdentityPublicKey;

    /// The withdrawal stores its remaining balance into the record as stored
    /// now: public keys and a revision written while it ran are kept, not
    /// replaced by the ones it fetched before the network call.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn withdrawal_does_not_restore_stale_public_keys_or_revision() {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let read = || {
            staged
                .ctx
                .get_local_qualified_identity(&staged.id)
                .unwrap()
                .unwrap()
        };
        let mut withdrawn = read();
        let mut changed = withdrawn.clone();
        let added = IdentityPublicKey::random_key(9, Some(9), PlatformVersion::latest());
        changed.identity.add_public_key(added.clone());
        changed.identity.bump_revision();
        staged
            .ctx
            .update_local_qualified_identity(&changed)
            .unwrap();

        withdrawn.identity.set_balance(4_321);
        staged
            .ctx
            .store_paid_balance(&withdrawn, BalanceProvenance::ObservedAt(7));

        let after = read();
        assert_eq!(
            after.identity.public_keys(),
            changed.identity.public_keys(),
            "the withdrawal must not restore the public keys it read before"
        );
        assert_eq!(after.identity.revision(), changed.identity.revision());
        assert_eq!(after.identity.balance(), 4_321);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }
}
