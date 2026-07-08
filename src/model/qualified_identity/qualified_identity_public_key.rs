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
