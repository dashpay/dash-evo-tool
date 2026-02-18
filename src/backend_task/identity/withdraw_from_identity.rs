use crate::backend_task::FeeResult;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
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
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk_guard = self.sdk.load().as_ref().clone();

        // First, refresh the identity from Platform to get the latest revision and balance
        tracing::info!(
            identity_id = %qualified_identity.identity.id().to_string(Encoding::Base58),
            local_revision = qualified_identity.identity.revision(),
            "Refreshing identity from Platform before withdrawal"
        );

        let refreshed_identity =
            Identity::fetch_by_identifier(&sdk_guard, qualified_identity.identity.id())
                .await
                .map_err(|e| format!("Failed to fetch identity from Platform: {}", e))?
                .ok_or_else(|| "Identity not found on Platform".to_string())?;

        tracing::info!(
            platform_revision = refreshed_identity.revision(),
            platform_balance = refreshed_identity.balance(),
            "Fetched identity from Platform"
        );

        // Update the qualified identity with the refreshed identity data
        qualified_identity.identity = refreshed_identity;

        // Log withdrawal attempt details
        tracing::info!(
            identity_id = %qualified_identity.identity.id().to_string(Encoding::Base58),
            to_address = ?to_address,
            credits = credits,
            key_id = ?id,
            identity_balance = qualified_identity.identity.balance(),
            identity_revision = qualified_identity.identity.revision(),
            "Starting withdrawal from identity"
        );

        // Log the key being used
        let signing_key =
            id.and_then(|key_id| qualified_identity.identity.get_public_key_by_id(key_id));
        if let Some(key) = &signing_key {
            tracing::info!(
                key_id = key.id(),
                key_purpose = ?key.purpose(),
                key_type = ?key.key_type(),
                key_security_level = ?key.security_level(),
                "Using signing key for withdrawal"
            );
        } else {
            tracing::warn!("No signing key specified for withdrawal");
        }

        // Log available private keys in the qualified identity
        tracing::debug!(
            num_private_keys = qualified_identity.private_keys.private_keys.len(),
            num_wallets = qualified_identity.associated_wallets.len(),
            "Qualified identity key info"
        );

        // Track balance before withdrawal for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_credit_withdrawal();

        let remaining_balance = qualified_identity
            .identity
            .clone()
            .withdraw(
                &sdk_guard,
                to_address,
                credits,
                Some(1),
                signing_key,
                qualified_identity.clone(),
                None,
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Withdrawal failed");
                format!("Withdrawal error: {}", e)
            })?;

        // Calculate and log actual fee paid
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
            .map_err(|e| format!("Database error: {}", e))
    }
}
