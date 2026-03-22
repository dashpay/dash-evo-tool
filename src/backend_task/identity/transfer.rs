use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::Identifier;
use dash_sdk::platform::transition::transfer::TransferToIdentity;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn transfer_to_identity(
        &self,
        mut qualified_identity: QualifiedIdentity,
        to_identifier: Identifier,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = self.sdk.load().as_ref().clone();

        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_credit_transfer();

        let (sender_balance, receiver_balance) = qualified_identity
            .identity
            .clone()
            .transfer_credits(
                &sdk,
                to_identifier,
                credits,
                id.and_then(|key_id| qualified_identity.identity.get_public_key_by_id(key_id)),
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

        qualified_identity.identity.set_balance(sender_balance);

        if let Some(receiver) = self
            .load_local_qualified_identities()?
            .iter_mut()
            .find(|qi| qi.identity.id() == to_identifier)
        {
            receiver.identity.set_balance(receiver_balance);
            self.update_local_qualified_identity(receiver)
                .map_err(|e| TaskError::Database { source: e })?;
        }

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        self.update_local_qualified_identity(&qualified_identity)
            .map(|_| BackendTaskSuccessResult::TransferredCredits(fee_result))
            .map_err(|e| TaskError::Database { source: e })
    }
}
