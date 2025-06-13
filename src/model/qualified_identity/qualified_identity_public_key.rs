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
    pub fn from_identity_public_key_with_wallets_check(
        value: IdentityPublicKey,
        network: Network,
        wallets: &[&Arc<RwLock<Wallet>>],
    ) -> Self {
        // Initialize `in_wallet_at_derivation_path` as `None`
        let mut in_wallet_at_derivation_path = None;

        match value.key_type() {
            KeyType::ECDSA_SECP256K1 => {
                // Check if data is a full public key (33 bytes) or just a hash (20 bytes)
                if value.data().len() == 20 {
                    // This is actually a hash, treat it as ECDSA_HASH160
                    let hash160_data = value.data().as_slice();
                    let pubkey_hash = PubkeyHash::from_slice(hash160_data)
                        .expect("Expected valid 20-byte pubkey hash for ECDSA_SECP256K1 with hash data");

                    let address = Address::new(network, Payload::PubkeyHash(pubkey_hash));

                    let testnet_address = if network != Network::Dash {
                        Some(Address::new(
                            Network::Testnet,
                            Payload::PubkeyHash(pubkey_hash),
                        ))
                    } else {
                        None
                    };

                    // Iterate over each wallet to check for matching derivation paths
                    for locked_wallet in wallets {
                        let wallet = locked_wallet.read().unwrap();
                        if let Some(derivation_path) = wallet.known_addresses.get(&address) {
                            in_wallet_at_derivation_path = Some(WalletDerivationPath {
                                wallet_seed_hash: wallet.seed_hash(),
                                derivation_path: derivation_path.clone(),
                            });
                        }
                        if in_wallet_at_derivation_path.is_some() {
                            break;
                        }

                        if let Some(testnet_address) = testnet_address.as_ref() {
                            if let Some(derivation_path) = wallet.known_addresses.get(testnet_address) {
                                in_wallet_at_derivation_path = Some(WalletDerivationPath {
                                    wallet_seed_hash: wallet.seed_hash(),
                                    derivation_path: derivation_path.clone(),
                                });
                            }
                            if in_wallet_at_derivation_path.is_some() {
                                break;
                            }
                        }
                    }
                } else {
                    // This is a full public key (expected 33 bytes)
                    let pubkey = PublicKey::from_slice(value.data().as_slice())
                        .map_err(|e| format!("Expected valid public key: {}", e))
                        .expect("Expected valid public key");

                    let address = Address::p2pkh(&pubkey, network);

                    let testnet_address = if network != Network::Dash {
                        Some(Address::p2pkh(&pubkey, Network::Testnet))
                    } else {
                        None
                    };

                    // Iterate over each wallet to check for matching derivation paths
                    for locked_wallet in wallets {
                        let wallet = locked_wallet.read().unwrap();
                        if let Some(derivation_path) = wallet.known_addresses.get(&address) {
                            in_wallet_at_derivation_path = Some(WalletDerivationPath {
                                wallet_seed_hash: wallet.seed_hash(),
                                derivation_path: derivation_path.clone(),
                            });
                        }
                        if in_wallet_at_derivation_path.is_some() {
                            break;
                        }

                        if let Some(testnet_address) = testnet_address.as_ref() {
                            if let Some(derivation_path) = wallet.known_addresses.get(testnet_address) {
                                in_wallet_at_derivation_path = Some(WalletDerivationPath {
                                    wallet_seed_hash: wallet.seed_hash(),
                                    derivation_path: derivation_path.clone(),
                                });
                            }
                            if in_wallet_at_derivation_path.is_some() {
                                break;
                            }
                        }
                    }
                }

                Self {
                    identity_public_key: value,
                    in_wallet_at_derivation_path,
                }
            }
            KeyType::ECDSA_HASH160 => {
                let hash160_data = value.data().as_slice();
                let pubkey_hash = PubkeyHash::from_slice(hash160_data)
                    .expect("Expected valid 20-byte pubkey hash for ECDSA_HASH160");

                let address = Address::new(network, Payload::PubkeyHash(pubkey_hash));

                let testnet_address = if network != Network::Dash {
                    Some(Address::new(
                        Network::Testnet,
                        Payload::PubkeyHash(pubkey_hash),
                    ))
                } else {
                    None
                };

                // Iterate over each wallet to check for matching derivation paths
                for locked_wallet in wallets {
                    let wallet = locked_wallet.read().unwrap();
                    if let Some(derivation_path) = wallet.known_addresses.get(&address) {
                        in_wallet_at_derivation_path = Some(WalletDerivationPath {
                            wallet_seed_hash: wallet.seed_hash(),
                            derivation_path: derivation_path.clone(),
                        });
                    }
                    if in_wallet_at_derivation_path.is_some() {
                        break;
                    }

                    if let Some(testnet_address) = testnet_address.as_ref() {
                        if let Some(derivation_path) = wallet.known_addresses.get(testnet_address) {
                            in_wallet_at_derivation_path = Some(WalletDerivationPath {
                                wallet_seed_hash: wallet.seed_hash(),
                                derivation_path: derivation_path.clone(),
                            });
                        }
                        if in_wallet_at_derivation_path.is_some() {
                            break;
                        }
                    }
                }

                Self {
                    identity_public_key: value,
                    in_wallet_at_derivation_path,
                }
            }
            _ => Self {
                identity_public_key: value,
                in_wallet_at_derivation_path: None,
            },
        }
    }
}
