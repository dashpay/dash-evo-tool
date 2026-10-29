//! A masternode voting key the masternode list no longer names.
//!
//! When a node's voting key is rotated, the key and voter identity this
//! device stores for it stop being the node's current ones. DET never removes
//! them on its own: the list it compares against can lag the chain (see
//! `WalletBackend::masternode_voting_key_hash`), so a stale list could make
//! the current key look retired. Instead the node's page asks
//! [`AppContext::check_retired_voting_key`] whether the stored key still
//! matches, and the user removes an old one with
//! [`AppContext::remove_retired_voting_key`].

use crate::backend_task::{BackendTaskSuccessResult, TaskError};
use crate::context::AppContext;
use crate::model::masternode_keys::{retired_voting_keys, stores_voting_key};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

impl AppContext {
    /// Whether the node `identity_id` stores a voting key or voter identity
    /// the masternode list no longer names. Read-only. Answers `false`
    /// whenever that cannot be told: nothing stored, the node gone, or the
    /// list not current.
    pub(super) async fn check_retired_voting_key(
        &self,
        identity_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let retired = match self.get_local_qualified_identity(&identity_id)? {
            Some(node) if stores_voting_key(&node) => {
                match self.current_voting_key_hash(identity_id).await {
                    Some(current) => !retired_voting_keys(&node, current).is_empty(),
                    None => false,
                }
            }
            _ => false,
        };
        Ok(BackendTaskSuccessResult::RetiredVotingKeyStatus {
            identity_id,
            retired,
        })
    }

    /// Remove the voting keys and voter identity the node `identity_id`
    /// stores that the masternode list no longer names, after the user asked
    /// for it. The list is read again now, not taken from when the notice was
    /// shown; a key that is current again is kept.
    pub(super) async fn remove_retired_voting_key(
        &self,
        identity_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let current = self.current_voting_key_hash(identity_id).await;
        self.remove_retired_voting_key_against(identity_id, current)
    }

    /// The list's current voting key hash for the node, or `None` when it
    /// cannot be read or is not current.
    async fn current_voting_key_hash(&self, identity_id: Identifier) -> Option<[u8; 20]> {
        match self.wallet_backend() {
            Ok(backend) => backend.masternode_voting_key_hash(identity_id).await,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "The wallet backend is not ready to read the masternode list"
                );
                None
            }
        }
    }

    /// [`Self::remove_retired_voting_key`] against `current`, the voting key
    /// hash just read from the masternode list.
    ///
    /// Everything is decided and done under the node's record guard, so the
    /// keys and voter identity removed are the ones the record names now, not
    /// before a concurrent reload changed them. Lock order: the
    /// storage-migration gate, the node's record guard, then the record guard
    /// of a separate voter record (as identity removal takes them). This is
    /// the only place two record guards nest; the gate serializes it against
    /// every identity removal, and nothing takes a voter's guard before its
    /// node's.
    ///
    /// A separate voter record an older version kept is removed first, so a
    /// failure there leaves the node as it was for a retry. Vault secrets go
    /// before the record entries: a stored placement is what makes a vault
    /// label reachable, so a failure after the secrets are gone leaves a
    /// removal the user can simply repeat.
    fn remove_retired_voting_key_against(
        &self,
        identity_id: Identifier,
        current: Option<[u8; 20]>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let current = current.ok_or(TaskError::VotingKeyCheckUnavailable)?;
        let _migration_guard = self
            .try_lock_prepare_gate()
            .map_err(|_| TaskError::WalletStorageNotReady)?;
        if self.migration_status().state().is_in_progress() {
            return Err(TaskError::WalletStorageNotReady);
        }
        let lock = self.identity_record_lock(identity_id);
        let _record_guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut node = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let retired = retired_voting_keys(&node, current);
        if retired.is_empty() {
            return Ok(BackendTaskSuccessResult::RetiredVotingKeyKept { identity_id });
        }
        if retired.voter_identity
            && let Some((voter, _)) = node.associated_voter_identity.as_ref()
            && voter.id() != identity_id
        {
            self.remove_voter_record_locked(voter.id())?;
        }
        self.delete_identity_key_secrets(&identity_id, retired.placements.clone())?;
        for placement in &retired.placements {
            node.private_keys.remove_at(placement);
        }
        if retired.voter_identity {
            node.associated_voter_identity = None;
        }
        self.write_local_qualified_identity_locked(&node)?;
        Ok(BackendTaskSuccessResult::RetiredVotingKeyRemoved { identity_id })
    }

    /// Remove the separate record an older version kept for a node's voter
    /// identity, if there is one. The caller holds the storage-migration gate
    /// and the node's record guard. A failure that leaves the record listed
    /// is returned; one after it was delisted is not, as the next start
    /// finishes that cleanup.
    fn remove_voter_record_locked(&self, voter_id: Identifier) -> Result<(), TaskError> {
        let lock = self.identity_record_lock(voter_id);
        let _record_guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.is_identity_listed(&voter_id)? {
            return Ok(());
        }
        match self.delete_local_qualified_identity_locked(&voter_id) {
            Ok(_) => Ok(()),
            Err(error) if self.is_identity_listed(&voter_id).unwrap_or(true) => Err(error),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    %voter_id,
                    "Old voter identity removed but its vault cleanup is still pending; the next start finishes it"
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_staging::{
        StagedIdentity, stage_identity_with_vaulted_keys, stage_identity_with_voter_twin,
    };
    use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget};
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::{Identity, IdentityPublicKey};
    use dash_sdk::dpp::version::PlatformVersion;

    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;
    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;

    /// A stored masternode with vaulted main-identity keys and a voting key,
    /// held in the vault, on a loaded voter identity.
    async fn node_with_vaulted_voting_key() -> (StagedIdentity, IdentityPublicKey) {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;
        let voting = IdentityPublicKey::random_key(7, Some(7), PlatformVersion::latest());
        let voter = Identity::create_basic_identity(
            Identifier::from([0x3C; 32]),
            PlatformVersion::latest(),
        )
        .unwrap();
        IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .store(&V, voting.id(), &rand::random::<[u8; 32]>())
            .unwrap();
        staged
            .ctx
            .edit_local_qualified_identity(&staged.id, |node| {
                node.identity_type = IdentityType::Masternode;
                node.private_keys.insert_at(
                    (V, voting.id()),
                    (
                        QualifiedIdentityPublicKey::from(voting.clone()),
                        PrivateKeyData::InVault,
                    ),
                );
                node.associated_voter_identity = Some((voter, voting.clone()));
                Ok(())
            })
            .unwrap();
        (staged, voting)
    }

    fn stored(staged: &StagedIdentity) -> crate::model::qualified_identity::QualifiedIdentity {
        staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap()
    }

    fn voting_secret_held(staged: &StagedIdentity, key_id: u32) -> bool {
        IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .get(&V, key_id)
            .unwrap()
            .is_some()
    }

    /// The list names another voting key: the old key, its vault secret and
    /// the voter identity go; the node's own keys stay.
    #[tokio::test]
    async fn removes_a_voting_key_the_list_no_longer_names() {
        let (staged, voting) = node_with_vaulted_voting_key().await;

        let result = staged
            .ctx
            .remove_retired_voting_key_against(staged.id, Some([0xEE; 20]))
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { identity_id } if identity_id == staged.id
        ));
        let node = stored(&staged);
        assert!(node.associated_voter_identity.is_none());
        assert!(!node.private_keys.has(&(V, voting.id())));
        assert!(!voting_secret_held(&staged, voting.id()));
        for key_id in [1, 2] {
            assert!(
                node.private_keys.has(&(M, key_id)),
                "main key {key_id} kept"
            );
            assert!(
                IdentityKeyView::new(&staged.store, staged.id.to_buffer())
                    .get(&M, key_id)
                    .unwrap()
                    .is_some(),
                "main key {key_id} secret kept"
            );
        }
    }

    /// An older version kept the voter identity as a record of its own: it
    /// goes with the old voting key, so no stray voter is left listed.
    #[tokio::test]
    async fn removes_the_stored_copy_of_the_old_voter_identity() {
        let staged = stage_identity_with_voter_twin(rand::random(), rand::random()).await;
        let voter_id = staged.voter_id.unwrap();

        let result = staged
            .ctx
            .remove_retired_voting_key_against(staged.id, Some([0xEE; 20]))
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { .. }
        ));
        assert!(stored(&staged).associated_voter_identity.is_none());
        assert!(!staged.ctx.is_identity_listed(&voter_id).unwrap());
        assert!(staged.ctx.is_identity_listed(&staged.id).unwrap());
    }

    /// A reload can switch the node's voter identity from A to B while the
    /// removal waits for the node's record. The removal must act on what the
    /// record names once it holds it: B's separate record goes with B's
    /// association, and A's record, which the node no longer names, stays.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_voter_switched_while_waiting_is_the_one_removed() {
        use crate::context::lock_probe::{LockProbe, LockSite};
        use crate::context::test_staging::qi_with_plaintext_and_derived_at;
        let staged = stage_identity_with_voter_twin(rand::random(), rand::random()).await;
        let voter_a = staged.voter_id.unwrap();
        let mut voter_b = qi_with_plaintext_and_derived_at(
            Identifier::from([0x3D; 32]),
            rand::random(),
            rand::random(),
        );
        voter_b.private_keys = Default::default();
        staged
            .ctx
            .insert_local_qualified_identity(&voter_b, &None)
            .unwrap();
        let mut switched = stored(&staged);
        switched.associated_voter_identity = Some((
            voter_b.identity.clone(),
            IdentityPublicKey::random_key(5, Some(5), PlatformVersion::latest()),
        ));

        let probe = LockProbe::new();
        let removal_probe = probe.clone();
        let lock = staged.ctx.identity_record_lock(staged.id);
        let guard = lock.lock().unwrap();
        let ctx = std::sync::Arc::clone(&staged.ctx);
        let node_id = staged.id;
        let removal = std::thread::spawn(move || {
            let _attached = removal_probe.attach();
            ctx.remove_retired_voting_key_against(node_id, Some([0xEE; 20]))
        });
        while probe.count(LockSite::RecordRequest) == 0 {
            std::thread::yield_now();
        }
        staged
            .ctx
            .write_local_qualified_identity_locked(&switched)
            .unwrap();
        drop(guard);
        let result = removal.join().unwrap().unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { .. }
        ));
        assert!(stored(&staged).associated_voter_identity.is_none());
        assert!(
            !staged
                .ctx
                .is_identity_listed(&voter_b.identity.id())
                .unwrap(),
            "the voter the node named when the removal ran is removed"
        );
        assert!(
            staged.ctx.is_identity_listed(&voter_a).unwrap(),
            "a voter the node no longer names is left alone"
        );
    }

    /// By the time of the click the list names the stored key again: nothing
    /// is removed, and the result says the key was kept.
    #[tokio::test]
    async fn keeps_a_voting_key_that_is_current_again() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let before = stored(&staged);

        let result = staged
            .ctx
            .remove_retired_voting_key_against(staged.id, Some(voting.public_key_hash().unwrap()))
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyKept { identity_id } if identity_id == staged.id
        ));
        assert_eq!(stored(&staged), before);
        assert!(voting_secret_held(&staged, voting.id()));
    }

    /// Without a current list nothing can be checked, so nothing is removed.
    #[tokio::test]
    async fn refuses_without_a_current_list() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let before = stored(&staged);

        let result = staged
            .ctx
            .remove_retired_voting_key_against(staged.id, None);

        assert!(matches!(result, Err(TaskError::VotingKeyCheckUnavailable)));
        assert_eq!(stored(&staged), before);
        assert!(voting_secret_held(&staged, voting.id()));
    }

    /// A node with no voting state is never flagged, and the check needs no
    /// masternode list to say so.
    #[tokio::test]
    async fn a_node_without_a_voting_key_is_not_flagged() {
        let staged = stage_identity_with_vaulted_keys(rand::random(), rand::random()).await;

        let result = staged
            .ctx
            .check_retired_voting_key(staged.id)
            .await
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyStatus { retired: false, .. }
        ));
    }

    /// Offline, the masternode list is never current: a stored voting key is
    /// not flagged, so a lagging list cannot raise a false alarm.
    #[tokio::test]
    async fn a_voting_key_is_not_flagged_without_a_current_list() {
        let (staged, _) = node_with_vaulted_voting_key().await;

        let result = staged
            .ctx
            .check_retired_voting_key(staged.id)
            .await
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyStatus { retired: false, .. }
        ));
    }
}
