//! A masternode voting key the masternode list no longer names.
//!
//! When a node's voting key is rotated, the key and voter identity this
//! device stores for it stop being the node's current ones. DET never removes
//! them on its own: the list it compares against can lag the chain (see
//! `WalletBackend::masternode_voting_key_hash`), so a stale list could make
//! the current key look retired. Instead the node's page asks
//! [`AppContext::check_retired_voting_key`] whether the stored key still
//! matches, and the user removes an old one with
//! [`AppContext::remove_retired_voting_key`], which then looks in the wallets
//! for the current one.

use super::{IdentityInputToLoad, IdentityLoadMode};
use crate::backend_task::{BackendTaskSuccessResult, CacheDomain, TaskError, TaskResult};
use crate::context::AppContext;
use crate::model::masternode_keys::{
    StoredVotingKeys, VotingKeyAvailability, retired_voting_keys, stored_voting_keys,
    stores_voting_key, voting_key_hash,
};
use crate::model::qualified_identity::encrypted_key_storage::{PrivateKeyData, same_key};
use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use crate::model::secret::Secret;
use crate::utils::egui_mpsc::SenderAsync;
use crate::wallet_backend::IdentityKeyView;
use crate::wallet_backend::secret_seam::SecretScheme;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::ProTxHash;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{IdentityPublicKey, KeyID};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use dash_sdk::platform::transition::masternode_vote_keys::{select_voting_key, voter_identity_id};
use std::collections::BTreeSet;

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
    ///
    /// The stored voting keys are read before the list. If they are the same
    /// under the record guard, the list read is at least as new as whatever
    /// reload stored them, so it can judge them. If not, it cannot, and
    /// nothing is removed.
    ///
    /// After removing an old key, try to find the current one in the wallets
    /// and check whether the device can use it. The search skips automatic
    /// matching when the node already stores the current key.
    pub(super) async fn remove_retired_voting_key(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        sender: &SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let node = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let expected = stored_voting_keys(&node);
        let current = self.current_voting_key_hash(identity_id).await;
        let mut result = self.remove_retired_voting_key_against(identity_id, current, &expected)?;
        if let BackendTaskSuccessResult::RetiredVotingKeyRemoved { successor, .. } = &mut result {
            self.find_current_voting_key_in_wallets(sdk, identity_id, node.identity_type)
                .await;
            *successor = self.current_voting_key_availability(identity_id).await;
            self.notify_cache_changed(CacheDomain::Identities, sender)
                .await;
        }
        Ok(result)
    }

    async fn current_voting_key_availability(
        &self,
        identity_id: Identifier,
    ) -> VotingKeyAvailability {
        let Some(current) = self.current_voting_key_hash(identity_id).await else {
            return VotingKeyAvailability::Unverified;
        };
        let node = match self.get_local_qualified_identity(&identity_id) {
            Ok(Some(node)) => node,
            Ok(None) => return VotingKeyAvailability::Unverified,
            Err(error) => {
                tracing::warn!(?error, %identity_id, "Could not check the node's saved successor voting key");
                return VotingKeyAvailability::Unverified;
            }
        };
        let Some((voter, key)) = &node.associated_voter_identity else {
            return VotingKeyAvailability::Missing;
        };
        if !select_voting_key(voter, &current).is_some_and(|current| same_key(&current, key))
            || key.is_disabled()
        {
            return VotingKeyAvailability::Missing;
        }
        let secret_store = self.secret_store();
        let vault = IdentityKeyView::new(&secret_store, identity_id.to_buffer());
        let mut availability = VotingKeyAvailability::Missing;
        for placement in node.private_keys.candidates(key) {
            let Some((_, data)) = node.private_keys.entry_at(&placement) else {
                continue;
            };
            match data {
                PrivateKeyData::InVault => match vault.scheme(&placement.0, placement.1) {
                    Ok(SecretScheme::Absent) => {}
                    Ok(_) => return VotingKeyAvailability::Available,
                    Err(error) => {
                        tracing::warn!(?error, %identity_id, "Could not check the vault for the successor voting key");
                        availability = VotingKeyAvailability::Unverified;
                    }
                },
                data if node.key_secret_is_obtainable(data) => {
                    return VotingKeyAvailability::Available;
                }
                _ => {}
            }
        }
        availability
    }

    /// Merge the node's current voting key from a wallet that holds it, by
    /// loading the node again with nothing typed.
    ///
    /// Best effort: the old key is already gone, so a failure is only logged
    /// and the next load of the node can still find the key.
    async fn find_current_voting_key_in_wallets(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        identity_type: IdentityType,
    ) {
        let input = IdentityInputToLoad {
            identity_id_input: identity_id.to_string(Encoding::Hex),
            identity_type,
            alias_input: String::new(),
            voting_private_key_input: Secret::default(),
            owner_private_key_input: Secret::default(),
            payout_address_private_key_input: Secret::default(),
            keys_input: Vec::new(),
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::MergeIntoExisting,
            load_token: None,
        };
        if let Err(error) = self.load_identity(sdk, input).await {
            tracing::warn!(
                ?error,
                %identity_id,
                "Could not look for the current voting key in this device's wallets after removing the old one"
            );
        }
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
    /// hash just read from the masternode list, and `expected`, the voting
    /// keys the record named before that read.
    ///
    /// Everything is decided and done under the node's record guard. A
    /// concurrent reload that changed the voting keys since `expected` was
    /// read fails the removal with nothing removed, because `current` may
    /// predate the key that reload stored. Lock order: the
    /// storage-migration gate, the node's record guard, then the record guard
    /// of a separate voter record (as identity removal takes them). This is
    /// the only place two record guards nest; the gate serializes it against
    /// every identity removal, and nothing takes a voter's guard before its
    /// node's.
    ///
    /// A separate voter record an older version kept is handled first. A
    /// failure before that record is delisted leaves the node ready for a
    /// retry, and any cleanup after delisting can resume on startup. For
    /// individual placements, delete vault secrets before removing their
    /// entries so a failed deletion can still be retried.
    fn remove_retired_voting_key_against(
        &self,
        identity_id: Identifier,
        current: Option<[u8; 20]>,
        expected: &StoredVotingKeys,
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
        if stored_voting_keys(&node) != *expected {
            return Err(TaskError::VotingKeyChangedDuringRemoval);
        }
        let retired = retired_voting_keys(&node, current);
        if retired.is_empty() {
            return Ok(BackendTaskSuccessResult::RetiredVotingKeyKept { identity_id });
        }
        let mut cleanup_pending = false;
        if retired.voter_identity
            && let Some((voter, voting_key)) = node.associated_voter_identity.as_ref()
            && voter.id() != identity_id
        {
            cleanup_pending =
                self.remove_voter_record_locked(identity_id, voter.id(), voting_key)?;
        }
        self.remove_voting_placements(&mut node, &retired.placements)?;
        if retired.voter_identity {
            node.associated_voter_identity = None;
        }
        self.write_local_qualified_identity_locked(&node)?;
        Ok(BackendTaskSuccessResult::RetiredVotingKeyRemoved {
            identity_id,
            cleanup_pending,
            successor: VotingKeyAvailability::Unverified,
        })
    }

    /// Remove exact copies from the voter record an older version kept.
    /// Delist only a dedicated copy whose complete vault inventory can be
    /// attributed to this voting key. Returns whether startup must resume
    /// cleanup after a completed delisting.
    ///
    /// The removal cancels vote targets filed under `voter_id` itself. That
    /// touches no schedule the node can still cast: those are keyed by the
    /// node's own id (its ProTxHash) and stay scheduled.
    fn remove_voter_record_locked(
        &self,
        node_id: Identifier,
        voter_id: Identifier,
        voting_key: &IdentityPublicKey,
    ) -> Result<bool, TaskError> {
        let pro_tx_hash = ProTxHash::from_byte_array(node_id.to_buffer());
        if !voting_key_hash(voting_key)
            .is_some_and(|hash| voter_identity_id(pro_tx_hash, &hash) == voter_id)
        {
            return Ok(false);
        }
        let lock = self.identity_record_lock(voter_id);
        let _record_guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.is_identity_listed(&voter_id)? {
            return self.identity_removal_cleanup_pending(voter_id);
        }
        let Some(mut voter) = self.get_local_qualified_identity(&voter_id)? else {
            return Ok(false);
        };
        if !voter
            .identity
            .get_public_key_by_id(voting_key.id())
            .is_some_and(|key| same_key(key, voting_key))
        {
            return Ok(false);
        }
        // A matching key also published by another of this record's identities
        // cannot be attributed to the old voter alone.
        if voter
            .associated_voter_identity
            .iter()
            .chain(voter.associated_operator_identity.iter())
            .any(|(identity, _)| {
                identity
                    .get_public_key_by_id(voting_key.id())
                    .is_some_and(|key| same_key(key, voting_key))
            })
        {
            return Ok(false);
        }
        let matches: Vec<_> = voter.private_keys.candidates(voting_key).collect();
        let dedicated = voter.identity.public_keys().len() == 1
            && voter.identity_type == IdentityType::User
            && voter.associated_voter_identity.is_none()
            && voter.associated_operator_identity.is_none()
            && self.stored_identity_wallet_link(&voter_id)?.is_none()
            && self
                .identity_removal_key_placements(voter_id)?
                .is_subset(&vaulted_placements(&voter, &matches));
        if !dedicated {
            if !matches.is_empty() {
                self.remove_voting_placements(&mut voter, &matches)?;
                self.write_local_qualified_identity_locked(&voter)?;
            }
            return Ok(false);
        }
        match self.delete_local_qualified_identity_locked(&voter_id) {
            Ok(cleanup) => Ok(cleanup.is_incomplete()),
            Err(error) if self.is_identity_listed(&voter_id).unwrap_or(true) => Err(error),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    %voter_id,
                    "Old voter identity removed but its vault cleanup is still pending; the next start finishes it"
                );
                Ok(true)
            }
        }
    }

    /// Delete the secret before its placement so a failed cleanup can be
    /// retried. A legacy or wallet reference does not identify a vault slot.
    fn remove_voting_placements(
        &self,
        identity: &mut QualifiedIdentity,
        placements: &[(PrivateKeyTarget, KeyID)],
    ) -> Result<(), TaskError> {
        self.delete_identity_key_secrets(
            &identity.identity.id(),
            vaulted_placements(identity, placements),
        )?;
        for placement in placements {
            identity.private_keys.remove_at(placement);
        }
        Ok(())
    }
}

fn vaulted_placements(
    identity: &QualifiedIdentity,
    placements: &[(PrivateKeyTarget, KeyID)],
) -> BTreeSet<(PrivateKeyTarget, KeyID)> {
    placements
        .iter()
        .filter(|placement| identity.private_keys.is_in_vault(placement))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_staging::{
        StagedIdentity, fail_removals_after_delisting, stage_identity_with_vaulted_keys,
        stage_identity_with_voter_twin,
    };
    use crate::context::test_support::{bare_user_identity, identity_public_key};
    use crate::model::dpns_voting::{
        DpnsVoteTarget, DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
    };
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::dashcore::{Network, PrivateKey, ProTxHash};
    use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::{Identity, IdentityPublicKey, KeyType, Purpose, SecurityLevel};
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;

    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;
    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const O: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnOperatorIdentity;

    fn key_for_secret(id: u32, purpose: Purpose, secret: &[u8; 32]) -> IdentityPublicKey {
        let hash = PrivateKey::from_byte_array(secret, Network::Testnet)
            .unwrap()
            .public_key()
            .pubkey_hash()
            .to_byte_array();
        identity_public_key(
            id,
            purpose,
            SecurityLevel::HIGH,
            KeyType::ECDSA_HASH160,
            hash.to_vec(),
        )
    }

    /// A stored masternode with vaulted main-identity keys and a voting key,
    /// held in the vault, on a loaded voter identity.
    async fn node_with_vaulted_voting_key() -> (StagedIdentity, IdentityPublicKey) {
        let staged = stage_identity_with_vaulted_keys(
            crate::test_support::keys::secret_bytes(),
            crate::test_support::keys::secret_bytes(),
        )
        .await;
        let secret = crate::test_support::keys::secret_bytes();
        let voting = key_for_secret(7, Purpose::VOTING, &secret);
        let mut voter = Identity::create_basic_identity(
            dash_sdk::platform::transition::masternode_vote_keys::voter_identity_id(
                ProTxHash::from_byte_array(staged.id.to_buffer()),
                &voting.public_key_hash().unwrap(),
            ),
            PlatformVersion::latest(),
        )
        .unwrap();
        voter.add_public_key(voting.clone());
        IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .store(&V, voting.id(), &secret)
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

    fn stage_legacy_voter(staged: &StagedIdentity, voting: &IdentityPublicKey) -> Identifier {
        let node = stored(staged);
        let (voter, _) = node.associated_voter_identity.as_ref().unwrap();
        let id = voter.id();
        let secret = IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .get(&V, voting.id())
            .unwrap()
            .unwrap();
        let mut mirror = bare_user_identity(id, Network::Testnet);
        mirror.identity = voter.clone();
        mirror.private_keys.insert_at(
            (M, voting.id()),
            (
                QualifiedIdentityPublicKey::from(voting.clone()),
                PrivateKeyData::Clear(*secret),
            ),
        );
        staged
            .ctx
            .insert_local_qualified_identity(&mirror, &None)
            .unwrap();
        id
    }

    #[tokio::test]
    async fn removal_preserves_an_unrelated_key_with_the_same_id_and_removes_voting_duplicates() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let key_id = voting.id();
        let vault = IdentityKeyView::new(&staged.store, staged.id.to_buffer());
        let voting_secret = vault.get(&V, key_id).unwrap().unwrap();
        let unrelated_secret = crate::test_support::keys::secret_bytes();
        let unrelated = key_for_secret(key_id, Purpose::AUTHENTICATION, &unrelated_secret);
        for target in [&M, &O] {
            vault.store(target, key_id, &voting_secret).unwrap();
        }
        vault.store(&V, key_id, &unrelated_secret).unwrap();
        staged
            .ctx
            .edit_local_qualified_identity(&staged.id, |node| {
                for (target, key) in [
                    (M, voting.clone()),
                    (O, voting.clone()),
                    (V, unrelated.clone()),
                ] {
                    node.private_keys.insert_at(
                        (target, key_id),
                        (
                            QualifiedIdentityPublicKey::from(key),
                            PrivateKeyData::InVault,
                        ),
                    );
                }
                Ok(())
            })
            .unwrap();

        staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .expect("remove the retired voting key");

        let node = stored(&staged);
        assert!(!node.private_keys.candidates(&voting).any(|_| true));
        assert!(node.private_keys.candidates(&unrelated).any(|_| true));
        assert_eq!(*vault.get(&V, key_id).unwrap().unwrap(), unrelated_secret);
        for target in [&M, &O] {
            assert!(vault.get(target, key_id).unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn unrelated_legacy_voter_slot_does_not_raise_a_retired_voting_key_notice() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let secret = crate::test_support::keys::secret_bytes();
        let other = key_for_secret(voting.id(), Purpose::AUTHENTICATION, &secret);
        staged
            .ctx
            .edit_local_qualified_identity(&staged.id, |node| {
                node.associated_voter_identity = None;
                node.private_keys.insert_at(
                    (V, voting.id()),
                    (
                        QualifiedIdentityPublicKey::from(other),
                        PrivateKeyData::Clear(secret),
                    ),
                );
                Ok(())
            })
            .unwrap();
        staged
            .ctx
            .wallet_backend()
            .unwrap()
            .set_masternode_voting_hash_for_test([0xEE; 20]);
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

    #[tokio::test]
    async fn retirement_warns_when_a_delisted_legacy_voters_cleanup_is_pending() {
        use crate::ui::ScreenLike;
        use crate::ui::components::message_banner::global_banner_texts;
        use crate::ui::masternodes::list_screen::MasternodesScreen;

        let (staged, voting) = node_with_vaulted_voting_key().await;
        let voter_id = stage_legacy_voter(&staged, &voting);
        fail_removals_after_delisting(&staged.ctx);
        let result = staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .expect("retirement committed despite pending cleanup");
        assert!(!staged.ctx.is_identity_listed(&voter_id).unwrap());
        assert!(
            IdentityKeyView::new(&staged.store, voter_id.to_buffer())
                .get(&M, voting.id())
                .unwrap()
                .is_some()
        );
        let mut screen = MasternodesScreen::new(&staged.ctx);
        screen.display_task_result(result);
        assert!(
            global_banner_texts(staged.ctx.egui_ctx())
                .iter()
                .any(|banner| banner.contains("Restart"))
        );
    }

    /// The list names another voting key: the old key, its vault secret and
    /// the voter identity go; the node's own keys stay.
    #[tokio::test]
    async fn removes_a_voting_key_the_list_no_longer_names() {
        let (staged, voting) = node_with_vaulted_voting_key().await;

        let result = staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { identity_id, .. } if identity_id == staged.id
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
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let voter_id = stage_legacy_voter(&staged, &voting);

        let result = staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { .. }
        ));
        assert!(stored(&staged).associated_voter_identity.is_none());
        assert!(!staged.ctx.is_identity_listed(&voter_id).unwrap());
        assert!(staged.ctx.is_identity_listed(&staged.id).unwrap());
    }

    #[tokio::test]
    async fn removes_only_the_exact_voting_copy_from_a_mixed_voter_record() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let voter_id = stage_legacy_voter(&staged, &voting);
        let secret = crate::test_support::keys::secret_bytes();
        let other = key_for_secret(voting.id(), Purpose::AUTHENTICATION, &secret);
        staged
            .ctx
            .edit_local_qualified_identity(&voter_id, |voter| {
                voter.private_keys.insert_at(
                    (O, voting.id()),
                    (
                        QualifiedIdentityPublicKey::from(other.clone()),
                        PrivateKeyData::Clear(secret),
                    ),
                );
                Ok(())
            })
            .unwrap();

        let result = staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .unwrap();

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved {
                cleanup_pending: false,
                ..
            }
        ));
        assert!(staged.ctx.is_identity_listed(&voter_id).unwrap());
        let voter = staged
            .ctx
            .get_local_qualified_identity(&voter_id)
            .unwrap()
            .unwrap();
        assert!(voter.private_keys.candidates(&other).next().is_some());
        assert!(voter.private_keys.candidates(&voting).next().is_none());
        let vault = IdentityKeyView::new(&staged.store, voter_id.to_buffer());
        assert_eq!(*vault.get(&O, other.id()).unwrap().unwrap(), secret);
        assert!(vault.get(&M, voting.id()).unwrap().is_none());
    }

    #[tokio::test]
    async fn unclassified_retained_voter_secret_is_preserved() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let voter_id = stage_legacy_voter(&staged, &voting);
        let hidden = crate::test_support::keys::secret_bytes();
        let vault = IdentityKeyView::new(&staged.store, voter_id.to_buffer());
        let lock = staged.ctx.identity_record_lock(voter_id);
        {
            let _guard = lock.lock().unwrap();
            staged
                .ctx
                .record_identity_import_keys(&voter_id, &BTreeSet::from([(O, 21)]))
                .unwrap();
            vault.store(&O, 21, &hidden).unwrap();
        }

        staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .unwrap();

        assert!(staged.ctx.is_identity_listed(&voter_id).unwrap());
        assert_eq!(*vault.get(&O, 21).unwrap().unwrap(), hidden);
        assert!(vault.get(&M, voting.id()).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_retired_voting_key_does_not_remove_a_copy_from_an_unrelated_voter() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let other_id = Identifier::from([0x4D; 32]);
        let mut other = bare_user_identity(other_id, Network::Testnet).identity;
        other.add_public_key(voting.clone());
        staged
            .ctx
            .edit_local_qualified_identity(&staged.id, |node| {
                node.associated_voter_identity = Some((other, voting.clone()));
                Ok(())
            })
            .unwrap();
        assert_eq!(stage_legacy_voter(&staged, &voting), other_id);

        staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .unwrap();

        assert!(stored(&staged).associated_voter_identity.is_none());
        assert!(!voting_secret_held(&staged, voting.id()));
        assert!(staged.ctx.is_identity_listed(&other_id).unwrap());
        assert!(
            IdentityKeyView::new(&staged.store, other_id.to_buffer())
                .get(&M, voting.id())
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn removing_a_retired_voting_key_reports_a_current_successor_already_saved() {
        let (staged, current) = node_with_vaulted_voting_key().await;
        let old_secret = crate::test_support::keys::secret_bytes();
        let old = key_for_secret(8, Purpose::VOTING, &old_secret);
        staged
            .ctx
            .edit_local_qualified_identity(&staged.id, |node| {
                node.private_keys.insert_at(
                    (V, old.id()),
                    (
                        QualifiedIdentityPublicKey::from(old.clone()),
                        PrivateKeyData::Clear(old_secret),
                    ),
                );
                Ok(())
            })
            .unwrap();
        staged
            .ctx
            .wallet_backend()
            .unwrap()
            .set_masternode_voting_hash_for_test(current.public_key_hash().unwrap());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let sender = SenderAsync::new(tx, staged.ctx.egui_ctx().clone());

        let outcome = staged
            .ctx
            .remove_retired_voting_key(&Sdk::new_mock(), staged.id, &sender)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            BackendTaskSuccessResult::RetiredVotingKeyRemoved {
                successor: VotingKeyAvailability::Available,
                ..
            }
        ));
        assert!(voting_secret_held(&staged, current.id()));
        assert!(!voting_secret_held(&staged, old.id()));

        IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .delete(&V, current.id())
            .unwrap();
        assert_eq!(
            staged.ctx.current_voting_key_availability(staged.id).await,
            VotingKeyAvailability::Missing
        );
    }

    /// A reload can switch the node's voter identity from A to B while the
    /// removal waits for the node's record. The list was read before that
    /// reload, so it cannot judge B: both voters and B's association stay.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_voter_switched_while_waiting_keeps_everything() {
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
        let expected = stored_voting_keys(&stored(&staged));
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
            ctx.remove_retired_voting_key_against(node_id, Some([0xEE; 20]), &expected)
        });
        let removal = probe.wait_for(
            LockSite::RecordRequest,
            removal,
            std::time::Duration::from_secs(30),
        );
        staged
            .ctx
            .write_local_qualified_identity_locked(&switched)
            .unwrap();
        drop(guard);
        let result = removal.join().unwrap();

        assert!(matches!(
            result,
            Err(TaskError::VotingKeyChangedDuringRemoval)
        ));
        assert_eq!(stored(&staged), switched);
        assert!(
            staged
                .ctx
                .is_identity_listed(&voter_b.identity.id())
                .unwrap(),
            "the voter the reload just stored stays"
        );
        assert!(
            staged.ctx.is_identity_listed(&voter_a).unwrap(),
            "nothing is decided from a list that predates the reload"
        );
    }

    /// A reload stores the node's newer voting key after the list was read,
    /// and the list read names another key. That key is not the one the list
    /// judged, so it and its vault secret survive.
    #[tokio::test]
    async fn a_voting_key_stored_after_the_list_was_read_survives() {
        let (staged, old) = node_with_vaulted_voting_key().await;
        let expected = stored_voting_keys(&stored(&staged));
        let newer = IdentityPublicKey::random_key(8, Some(8), PlatformVersion::latest());
        IdentityKeyView::new(&staged.store, staged.id.to_buffer())
            .store(&V, newer.id(), &rand::random::<[u8; 32]>())
            .unwrap();
        staged
            .ctx
            .edit_local_qualified_identity(&staged.id, |node| {
                node.private_keys.insert_at(
                    (V, newer.id()),
                    (
                        QualifiedIdentityPublicKey::from(newer.clone()),
                        PrivateKeyData::InVault,
                    ),
                );
                let (_, key) = node.associated_voter_identity.as_mut().unwrap();
                *key = newer.clone();
                Ok(())
            })
            .unwrap();
        let reloaded = stored(&staged);

        let result = staged.ctx.remove_retired_voting_key_against(
            staged.id,
            Some(old.public_key_hash().unwrap()),
            &expected,
        );

        assert!(matches!(
            result,
            Err(TaskError::VotingKeyChangedDuringRemoval)
        ));
        assert_eq!(stored(&staged), reloaded);
        assert!(voting_secret_held(&staged, newer.id()));
        assert!(voting_secret_held(&staged, old.id()));
    }

    /// A schedule belongs to the node, keyed by its ProTxHash: it outlives the
    /// voter record and is cast once the node's current voting key is added.
    /// Only a target filed under the old voter record's own id is cancelled,
    /// as nothing could ever cast it: that record has no voter of its own.
    #[tokio::test]
    async fn removing_an_old_voter_record_keeps_the_nodes_schedule() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let voter_id = stage_legacy_voter(&staged, &voting);
        let schedule = |voter_id: Identifier, poll: u8| {
            let mut operation = AppContext::new_dpns_vote_operation(vec![DpnsVoteTarget {
                key: DpnsVoteTargetKey {
                    network: Network::Testnet,
                    voter_id,
                    vote_poll_id: Identifier::from([poll; 32]),
                },
                voter_alias: None,
                contested_name: "contested".to_owned(),
                requested_choice: ResourceVoteChoice::Lock,
                current_choice: None,
                timing: VoteTiming::Scheduled(42),
            }]);
            operation.targets[0].status = DpnsVoteTargetStatus::Scheduled;
            staged
                .ctx
                .insert_dpns_vote_operation(&mut operation, None)
                .unwrap();
            operation.id
        };
        let node_schedule = schedule(staged.id, 1);
        let voter_schedule = schedule(voter_id, 2);

        staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some([0xEE; 20]),
                &stored_voting_keys(&stored(&staged)),
            )
            .unwrap();

        let status = |id| staged.ctx.dpns_vote_operation(id).unwrap().unwrap().targets[0].status;
        assert!(!staged.ctx.is_identity_listed(&voter_id).unwrap());
        assert_eq!(status(node_schedule), DpnsVoteTargetStatus::Scheduled);
        assert_eq!(status(voter_schedule), DpnsVoteTargetStatus::Cancelled);
    }

    /// By the time of the click the list names the stored key again: nothing
    /// is removed, and the result says the key was kept.
    #[tokio::test]
    async fn keeps_a_voting_key_that_is_current_again() {
        let (staged, voting) = node_with_vaulted_voting_key().await;
        let before = stored(&staged);

        let result = staged
            .ctx
            .remove_retired_voting_key_against(
                staged.id,
                Some(voting.public_key_hash().unwrap()),
                &stored_voting_keys(&before),
            )
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

        let result = staged.ctx.remove_retired_voting_key_against(
            staged.id,
            None,
            &stored_voting_keys(&before),
        );

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
