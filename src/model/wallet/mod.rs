mod asset_lock_transaction;
pub mod encryption;
pub mod single_key;
mod utxos;

use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::{AddressWitness, PlatformAddress};
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{
    ChildNumber, DerivationPath, ExtendedPubKey, KeyDerivationType,
};
use dash_sdk::dpp::key_wallet::psbt::serialize::Serialize;
use dash_sdk::dpp::prelude::AddressNonce;

use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1};
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::{
    Address, BlockHash, InstantLock, Network, OutPoint, PrivateKey, PublicKey, ScriptBuf,
    Transaction, TxIn, TxOut, Txid,
};
use dash_sdk::dpp::platform_value::BinaryData;
use std::cmp;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Debug;
use std::ops::Range;
use std::sync::{Arc, RwLock};

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
    CoinJoin = 15,
    /// DIP-17: Platform Payment Addresses
    PlatformPayment = 16,
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
            15 => Ok(DerivationPathReference::CoinJoin),
            16 => Ok(DerivationPathReference::PlatformPayment),
            255 => Ok(DerivationPathReference::Root),
            value => Err(format!(
                "value {} not convertable to a DerivationPathReference",
                value
            )),
        }
    }
}

/// Helper methods for working with derivation paths we care about when presenting wallet data.
pub trait DerivationPathHelpers {
    fn is_bip44(&self, network: Network) -> bool;
    fn is_bip44_external(&self, network: Network) -> bool;
    fn is_bip44_change(&self, network: Network) -> bool;
    fn is_asset_lock_funding(&self, network: Network) -> bool;
    fn is_platform_payment(&self, network: Network) -> bool;
    fn bip44_account_index(&self) -> Option<u32>;
    fn bip44_address_index(&self) -> Option<u32>;
    fn platform_payment_path(
        network: Network,
        account: u32,
        key_class: u32,
        index: u32,
    ) -> DerivationPath;
}

impl DerivationPathHelpers for DerivationPath {
    fn is_bip44(&self, network: Network) -> bool {
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        let components = self.as_ref();
        components.len() >= 4
            && components[0] == ChildNumber::Hardened { index: 44 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
    }

    fn is_bip44_external(&self, network: Network) -> bool {
        if !self.is_bip44(network) {
            return false;
        }
        let components = self.as_ref();
        components.len() >= 5 && components[3] == ChildNumber::Normal { index: 0 }
    }

    fn is_bip44_change(&self, network: Network) -> bool {
        if !self.is_bip44(network) {
            return false;
        }
        let components = self.as_ref();
        components.len() >= 5 && components[3] == ChildNumber::Normal { index: 1 }
    }

    fn is_asset_lock_funding(&self, network: Network) -> bool {
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        let components = self.as_ref();
        components.len() == 5
            && components[0] == ChildNumber::Hardened { index: 9 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
            && components[2] == ChildNumber::Hardened { index: 5 }
            && components[3] == ChildNumber::Hardened { index: 1 }
    }

    fn bip44_account_index(&self) -> Option<u32> {
        self.as_ref().get(2).and_then(|child| match child {
            ChildNumber::Hardened { index } => Some(*index),
            _ => None,
        })
    }

    fn bip44_address_index(&self) -> Option<u32> {
        self.as_ref().last().and_then(|child| match child {
            ChildNumber::Normal { index } => Some(*index),
            ChildNumber::Hardened { index } => Some(*index),
            ChildNumber::Normal256 { .. } | ChildNumber::Hardened256 { .. } => None,
        })
    }

    /// Check if this path is a DIP-17 Platform payment path: m/9'/coin_type'/17'/account'/key_class'/index
    fn is_platform_payment(&self, network: Network) -> bool {
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        let components = self.as_ref();
        // DIP-17: m/9'/coin_type'/17'/account'/key_class'/index
        components.len() == 6
            && components[0] == ChildNumber::Hardened { index: 9 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
            && components[2] == ChildNumber::Hardened { index: 17 }
    }

    /// Create a DIP-17 Platform payment derivation path: m/9'/coin_type'/17'/account'/key_class'/index
    fn platform_payment_path(
        network: Network,
        account: u32,
        key_class: u32,
        index: u32,
    ) -> DerivationPath {
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        DerivationPath::from(vec![
            ChildNumber::Hardened { index: 9 },
            ChildNumber::Hardened { index: coin_type },
            ChildNumber::Hardened { index: 17 },
            ChildNumber::Hardened { index: account },
            ChildNumber::Hardened { index: key_class },
            ChildNumber::Normal { index },
        ])
    }
}

use crate::context::AppContext;
use bitflags::bitflags;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::Identity;
use zeroize::Zeroize;

const BOOTSTRAP_BIP44_EXTERNAL_COUNT: u32 = 32;
const BOOTSTRAP_BIP44_CHANGE_COUNT: u32 = 16;
const BOOTSTRAP_BIP32_ACCOUNT_COUNT: u32 = 1;
const BOOTSTRAP_BIP32_ADDRESS_COUNT: u32 = 16;
const BOOTSTRAP_COINJOIN_ACCOUNT_COUNT: u32 = 1;
const BOOTSTRAP_COINJOIN_ADDRESS_COUNT: u32 = 16;
const BOOTSTRAP_IDENTITY_REGISTRATION_FALLBACK: u32 = 8;
const BOOTSTRAP_IDENTITY_INVITATION_COUNT: u32 = 8;
const BOOTSTRAP_IDENTITY_TOPUP_PER_REGISTRATION: u32 = 4;
const BOOTSTRAP_IDENTITY_TOPUP_NOT_BOUND_COUNT: u32 = 8;
const BOOTSTRAP_PROVIDER_ADDRESS_COUNT: u32 = 4;
/// DIP-17: Number of Platform payment addresses to bootstrap per key class
const BOOTSTRAP_PLATFORM_PAYMENT_ADDRESS_COUNT: u32 = 20;

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
        const DASHPAY = 1 << 8;

        // Composite flags
        const IS_FOR_AUTHENTICATION = Self::SINGLE_USER_AUTHENTICATION.bits() | Self::MULTIPLE_USER_AUTHENTICATION.bits();
        const IS_FOR_FUNDS = Self::CLEAR_FUNDS.bits()
            | Self::ANONYMOUS_FUNDS.bits()
            | Self::VIEW_ONLY_FUNDS.bits()
            | Self::PROTECTED_FUNDS.bits()
            | Self::DASHPAY.bits();
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct AddressInfo {
    pub address: Address,
    pub path_type: DerivationPathType,
    pub path_reference: DerivationPathReference,
}

#[derive(Debug, Clone)]
pub struct WalletArcRef {
    pub wallet: Arc<RwLock<Wallet>>,
    pub seed_hash: WalletSeedHash,
}

impl From<Arc<RwLock<Wallet>>> for WalletArcRef {
    fn from(wallet: Arc<RwLock<Wallet>>) -> Self {
        let seed_hash = { wallet.read().unwrap().seed_hash() };
        Self { wallet, seed_hash }
    }
}

impl PartialEq for WalletArcRef {
    fn eq(&self, other: &Self) -> bool {
        self.seed_hash == other.seed_hash
    }
}

/// Information about a Platform address balance and nonce
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlatformAddressInfo {
    pub balance: Credits,
    pub nonce: AddressNonce,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wallet {
    pub wallet_seed: WalletSeed,
    pub uses_password: bool,
    pub master_bip44_ecdsa_extended_public_key: ExtendedPubKey,
    pub address_balances: BTreeMap<Address, u64>,
    /// Historical total received per address (not just current UTXOs)
    pub address_total_received: BTreeMap<Address, u64>,
    pub known_addresses: BTreeMap<Address, DerivationPath>,
    pub watched_addresses: BTreeMap<DerivationPath, AddressInfo>,
    #[allow(clippy::type_complexity)]
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
    pub transactions: Vec<WalletTransaction>,
    pub is_main: bool,
    pub confirmed_balance: u64,
    pub unconfirmed_balance: u64,
    pub total_balance: u64,
    /// DIP-17: Platform address balances and nonces (keyed by Core Address for lookup)
    pub platform_address_info: BTreeMap<Address, PlatformAddressInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WalletTransaction {
    pub txid: Txid,
    pub transaction: Transaction,
    pub timestamp: u64,
    pub height: Option<u32>,
    pub block_hash: Option<BlockHash>,
    pub net_amount: i64,
    pub fee: Option<u64>,
    pub label: Option<String>,
    pub is_ours: bool,
}

impl WalletTransaction {
    pub fn is_incoming(&self) -> bool {
        self.net_amount > 0
    }

    pub fn is_outgoing(&self) -> bool {
        self.net_amount < 0
    }

    pub fn is_confirmed(&self) -> bool {
        self.height.is_some()
    }

    pub fn amount_abs(&self) -> u64 {
        self.net_amount.unsigned_abs()
    }
}

pub type WalletSeedHash = [u8; 32];

#[derive(Debug, Clone, PartialEq)]
pub enum WalletSeed {
    Open(OpenWalletSeed),
    Closed(ClosedWalletSeed),
}
#[derive(Clone, PartialEq)]
pub struct OpenKeyItem<const N: usize> {
    pub seed: [u8; N],
    pub wallet_info: ClosedKeyItem,
}

impl<const N: usize> Debug for OpenKeyItem<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hash = ClosedKeyItem::compute_seed_hash(&self.seed);
        f.debug_struct("OpenKeyItem")
            .field("seed_hash", &hex::encode(hash))
            .finish()
    }
}

// Type alias for OpenWalletSeed with a fixed seed size of 64 bytes
pub type OpenWalletSeed = OpenKeyItem<64>;

#[derive(Debug, Clone, PartialEq)]
pub struct ClosedKeyItem {
    pub seed_hash: WalletSeedHash, // SHA-256 hash of the seed
    pub encrypted_seed: Vec<u8>,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub password_hint: Option<String>,
}

pub type ClosedWalletSeed = ClosedKeyItem;

impl WalletSeed {
    /// Opens the wallet by decrypting the seed using the provided password.
    pub fn open(&mut self, password: &str) -> Result<(), String> {
        match self {
            WalletSeed::Open(_) => {
                // Wallet is already open
                Ok(())
            }
            WalletSeed::Closed(closed_seed) => {
                // Try to decrypt the seed
                let seed = closed_seed.decrypt_seed(password)?;
                let open_wallet_seed = OpenWalletSeed {
                    seed,
                    wallet_info: closed_seed.clone(),
                };
                *self = WalletSeed::Open(open_wallet_seed);
                Ok(())
            }
        }
    }

    /// Opens the wallet by decrypting the seed without using a password.
    pub fn open_no_password(&mut self) -> Result<(), String> {
        match self {
            WalletSeed::Open(_) => {
                // Wallet is already open
                Ok(())
            }
            WalletSeed::Closed(closed_seed) => {
                let open_wallet_seed =
                    OpenWalletSeed {
                        seed: closed_seed.encrypted_seed.clone().try_into().map_err(
                            |e: Vec<u8>| {
                                format!("incorred seed size, expected 64 bytes, got {}", e.len())
                            },
                        )?,
                        wallet_info: closed_seed.clone(),
                    };
                *self = WalletSeed::Open(open_wallet_seed);
                Ok(())
            }
        }
    }

    /// Closes the wallet by securely erasing the seed and transitioning to Closed state.
    // Allow dead_code: This method provides explicit wallet closure functionality,
    // useful for security-conscious applications requiring manual wallet management
    #[allow(dead_code)]
    pub fn close(&mut self) {
        match self {
            WalletSeed::Open(open_seed) => {
                // Zeroize the seed
                open_seed.seed.zeroize();
                // Transition back to ClosedWalletSeed
                let closed_seed = open_seed.wallet_info.clone();
                *self = WalletSeed::Closed(closed_seed);
            }
            WalletSeed::Closed(_) => {
                // Wallet is already closed
            }
        }
    }
}

impl Drop for WalletSeed {
    fn drop(&mut self) {
        // Securely erase sensitive data
        if let WalletSeed::Open(open_seed) = self {
            open_seed.seed.zeroize();
        }
    }
}

impl Wallet {
    pub fn is_open(&self) -> bool {
        matches!(self.wallet_seed, WalletSeed::Open(_))
    }
    pub fn has_balance(&self) -> bool {
        self.confirmed_balance_duffs() > 0 || self.unconfirmed_balance > 0
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

    pub fn confirmed_balance_duffs(&self) -> u64 {
        if self.total_balance > 0 || self.confirmed_balance > 0 || self.unconfirmed_balance > 0 {
            self.confirmed_balance
        } else {
            self.max_balance()
        }
    }

    pub fn unconfirmed_balance_duffs(&self) -> u64 {
        self.unconfirmed_balance
    }

    pub fn total_balance_duffs(&self) -> u64 {
        if self.total_balance > 0 {
            self.total_balance
        } else {
            self.max_balance()
        }
    }

    pub fn update_spv_balances(&mut self, confirmed: u64, unconfirmed: u64, total: u64) {
        self.confirmed_balance = confirmed;
        self.unconfirmed_balance = unconfirmed;
        self.total_balance = total;
    }

    pub fn bootstrap_known_addresses(&mut self, app_context: &AppContext) {
        if !self.is_open() {
            tracing::debug!("Skipping address bootstrap for locked wallet");
            return;
        }

        let network = app_context.network;

        if let Err(err) = self.bootstrap_bip44_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap BIP44 addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_bip32_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap BIP32 addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_coinjoin_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap CoinJoin addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_identity_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap identity addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_provider_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap provider addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_platform_payment_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap Platform payment addresses: {}", err);
        }
    }

    pub fn set_transactions(&mut self, transactions: Vec<WalletTransaction>) {
        self.transactions = transactions;
    }

    pub(crate) fn seed_bytes(&self) -> Result<&[u8; 64], String> {
        match &self.wallet_seed {
            WalletSeed::Open(opened) => Ok(&opened.seed),
            WalletSeed::Closed(_) => Err("Wallet is closed, please decrypt it first".to_string()),
        }
    }

    pub fn seed_hash(&self) -> [u8; 32] {
        match &self.wallet_seed {
            WalletSeed::Open(opened) => opened.wallet_info.seed_hash,
            WalletSeed::Closed(closed) => closed.seed_hash,
        }
    }

    pub fn encrypted_seed_slice(&self) -> &[u8] {
        match &self.wallet_seed {
            WalletSeed::Open(opened) => opened.wallet_info.encrypted_seed.as_slice(),
            WalletSeed::Closed(closed) => closed.encrypted_seed.as_slice(),
        }
    }

    pub fn salt(&self) -> &[u8] {
        match &self.wallet_seed {
            WalletSeed::Open(opened) => opened.wallet_info.salt.as_slice(),
            WalletSeed::Closed(closed) => closed.salt.as_slice(),
        }
    }

    pub fn nonce(&self) -> &[u8] {
        match &self.wallet_seed {
            WalletSeed::Open(opened) => opened.wallet_info.nonce.as_slice(),
            WalletSeed::Closed(closed) => closed.nonce.as_slice(),
        }
    }

    pub fn password_hint(&self) -> &Option<String> {
        match &self.wallet_seed {
            WalletSeed::Open(opened) => &opened.wallet_info.password_hint,
            WalletSeed::Closed(closed) => &closed.password_hint,
        }
    }

    // Allow dead_code: This utility method finds wallets by seed hash in collections,
    // useful for wallet lookup operations and multi-wallet management
    #[allow(dead_code)]
    pub fn find_in_arc_rw_lock_slice(
        slice: &[Arc<RwLock<Wallet>>],
        wallet_seed_hash: WalletSeedHash,
    ) -> Option<Arc<RwLock<Wallet>>> {
        for wallet in slice {
            // Attempt to read the wallet from the RwLock
            let wallet_ref = wallet.read().unwrap();
            // Check if the wallet's seed hash matches the provided wallet_seed_hash
            if wallet_ref.seed_hash() == wallet_seed_hash {
                // Return a clone of the Arc<RwLock<Wallet>> that matches
                return Some(wallet.clone());
            }
        }
        // Return None if no wallet with the matching seed hash is found
        None
    }

    pub fn derive_private_key_in_arc_rw_lock_slice(
        slice: &[Arc<RwLock<Wallet>>],
        wallet_seed_hash: WalletSeedHash,
        derivation_path: &DerivationPath,
        network: Network,
    ) -> Result<Option<[u8; 32]>, String> {
        for wallet in slice {
            // Attempt to read the wallet from the RwLock
            let wallet_ref = wallet.read().unwrap();
            // Check if this wallet's seed hash matches the target hash
            if wallet_ref.seed_hash() == wallet_seed_hash {
                // Attempt to derive the private key using the provided derivation path
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(wallet_ref.seed_bytes()?, network)
                    .map_err(|e| e.to_string())?;
                return Ok(Some(extended_private_key.private_key.secret_bytes()));
            }
        }
        // Return None if no wallet with the matching seed hash is found
        Ok(None)
    }

    pub fn private_key_at_derivation_path(
        &self,
        derivation_path: &DerivationPath,
        network: Network,
    ) -> Result<PrivateKey, String> {
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .map_err(|e| e.to_string())?;
        Ok(extended_private_key.to_priv())
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
                    .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
                    .map(|extended_private_key| extended_private_key.to_priv())
                    .map_err(|e| e.to_string())
            })
            .transpose()
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
            },
        );

        if app_context.core_backend_mode() == crate::spv::CoreBackendMode::Rpc
            && let Ok(client) = app_context.core_client.read()
        {
            let _ = client.import_address(&address, None, Some(false));
        }

        tracing::trace!(
            address = ?&address,
            network = &address.network().to_string(),
            "registered new address"
        );
        Ok(())
    }

    fn bootstrap_bip44_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let coin_type = Self::coin_type(network);
        let secp = Secp256k1::new();
        for (change_flag, max) in [
            (false, BOOTSTRAP_BIP44_EXTERNAL_COUNT),
            (true, BOOTSTRAP_BIP44_CHANGE_COUNT),
        ] {
            for index in 0..max {
                let child_path = [
                    ChildNumber::Normal {
                        index: change_flag as u32,
                    },
                    ChildNumber::Normal { index },
                ];
                let derived = self
                    .master_bip44_ecdsa_extended_public_key
                    .derive_pub(&secp, &child_path)
                    .map_err(|e| e.to_string())?;
                let dash_public_key = PublicKey::from_slice(&derived.public_key.serialize())
                    .map_err(|e| e.to_string())?;
                let derivation_path = DerivationPath::from(vec![
                    ChildNumber::Hardened { index: 44 },
                    ChildNumber::Hardened { index: coin_type },
                    ChildNumber::Hardened { index: 0 },
                    ChildNumber::Normal {
                        index: change_flag as u32,
                    },
                    ChildNumber::Normal { index },
                ]);
                self.register_address_from_public_key(
                    &dash_public_key,
                    &derivation_path,
                    DerivationPathType::CLEAR_FUNDS,
                    DerivationPathReference::BIP44,
                    app_context,
                )?;
            }
        }
        Ok(())
    }

    fn bootstrap_bip32_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        for account in 0..BOOTSTRAP_BIP32_ACCOUNT_COUNT {
            for index in 0..BOOTSTRAP_BIP32_ADDRESS_COUNT {
                let derivation_path = DerivationPath::from(vec![
                    ChildNumber::Hardened { index: account },
                    ChildNumber::Normal { index },
                ]);
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(&seed, network)
                    .map_err(|e| e.to_string())?;
                let private_key = extended_private_key.to_priv();
                self.register_address_from_private_key(
                    &private_key,
                    &derivation_path,
                    DerivationPathType::CLEAR_FUNDS,
                    DerivationPathReference::BIP32,
                    app_context,
                )?;
            }
        }
        Ok(())
    }

    fn bootstrap_coinjoin_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        for account in 0..BOOTSTRAP_COINJOIN_ACCOUNT_COUNT {
            let base_path = DerivationPath::coinjoin_path(network, account);
            for index in 0..BOOTSTRAP_COINJOIN_ADDRESS_COUNT {
                let mut components = base_path.as_ref().to_vec();
                components.push(ChildNumber::Normal { index });
                let derivation_path = DerivationPath::from(components);
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(&seed, network)
                    .map_err(|e| e.to_string())?;
                let private_key = extended_private_key.to_priv();
                self.register_address_from_private_key(
                    &private_key,
                    &derivation_path,
                    DerivationPathType::ANONYMOUS_FUNDS,
                    DerivationPathReference::ProviderFunds,
                    app_context,
                )?;
            }
        }
        Ok(())
    }

    fn bootstrap_identity_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let registration_indices = self.identity_registration_indices();
        self.bootstrap_identity_registration_addresses(
            network,
            app_context,
            &registration_indices,
        )?;
        self.bootstrap_identity_invitation_addresses(network, app_context)?;
        self.bootstrap_identity_topup_addresses(network, app_context, &registration_indices)?;
        Ok(())
    }

    fn bootstrap_identity_registration_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
        registration_indices: &BTreeSet<u32>,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        for &index in registration_indices {
            let derivation_path = DerivationPath::identity_registration_path(network, index);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(&seed, network)
                .map_err(|e| e.to_string())?;
            let private_key = extended_private_key.to_priv();
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::CREDIT_FUNDING,
                DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                app_context,
            )?;
        }
        Ok(())
    }

    fn bootstrap_identity_invitation_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        for index in 0..BOOTSTRAP_IDENTITY_INVITATION_COUNT {
            let derivation_path = DerivationPath::identity_invitation_path(network, index);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(&seed, network)
                .map_err(|e| e.to_string())?;
            let private_key = extended_private_key.to_priv();
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::CREDIT_FUNDING,
                DerivationPathReference::BlockchainIdentityCreditInvitationFunding,
                app_context,
            )?;
        }
        Ok(())
    }

    fn bootstrap_identity_topup_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
        registration_indices: &BTreeSet<u32>,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        for &registration_index in registration_indices {
            for top_up_index in 0..BOOTSTRAP_IDENTITY_TOPUP_PER_REGISTRATION {
                let derivation_path =
                    DerivationPath::identity_top_up_path(network, registration_index, top_up_index);
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(&seed, network)
                    .map_err(|e| e.to_string())?;
                let private_key = extended_private_key.to_priv();
                self.register_address_from_private_key(
                    &private_key,
                    &derivation_path,
                    DerivationPathType::CREDIT_FUNDING,
                    DerivationPathReference::BlockchainIdentityCreditTopupFunding,
                    app_context,
                )?;
            }
        }
        self.bootstrap_identity_topup_not_bound_addresses(network, app_context, &seed)
    }

    fn bootstrap_identity_topup_not_bound_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
        seed: &[u8; 64],
    ) -> Result<(), String> {
        let base_path = AccountType::IdentityTopUpNotBoundToIdentity
            .derivation_path(network)
            .map_err(|e| e.to_string())?;
        for index in 0..BOOTSTRAP_IDENTITY_TOPUP_NOT_BOUND_COUNT {
            let mut components = base_path.as_ref().to_vec();
            components.push(ChildNumber::Normal { index });
            let derivation_path = DerivationPath::from(components);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(seed, network)
                .map_err(|e| e.to_string())?;
            let private_key = extended_private_key.to_priv();
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::CREDIT_FUNDING,
                DerivationPathReference::BlockchainIdentityCreditTopupFunding,
                app_context,
            )?;
        }
        Ok(())
    }

    fn identity_registration_indices(&self) -> BTreeSet<u32> {
        let mut indices: BTreeSet<u32> = self.identities.keys().copied().collect();
        let fallback_limit = BOOTSTRAP_IDENTITY_REGISTRATION_FALLBACK;
        let max_existing = indices.iter().copied().max().unwrap_or(0);
        let target = cmp::max(max_existing.saturating_add(2), fallback_limit);
        indices.extend(0..target);
        indices
    }

    fn bootstrap_provider_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        self.bootstrap_provider_account(network, app_context, AccountType::ProviderVotingKeys)?;
        self.bootstrap_provider_account(network, app_context, AccountType::ProviderOwnerKeys)?;
        Ok(())
    }

    fn bootstrap_provider_account(
        &mut self,
        network: Network,
        app_context: &AppContext,
        account_type: AccountType,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        let base_path = account_type
            .derivation_path(network)
            .map_err(|e| e.to_string())?;
        let key_wallet_reference = account_type.derivation_path_reference();
        let path_reference = DerivationPathReference::try_from(key_wallet_reference as u32)
            .unwrap_or(DerivationPathReference::Unknown);
        for provider_index in 0..BOOTSTRAP_PROVIDER_ADDRESS_COUNT {
            let mut components = base_path.as_ref().to_vec();
            components.push(ChildNumber::Hardened {
                index: provider_index,
            });
            let derivation_path = DerivationPath::from(components);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(&seed, network)
                .map_err(|e| e.to_string())?;
            let private_key = extended_private_key.to_priv();
            self.register_address_from_private_key(
                &private_key,
                &derivation_path,
                DerivationPathType::CLEAR_FUNDS,
                path_reference,
                app_context,
            )?;
        }
        Ok(())
    }

    /// Bootstrap DIP-17 Platform payment addresses (dashevo/tdashevo Bech32m prefix per DIP-18)
    /// These addresses are for receiving Dash Credits on Platform, independent of identities.
    fn bootstrap_platform_payment_addresses(
        &mut self,
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let seed = *self.seed_bytes()?;
        // Default account 0', default key_class 0' (as per DIP-17)
        let account = 0u32;
        let key_class = 0u32;

        for index in 0..BOOTSTRAP_PLATFORM_PAYMENT_ADDRESS_COUNT {
            let derivation_path =
                DerivationPath::platform_payment_path(network, account, key_class, index);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(&seed, network)
                .map_err(|e| e.to_string())?;
            let private_key = extended_private_key.to_priv();

            // Create a P2PKH address for platform payment
            let secp = Secp256k1::new();
            let public_key = private_key.public_key(&secp);
            let platform_address = Address::p2pkh(&public_key, network);

            // Register the Platform address
            self.register_platform_address(
                platform_address,
                &derivation_path,
                DerivationPathType::CLEAR_FUNDS,
                DerivationPathReference::PlatformPayment,
                app_context,
            )?;
        }
        Ok(())
    }

    /// Register a Platform payment address (DIP-17/18).
    /// Platform addresses use different version bytes and are NOT valid on Core chain.
    fn register_platform_address(
        &mut self,
        address: Address,
        derivation_path: &DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
        app_context: &AppContext,
    ) -> Result<(), String> {
        // Store the address in known_addresses and watched_addresses
        // Note: We don't import to Core wallet since Platform addresses are not valid there
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
            },
        );

        tracing::trace!(
            address = ?&address,
            network = &app_context.network.to_string(),
            "registered new Platform payment address"
        );
        Ok(())
    }

    fn coin_type(network: Network) -> u32 {
        match network {
            Network::Dash => 5,
            _ => 1,
        }
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

    /// Generate a Platform receive address .
    /// Either returns an existing unused Platform address or generates a new one.
    pub fn platform_receive_address(
        &mut self,
        network: Network,
        skip_known_addresses_with_no_funds: bool,
        register: Option<&AppContext>,
    ) -> Result<Address, String> {
        let seed = *self.seed_bytes()?;
        let secp = Secp256k1::new();
        let account = 0u32;
        let key_class = 0u32;

        // Find the highest index in existing Platform payment addresses
        let existing_indices: Vec<u32> = self
            .watched_addresses
            .iter()
            .filter(|(path, _)| path.is_platform_payment(network))
            .filter_map(|(path, _)| {
                // Extract the index from the path (last component)
                path.into_iter().last().and_then(|child| match child {
                    ChildNumber::Normal { index } | ChildNumber::Hardened { index } => Some(*index),
                    _ => None,
                })
            })
            .collect();

        // If skipping known addresses with no funds, try to find one with zero balance
        if !skip_known_addresses_with_no_funds {
            // Return first existing address with zero balance
            for (path, info) in &self.watched_addresses {
                if path.is_platform_payment(network) {
                    if let Some(addr_info) = self.platform_address_info.get(&info.address) {
                        if addr_info.balance == 0 {
                            return Ok(info.address.clone());
                        }
                    } else {
                        // No balance info means zero balance
                        return Ok(info.address.clone());
                    }
                }
            }
        }

        // Generate a new Platform address at the next index
        let next_index = existing_indices.iter().max().map(|m| m + 1).unwrap_or(0);

        let derivation_path =
            DerivationPath::platform_payment_path(network, account, key_class, next_index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&seed, network)
            .map_err(|e| e.to_string())?;
        let private_key = extended_private_key.to_priv();
        let public_key = private_key.public_key(&secp);

        // Create a P2PKH address for platform payment
        let platform_address = Address::p2pkh(&public_key, network);

        // Register the new address
        if let Some(app_context) = register {
            self.register_platform_address(
                platform_address.clone(),
                &derivation_path,
                DerivationPathType::CLEAR_FUNDS,
                DerivationPathReference::PlatformPayment,
                app_context,
            )?;
        } else {
            // Just update local state without persisting
            self.known_addresses
                .insert(platform_address.clone(), derivation_path.clone());
            self.watched_addresses.insert(
                derivation_path,
                AddressInfo {
                    address: platform_address.clone(),
                    path_type: DerivationPathType::CLEAR_FUNDS,
                    path_reference: DerivationPathReference::PlatformPayment,
                },
            );
        }

        Ok(platform_address)
    }

    pub fn derive_bip44_address(
        &self,
        network: Network,
        change: bool,
        address_index: u32,
    ) -> Result<Address, String> {
        let secp = Secp256k1::new();
        let path_extension = [
            ChildNumber::Normal {
                index: change as u32,
            },
            ChildNumber::Normal {
                index: address_index,
            },
        ];
        let public_key = self
            .master_bip44_ecdsa_extended_public_key
            .derive_pub(&secp, &path_extension)
            .map_err(|e| e.to_string())?
            .to_pub();
        Ok(Address::p2pkh(&public_key, network))
    }

    pub fn build_standard_payment_transaction(
        &mut self,
        network: Network,
        recipient: &Address,
        amount: u64,
        fee: u64,
        subtract_fee_from_amount: bool,
        register_addresses: Option<&AppContext>,
    ) -> Result<Transaction, String> {
        if recipient.network() != &network {
            return Err(format!(
                "Recipient address network ({}) does not match wallet network ({})",
                recipient.network(),
                network
            ));
        }

        let (utxos, change_option) = self
            .take_unspent_utxos_for(amount, fee, subtract_fee_from_amount)
            .ok_or_else(|| "Insufficient funds".to_string())?;

        let send_value = if change_option.is_none() && subtract_fee_from_amount {
            let total_input: u64 = utxos.values().map(|(tx_out, _)| tx_out.value).sum();
            total_input
                .checked_sub(fee)
                .ok_or_else(|| "Fee exceeds available amount".to_string())?
        } else {
            amount
        };

        if send_value == 0 {
            return Err("Amount is zero after subtracting fee".to_string());
        }

        let mut outputs = vec![TxOut {
            value: send_value,
            script_pubkey: recipient.script_pubkey(),
        }];

        if let Some(change) = change_option {
            let change_address = self.change_address(network, register_addresses)?;
            outputs.push(TxOut {
                value: change,
                script_pubkey: change_address.script_pubkey(),
            });
        }

        let mut tx = Transaction {
            version: 2,
            lock_time: 0,
            input: utxos
                .keys()
                .map(|outpoint| TxIn {
                    previous_output: *outpoint,
                    ..Default::default()
                })
                .collect(),
            output: outputs,
            special_transaction_payload: None,
        };

        let sighash_flag = 1u32;
        let cache = SighashCache::new(&tx);
        let sighashes: Vec<_> = tx
            .input
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let script_pubkey = utxos
                    .get(&input.previous_output)
                    .expect("missing utxo when computing sighash")
                    .0
                    .script_pubkey
                    .clone();
                cache
                    .legacy_signature_hash(i, &script_pubkey, sighash_flag)
                    .expect("failed to compute sighash")
            })
            .collect();

        let secp = Secp256k1::new();
        let mut utxo_lookup = utxos.clone();

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                let (_, input_address) = utxo_lookup
                    .remove(&input.previous_output)
                    .expect("utxo missing when signing");
                let private_key = self
                    .private_key_for_address(&input_address, network)?
                    .ok_or_else(|| format!("Address {} not managed by wallet", input_address))?;
                let message = Message::from_digest(sighash.into());
                let sig = secp.sign_ecdsa(&message, &private_key.inner);
                let mut serialized_sig = sig.serialize_der().to_vec();
                let mut script_sig = vec![serialized_sig.len() as u8 + 1];
                script_sig.append(&mut serialized_sig);
                script_sig.push(1);
                let mut serialized_pub_key = private_key.public_key(&secp).serialize();
                script_sig.push(serialized_pub_key.len() as u8);
                script_sig.append(&mut serialized_pub_key);
                input.script_sig = ScriptBuf::from_bytes(script_sig);
                Ok::<(), String>(())
            })?;

        Ok(tx)
    }

    /// Build a transaction with multiple recipients
    pub fn build_multi_recipient_payment_transaction(
        &mut self,
        network: Network,
        recipients: &[(Address, u64)],
        fee: u64,
        subtract_fee_from_amount: bool,
        register_addresses: Option<&AppContext>,
    ) -> Result<Transaction, String> {
        if recipients.is_empty() {
            return Err("No recipients specified".to_string());
        }

        // Validate all recipients are on the correct network
        for (recipient, _) in recipients {
            if recipient.network() != &network {
                return Err(format!(
                    "Recipient address network ({}) does not match wallet network ({})",
                    recipient.network(),
                    network
                ));
            }
        }

        // Calculate total amount needed
        let total_amount: u64 = recipients.iter().map(|(_, amount)| *amount).sum();

        let (utxos, change_option) = self
            .take_unspent_utxos_for(total_amount, fee, subtract_fee_from_amount)
            .ok_or_else(|| "Insufficient funds".to_string())?;

        // Build outputs for each recipient
        let mut outputs: Vec<TxOut> = if change_option.is_none() && subtract_fee_from_amount {
            // If we're subtracting fee and using all funds, we need to reduce recipient amounts proportionally
            let total_input: u64 = utxos.values().map(|(tx_out, _)| tx_out.value).sum();
            let available_after_fee = total_input
                .checked_sub(fee)
                .ok_or_else(|| "Fee exceeds available amount".to_string())?;

            // Distribute the reduction proportionally across recipients
            let reduction_ratio = available_after_fee as f64 / total_amount as f64;

            recipients
                .iter()
                .map(|(recipient, amount)| {
                    let adjusted_amount = (*amount as f64 * reduction_ratio) as u64;
                    TxOut {
                        value: adjusted_amount,
                        script_pubkey: recipient.script_pubkey(),
                    }
                })
                .collect()
        } else {
            recipients
                .iter()
                .map(|(recipient, amount)| TxOut {
                    value: *amount,
                    script_pubkey: recipient.script_pubkey(),
                })
                .collect()
        };

        // Check that no output is zero
        if outputs.iter().any(|o| o.value == 0) {
            return Err("One or more amounts are zero after subtracting fee".to_string());
        }

        // Add change output if needed
        if let Some(change) = change_option {
            let change_address = self.change_address(network, register_addresses)?;
            outputs.push(TxOut {
                value: change,
                script_pubkey: change_address.script_pubkey(),
            });
        }

        let mut tx = Transaction {
            version: 2,
            lock_time: 0,
            input: utxos
                .keys()
                .map(|outpoint| TxIn {
                    previous_output: *outpoint,
                    ..Default::default()
                })
                .collect(),
            output: outputs,
            special_transaction_payload: None,
        };

        let sighash_flag = 1u32;
        let cache = SighashCache::new(&tx);
        let sighashes: Vec<_> = tx
            .input
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let script_pubkey = utxos
                    .get(&input.previous_output)
                    .expect("missing utxo when computing sighash")
                    .0
                    .script_pubkey
                    .clone();
                cache
                    .legacy_signature_hash(i, &script_pubkey, sighash_flag)
                    .expect("failed to compute sighash")
            })
            .collect();

        let secp = Secp256k1::new();
        let mut utxo_lookup = utxos.clone();

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                let (_, input_address) = utxo_lookup
                    .remove(&input.previous_output)
                    .expect("utxo missing when signing");
                let private_key = self
                    .private_key_for_address(&input_address, network)?
                    .ok_or_else(|| format!("Address {} not managed by wallet", input_address))?;
                let message = Message::from_digest(sighash.into());
                let sig = secp.sign_ecdsa(&message, &private_key.inner);
                let mut serialized_sig = sig.serialize_der().to_vec();
                let mut script_sig = vec![serialized_sig.len() as u8 + 1];
                script_sig.append(&mut serialized_sig);
                script_sig.push(1);
                let mut serialized_pub_key = private_key.public_key(&secp).serialize();
                script_sig.push(serialized_pub_key.len() as u8);
                script_sig.append(&mut serialized_pub_key);
                input.script_sig = ScriptBuf::from_bytes(script_sig);
                Ok::<(), String>(())
            })?;

        Ok(tx)
    }

    pub fn update_address_balance(
        &mut self,
        address: &Address,
        new_balance: Duffs,
        context: &AppContext,
    ) -> Result<(), String> {
        // Check if the new balance differs from the current one.
        if let Some(current_balance) = self.address_balances.get(address)
            && *current_balance == new_balance
        {
            // If the balance hasn't changed, skip the update.
            return Ok(());
        }

        // If there's no current balance or it has changed, update it.
        self.address_balances.insert(address.clone(), new_balance);

        // Update the database with the new balance.
        context
            .db
            .update_address_balance(&self.seed_hash(), address, new_balance)
            .map_err(|e| e.to_string())
    }

    pub fn update_address_total_received(
        &mut self,
        address: &Address,
        total_received: Duffs,
        context: &AppContext,
    ) -> Result<(), String> {
        // Check if the total received differs from the current value
        if let Some(current_total) = self.address_total_received.get(address)
            && *current_total == total_received
        {
            // If the total received hasn't changed, skip the update.
            return Ok(());
        }

        // Update in memory
        self.address_total_received
            .insert(address.clone(), total_received);

        // Update the database
        context
            .db
            .update_address_total_received(&self.seed_hash(), address, total_received)
            .map_err(|e| e.to_string())
    }

    /// Get all Platform payment addresses from this wallet
    pub fn platform_addresses(&self, network: Network) -> Vec<(Address, PlatformAddress)> {
        self.watched_addresses
            .iter()
            .filter(|(path, _)| path.is_platform_payment(network))
            .filter_map(|(_, info)| {
                PlatformAddress::try_from(info.address.clone())
                    .ok()
                    .map(|platform_addr| (info.address.clone(), platform_addr))
            })
            .collect()
    }

    /// Get the total Platform balance (sum of all Platform address balances)
    pub fn total_platform_balance(&self) -> Credits {
        self.platform_address_info
            .values()
            .map(|info| info.balance)
            .sum()
    }

    /// Update Platform address info (balance and nonce)
    pub fn set_platform_address_info(
        &mut self,
        address: Address,
        balance: Credits,
        nonce: AddressNonce,
    ) {
        self.platform_address_info
            .insert(address, PlatformAddressInfo { balance, nonce });
    }

    /// Get the private key for a Platform address
    #[allow(clippy::result_large_err)]
    pub fn get_platform_address_private_key(
        &self,
        platform_address: &PlatformAddress,
        network: Network,
    ) -> Result<PrivateKey, ProtocolError> {
        // Find the derivation path by looking through watched_addresses
        // and matching the PlatformAddress
        let derivation_path = self
            .watched_addresses
            .iter()
            .filter(|(path, _)| path.is_platform_payment(network))
            .find_map(|(path, info)| {
                // Try to convert the stored address to a PlatformAddress and compare
                PlatformAddress::try_from(info.address.clone())
                    .ok()
                    .filter(|addr| addr == platform_address)
                    .map(|_| path.clone())
            })
            .ok_or_else(|| {
                ProtocolError::Generic(format!(
                    "Platform address {:?} not found in wallet",
                    platform_address
                ))
            })?;

        // Get the seed bytes
        let seed = *self.seed_bytes().map_err(ProtocolError::Generic)?;

        // Derive the private key
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&seed, network)
            .map_err(|e| ProtocolError::Generic(e.to_string()))?;

        Ok(extended_private_key.to_priv())
    }
}

/// Signer implementation for Platform addresses
/// Allows the wallet to sign transactions that spend from Platform addresses
impl Signer<PlatformAddress> for Wallet {
    fn sign(
        &self,
        platform_address: &PlatformAddress,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        // Only P2PKH addresses are supported for now
        if !platform_address.is_p2pkh() {
            return Err(ProtocolError::Generic(
                "Only P2PKH Platform addresses are currently supported for signing".to_string(),
            ));
        }

        // We need the network to derive the key, but we don't have it here.
        // Try both networks - the address will only exist in one
        let private_key = self
            .get_platform_address_private_key(platform_address, Network::Dash)
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Testnet))
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Devnet))
            .or_else(|_| {
                self.get_platform_address_private_key(platform_address, Network::Regtest)
            })?;

        // Sign the data
        let signature = dash_sdk::dpp::dashcore::signer::sign(data, private_key.inner.as_ref())
            .map_err(|e| ProtocolError::Generic(format!("Failed to sign: {}", e)))?;

        Ok(BinaryData::new(signature.to_vec()))
    }

    fn sign_create_witness(
        &self,
        platform_address: &PlatformAddress,
        data: &[u8],
    ) -> Result<AddressWitness, ProtocolError> {
        // Only P2PKH addresses are supported for now
        if !platform_address.is_p2pkh() {
            return Err(ProtocolError::Generic(
                "Only P2PKH Platform addresses are currently supported for signing".to_string(),
            ));
        }

        // Get the private key (try all networks)
        let private_key = self
            .get_platform_address_private_key(platform_address, Network::Dash)
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Testnet))
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Devnet))
            .or_else(|_| {
                self.get_platform_address_private_key(platform_address, Network::Regtest)
            })?;

        // Sign the data - produces a compact recoverable signature
        // The public key will be recovered from the signature during verification
        let signature = dash_sdk::dpp::dashcore::signer::sign(data, private_key.inner.as_ref())
            .map_err(|e| ProtocolError::Generic(format!("Failed to sign: {}", e)))?;

        Ok(AddressWitness::P2pkh {
            signature: BinaryData::new(signature.to_vec()),
        })
    }

    fn can_sign_with(&self, platform_address: &PlatformAddress) -> bool {
        // Only P2PKH addresses are supported
        if !platform_address.is_p2pkh() {
            return false;
        }

        // Check if we have the private key for this address
        self.get_platform_address_private_key(platform_address, Network::Dash)
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Testnet))
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Devnet))
            .or_else(|_| self.get_platform_address_private_key(platform_address, Network::Regtest))
            .is_ok()
    }
}
