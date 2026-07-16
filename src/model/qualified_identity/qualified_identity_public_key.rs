use crate::model::qualified_identity::encrypted_key_storage::WalletDerivationPath;
use crate::model::wallet::Wallet;
use bincode::{Decode, Encode};
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, PubkeyHash, PublicKey};
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::{
    dashcore::Network, identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0,
};
use dash_sdk::platform::IdentityPublicKey;
use std::sync::{Arc, RwLock};

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct QualifiedIdentityPublicKey {
    pub identity_public_key: IdentityPublicKey,
    pub in_wallet_at_derivation_path: Option<WalletDerivationPath>,
}

impl From<IdentityPublicKey> for QualifiedIdentityPublicKey {
    fn from(value: IdentityPublicKey) -> Self {
        Self {
            identity_public_key: value,
            in_wallet_at_derivation_path: None,
        }
    }
}

impl QualifiedIdentityPublicKey {
    pub fn from_identity_public_key_in_wallet(
        identity_public_key: IdentityPublicKey,
        in_wallet_at_derivation_path: Option<WalletDerivationPath>,
    ) -> Self {
        Self {
            identity_public_key,
            in_wallet_at_derivation_path,
        }
    }
    /// Build a qualified key, linking it to a wallet derivation path when one of
    /// the key's candidate addresses is known to any of `wallets`.
    ///
    /// The key data is network-supplied and may be malformed; a key that cannot
    /// be parsed into an address is kept unlinked (logged and skipped) rather
    /// than panicking.
    pub fn from_identity_public_key_with_wallets_check(
        value: IdentityPublicKey,
        network: Network,
        wallets: &[&Arc<RwLock<Wallet>>],
    ) -> Self {
        let addresses = candidate_addresses(&value, network);
        let in_wallet_at_derivation_path = find_wallet_path(wallets, &addresses);
        Self {
            identity_public_key: value,
            in_wallet_at_derivation_path,
        }
    }
}

/// The addresses a key could resolve to on the active network (plus the Testnet
/// variant on non-mainnet networks). Empty when the key type carries no address
/// or its data is malformed.
fn candidate_addresses(value: &IdentityPublicKey, network: Network) -> Vec<Address> {
    let from_pubkey_hash = |pubkey_hash: PubkeyHash| {
        let mut addresses = vec![Address::new(network, Payload::PubkeyHash(pubkey_hash))];
        if network != Network::Mainnet {
            addresses.push(Address::new(
                Network::Testnet,
                Payload::PubkeyHash(pubkey_hash),
            ));
        }
        addresses
    };

    match value.key_type() {
        // A 20-byte payload is a pubkey hash carried on an ECDSA_SECP256K1 key.
        KeyType::ECDSA_SECP256K1 if value.data().len() == 20 => {
            match PubkeyHash::from_slice(value.data().as_slice()) {
                Ok(pubkey_hash) => from_pubkey_hash(pubkey_hash),
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping identity key with malformed 20-byte hash");
                    vec![]
                }
            }
        }
        KeyType::ECDSA_SECP256K1 => match PublicKey::from_slice(value.data().as_slice()) {
            Ok(pubkey) => {
                let mut addresses = vec![Address::p2pkh(&pubkey, network)];
                if network != Network::Mainnet {
                    addresses.push(Address::p2pkh(&pubkey, Network::Testnet));
                }
                addresses
            }
            Err(e) => {
                tracing::warn!(error = %e, "Skipping identity key with malformed public key");
                vec![]
            }
        },
        KeyType::ECDSA_HASH160 => match PubkeyHash::from_slice(value.data().as_slice()) {
            Ok(pubkey_hash) => from_pubkey_hash(pubkey_hash),
            Err(e) => {
                tracing::warn!(error = %e, "Skipping identity key with malformed 20-byte hash");
                vec![]
            }
        },
        _ => vec![],
    }
}

/// The stored derivation path of the first `addresses` entry known to any of
/// `wallets`, searched wallet-by-wallet then address-by-address.
fn find_wallet_path(
    wallets: &[&Arc<RwLock<Wallet>>],
    addresses: &[Address],
) -> Option<WalletDerivationPath> {
    for locked_wallet in wallets {
        let wallet = locked_wallet
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for address in addresses {
            if let Some(derivation_path) = wallet.known_addresses.get(address) {
                return Some(WalletDerivationPath {
                    wallet_seed_hash: wallet.seed_hash(),
                    derivation_path: derivation_path.clone(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
    use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
    use dash_sdk::dpp::platform_value::BinaryData;

    const TEST_SEED: [u8; 64] = [7u8; 64];

    /// A real curve public key keyed off `n`, so its p2pkh address is a valid
    /// point (not a fabricated hash).
    fn pubkey(n: u8) -> PublicKey {
        let mut sk_bytes = [3u8; 32];
        sk_bytes[31] = n.max(1);
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&sk_bytes).expect("valid secret key");
        PublicKey::new(sk.public_key(&secp))
    }

    /// An identity public key of `key_type` carrying `data`. `id`/`purpose`/
    /// `security_level` are immaterial to address resolution.
    fn identity_key(key_type: KeyType, data: Vec<u8>) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id: 0,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type,
            read_only: false,
            data: BinaryData::new(data),
            disabled_at: None,
        })
    }

    /// A wallet holding exactly `entries` in `known_addresses`, plus the fresh
    /// receive address `new_from_seed` seeds. Uses a distinct seed per `salt`
    /// so multi-wallet tests get distinct `seed_hash`es.
    fn wallet_with(salt: u8, entries: &[(Address, DerivationPath)]) -> Arc<RwLock<Wallet>> {
        let mut seed = TEST_SEED;
        seed[0] = salt;
        let mut wallet = Wallet::new_from_seed(seed, Network::Testnet, None, None).expect("wallet");
        for (address, path) in entries {
            wallet.known_addresses.insert(address.clone(), path.clone());
        }
        Arc::new(RwLock::new(wallet))
    }

    /// The DIP-13/15 identity-authentication path an identity's own auth key is
    /// registered at. These addresses live ONLY in DET's `known_addresses` —
    /// the upstream managed wallet has no account type for them, so they are
    /// absent from the display snapshot's `address_paths`. This is why the
    /// resolver must read the legacy map and cannot be migrated to the snapshot.
    fn identity_auth_path(identity_index: u32, key_index: u32) -> DerivationPath {
        DerivationPath::identity_authentication_path(
            Network::Testnet,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        )
    }

    /// The signing-critical case: a User identity's ECDSA authentication key
    /// (33-byte serialized pubkey) registered at its identity-authentication
    /// path resolves to the owning wallet's seed hash and that exact path — the
    /// linkage that lets the wallet later derive and sign with the key.
    #[test]
    fn resolves_identity_authentication_key_via_known_addresses() {
        let pk = pubkey(1);
        let address = Address::p2pkh(&pk, Network::Testnet);
        let path = identity_auth_path(0, 0);
        let wallet = wallet_with(1, &[(address.clone(), path.clone())]);
        let seed_hash = wallet.read().unwrap().seed_hash();

        let key = identity_key(KeyType::ECDSA_SECP256K1, pk.to_bytes());
        let resolved = QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
            key,
            Network::Testnet,
            &[&wallet],
        );

        let link = resolved
            .in_wallet_at_derivation_path
            .expect("key registered in known_addresses must link to a wallet path");
        assert_eq!(link.wallet_seed_hash, seed_hash);
        assert_eq!(link.derivation_path, path);
    }

    /// An ECDSA_HASH160 key (20-byte pubkey hash) resolves via the same address
    /// its hash yields.
    #[test]
    fn resolves_ecdsa_hash160_key() {
        let pk = pubkey(2);
        let address = Address::p2pkh(&pk, Network::Testnet);
        let path = identity_auth_path(0, 1);
        let wallet = wallet_with(2, &[(address.clone(), path.clone())]);

        let key = identity_key(
            KeyType::ECDSA_HASH160,
            pk.pubkey_hash().to_byte_array().to_vec(),
        );
        let resolved = QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
            key,
            Network::Testnet,
            &[&wallet],
        );

        assert_eq!(
            resolved
                .in_wallet_at_derivation_path
                .expect("hash160 key must link")
                .derivation_path,
            path
        );
    }

    /// An ECDSA_SECP256K1 key whose data is a 20-byte hash (not a full pubkey)
    /// resolves via the hash-payload branch of `candidate_addresses`.
    #[test]
    fn resolves_secp256k1_hash_payload_key() {
        let pk = pubkey(3);
        let address = Address::p2pkh(&pk, Network::Testnet);
        let path = identity_auth_path(1, 0);
        let wallet = wallet_with(3, &[(address.clone(), path.clone())]);

        let key = identity_key(
            KeyType::ECDSA_SECP256K1,
            pk.pubkey_hash().to_byte_array().to_vec(),
        );
        let resolved = QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
            key,
            Network::Testnet,
            &[&wallet],
        );

        assert!(resolved.in_wallet_at_derivation_path.is_some());
    }

    /// A key whose address is not in any wallet's `known_addresses` stays
    /// unlinked — the resolver must never fabricate a path.
    #[test]
    fn returns_none_when_key_address_is_absent() {
        let wallet = wallet_with(4, &[]);
        let stranger = pubkey(9);

        let key = identity_key(KeyType::ECDSA_SECP256K1, stranger.to_bytes());
        let resolved = QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
            key,
            Network::Testnet,
            &[&wallet],
        );

        assert!(resolved.in_wallet_at_derivation_path.is_none());
    }

    /// The resolver searches every supplied wallet: a key registered only in the
    /// second wallet resolves to that wallet's seed hash, not the first.
    #[test]
    fn searches_across_multiple_wallets() {
        let pk = pubkey(5);
        let address = Address::p2pkh(&pk, Network::Testnet);
        let path = identity_auth_path(2, 0);

        let empty = wallet_with(5, &[]);
        let holder = wallet_with(6, &[(address.clone(), path.clone())]);
        let holder_seed_hash = holder.read().unwrap().seed_hash();

        let key = identity_key(KeyType::ECDSA_SECP256K1, pk.to_bytes());
        let resolved = QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
            key,
            Network::Testnet,
            &[&empty, &holder],
        );

        assert_eq!(
            resolved
                .in_wallet_at_derivation_path
                .expect("key in the second wallet must link")
                .wallet_seed_hash,
            holder_seed_hash
        );
    }

    /// A non-address key type (e.g. BLS) yields no candidate address and stays
    /// unlinked rather than erroring.
    #[test]
    fn non_address_key_type_stays_unlinked() {
        let wallet = wallet_with(7, &[]);
        let key = identity_key(KeyType::BLS12_381, vec![0u8; 48]);
        let resolved = QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
            key,
            Network::Testnet,
            &[&wallet],
        );
        assert!(resolved.in_wallet_at_derivation_path.is_none());
    }
}
