use crate::model::qualified_identity::encrypted_key_storage::WalletDerivationPath;
use crate::model::wallet::Wallet;
use bincode::{Decode, Encode};
use dash_sdk::dpp::dashcore::{Address, PublicKey};
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

        let pubkey =
            PublicKey::from_slice(value.data().as_slice()).expect("Expected valid public key");

        let address = Address::p2pkh(&pubkey, network);

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
        }

        Self {
            identity_public_key: value,
            in_wallet_at_derivation_path,
        }
    }
}
