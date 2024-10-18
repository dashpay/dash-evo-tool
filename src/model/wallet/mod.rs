use bincode::{Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::bip32::KeyDerivationType;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};

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

    pub fn max_balance(&self, network: Network) -> u64 {
        0
    }

    pub fn unused_bip_44_public_key(&self, network: Network) -> PublicKey {
        let derivation_path = DerivationPath::bip_44_payment_path(network, 0, false, 0);
        let extended_public_key = derivation_path
            .derive_pub_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        extended_public_key.to_pub()
    }

    pub fn identity_authentication_ecdsa_public_key(
        &self,
        network: Network,
        identity_index: u32,
        key_index: u32,
    ) -> PublicKey {
        let derivation_path = DerivationPath::identity_authentication_path(
            network,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        );
        let extended_public_key = derivation_path
            .derive_pub_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        extended_public_key.to_pub()
    }

    pub fn identity_authentication_ecdsa_private_key(
        &self,
        network: Network,
        identity_index: u32,
        key_index: u32,
    ) -> PrivateKey {
        let derivation_path = DerivationPath::identity_authentication_path(
            network,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        );
        let extended_public_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        extended_public_key.to_priv()
    }

    pub fn receive_address(&self, network: Network) -> Address {
        Address::p2pkh(&self.unused_bip_44_public_key(network), network)
    }
}
