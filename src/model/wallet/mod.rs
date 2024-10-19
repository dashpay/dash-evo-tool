mod utxos;
mod asset_lock_transaction;

use dash_sdk::dashcore_rpc::dashcore::bip32::KeyDerivationType;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, PrivateKey, PublicKey, TxOut};
use std::collections::{BTreeMap, HashMap};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum DerivationPathReference {
    Unknown = 0,
    BIP32 = 1,
    BIP44 = 2,
    BlockchainIdentities = 3,
    ProviderFunds = 4,
    ProviderVotingKeys = 5,
    ProviderOperatorKeys = 6,
    ProviderOwnerKeys = 7,
    ContactBasedFunds = 8,
    ContactBasedFundsRoot = 9,
    ContactBasedFundsExternal = 10,
    BlockchainIdentityCreditRegistrationFunding = 11,
    BlockchainIdentityCreditTopupFunding = 12,
    BlockchainIdentityCreditInvitationFunding = 13,
    ProviderPlatformNodeKeys = 14,
    Root = 255,
}

impl TryFrom<u32> for DerivationPathReference {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(DerivationPathReference::Unknown),
            1 => Ok(DerivationPathReference::BIP32),
            2 => Ok(DerivationPathReference::BIP44),
            3 => Ok(DerivationPathReference::BlockchainIdentities),
            4 => Ok(DerivationPathReference::ProviderFunds),
            5 => Ok(DerivationPathReference::ProviderVotingKeys),
            6 => Ok(DerivationPathReference::ProviderOperatorKeys),
            7 => Ok(DerivationPathReference::ProviderOwnerKeys),
            8 => Ok(DerivationPathReference::ContactBasedFunds),
            9 => Ok(DerivationPathReference::ContactBasedFundsRoot),
            10 => Ok(DerivationPathReference::ContactBasedFundsExternal),
            11 => Ok(DerivationPathReference::BlockchainIdentityCreditRegistrationFunding),
            12 => Ok(DerivationPathReference::BlockchainIdentityCreditTopupFunding),
            13 => Ok(DerivationPathReference::BlockchainIdentityCreditInvitationFunding),
            14 => Ok(DerivationPathReference::ProviderPlatformNodeKeys),
            255 => Ok(DerivationPathReference::Root),
            value => Err(format!(
                "value {} not convertable to a DerivationPathReference",
                value
            )),
        }
    }
}

use crate::context::AppContext;
use bitflags::bitflags;
use dash_sdk::dpp::balances::credits::Duffs;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
    pub struct DerivationPathType: u32 {
        const UNKNOWN = 0;
        const CLEAR_FUNDS = 1;
        const ANONYMOUS_FUNDS = 1 << 1;
        const VIEW_ONLY_FUNDS = 1 << 2;
        const SINGLE_USER_AUTHENTICATION = 1 << 3;
        const MULTIPLE_USER_AUTHENTICATION = 1 << 4;
        const PARTIAL_PATH = 1 << 5;
        const PROTECTED_FUNDS = 1 << 6;
        const CREDIT_FUNDING = 1 << 7;

        // Composite flags
        const IS_FOR_AUTHENTICATION = Self::SINGLE_USER_AUTHENTICATION.bits() | Self::MULTIPLE_USER_AUTHENTICATION.bits();
        const IS_FOR_FUNDS = Self::CLEAR_FUNDS.bits()
            | Self::ANONYMOUS_FUNDS.bits()
            | Self::VIEW_ONLY_FUNDS.bits()
            | Self::PROTECTED_FUNDS.bits();
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct AddressInfo {
    pub address: Address,
    pub path_type: DerivationPathType,
    pub path_reference: DerivationPathReference,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wallet {
    pub(crate) seed: [u8; 64],
    pub address_balances: BTreeMap<Address, u64>,
    pub watched_addresses: BTreeMap<DerivationPath, AddressInfo>,
    pub alias: Option<String>,
    pub utxos: Option<HashMap<Address, HashMap<OutPoint, TxOut>>>,
    pub is_main: bool,
    pub password_hint: Option<String>,
}

impl Wallet {
    pub fn has_balance(&self) -> bool {
        self.max_balance() > 0
    }

    pub fn max_balance(&self) -> u64 {
        self.address_balances.values().sum::<Duffs>()
    }

    pub fn unused_bip_44_public_key(
        &mut self,
        network: Network,
        change: bool,
        register: Option<&AppContext>,
    ) -> Result<PublicKey, String> {
        let mut address_index = 0;
        let mut found_unused_derivation_path = None;
        while found_unused_derivation_path.is_none() {
            let derivation_path =
                DerivationPath::bip_44_payment_path(network, 0, change, address_index);
            if self.watched_addresses.get(&derivation_path).is_none() {
                let public_key = derivation_path
                    .derive_pub_ecdsa_for_master_seed(&self.seed, network)
                    .expect("derivation should not be able to fail")
                    .to_pub();
                if let Some(app_context) = register {
                    let address = Address::p2pkh(&public_key, network);
                    app_context
                        .db
                        .add_address(
                            &self.seed,
                            &address,
                            &derivation_path,
                            DerivationPathReference::BIP44,
                            DerivationPathType::CLEAR_FUNDS,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    self.watched_addresses.insert(
                        derivation_path.clone(),
                        AddressInfo {
                            address,
                            path_type: DerivationPathType::CLEAR_FUNDS,
                            path_reference: DerivationPathReference::BIP44,
                        },
                    );
                }
                found_unused_derivation_path = Some(derivation_path);
                break;
            } else {
                address_index += 1;
            }
        }

        let extended_public_key = found_unused_derivation_path
            .unwrap()
            .derive_pub_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        Ok(extended_public_key.to_pub())
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

    pub fn identity_registration_ecdsa_public_key(
        &self,
        network: Network,
        index: u32,
    ) -> PublicKey {
        let derivation_path = DerivationPath::identity_registration_path(
            network,
            index,
        );
        let extended_public_key = derivation_path
            .derive_pub_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        extended_public_key.to_pub()
    }

    pub fn identity_registration_ecdsa_private_key(
        &self,
        network: Network,
        index: u32,
    ) -> PrivateKey {
        let derivation_path = DerivationPath::identity_registration_path(
            network,
            index,
        );
        let extended_public_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        extended_public_key.to_priv()
    }

    pub fn receive_address(
        &mut self,
        network: Network,
        register: Option<&AppContext>,
    ) -> Result<Address, String> {
        Ok(Address::p2pkh(
            &self.unused_bip_44_public_key(network, false, register)?,
            network,
        ))
    }

    pub fn change_address(
        &mut self,
        network: Network,
        register: Option<&AppContext>,
    ) -> Result<Address, String> {
        Ok(Address::p2pkh(
            &self.unused_bip_44_public_key(network, true, register)?,
            network,
        ))
    }

    pub fn update_address_balance(
        &mut self,
        address: &Address,
        new_balance: Duffs,
        context: &AppContext,
    ) -> Result<(), String> {
        // Check if the new balance differs from the current one.
        if let Some(current_balance) = self.address_balances.get(address) {
            if *current_balance == new_balance {
                // If the balance hasn't changed, skip the update.
                return Ok(());
            }
        }

        // If there's no current balance or it has changed, update it.
        self.address_balances.insert(address.clone(), new_balance);

        // Update the database with the new balance.
        context
            .db
            .update_address_balance(&self.seed, address, new_balance)
            .map_err(|e| e.to_string())
    }
}
