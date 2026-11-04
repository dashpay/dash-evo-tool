//! The key hashes a masternode or evonode registered, as its Platform
//! identity carries them.
//!
//! A masternode's Platform identity (id = its ProTxHash) mirrors two of the
//! keys from its ProRegTx: the owner key as an `OWNER` key and the payout
//! address as a `TRANSFER` key, both stored as the 20-byte hash160 of the
//! public key (`ECDSA_HASH160`). Those hashes are what a wallet compares its
//! provider keys against to tell whether it holds them. Pure data — no IO.

use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, IdentityPublicKey, KeyID, KeyType, Purpose};
use dash_sdk::platform::Identifier;
use std::collections::BTreeSet;

/// The hash160 of each masternode key a wallet can be checked against.
/// `None` for a key the node does not expose in a wallet-matchable form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MasternodeKeyHashes {
    /// The owner key, from the identity's active `OWNER` key.
    pub owner: Option<[u8; 20]>,
    /// The key behind a P2PKH payout address, from the identity's active
    /// `TRANSFER` key. `None` for a script-hash (P2SH) payout, which no single
    /// wallet key controls.
    pub payout: Option<[u8; 20]>,
    /// The voting key. Not on the masternode identity; read from the
    /// masternode list by the caller.
    pub voting: Option<[u8; 20]>,
}

impl MasternodeKeyHashes {
    /// The owner and payout hashes of a masternode identity. `voting` is left
    /// `None` for the caller to fill from the masternode list.
    pub fn from_identity(identity: &Identity) -> Self {
        Self {
            owner: active_key_hash(identity, Purpose::OWNER),
            payout: active_key_hash(identity, Purpose::TRANSFER),
            voting: None,
        }
    }

    /// Whether there is anything to look for.
    pub fn is_empty(&self) -> bool {
        self.owner.is_none() && self.payout.is_none() && self.voting.is_none()
    }
}

/// Which masternode key signs a credit withdrawal. Platform treats them
/// differently: the owner key may only pay the registered payout address,
/// while the payout key may pay any address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasternodeWithdrawalRole {
    /// The owner key (identity purpose `OWNER`).
    Owner,
    /// The payout-address key (identity purpose `TRANSFER`).
    Payout,
}

impl MasternodeWithdrawalRole {
    /// The role a withdrawal signed with `key` uses, or `None` when the key
    /// cannot sign a masternode withdrawal.
    pub fn for_key(key: &IdentityPublicKey) -> Option<Self> {
        match key.purpose() {
            Purpose::OWNER => Some(Self::Owner),
            Purpose::TRANSFER => Some(Self::Payout),
            _ => None,
        }
    }
}

/// A masternode withdrawal signed with a key one of this device's wallets
/// holds, so it can go through the wallet's own masternode withdrawal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletMasternodeWithdrawal {
    /// The wallet holding the signing key.
    pub wallet_seed_hash: WalletSeedHash,
    /// Which masternode key signs.
    pub role: MasternodeWithdrawalRole,
    /// The node's key hashes; the signing role's hash is the signing key's.
    pub key_hashes: MasternodeKeyHashes,
    /// For an owner-key withdrawal, the owner key's provider index as its
    /// stored derivation path records it. The wallet checks this index
    /// first, so an owner key above its default scan window still resolves
    /// after a restore. `None` for a payout-key withdrawal.
    pub owner_key_index_hint: Option<u32>,
}

/// The voting state a node stores that the masternode list no longer names:
/// what removing an old voting key takes away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetiredVotingKeys {
    /// The stored voter identity votes with a key other than the current one.
    pub voter_identity: bool,
    /// Stored voting keys whose hash is not the current voting key's.
    pub placements: Vec<(PrivateKeyTarget, KeyID)>,
}

impl RetiredVotingKeys {
    /// Whether the node stores nothing but its current voting key.
    pub fn is_empty(&self) -> bool {
        !self.voter_identity && self.placements.is_empty()
    }
}

/// Whether `node` stores any voting state — a voter identity or a key on it —
/// that the masternode list could have retired.
pub fn stores_voting_key(node: &QualifiedIdentity) -> bool {
    node.associated_voter_identity.is_some()
        || node
            .private_keys
            .identity_public_keys()
            .into_iter()
            .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity)
}

/// The stored voting state of `node` that differs from `current`, the voting
/// key hash the masternode list names for it. Covers the voter identity's key
/// and every key stored on the voter identity; main-identity keys are never
/// voting keys of a node and are left out.
pub fn retired_voting_keys(node: &QualifiedIdentity, current: [u8; 20]) -> RetiredVotingKeys {
    let retired = |key: &IdentityPublicKey| key.public_key_hash().ok() != Some(current);
    RetiredVotingKeys {
        voter_identity: node
            .associated_voter_identity
            .as_ref()
            .is_some_and(|(_, key)| retired(key)),
        placements: node
            .private_keys
            .identity_public_keys()
            .into_iter()
            .filter(|(target, key)| {
                **target == PrivateKeyTarget::PrivateKeyOnVoterIdentity
                    && retired(&key.identity_public_key)
            })
            .map(|(target, key)| (target.clone(), key.identity_public_key.id()))
            .collect(),
    }
}

/// What a node's record says about its voting keys: the voter identity with
/// its key's hash, and the hash of every key stored on the voter identity.
/// Compared to tell whether a record's voting state changed between two reads.
/// A key whose hash cannot be computed counts as `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVotingKeys {
    voter_identity: Option<(Identifier, Option<[u8; 20]>)>,
    placements: BTreeSet<(KeyID, Option<[u8; 20]>)>,
}

/// The voting keys `node` stores right now.
pub fn stored_voting_keys(node: &QualifiedIdentity) -> StoredVotingKeys {
    StoredVotingKeys {
        voter_identity: node
            .associated_voter_identity
            .as_ref()
            .map(|(voter, key)| (voter.id(), key.public_key_hash().ok())),
        placements: node
            .private_keys
            .identity_public_keys()
            .into_iter()
            .filter(|(target, _)| **target == PrivateKeyTarget::PrivateKeyOnVoterIdentity)
            .map(|(_, key)| {
                let key = &key.identity_public_key;
                (key.id(), key.public_key_hash().ok())
            })
            .collect(),
    }
}

/// hash160 data of the first active `ECDSA_HASH160` key with `purpose`.
fn active_key_hash(identity: &Identity, purpose: Purpose) -> Option<[u8; 20]> {
    identity
        .public_keys()
        .values()
        .filter(|key| {
            key.purpose() == purpose
                && key.key_type() == KeyType::ECDSA_HASH160
                && !key.is_disabled()
        })
        .find_map(|key| <[u8; 20]>::try_from(key.data().as_slice()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::identity::SecurityLevel;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::v0::IdentityV0;
    use dash_sdk::dpp::platform_value::BinaryData;
    use std::collections::BTreeMap;

    fn key(
        id: u32,
        purpose: Purpose,
        key_type: KeyType,
        data: Vec<u8>,
        disabled: bool,
    ) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose,
            security_level: SecurityLevel::CRITICAL,
            contract_bounds: None,
            key_type,
            read_only: false,
            data: BinaryData::new(data),
            disabled_at: disabled.then_some(1),
        })
    }

    fn identity(keys: Vec<IdentityPublicKey>) -> Identity {
        Identity::V0(IdentityV0 {
            id: Identifier::from([9u8; 32]),
            public_keys: keys
                .into_iter()
                .map(|k| (k.id(), k))
                .collect::<BTreeMap<_, _>>(),
            balance: 0,
            revision: 0,
        })
    }

    #[test]
    fn hashes_come_from_the_active_owner_and_transfer_keys() {
        let id = identity(vec![
            key(
                0,
                Purpose::OWNER,
                KeyType::ECDSA_HASH160,
                vec![0x01; 20],
                true,
            ),
            key(
                1,
                Purpose::OWNER,
                KeyType::ECDSA_HASH160,
                vec![0x02; 20],
                false,
            ),
            key(
                2,
                Purpose::TRANSFER,
                KeyType::ECDSA_HASH160,
                vec![0x03; 20],
                false,
            ),
        ]);
        let hashes = MasternodeKeyHashes::from_identity(&id);
        assert_eq!(hashes.owner, Some([0x02; 20]), "disabled owner key skipped");
        assert_eq!(hashes.payout, Some([0x03; 20]));
        assert_eq!(hashes.voting, None);
        assert!(!hashes.is_empty());
    }

    #[test]
    fn script_hash_payout_has_no_wallet_key() {
        let id = identity(vec![key(
            0,
            Purpose::TRANSFER,
            KeyType::BIP13_SCRIPT_HASH,
            vec![0x04; 20],
            false,
        )]);
        let hashes = MasternodeKeyHashes::from_identity(&id);
        assert_eq!(hashes.payout, None);
        assert!(hashes.is_empty());
    }

    #[test]
    fn withdrawal_role_follows_key_purpose() {
        let owner = key(
            0,
            Purpose::OWNER,
            KeyType::ECDSA_HASH160,
            vec![0; 20],
            false,
        );
        let payout = key(
            1,
            Purpose::TRANSFER,
            KeyType::ECDSA_HASH160,
            vec![0; 20],
            false,
        );
        let auth = key(
            2,
            Purpose::AUTHENTICATION,
            KeyType::ECDSA_SECP256K1,
            vec![0; 33],
            false,
        );
        assert_eq!(
            MasternodeWithdrawalRole::for_key(&owner),
            Some(MasternodeWithdrawalRole::Owner)
        );
        assert_eq!(
            MasternodeWithdrawalRole::for_key(&payout),
            Some(MasternodeWithdrawalRole::Payout)
        );
        assert_eq!(MasternodeWithdrawalRole::for_key(&auth), None);
    }

    /// A node with `voting` stored on its voter identity and that identity
    /// loaded, plus an owner key on the main identity.
    fn node_with_voter(voting: &IdentityPublicKey) -> QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType};
        let owner = key(
            0,
            Purpose::OWNER,
            KeyType::ECDSA_HASH160,
            vec![0x05; 20],
            false,
        );
        let mut private_keys = KeyStorage::default();
        for (target, public) in [
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, owner),
            (PrivateKeyTarget::PrivateKeyOnVoterIdentity, voting.clone()),
        ] {
            private_keys.insert_at(
                (target, public.id()),
                (
                    QualifiedIdentityPublicKey::from(public),
                    PrivateKeyData::Clear(rand::random()),
                ),
            );
        }
        QualifiedIdentity {
            identity: identity(vec![]),
            associated_voter_identity: Some((identity(vec![voting.clone()]), voting.clone())),
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
    }

    fn voting_key(id: u32, hash: u8) -> IdentityPublicKey {
        key(
            id,
            Purpose::VOTING,
            KeyType::ECDSA_HASH160,
            vec![hash; 20],
            false,
        )
    }

    /// The list names another voting key: the voter identity and the stored
    /// voting key are retired, the owner key on the main identity is not.
    #[test]
    fn a_voting_key_the_list_no_longer_names_is_retired() {
        let node = node_with_voter(&voting_key(1, 0x21));

        let retired = retired_voting_keys(&node, [0x22; 20]);

        assert!(retired.voter_identity);
        assert_eq!(
            retired.placements,
            vec![(PrivateKeyTarget::PrivateKeyOnVoterIdentity, 1)]
        );
        assert!(!retired.is_empty());
    }

    /// The list still names the stored voting key: nothing is retired.
    #[test]
    fn the_current_voting_key_is_not_retired() {
        let node = node_with_voter(&voting_key(1, 0x21));

        assert!(retired_voting_keys(&node, [0x21; 20]).is_empty());
    }

    /// Only the stored voting keys that differ are retired: a second, current
    /// one stays, and so does a voter identity voting with it.
    #[test]
    fn only_the_differing_voting_keys_are_retired() {
        use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        let current = voting_key(2, 0x22);
        let mut node = node_with_voter(&current);
        node.private_keys.insert_at(
            (PrivateKeyTarget::PrivateKeyOnVoterIdentity, 1),
            (
                QualifiedIdentityPublicKey::from(voting_key(1, 0x21)),
                PrivateKeyData::Clear(rand::random()),
            ),
        );

        let retired = retired_voting_keys(&node, [0x22; 20]);

        assert!(!retired.voter_identity);
        assert_eq!(
            retired.placements,
            vec![(PrivateKeyTarget::PrivateKeyOnVoterIdentity, 1)]
        );
    }

    /// The comparison sees a new voting key, whichever way the record names
    /// it, and ignores everything that is not voting state.
    #[test]
    fn stored_voting_keys_change_only_with_the_voting_state() {
        use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        let node = node_with_voter(&voting_key(1, 0x21));
        let before = stored_voting_keys(&node);

        let mut renamed = node.clone();
        renamed.alias = Some("renamed".to_owned());
        assert_eq!(stored_voting_keys(&renamed), before);

        let mut new_voter_key = node.clone();
        let (_, voter_key) = new_voter_key.associated_voter_identity.as_mut().unwrap();
        *voter_key = voting_key(1, 0x22);
        assert_ne!(stored_voting_keys(&new_voter_key), before);

        let mut new_placement = node;
        new_placement.private_keys.insert_at(
            (PrivateKeyTarget::PrivateKeyOnVoterIdentity, 2),
            (
                QualifiedIdentityPublicKey::from(voting_key(2, 0x22)),
                PrivateKeyData::Clear(rand::random()),
            ),
        );
        assert_ne!(stored_voting_keys(&new_placement), before);
    }
}
