use dash_sdk::dashcore_rpc::dashcore::bip32::{ChildNumber, ExtendedPubKey, KeyDerivationType};
use dash_sdk::dpp;

use crate::kms::Kms;
use crate::kms::generic::key_handle::{KeyHandle, SeedHash};
use bitflags::bitflags;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    Address, InstantLock, Network, OutPoint, PrivateKey, PublicKey, Transaction, TxOut,
};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::Identity;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::process::Child;
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

// todo: remove, just temporary placeholder atm
pub type AppContext = bool;

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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct AddressInfo {
    pub address: Address,
    /// Address balance; if None, the balance is unknown.
    pub balance: Option<Credits>,
    pub derivation_path: DerivationPath,
    pub path_type: DerivationPathType,
    pub path_reference: DerivationPathReference,
}

pub struct AssetLock {
    transaction: Transaction,
    address: Address,
    credits: Credits,
    instant_lock: Option<InstantLock>,
    asset_lock_proof: Option<AssetLockProof>,
}

/// Address watcher for tracking addresses and their associated information.
pub struct AddressWatcher {
    pub addresses: BTreeMap<Address, AddressInfo>,
}

fn derivation_path_starts_with(path:&DerivationPath, prefix:&DerivationPath) -> bool {
    if path.len() < prefix.len() {
        return false;
    }
    for i in 0..prefix.len() {
        if path[i] != prefix[i] {
            return false;
        }
    }
    true
}

impl AddressWatcher {
    /// Given some prefix of a derivation path, finds the first unused ChildNumber
    /// with the given prefix.
    /// 
    /// Returns list of unused indexes with the given prefix.
    /// It is assumed that all indexes after the last used index are unused.
    /// 
    /// If `skip_known_addresses_with_no_funds` is true, it will assume addresses with zero balance (and with None balance) as used.
   pub fn bip44_unused_indexes_with_prefix(&self, prefix:&DerivationPath, 
        skip_known_addresses_with_no_funds: bool) -> BTreeSet<ChildNumber> {
            let  used_indexes = self.addresses.iter()
                .filter(|(_, info)| {
                    if info.derivation_path.len() < prefix.len()+1 {
                        return false; // Derivation path is shorter than prefix + 1 (the last index)
                    }
                    if !derivation_path_starts_with(&info.derivation_path, prefix) {
                        return false; // Not matching prefix
                    }

                    // matching prefix, now check if we should assume it as used
                    if info.balance.unwrap_or(0) == 0 {                        
                        return  skip_known_addresses_with_no_funds; // we assume it used if we skip known addresses with no funds
                    } else {
                        return true; // Address has funds, so it's used
                    }
                })
                .map(|(_, info)| {
                    if let Some(ChildNumber::Normal { index }) = info.derivation_path.into_iter().last() {
                        *index
                    } else {
                        0 // Default to 0 if no normal child number found
                    }
                })
                .collect::<BTreeSet<_>>();

                let mut result = BTreeSet::new();
                let mut next_index = 0;
                for used_index in used_indexes {
                    while next_index < used_index {
                        result.insert(ChildNumber::Normal { index: next_index });
                        next_index += 1;
                    }
                    next_index =used_index+1; // Skip the used index                    
                };
                
                // Add the next index after the last used index
                result.insert(ChildNumber::Normal { index: next_index });

                result
        }

    /// Finds first unused ChildNumber of bip44 payment path.
    /// If `change` is true, it will look for change addresses (index 1),
    /// otherwise it will look for receive addresses (index 0).
    ///
    /// If `skip_known_addresses_with_no_funds` is true, it will consider addresses with zero balance (and with None balance) as used.
    fn first_unused_index(
        &self,
        change: bool,
        network: &Network,
        skip_known_addresses_with_no_funds: bool,
    ) -> ChildNumber {
        let mut address_index = 0;
        let mut found_unused_derivation_path = None;
        let mut known_public_key = None;
        while found_unused_derivation_path.is_none() {
            let derivation_path_extension = DerivationPath::from(
                [
                    ChildNumber::Normal {
                        index: change.into(),
                    },
                    ChildNumber::Normal {
                        index: address_index,
                    },
                ]
                .as_slice(),
            );
            let derivation_path_prefix =
                DerivationPath::bip_44_payment_path(*network, 0, change, 0);

            // Finds the address at `derivation_path` and check if it is a match; note we need to find None here.
            if let Some(_) = self.addresses.iter().find(|(_, info)| {
                if info.derivation_path == derivation_path {
                    if info.balance.unwrap_or(0) == 0  {
                        // Address has zero balance, so it's a good candidate if we don't skip known addresses with no funds
                        !skip_known_addresses_with_no_funds
                    if skip_known_addresses_with_no_funds && {
                        // Address has no funds, but we still assume it's  used
                        false 
                    } else {
                        //
                        true // Address is a match
                    }}
                } else {
                    false // derivation path does not match
                }
            }) {
                // Address is known
                let address = &address_info.address;
                let balance = address_info.balance.unwrap_or(0);

                if balance > 0 {
                    // Address has funds, skip it
                    address_index += 1;
                    continue;
                }

                // Address is known and has zero balance
                if !skip_known_addresses_with_no_funds {
                    // We can use this address
                    found_unused_derivation_path = Some(derivation_path.clone());
                    let secp = Secp256k1::new();
                    let public_key = self
                        .master_bip44_ecdsa_extended_public_key
                        .derive_pub(&secp, &derivation_path_extension)
                        .map_err(|e| e.to_string())?
                        .to_pub();
                    known_public_key = Some(public_key);
                    break;
                } else {
                    // Skip known addresses with no funds
                    address_index += 1;
                    continue;
                }
            } else {
                let secp = Secp256k1::new();
                let public_key = self
                    .master_bip44_ecdsa_extended_public_key
                    .derive_pub(&secp, &derivation_path_extension)
                    .map_err(|e| e.to_string())?
                    .to_pub();
                known_public_key = Some(public_key);
                if let Some(app_context) = register {
                    let address = Address::p2pkh(&public_key, network);
                    app_context
                        .core_client
                        .read()
                        .expect("Core client lock was poisoned")
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

                    self.register_address(
                        address,
                        &derivation_path,
                        DerivationPathType::CLEAR_FUNDS,
                        DerivationPathReference::BIP44,
                        app_context,
                    )?;
                }
                found_unused_derivation_path = Some(derivation_path.clone());
                break;
            }
        }

        let derivation_path = found_unused_derivation_path.unwrap();
    }
}

impl AsRef<BTreeMap<Address, AddressInfo>> for AddressWatcher {
    fn as_ref(&self) -> &BTreeMap<Address, AddressInfo> {
        &self.addresses
    }
}

/// Core wallet for managing dash-core-specific cryptocurrency wallets.
///
/// It provides the core functionality for handling dash-core transactions,
/// including tracking of address balances, signing transactions, and
/// managing keys.
pub struct DashCoreWallet {
    pub master_bip44_ecdsa_extended_public_key: ExtendedPubKey,
    pub watched_addresses: AddressWatcher,
    pub unused_asset_locks: Vec<AssetLock>,
    pub alias: Option<String>,
    pub utxos: HashMap<Address, HashMap<OutPoint, TxOut>>,

    // TODO: move to higher-level wallet?
    pub key_store: crate::kms::generic::GenericKms,
    // TODO: move to higher-level wallet?
    pub wallet_seed: KeyHandle,
    network: Network,
}

impl DashCoreWallet {
    pub fn has_balance(&self) -> bool {
        self.max_balance() > 0
    }

    pub fn has_unused_asset_lock(&self) -> bool {
        !self.unused_asset_locks.is_empty()
    }

    pub fn max_balance(&self) -> u64 {
        self.utxos
            .values()
            .flat_map(|outpoints_to_tx_out| outpoints_to_tx_out.values().map(|tx_out| tx_out.value))
            .sum::<Duffs>()
    }

    pub fn seed_hash(&self) -> [u8; 32] {
        match &self.wallet_seed {
            KeyHandle::DerivationSeed { seed_hash, .. } => *seed_hash,
            _ => {
                // If the wallet seed is not a derivation seed, we cannot provide a seed hash.
                // This should not happen in a properly initialized wallet.
                panic!("Wallet seed is not a derivation seed, cannot provide seed hash");
            }
        }
    }

    pub fn unused_bip_44_public_key(
        &mut self,
        network: Network,
        skip_known_addresses_with_no_funds: bool,
        change: bool,
        register: Option<bool>,
    ) -> Result<KeyHandle, String> {
        // First, we try to find some known address with zero balance; once we find it, we will use it.
        let unused_address = self
            .watched_addresses
            .addresses
            .iter()
            .filter_map(|(address, address_info)| {
                // we assume addresses with unknown balance are not used
                if !skip_known_addresses_with_no_funds && address_info.balance.unwrap_or(1) == 0 {
                    self.watched_addresses
                        .addresses
                        .get(address)
                        .and_then(|info| {
                            let path = info.derivation_path;
                            if path.len() < 2 {
                                return None; // We need at least two segments to determine change.
                            }

                            // is change address when item before last is 1
                            match path[path.len() - 2] {
                                ChildNumber::Normal { index } if index == 1 => Some(info),
                                _ => None,
                            }
                        })
                } else {
                    None
                }
            })
            .next();

        // We found a known address with zero balance, so we can use it.
        if let Some(address_info) = unused_address {
            let key_handle = KeyHandle::Derived {
                seed_hash: self.seed_hash(),
                derivation_path: address_info.derivation_path,
                network: self.network(),
            };

            return Ok(key_handle);
        }

        // otherwise, we need to generate a new address

        let unused_index = self.watched_addresses.first_unused_index();

        let mut address_index = 0;
        let mut found_unused_derivation_path = None;
        let mut known_public_key = None;
        while found_unused_derivation_path.is_none() {
            let derivation_path_extension = DerivationPath::from(
                [
                    ChildNumber::Normal {
                        index: change.into(),
                    },
                    ChildNumber::Normal {
                        index: address_index,
                    },
                ]
                .as_slice(),
            );
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
                    let secp = Secp256k1::new();
                    let public_key = self
                        .master_bip44_ecdsa_extended_public_key
                        .derive_pub(&secp, &derivation_path_extension)
                        .map_err(|e| e.to_string())?
                        .to_pub();
                    known_public_key = Some(public_key);
                    break;
                } else {
                    // Skip known addresses with no funds
                    address_index += 1;
                    continue;
                }
            } else {
                let secp = Secp256k1::new();
                let public_key = self
                    .master_bip44_ecdsa_extended_public_key
                    .derive_pub(&secp, &derivation_path_extension)
                    .map_err(|e| e.to_string())?
                    .to_pub();
                known_public_key = Some(public_key);
                if let Some(app_context) = register {
                    let address = Address::p2pkh(&public_key, network);
                    app_context
                        .core_client
                        .read()
                        .expect("Core client lock was poisoned")
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

                    self.register_address(
                        address,
                        &derivation_path,
                        DerivationPathType::CLEAR_FUNDS,
                        DerivationPathReference::BIP44,
                        app_context,
                    )?;
                }
                found_unused_derivation_path = Some(derivation_path.clone());
                break;
            }
        }

        let derivation_path = found_unused_derivation_path.unwrap();
        let known_public_key = known_public_key.unwrap();
        Ok((known_public_key, derivation_path))
    }

    pub fn identity_authentication_ecdsa_public_key(
        &self,
        network: Network,
        identity_index: u32,
        key_index: u32,
    ) -> Result<PublicKey, String> {
        let derivation_path = DerivationPath::identity_authentication_path(
            network,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        );
        let extended_public_key = derivation_path
            .derive_pub_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .map_err(|e| e.to_string())?;
        Ok(extended_public_key.to_pub())
    }

    #[allow(clippy::type_complexity)]
    pub fn identity_authentication_ecdsa_public_keys_data_map(
        &mut self,
        network: Network,
        identity_index: u32,
        key_index_range: Range<u32>,
        register_addresses: Option<&AppContext>,
    ) -> Result<(BTreeMap<Vec<u8>, u32>, BTreeMap<[u8; 20], u32>), String> {
        let mut public_key_result_map = BTreeMap::new();
        let mut public_key_hash_result_map = BTreeMap::new();
        for key_index in key_index_range {
            let derivation_path = DerivationPath::identity_authentication_path(
                network,
                KeyDerivationType::ECDSA,
                identity_index,
                key_index,
            );
            let extended_public_key = derivation_path
                .derive_pub_ecdsa_for_master_seed(self.seed_bytes()?, network)
                .map_err(|e| e.to_string())?;

            let public_key = extended_public_key.to_pub();
            public_key_result_map.insert(
                extended_public_key.public_key.serialize().to_vec(),
                key_index,
            );
            public_key_hash_result_map.insert(public_key.pubkey_hash().to_byte_array(), key_index);
            if let Some(app_context) = register_addresses {
                self.register_address_from_public_key(
                    &public_key,
                    &derivation_path,
                    DerivationPathType::SINGLE_USER_AUTHENTICATION,
                    DerivationPathReference::BlockchainIdentities,
                    app_context,
                )?;
            }
        }

        Ok((public_key_result_map, public_key_hash_result_map))
    }

    pub fn identity_authentication_ecdsa_private_key(
        &mut self,
        network: Network,
        identity_index: u32,
        key_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<(PrivateKey, DerivationPath), String> {
        let derivation_path = DerivationPath::identity_authentication_path(
            network,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        );
        let extended_public_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .expect("derivation should not be able to fail");

        let private_key = extended_public_key.to_priv();
        if let Some(app_context) = register_addresses {
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::SINGLE_USER_AUTHENTICATION,
                DerivationPathReference::BlockchainIdentities,
                app_context,
            )?;
        }

        Ok((private_key, derivation_path))
    }

    fn register_address_from_private_key(
        &mut self,
        private_key: &PrivateKey,
        derivation_path: &DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let secp = Secp256k1::new();
        let address = Address::p2pkh(&private_key.public_key(&secp), app_context.network);
        self.register_address(
            address,
            derivation_path,
            path_type,
            path_reference,
            app_context,
        )
    }

    fn register_address_from_public_key(
        &mut self,
        public_key: &PublicKey,
        derivation_path: &DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let address = Address::p2pkh(public_key, app_context.network);
        self.register_address(
            address,
            derivation_path,
            path_type,
            path_reference,
            app_context,
        )
    }
    fn register_address(
        &mut self,
        address: Address,
        derivation_path: &DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
        app_context: &AppContext,
    ) -> Result<(), String> {
        if !address.network().eq(&app_context.network) {
            return Err(format!(
                "address {} network {} does not match wallet network {}",
                address,
                address.network(),
                app_context.network
            ));
        }

        app_context
            .db
            .add_address_if_not_exists(
                &self.seed_hash(),
                &address,
                &app_context.network,
                derivation_path,
                path_reference,
                path_type,
                None,
            )
            .map_err(|e| e.to_string())?;
        self.known_addresses
            .insert(address.clone(), derivation_path.clone());
        self.watched_addresses.insert(
            derivation_path.clone(),
            AddressInfo {
                address: address.clone(),
                path_type,
                path_reference,
                balance: Credits::default(),
                derivation_path: derivation_path.clone(),
            },
        );

        tracing::trace!(
            address = ?&address,
            network = &address.network().to_string(),
            "registered new address"
        );
        Ok(())
    }

    pub fn identity_top_up_ecdsa_private_key(
        &mut self,
        network: Network,
        identity_index: u32,
        top_up_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<PrivateKey, String> {
        let derivation_path =
            DerivationPath::identity_top_up_path(network, identity_index, top_up_index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .expect("derivation should not be able to fail");
        let private_key = extended_private_key.to_priv();

        if let Some(app_context) = register_addresses {
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::CREDIT_FUNDING,
                DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                app_context,
            )?;
        }
        Ok(private_key)
    }

    /// Generate Core key for identity registration
    pub fn identity_registration_ecdsa_private_key(
        &mut self,
        network: Network,
        index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<PrivateKey, String> {
        let derivation_path = DerivationPath::identity_registration_path(network, index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .expect("derivation should not be able to fail");
        let private_key = extended_private_key.to_priv();

        if let Some(app_context) = register_addresses {
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::CREDIT_FUNDING,
                DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                app_context,
            )?;
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

    // Allow dead_code: This method provides receive addresses with derivation paths,
    // useful for advanced address management and BIP44 path tracking
    #[allow(dead_code)]
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

    // Allow dead_code: This method provides change addresses with derivation paths,
    // useful for advanced address management and BIP44 path tracking
    #[allow(dead_code)]
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
            .update_address_balance(&self.seed_hash(), address, new_balance)
            .map_err(|e| e.to_string())
    }

    fn network(&self) -> Network {
        todo!()
    }
}
