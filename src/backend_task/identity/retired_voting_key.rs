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
    /// A separate record an older version kept for the old voter identity is
    /// removed first, so a failure there leaves the node as it was and the
    /// notice in place for a retry. The node's own read-modify-write then
    /// runs under its record lock (taken by `edit_local_qualified_identity`)
    /// and recomputes what is retired, so the keys removed are the ones
    /// stored now. Vault secrets go before the record entries: a stored
    /// placement is what makes a vault label reachable, so a failure after
    /// the secrets are gone leaves a removal the user can simply repeat.
    fn remove_retired_voting_key_against(
        &self,
        identity_id: Identifier,
        current: Option<[u8; 20]>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let current = current.ok_or(TaskError::VotingKeyCheckUnavailable)?;
        let node = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        if retired_voting_keys(&node, current).voter_identity
            && let Some((voter, _)) = node.associated_voter_identity.as_ref()
            && voter.id() != identity_id
        {
            self.remove_voter_record(voter.id())?;
        }
        let mut kept = false;
        self.edit_local_qualified_identity(&identity_id, |node| {
            let retired = retired_voting_keys(node, current);
            if retired.is_empty() {
                kept = true;
                return Ok(());
            }
            self.delete_identity_key_secrets(&identity_id, retired.placements.clone())?;
            for placement in &retired.placements {
                node.private_keys.remove_at(placement);
            }
            if retired.voter_identity {
                node.associated_voter_identity = None;
            }
            Ok(())
        })?;
        Ok(if kept {
            BackendTaskSuccessResult::RetiredVotingKeyKept { identity_id }
        } else {
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { identity_id }
        })
    }

    /// Remove the separate record an older version kept for a node's voter
    /// identity, if there is one. A failure that leaves the record listed is
    /// returned; one after it was delisted is not, as the next start finishes
    /// that cleanup.
    fn remove_voter_record(&self, voter_id: Identifier) -> Result<(), TaskError> {
        if !self.is_identity_listed(&voter_id)? {
            return Ok(());
        }
        match self.delete_local_qualified_identity_with_outcome(&voter_id) {
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
    const VOTING_SECRET: [u8; 32] = [0x5A; 32];

    /// A stored masternode with vaulted main-identity keys and a voting key,
    /// held in the vault, on a loaded voter identity.
    async fn node_with_vaulted_voting_key() -> (StagedIdentity, IdentityPublicKey) {
        let staged = stage_identity_with_vaulted_keys([0x77; 32], [0x88; 32]).await;
        let voting = IdentityPublicKey::random_key(7, Some(7), PlatformVersion::latest());
        let voter = Identity::create_basic_identity(
            Identifier::from([0x3C; 32]),
            PlatformVersion::latest(),
        )
        .unwrap();
        IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .store(&V, voting.id(), &VOTING_SECRET)
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
        let staged = stage_identity_with_voter_twin([0x77; 32], [0x88; 32]).await;
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
        let staged = stage_identity_with_vaulted_keys([0x77; 32], [0x88; 32]).await;

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
