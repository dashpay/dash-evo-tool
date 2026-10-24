//! Usage limits of identity authentication keys (protocol version 14).

use std::collections::BTreeMap;

use std::sync::Arc;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::identity_key_limits::{
    KeyLimitsError, KeyLimitsRaise, ReviewedSignerUnusable, reviewed_key_limits_signer,
};
use crate::model::identity_key_usability::now_ms;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::platform::Identity;
use dash_sdk::platform::identity_keys_remaining_budgets::{
    IdentityKeysRemainingBudgets, IdentityKeysRemainingBudgetsQuery,
};
use dash_sdk::platform::transition::update_identity_key_limits::UpdateIdentityKeyLimits;
use dash_sdk::platform::{Fetch, Identifier};
use dash_sdk::query_types::IdentityBalance;

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
        // A network below protocol version 14 answers this query on no node,
        // so a request would exhaust the SDK's address pool.
        if !FeatureGate::IdentityKeyLimits.is_available(self) {
            return Err(TaskError::KeyRemainingBudgetsNotSupported);
        }
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
    /// Raise the limits of a key of `qualified_identity` exactly as reviewed
    /// in `raise`.
    ///
    /// The key is read as Platform holds it now. If its limits are not the
    /// ones the user reviewed (raised elsewhere meanwhile), nothing is signed:
    /// the live keys replace the local copies and the raise is refused, so the
    /// screen quotes it again. The same holds when the reviewed signer can no
    /// longer sign it — another key is never picked in its place. The
    /// updated key replaces the local copy.
    pub(super) async fn raise_key_limits(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        raise: KeyLimitsRaise,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !FeatureGate::IdentityKeyLimits.is_available(self) {
            return Err(TaskError::KeyLimitsNotSupported);
        }
        let identity_id = qualified_identity.identity.id();
        let key_id = raise.key_id;
        let identity = Identity::fetch_by_identifier(sdk, identity_id)
            .await?
            .ok_or(TaskError::IdentityMissingOnNetwork { identity_id })?;
        let key = identity
            .public_keys()
            .get(&key_id)
            .ok_or(TaskError::IdentityKeyNotFound { key_id })?;
        let refusal = match raise.check_against(key, now_ms()) {
            Err(error) => Some(TaskError::from(error)),
            Ok(()) => {
                reviewed_key_limits_signer(identity.public_keys(), raise.signing_key_id, |key| {
                    qualified_identity.can_sign_with(key)
                })
                .err()
                .map(refused_signer_error)
            }
        };
        if let Some(refusal) = refusal {
            // Nothing is signed. Keep the keys as Platform holds them, so
            // the screen's next review quotes the live limits and picks its
            // signer from the live key set.
            let live_keys: Vec<_> = identity.public_keys().values().cloned().collect();
            self.edit_local_qualified_identity(&identity_id, move |fresh| {
                for key in live_keys {
                    fresh.identity.add_public_key(key);
                }
                Ok(())
            })?;
            return Err(refusal);
        }

        let signing_key = identity
            .public_keys()
            .get(&raise.signing_key_id)
            .cloned()
            .ok_or(TaskError::NoKeyLimitsSigningKey)?;
        let signer = Arc::new(qualified_identity);

        let updated = identity
            .update_key_limits(
                sdk,
                key_id,
                raise.total_budget,
                raise.expires_at,
                Some(&signing_key),
                Arc::clone(&signer),
                None,
            )
            .await?;

        // The update paid a fee, but its proof holds only the rewritten key:
        // read the balance after it, proved, so the stored identity does not
        // overstate what later fees can draw on.
        let balance = IdentityBalance::fetch(sdk, identity_id)
            .await
            .map_err(TaskError::from);
        self.finish_raise(identity_id, updated, balance)
    }
}

impl AppContext {
    /// Store the outcome of a raise the network accepted: `key` as rewritten,
    /// and `balance`, the identity's balance read after the update.
    fn finish_raise(
        &self,
        identity_id: Identifier,
        key: dash_sdk::dpp::identity::IdentityPublicKey,
        balance: Result<Option<Credits>, TaskError>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let (balance, balance_error) = match balance {
            Ok(Some(balance)) => (Some(balance), None),
            Ok(None) => (None, Some(None)),
            Err(error) => (None, Some(Some(Box::new(error)))),
        };
        self.persist_raised_key(&identity_id, key.clone(), balance)?;
        // The raise is made and its key stored; only the balance is stale.
        if let Some(source) = balance_error {
            return Err(TaskError::KeyLimitsRaisedButBalanceNotRefreshed { source });
        }
        Ok(BackendTaskSuccessResult::IdentityKeyLimitsRaised { identity_id, key })
    }

    /// Store `key`, whose limits the network has just raised, and `balance`,
    /// when read, on the record of `identity_id`, in one write. The raise is
    /// already on the network, so a failure here is reported as
    /// [`TaskError::KeyLimitsRaisedButNotSaved`], not as a failed raise.
    fn persist_raised_key(
        &self,
        identity_id: &Identifier,
        key: dash_sdk::dpp::identity::IdentityPublicKey,
        balance: Option<Credits>,
    ) -> Result<(), TaskError> {
        self.edit_local_qualified_identity(identity_id, move |fresh| {
            fresh.identity.add_public_key(key);
            if let Some(balance) = balance {
                fresh.identity.set_balance(balance);
            }
            Ok(())
        })
        .map(|_| ())
        .map_err(|source| TaskError::KeyLimitsRaisedButNotSaved {
            source: Box::new(source),
        })
    }
}

/// The error for a reviewed signer Platform no longer accepts.
fn refused_signer_error(reason: ReviewedSignerUnusable) -> TaskError {
    match reason {
        ReviewedSignerUnusable::SignerChanged => KeyLimitsError::SignerChanged.into(),
        ReviewedSignerUnusable::NoSigner => TaskError::NoKeyLimitsSigningKey,
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

    pub(crate) fn user_identity() -> QualifiedIdentity {
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

    /// A reviewed signer replaced on Platform asks for a fresh review; only
    /// when no key can sign at all is the user told to import one.
    #[test]
    fn a_changed_signer_asks_for_a_new_review_not_a_key_import() {
        let changed = refused_signer_error(ReviewedSignerUnusable::SignerChanged);
        assert!(matches!(
            changed,
            TaskError::InvalidKeyLimits {
                source: KeyLimitsError::SignerChanged
            }
        ));
        assert!(changed.to_string().contains("Review the change again"));

        assert!(matches!(
            refused_signer_error(ReviewedSignerUnusable::NoSigner),
            TaskError::NoKeyLimitsSigningKey
        ));
    }

    /// Protocol versions below 14: not known yet (0), and mainnet's 13.
    const PRE_V14_PROTOCOL_VERSIONS: [u32; 2] = [0, 13];

    /// A raise the network accepted stores the rewritten key and the balance
    /// read after it, so the stored balance reflects the update's fee.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_raise_stores_the_key_and_the_balance_after_its_fee() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;

        let staged = stage_identity_with_vaulted_keys([0xA1; 32], [0xB1; 32]).await;
        let ctx = &staged.ctx;
        let stored = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read")
            .expect("staged");
        let key = dash_sdk::dpp::identity::IdentityPublicKey::random_key(
            7,
            Some(7),
            PlatformVersion::latest(),
        )
        .with_limits(Some(9_000), None);
        let before = stored.identity.balance();

        let result = ctx.finish_raise(staged.id, key.clone(), Ok(Some(before + 1_234)));

        assert!(
            matches!(
                result,
                Ok(BackendTaskSuccessResult::IdentityKeyLimitsRaised { .. })
            ),
            "got {result:?}"
        );
        let after = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read")
            .expect("staged");
        assert_eq!(after.identity.balance(), before + 1_234);
        assert_eq!(
            after
                .identity
                .public_keys()
                .get(&key.id())
                .and_then(|k| k.total_budget()),
            Some(9_000)
        );
    }

    /// A balance that cannot be read after an accepted raise does not make the
    /// raise a failure: the key is stored, and the user is told to refresh.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unread_balance_after_a_raise_still_stores_the_key() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;

        let staged = stage_identity_with_vaulted_keys([0xA2; 32], [0xB2; 32]).await;
        let ctx = &staged.ctx;
        let stored = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read")
            .expect("staged");
        let key = dash_sdk::dpp::identity::IdentityPublicKey::random_key(
            7,
            Some(7),
            PlatformVersion::latest(),
        )
        .with_limits(Some(9_000), None);

        for balance in [Err(TaskError::IdentityNotFoundLocally), Ok(None)] {
            let result = ctx.finish_raise(staged.id, key.clone(), balance);
            assert!(
                matches!(
                    result,
                    Err(TaskError::KeyLimitsRaisedButBalanceNotRefreshed { .. })
                ),
                "got {result:?}"
            );
        }
        let after = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read")
            .expect("staged");
        assert_eq!(
            after
                .identity
                .public_keys()
                .get(&key.id())
                .and_then(|k| k.total_budget()),
            Some(9_000)
        );
        assert_eq!(after.identity.balance(), stored.identity.balance());
    }

    /// A raise the network accepted is reported as made, not as failed, when
    /// storing the raised key on this device fails afterward.
    #[test]
    fn a_raise_that_cannot_be_saved_is_reported_as_made() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let identity = user_identity();
        let key = dash_sdk::dpp::identity::IdentityPublicKey::random_key(
            1,
            Some(1),
            PlatformVersion::latest(),
        );

        // Nothing is stored for this identity, so the write fails.
        let result = ctx.persist_raised_key(&identity.identity.id(), key, Some(1));

        assert!(
            matches!(result, Err(TaskError::KeyLimitsRaisedButNotSaved { .. })),
            "got {result:?}"
        );
    }

    /// Before protocol version 14 is confirmed, no limits update is built or
    /// broadcast: an older network cannot decode it.
    #[tokio::test]
    async fn raising_limits_is_refused_until_the_network_supports_them() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = Sdk::new_mock();

        for version in PRE_V14_PROTOCOL_VERSIONS {
            ctx.set_platform_protocol_version(version);
            let raise = KeyLimitsRaise {
                key_id: 1,
                seen_total_budget: Some(1),
                seen_expires_at: None,
                total_budget: Some(2),
                expires_at: None,
                signing_key_id: 0,
            };
            let result = ctx.raise_key_limits(&sdk, user_identity(), raise).await;

            assert!(
                matches!(result, Err(TaskError::KeyLimitsNotSupported)),
                "protocol {version}: got {result:?}"
            );
        }
    }

    /// Before protocol version 14 is confirmed, the remaining-budget query is
    /// not sent: an older network does not answer it, on any node.
    #[tokio::test]
    async fn remaining_budgets_are_not_queried_until_the_network_supports_them() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = Sdk::new_mock();

        for version in PRE_V14_PROTOCOL_VERSIONS {
            ctx.set_platform_protocol_version(version);
            for key_ids in [vec![1, 2], vec![]] {
                let result = ctx
                    .fetch_key_remaining_budgets(&sdk, Identifier::random(), key_ids)
                    .await;
                assert!(
                    matches!(result, Err(TaskError::KeyRemainingBudgetsNotSupported)),
                    "protocol {version}: got {result:?}"
                );
            }
        }
    }

    /// A failed raise must reach the screen it came from, keyed by identity
    /// and key, so that screen re-enables its form.
    #[test]
    fn a_raise_is_attributed_to_its_identity_and_key() {
        let identity = user_identity();
        let identity_id = identity.identity.id();
        let task = BackendTask::IdentityTask(IdentityTask::RaiseKeyLimits {
            identity: Box::new(identity),
            raise: KeyLimitsRaise {
                key_id: 7,
                seen_total_budget: Some(1),
                seen_expires_at: None,
                total_budget: Some(2),
                expires_at: None,
                signing_key_id: 0,
            },
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
