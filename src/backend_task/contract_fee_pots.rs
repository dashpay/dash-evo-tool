//! Contract fee pots (protocol version 14): what a contract's document action
//! fees have paid into its owner pot and its moderators pot, and paying a pot
//! out to its recipients.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::contract_fee_pots::{SeatedModerators, has_elected_moderation};
use crate::model::identity_key_usability::{KeyRequirements, SigningScope};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::document_type::action_fees::ContractFeePot;
use dash_sdk::dpp::fee::Credits;
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
    /// Both fee pots of `contract_id`, with the seated moderation team that
    /// claims an elected contract's moderators pot. A contract nobody has paid
    /// fees to reads as two empty pots.
    pub(super) async fn fetch_contract_fee_pots(
        &self,
        sdk: &Sdk,
        contract_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !FeatureGate::ContractFeePots.is_available(self) {
            return Err(TaskError::ContractFeePotsNotSupported);
        }
        // A contract saved here without elected moderation has no seated
        // team, so its team is not read. One not saved here is read anyway.
        let elected = self
            .get_unqualified_contract_by_id(&contract_id)
            .ok()
            .flatten()
            .is_none_or(|contract| has_elected_moderation(&contract));
        // Only the pots are proved. The response metadata's epoch is not
        // signed, so nothing is decided from it.
        let (pots, seated_moderators) =
            tokio::join!(ContractFeePots::fetch(sdk, contract_id), async {
                if elected {
                    read_seated_moderators(sdk, contract_id).await
                } else {
                    SeatedModerators::NotSeated
                }
            });
        Ok(BackendTaskSuccessResult::ContractFeePots {
            contract_id,
            pots: pots?.unwrap_or_default(),
            seated_moderators,
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
        self.persist_paid_balances(&identity_id, &claimed.balances)?;

        Ok(BackendTaskSuccessResult::ContractFeesClaimed {
            contract_id,
            pot,
            claimant_balance,
            remaining_credits: claimed.remaining_credits,
        })
    }

    /// Store the balances after a claim the network paid out (`balances`,
    /// from the proof) on the records of the paid identities kept on this
    /// device; paid identities not kept here are skipped, but `claimant` must
    /// be kept here. The claim is already paid out, so a failure here is
    /// reported as [`TaskError::ContractFeesClaimedButBalanceNotSaved`], not
    /// as a failed claim.
    fn persist_paid_balances(
        &self,
        claimant: &Identifier,
        balances: &BTreeMap<Identifier, Credits>,
    ) -> Result<(), TaskError> {
        // Every write is tried, so one failure does not leave the other
        // paid identities stale; the first failure is reported.
        let mut first_failure = None;
        for (identity_id, balance) in balances {
            let saved = self.edit_local_qualified_identity(identity_id, |fresh| {
                fresh.identity.set_balance(*balance);
                Ok(())
            });
            match saved {
                Ok(_) => {}
                // A paid co-recipient not kept on this device has no record
                // to update.
                Err(TaskError::IdentityNotFoundLocally) if identity_id != claimant => {}
                Err(source) => {
                    first_failure.get_or_insert(source);
                }
            }
        }
        match first_failure {
            None => Ok(()),
            Some(source) => Err(TaskError::ContractFeesClaimedButBalanceNotSaved {
                source: Box::new(source),
            }),
        }
    }
}

/// The seated moderation team of `contract_id`, its leader and active
/// members: who Platform lets claim an elected contract's moderators pot.
///
/// A failed read is not an error of the pots read: the pots still show, and
/// the claim is left to Platform to accept or refuse.
async fn read_seated_moderators(sdk: &Sdk, contract_id: Identifier) -> SeatedModerators {
    match sdk.fetch_moderation_team(contract_id).await {
        Ok(None) => SeatedModerators::NotSeated,
        Ok(Some(team)) => {
            let mut members = team.members;
            members.insert(team.leader_id);
            SeatedModerators::Seated(members)
        }
        Err(error) => {
            tracing::debug!(
                ?error,
                %contract_id,
                "Could not read the seated moderation team; leaving moderator claims to Platform"
            );
            SeatedModerators::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A claim the network paid out is reported as paid out, not as failed,
    /// when storing the claimant's new balance on this device fails.
    #[test]
    fn a_claim_whose_balance_cannot_be_saved_is_reported_as_paid_out() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());

        // Nothing is stored for this identity, so the write fails.
        let claimant = Identifier::random();
        let result = ctx.persist_paid_balances(&claimant, &BTreeMap::from([(claimant, 1_000)]));

        assert!(
            matches!(
                result,
                Err(TaskError::ContractFeesClaimedButBalanceNotSaved { .. })
            ),
            "got {result:?}"
        );
    }

    /// The proof carries the new balance of every identity the pot paid: each
    /// one kept on this device is updated, not only the claimant's, and the
    /// paid identities not kept here are skipped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_claim_stores_the_new_balance_of_every_paid_identity_kept_here() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::wired_test_app_context(temp_dir.path()).await;
        let [claimant, co_moderator, stranger] = [1, 2, 3].map(|byte| Identifier::new([byte; 32]));
        for id in [claimant, co_moderator] {
            let identity = crate::context::test_support::bare_user_identity(id, ctx.network);
            ctx.insert_local_qualified_identity(&identity, &None)
                .expect("identity stored");
        }
        let balances =
            BTreeMap::from([(claimant, 1_000), (co_moderator, 2_000), (stranger, 3_000)]);

        ctx.persist_paid_balances(&claimant, &balances)
            .expect("balances saved");

        let balance = |id| {
            ctx.get_local_qualified_identity(&id)
                .expect("read")
                .map(|stored| stored.identity.balance())
        };
        assert_eq!(balance(claimant), Some(1_000));
        assert_eq!(balance(co_moderator), Some(2_000));
        assert_eq!(
            balance(stranger),
            None,
            "an identity not kept here is not added"
        );
    }

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

            let identity = crate::context::test_support::bare_user_identity(
                Identifier::random(),
                dash_sdk::dpp::dashcore::Network::Testnet,
            );
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
        let identity = crate::context::test_support::bare_user_identity(
            Identifier::random(),
            dash_sdk::dpp::dashcore::Network::Testnet,
        );

        let claimed = ctx
            .claim_contract_fees(&sdk, Identifier::random(), ContractFeePot::Owner, identity)
            .await;
        assert!(
            matches!(claimed, Err(TaskError::NoFeeClaimSigningKey)),
            "got {claimed:?}"
        );
    }
}
