use super::qualified_identity::PrivateKeyTarget;
use super::qualified_identity::QualifiedIdentity;
use super::wallet::{WalletSeedHash, auth_pubkey_cache::AuthPubkeyCache};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath, KeyDerivationType};
use std::collections::BTreeSet;

/// Stay inside the seed-recovery lookup window and bound derivation work.
pub fn derivation_index_limit(max_key_id: u32) -> u32 {
    max_key_id.saturating_add(6).min(4096)
}

/// Select the lowest unused index within the recoverable range.
pub fn first_free_index(limit: u32, occupied: &BTreeSet<u32>) -> Option<u32> {
    (0..limit).find(|index| !occupied.contains(index))
}

/// Resolve an unambiguous wallet and identity index from canonical ECDSA paths.
pub fn derivation_wallet(
    identity: &QualifiedIdentity,
    network: Network,
) -> Option<(WalletSeedHash, u32)> {
    let mut found = None;
    for (target, key) in identity.private_keys.identity_public_keys() {
        if *target != PrivateKeyTarget::PrivateKeyOnMainIdentity {
            continue;
        }
        let Some(path) = &key.in_wallet_at_derivation_path else {
            continue;
        };
        let children = path.derivation_path.as_ref();
        let [
            ..,
            ChildNumber::Hardened {
                index: identity_index,
            },
            ChildNumber::Hardened { index: key_index },
        ] = children
        else {
            continue;
        };
        if path.derivation_path
            != DerivationPath::identity_authentication_path(
                network,
                KeyDerivationType::ECDSA,
                *identity_index,
                *key_index,
            )
        {
            continue;
        }
        if identity
            .wallet_index
            .is_some_and(|index| index != *identity_index)
        {
            return None;
        }
        let coordinate = (path.wallet_seed_hash, *identity_index);
        if found.is_some_and(|previous| previous != coordinate) {
            return None;
        }
        found = Some(coordinate);
    }
    found.filter(|(seed_hash, _)| identity.associated_wallets.contains_key(seed_hash))
}

/// Include disabled keys and hash160 aliases: reusing their private key is unsafe.
pub fn occupied_indices(
    identity: &QualifiedIdentity,
    network: Network,
    seed_hash: WalletSeedHash,
    identity_index: u32,
    cache: &AuthPubkeyCache,
) -> BTreeSet<u32> {
    let mut occupied = BTreeSet::new();
    for (_, key) in identity.private_keys.identity_public_keys() {
        if let Some(path) = &key.in_wallet_at_derivation_path
            && path.wallet_seed_hash == seed_hash
            && let Some(ChildNumber::Hardened { index }) = path.derivation_path.as_ref().last()
            && path.derivation_path
                == DerivationPath::identity_authentication_path(
                    network,
                    KeyDerivationType::ECDSA,
                    identity_index,
                    *index,
                )
        {
            occupied.insert(*index);
        }
    }
    let hashes: BTreeSet<_> = identity
        .identity
        .public_keys()
        .values()
        .filter_map(|key| key.public_key_hash().ok())
        .collect();
    for index in 0..derivation_index_limit(identity.identity.get_public_key_max_id()) {
        if let Some(key) = cache.get(network, identity_index, index)
            && hashes.contains::<[u8; 20]>(&key.pubkey_hash().into())
        {
            occupied.insert(index);
        }
    }
    occupied
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    use super::super::qualified_identity::encrypted_key_storage::{
        KeyStorage, PrivateKeyData, WalletDerivationPath,
    };
    use super::super::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use super::super::qualified_identity::{IdentityStatus, IdentityType};
    use super::super::wallet::Wallet;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, Identity};
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};

    pub(crate) fn fixture() -> (QualifiedIdentity, AuthPubkeyCache, WalletSeedHash, [u8; 64]) {
        let seed = [42; 64];
        let wallet = Wallet::new_from_seed(seed, Network::Testnet, None, None).unwrap();
        let seed_hash = wallet.seed_hash();
        let mut cache = AuthPubkeyCache::default();
        for index in 0..8 {
            let key = wallet
                .identity_authentication_ecdsa_public_key_from_seed(
                    &seed,
                    Network::Testnet,
                    0,
                    index,
                )
                .unwrap();
            cache.insert(Network::Testnet, 0, index, &key);
        }
        let mut identity = QualifiedIdentity {
            identity: Identity::create_basic_identity(
                Identifier::from([1; 32]),
                PlatformVersion::latest(),
            )
            .unwrap(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(seed_hash, Arc::new(RwLock::new(wallet)))]),
            secret_access: None,
            wallet_index: Some(0),
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        let key = IdentityPublicKeyV0 {
            id: 0,
            key_type: KeyType::ECDSA_SECP256K1,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::MASTER,
            contract_bounds: None,
            read_only: false,
            disabled_at: None,
            data: cache.get(Network::Testnet, 0, 0).unwrap().to_bytes().into(),
        };
        identity.identity.add_public_key(key.clone().into());
        let path = WalletDerivationPath {
            wallet_seed_hash: seed_hash,
            derivation_path: DerivationPath::identity_authentication_path(
                Network::Testnet,
                KeyDerivationType::ECDSA,
                0,
                0,
            ),
        };
        identity.private_keys.insert_if_absent(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
            (
                QualifiedIdentityPublicKey {
                    identity_public_key: key.into(),
                    in_wallet_at_derivation_path: Some(path.clone()),
                },
                PrivateKeyData::AtWalletDerivationPath(path),
            ),
        );
        (identity, cache, seed_hash, seed)
    }
}

#[cfg(test)]
mod tests {
    use super::super::qualified_identity::encrypted_key_storage::KeyStorage;
    use super::test_support::fixture;
    use super::*;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
    #[test]
    fn disabled_hash160_without_metadata_reserves_its_derived_index() {
        let (mut identity, cache, seed_hash, _) = fixture();
        let hash: [u8; 20] = cache
            .get(Network::Testnet, 0, 2)
            .unwrap()
            .pubkey_hash()
            .into();
        identity.identity.add_public_key(
            IdentityPublicKeyV0 {
                id: 4,
                key_type: KeyType::ECDSA_HASH160,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::HIGH,
                contract_bounds: None,
                read_only: false,
                disabled_at: Some(1),
                data: hash.to_vec().into(),
            }
            .into(),
        );
        let occupied = occupied_indices(&identity, Network::Testnet, seed_hash, 0, &cache);
        assert_eq!(occupied, BTreeSet::from([0, 2]));
        assert_eq!(first_free_index(10, &occupied), Some(1));
    }

    #[test]
    fn wallet_resolution_rejects_wrong_network_and_identity_index() {
        let (mut identity, _, seed_hash, _) = fixture();
        assert_eq!(
            derivation_wallet(&identity, Network::Testnet),
            Some((seed_hash, 0))
        );
        assert_eq!(derivation_wallet(&identity, Network::Mainnet), None);
        identity.wallet_index = Some(1);
        assert_eq!(derivation_wallet(&identity, Network::Testnet), None);
    }

    #[test]
    fn wallet_path_roundtrip_recovers_the_signing_key_from_seed() {
        let (identity, cache, _, seed) = fixture();
        let bytes =
            bincode::encode_to_vec(&identity.private_keys, bincode::config::standard()).unwrap();
        let (restored, _): (KeyStorage, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        let wallets = identity
            .associated_wallets
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let (public, secret) = restored
            .get_resolve_with_seed(
                &(PrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
                &wallets,
                &seed,
                Network::Testnet,
            )
            .unwrap()
            .unwrap();
        assert!(
            public
                .identity_public_key
                .validate_private_key_bytes(&secret, Network::Testnet)
                .unwrap()
        );
        assert_eq!(
            public.identity_public_key.data().as_slice(),
            cache.get(Network::Testnet, 0, 0).unwrap().to_bytes()
        );
    }

    #[test]
    fn first_free_fills_holes_and_excludes_used_indices() {
        let occupied = BTreeSet::from([0, 2, 3]);
        assert_eq!(first_free_index(5, &occupied), Some(1));
        assert_eq!(first_free_index(1, &occupied), None);
    }

    #[test]
    fn recovery_window_is_bounded_and_includes_new_key() {
        assert_eq!(derivation_index_limit(1), 7);
        assert_eq!(derivation_index_limit(u32::MAX), 4096);
    }
}
