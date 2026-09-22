//! Usage limits of identity authentication keys (protocol version 14).

use std::collections::BTreeMap;

use std::sync::Arc;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::identity_key_limits::{key_limits_update_signer, raised_key_limits};
use crate::model::identity_key_usability::now_ms;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::platform::Identity;
use dash_sdk::platform::identity_keys_remaining_budgets::{
    IdentityKeysRemainingBudgets, IdentityKeysRemainingBudgetsQuery,
};
use dash_sdk::platform::transition::update_identity_key_limits::UpdateIdentityKeyLimits;
use dash_sdk::platform::{Fetch, Identifier};

use super::BackendTaskSuccessResult;

impl AppContext {
    /// What is left of the budgets of `key_ids` of `identity_id`.
    ///
    /// One entry per requested key: `Some(credits)` for a budgeted key (zero
    /// means the key can no longer sign), `None` for a key without a budget or
    /// one Platform does not know.
    pub(super) async fn fetch_key_remaining_budgets(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        key_ids: Vec<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if key_ids.is_empty() {
            return Ok(BackendTaskSuccessResult::IdentityKeyRemainingBudgets {
                identity_id,
                budgets: BTreeMap::new(),
            });
        }
        let fetched = IdentityKeysRemainingBudgets::fetch(
            sdk,
            IdentityKeysRemainingBudgetsQuery {
                identity_id,
                key_ids: key_ids.clone(),
            },
        )
        .await?
        .unwrap_or_default();

        Ok(BackendTaskSuccessResult::IdentityKeyRemainingBudgets {
            identity_id,
            budgets: remaining_budgets_by_key(&key_ids, &fetched),
        })
    }
}

impl AppContext {
    /// Raise the limits of key `key_id` of `qualified_identity`: add
    /// `add_budget` credits to its spending limit and/or extend its expiry by
    /// `extend_days`.
    ///
    /// The new limits are computed from the key as Platform holds it now, not
    /// from the local copy, so a key raised elsewhere meanwhile is not
    /// lowered back; a request Platform would refuse (and charge for) is
    /// refused here first. The updated key replaces the local copy.
    pub(super) async fn raise_key_limits(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        key_id: KeyID,
        add_budget: Option<Credits>,
        extend_days: Option<u32>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !FeatureGate::IdentityKeyLimits.is_available(self) {
            return Err(TaskError::KeyLimitsNotSupported);
        }
        let identity_id = qualified_identity.identity.id();
        let identity = Identity::fetch_by_identifier(sdk, identity_id)
            .await?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let key = identity
            .public_keys()
            .get(&key_id)
            .ok_or(TaskError::IdentityKeyNotFound { key_id })?;
        let (total_budget, expires_at) = raised_key_limits(key, add_budget, extend_days, now_ms())?;

        let signer = Arc::new(qualified_identity);
        let signing_key = key_limits_update_signer(identity.public_keys().values(), |key| {
            signer.can_sign_with(key)
        })
        .cloned()
        .ok_or(TaskError::NoKeyLimitsSigningKey)?;

        let updated = identity
            .update_key_limits(
                sdk,
                key_id,
                total_budget,
                expires_at,
                Some(&signing_key),
                Arc::clone(&signer),
                None,
            )
            .await?;

        let stored_key = updated.clone();
        self.edit_local_qualified_identity(&identity_id, move |fresh| {
            fresh.identity.add_public_key(stored_key);
            Ok(())
        })?;

        Ok(BackendTaskSuccessResult::IdentityKeyLimitsRaised {
            identity_id,
            key: updated,
        })
    }
}

/// One entry per requested key id, in key id order: the remaining budget
/// Platform reported, `None` when it reported none.
fn remaining_budgets_by_key(
    key_ids: &[KeyID],
    fetched: &IdentityKeysRemainingBudgets,
) -> BTreeMap<KeyID, Option<Credits>> {
    key_ids
        .iter()
        .map(|key_id| (*key_id, fetched.get(key_id).copied().flatten()))
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::backend_task::identity::IdentityTask;
    use crate::backend_task::{BackendTask, BackendTaskContext};
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::version::PlatformVersion;

    pub(in crate::backend_task::identity) fn user_identity() -> QualifiedIdentity {
        let identity =
            Identity::create_basic_identity(Identifier::random(), PlatformVersion::latest())
                .expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// Before protocol version 14 is confirmed, no limits update is built or
    /// broadcast: an older network cannot decode it.
    #[tokio::test]
    async fn raising_limits_is_refused_until_the_network_supports_them() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = Sdk::new_mock();

        let result = ctx
            .raise_key_limits(&sdk, user_identity(), 1, Some(1), None)
            .await;

        assert!(
            matches!(result, Err(TaskError::KeyLimitsNotSupported)),
            "got {result:?}"
        );
    }

    /// A failed raise must reach the screen it came from, keyed by identity
    /// and key, so that screen re-enables its form.
    #[test]
    fn a_raise_is_attributed_to_its_identity_and_key() {
        let identity = user_identity();
        let identity_id = identity.identity.id();
        let task = BackendTask::IdentityTask(IdentityTask::RaiseKeyLimits {
            identity: Box::new(identity),
            key_id: 7,
            add_budget: Some(1),
            extend_days: None,
        });
        assert_eq!(
            BackendTaskContext::from(&task).raise_key_limits_target(),
            Some((identity_id, 7))
        );
    }

    #[test]
    fn every_requested_key_gets_an_entry() {
        let fetched: IdentityKeysRemainingBudgets = [(1, Some(500)), (2, None), (3, Some(0))]
            .into_iter()
            .collect();
        let by_key = remaining_budgets_by_key(&[1, 2, 3, 4], &fetched);
        assert_eq!(
            by_key,
            BTreeMap::from([(1, Some(500)), (2, None), (3, Some(0)), (4, None)])
        );
    }
}
