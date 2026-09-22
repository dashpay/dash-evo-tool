//! Contract fee pots (protocol version 14): what a contract's document action
//! fees have paid into its owner pot and its moderators pot, and paying a pot
//! out to its recipients.

use std::sync::Arc;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::identity_key_usability::{KeyRequirements, SigningScope};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::document_type::action_fees::ContractFeePot;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::platform::contract_fee_pots::ContractFeePots;
use dash_sdk::platform::transition::contract_fee_claim::ClaimContractFees;
use dash_sdk::platform::{Fetch, Identifier};

/// The key requirements of a fee claim: a CRITICAL authentication key, and
/// since a claim is not a document batch, not one bound to a contract.
fn fee_claim_key_requirements() -> KeyRequirements<'static> {
    KeyRequirements::new(
        Purpose::AUTHENTICATION,
        &[SecurityLevel::CRITICAL],
        SigningScope::NonBatch,
    )
}

impl AppContext {
    /// Both fee pots of `contract_id`. A contract nobody has paid fees to
    /// reads as two empty pots.
    pub(super) async fn fetch_contract_fee_pots(
        &self,
        sdk: &Sdk,
        contract_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !FeatureGate::ContractFeePots.is_available(self) {
            return Err(TaskError::ContractFeePotsNotSupported);
        }
        let (pots, metadata) = ContractFeePots::fetch_with_metadata(sdk, contract_id, None).await?;
        Ok(BackendTaskSuccessResult::ContractFeePots {
            contract_id,
            pots: pots.unwrap_or_default(),
            // The epoch of the proved response; a pot already paid out in it
            // cannot be claimed again until the next one.
            current_epoch: u16::try_from(metadata.epoch).ok(),
        })
    }

    /// Pay out `pot` of `contract_id`, claimed by `qualified_identity`, and
    /// store the claimant's new balance.
    pub(super) async fn claim_contract_fees(
        &self,
        sdk: &Sdk,
        contract_id: Identifier,
        pot: ContractFeePot,
        qualified_identity: QualifiedIdentity,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !FeatureGate::ContractFeePots.is_available(self) {
            return Err(TaskError::ContractFeePotsNotSupported);
        }
        let identity_id = qualified_identity.identity.id();
        let signing_key = qualified_identity
            .signing_key_now(fee_claim_key_requirements())
            .cloned()
            .ok_or(TaskError::NoFeeClaimSigningKey)?;
        let identity = qualified_identity.identity.clone();
        let signer = Arc::new(qualified_identity);

        let claimed = identity
            .claim_contract_fees(sdk, contract_id, pot, Some(&signing_key), signer, None)
            .await?;

        let claimant_balance = claimed.balances.get(&identity_id).copied();
        if let Some(balance) = claimant_balance {
            self.edit_local_qualified_identity(&identity_id, |fresh| {
                fresh.identity.set_balance(balance);
                Ok(())
            })?;
        }

        Ok(BackendTaskSuccessResult::ContractFeesClaimed {
            contract_id,
            pot,
            claimant_balance,
            remaining_credits: claimed.remaining_credits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Before protocol version 14 is confirmed nothing is read or claimed:
    /// an older network has no fee pots.
    #[tokio::test]
    async fn fee_pots_need_a_network_that_keeps_them() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = Sdk::new_mock();

        // Not known yet (0), and mainnet's 13.
        for version in [0, 13] {
            ctx.set_platform_protocol_version(version);
            let fetched = ctx
                .fetch_contract_fee_pots(&sdk, Identifier::random())
                .await;
            assert!(
                matches!(fetched, Err(TaskError::ContractFeePotsNotSupported)),
                "protocol {version}: got {fetched:?}"
            );

            let identity = crate::backend_task::identity::key_limits_test_identity();
            let claimed = ctx
                .claim_contract_fees(&sdk, Identifier::random(), ContractFeePot::Owner, identity)
                .await;
            assert!(
                matches!(claimed, Err(TaskError::ContractFeePotsNotSupported)),
                "protocol {version}: got {claimed:?}"
            );
        }
    }

    /// A claimant with no CRITICAL unbound key is refused before anything is
    /// signed or broadcast.
    #[tokio::test]
    async fn a_claim_needs_a_critical_unbound_key() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        ctx.set_platform_protocol_version(dash_sdk::dpp::version::v14::PROTOCOL_VERSION_14);
        let sdk = Sdk::new_mock();
        let identity = crate::backend_task::identity::key_limits_test_identity();

        let claimed = ctx
            .claim_contract_fees(&sdk, Identifier::random(), ContractFeePot::Owner, identity)
            .await;
        assert!(
            matches!(claimed, Err(TaskError::NoFeeClaimSigningKey)),
            "got {claimed:?}"
        );
    }
}
