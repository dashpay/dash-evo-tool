use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::transition::withdraw_from_identity::WithdrawFromIdentity;
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

        let remaining_balance = qualified_identity
            .identity
            .clone()
            .withdraw(
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

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        self.update_local_qualified_identity(&qualified_identity)
            .map(|_| BackendTaskSuccessResult::WithdrewFromIdentity(fee_result))
    }
}
