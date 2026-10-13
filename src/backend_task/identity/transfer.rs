use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::transition::transfer::TransferToIdentity;
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
        mut qualified_identity: QualifiedIdentity,
        to_identifier: Identifier,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = self.sdk.load().as_ref().clone();
        let signing_key = transfer_signing_key(&qualified_identity, id)?;

        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = self.fee_estimator().estimate_credit_transfer();

        let (sender_balance, receiver_balance) = qualified_identity
            .identity
            .clone()
            .transfer_credits(
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

        qualified_identity.identity.set_balance(sender_balance);

        if let Some(receiver) = self
            .load_local_qualified_identities()?
            .iter_mut()
            .find(|qi| qi.identity.id() == to_identifier)
        {
            receiver.identity.set_balance(receiver_balance);
            self.update_local_qualified_identity(receiver)?;
        }

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        self.update_local_qualified_identity(&qualified_identity)
            .map(|_| BackendTaskSuccessResult::TransferredCredits(fee_result))
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
