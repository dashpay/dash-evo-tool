mod asset_lock_transaction;
mod utxos;

use dash_sdk::dashcore_rpc::dashcore::bip32::KeyDerivationType;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::{
    Address, InstantLock, Network, OutPoint, PrivateKey, PublicKey, Transaction, TxOut,
};
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
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::Identity;

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
    pub known_addresses: BTreeMap<Address, DerivationPath>,
    pub watched_addresses: BTreeMap<DerivationPath, AddressInfo>,
    pub unused_asset_locks: Vec<(
        Transaction,
        Address,
        Credits,
        Option<InstantLock>,
        Option<AssetLockProof>,
    )>,
    pub alias: Option<String>,
    pub identities: HashMap<u32, Identity>,
    pub utxos: HashMap<Address, HashMap<OutPoint, TxOut>>,
    pub is_main: bool,
    pub password_hint: Option<String>,
}

impl Wallet {
    pub fn has_balance(&self) -> bool {
        self.max_balance() > 0
    }

    pub fn has_unused_asset_lock(&self) -> bool {
        self.unused_asset_locks.len() > 0
    }

    pub fn max_balance(&self) -> u64 {
        self.address_balances.values().sum::<Duffs>()
    }

    pub fn private_key_for_address(
        &self,
        address: &Address,
        network: Network,
    ) -> Result<Option<PrivateKey>, String> {
        self.known_addresses
            .get(address)
            .map(|derivation_path| {
                derivation_path
                    .derive_priv_ecdsa_for_master_seed(&self.seed, network)
                    .map(|extended_private_key| extended_private_key.to_priv())
            })
            .transpose()
            .map_err(|e| e.to_string())
    }

    pub fn unused_bip_44_public_key(
        &mut self,
        network: Network,
        skip_known_addresses_with_no_funds: bool,
        change: bool,
        register: Option<&AppContext>,
    ) -> Result<(PublicKey, DerivationPath), String> {
        let mut address_index = 0;
        let mut found_unused_derivation_path = None;
        while found_unused_derivation_path.is_none() {
            let derivation_path =
                DerivationPath::bip_44_payment_path(network, 0, change, address_index);

            if let Some(address_info) = self.watched_addresses.get(&derivation_path) {
                // Address is known
                let address = &address_info.address;
                let balance = self.address_balances.get(address).cloned().unwrap_or(0);

                if balance > 0 {
                    // Address has funds, skip it
                    address_index += 1;
                    continue;
                }

                // Address is known and has zero balance
                if !skip_known_addresses_with_no_funds {
                    // We can use this address
                    found_unused_derivation_path = Some(derivation_path.clone());
                    break;
                } else {
                    // Skip known addresses with no funds
                    address_index += 1;
                    continue;
                }
            } else {
                // Address is not known, proceed to register it
                let public_key = derivation_path
                    .derive_pub_ecdsa_for_master_seed(&self.seed, network)
                    .expect("derivation should not be able to fail")
                    .to_pub();
                if let Some(app_context) = register {
                    let address = Address::p2pkh(&public_key, network);
                    app_context
                        .core_client
                        .import_address(
                            &address,
                            Some(
                                format!(
                                    "Managed by Dash Evo Tool {} {}",
                                    self.alias.clone().unwrap_or_default(),
                                    derivation_path
                                )
                                .as_str(),
                            ),
                            Some(false),
                        )
                        .map_err(|e| e.to_string())?;
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
                            address: address.clone(),
                            path_type: DerivationPathType::CLEAR_FUNDS,
                            path_reference: DerivationPathReference::BIP44,
                        },
                    );

                    // Add the address and its derivation path to `known_addresses`
                    self.known_addresses
                        .insert(address, derivation_path.clone());
                }
                found_unused_derivation_path = Some(derivation_path.clone());
                break;
            }
        }

        let derivation_path = found_unused_derivation_path.unwrap();
        let extended_public_key = derivation_path
            .derive_pub_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        Ok((extended_public_key.to_pub(), derivation_path))
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
        let derivation_path = DerivationPath::identity_registration_path(network, index);
        let extended_public_key = derivation_path
            .derive_pub_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        extended_public_key.to_pub()
    }

    pub fn identity_registration_ecdsa_private_key(
        &mut self,
        network: Network,
        index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<PrivateKey, String> {
        let derivation_path = DerivationPath::identity_registration_path(network, index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&self.seed, network)
            .expect("derivation should not be able to fail");
        let private_key = extended_private_key.to_priv();

        if let Some(app_context) = register_addresses {
            let secp = Secp256k1::new();
            let address = Address::p2pkh(&private_key.public_key(&secp), network);
            app_context
                .db
                .add_address(
                    &self.seed,
                    &address,
                    &derivation_path,
                    DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                    DerivationPathType::CREDIT_FUNDING,
                    None,
                )
                .map_err(|e| e.to_string())?;
            self.known_addresses
                .insert(address.clone(), derivation_path.clone());
            self.watched_addresses.insert(
                derivation_path.clone(),
                AddressInfo {
                    address: address.clone(),
                    path_type: DerivationPathType::CREDIT_FUNDING,
                    path_reference:
                        DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                },
            );
        }
        Ok(private_key)
    }

    pub fn receive_address(
        &mut self,
        network: Network,
        skip_known_addresses_with_no_funds: bool,
        register: Option<&AppContext>,
    ) -> Result<Address, String> {
        Ok(Address::p2pkh(
            &self
                .unused_bip_44_public_key(
                    network,
                    skip_known_addresses_with_no_funds,
                    false,
                    register,
                )?
                .0,
            network,
        ))
    }

    pub fn receive_address_with_derivation_path(
        &mut self,
        network: Network,
        register: Option<&AppContext>,
    ) -> Result<(Address, DerivationPath), String> {
        let (receive_public_key, derivation_path) =
            self.unused_bip_44_public_key(network, false, false, register)?;
        Ok((
            Address::p2pkh(&receive_public_key, network),
            derivation_path,
        ))
    }

    pub fn change_address(
        &mut self,
        network: Network,
        register: Option<&AppContext>,
    ) -> Result<Address, String> {
        Ok(Address::p2pkh(
            &self
                .unused_bip_44_public_key(network, false, true, register)?
                .0,
            network,
        ))
    }

    pub fn change_address_with_derivation_path(
        &mut self,
        network: Network,
        register: Option<&AppContext>,
    ) -> Result<(Address, DerivationPath), String> {
        let (receive_public_key, derivation_path) =
            self.unused_bip_44_public_key(network, false, true, register)?;
        Ok((
            Address::p2pkh(&receive_public_key, network),
            derivation_path,
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
