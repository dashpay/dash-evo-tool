use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::state_transition::identity_credit_transfer_transition::IdentityCreditTransferTransition;
use dash_sdk::platform::Identifier;
use dash_sdk::platform::transition::withdraw_from_identity::WithdrawFromIdentity;

impl AppContext {
    pub(super) async fn transfer_to_identity(
        &self,
        mut qualified_identity: QualifiedIdentity,
        to_identifier: Identifier,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<(), String> {
        let state_transition = IdentityCreditTransferTransition::try(
            self,
            script,
            amount,
            Pooling::Never,
            core_fee_per_byte.unwrap_or(1),
            user_fee_increase.unwrap_or_default(),
            signer,
            signing_withdrawal_key_to_use,
            PreferredKeyPurposeForSigningWithdrawal::TransferPreferred,
            new_identity_nonce,
            sdk.version(),
            None,
        )?;

        let result = state_transition.broadcast_and_wait(sdk, None).await?;

        let remaining_balance = qualified_identity
            .identity
            .clone()
            .withdraw(
                &self.sdk,
                to_address,
                credits,
                Some(1),
                id.and_then(|key_id| qualified_identity.identity.get_public_key_by_id(key_id)),
                qualified_identity.clone(),
                None,
            )
            .await
            .map_err(|e| format!("Withdrawal error: {}", e))?;
        qualified_identity.identity.set_balance(remaining_balance);
        self.insert_local_qualified_identity(&qualified_identity)
            .map_err(|e| format!("Database error: {}", e))
    }
}
