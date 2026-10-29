//! The key hashes a masternode or evonode registered, as its Platform
//! identity carries them.
//!
//! A masternode's Platform identity (id = its ProTxHash) mirrors two of the
//! keys from its ProRegTx: the owner key as an `OWNER` key and the payout
//! address as a `TRANSFER` key, both stored as the 20-byte hash160 of the
//! public key (`ECDSA_HASH160`). Those hashes are what a wallet compares its
//! provider keys against to tell whether it holds them. Pure data — no IO.

use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, IdentityPublicKey, KeyType, Purpose};

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
    use dash_sdk::dpp::platform_value::{BinaryData, Identifier};
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
}
