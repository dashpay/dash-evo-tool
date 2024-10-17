mod derivation_path;
mod index_path;
mod index_path_types;

use bincode::{Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};
use dash_sdk::dpp::identity::KeyType;
use rand::rngs::StdRng;
use rand::SeedableRng;

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct Wallet {
    pub(crate) seed: [u8; 64],
    pub alias: Option<String>,
    pub is_main: bool,
    pub password_hint: Option<String>,
}

impl Wallet {
    pub fn has_balance(&self) -> bool {
        false
    }

    pub fn unused_bip_44_public_key(&self, network: Network) -> PublicKey {
        KeyType::
        // Create a new Secp256k1 context
        let secp = Secp256k1::new();

        // Generate a random secret key using the system's secure random number generator
        let mut rng = StdRng::from_entropy();
        let secret_key = SecretKey::new(&mut rng);

        let private_key = PrivateKey::new(secret_key, network);

        // Generate the corresponding public key
        PublicKey::from_private_key(&secp, &private_key)
    }
    pub fn receive_address(&self, network: Network) -> Address {
        Address::p2pkh(&self.unused_bip_44_public_key(network), network)
    }
}
