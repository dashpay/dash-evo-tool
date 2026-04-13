pub mod encryption;
pub mod shielded;
pub mod single_key;
mod utxos;

use crate::backend_task::error::TaskError;
use crate::database::{Database, WalletError};
use crate::model::secret::Secret;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::key_wallet::bip32::{
    ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey, KeyDerivationType,
};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use dash_sdk::platform::address_sync::{AddressFunds, AddressIndex, AddressProvider};

use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::{
    Address, BlockHash, Network, OutPoint, PrivateKey, PublicKey, Transaction, TxOut, Txid,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::ops::Range;
use std::sync::{Arc, RwLock};

// BIP44 derivation path constants for Dash HD wallets.
// Mainnet: m/44'/5'/0'   Testnet/Devnet/Regtest: m/44'/1'/0'

/// BIP44 purpose index (standard for HD wallets).
pub const BIP44_PURPOSE: u32 = 44;

/// Dash mainnet coin type (registered in SLIP-0044).
pub const DASH_COIN_TYPE: u32 = 5;

/// Testnet coin type (shared across all testnet-like networks).
pub const DASH_TESTNET_COIN_TYPE: u32 = 1;

/// BIP44 account 0 path for Dash mainnet: `m/44'/5'/0'`.
pub const DASH_BIP44_ACCOUNT_0_PATH_MAINNET: [ChildNumber; 3] = [
    ChildNumber::Hardened {
        index: BIP44_PURPOSE,
    },
    ChildNumber::Hardened {
        index: DASH_COIN_TYPE,
    },
    ChildNumber::Hardened { index: 0 },
];

/// BIP44 account 0 path for Dash testnet/devnet/regtest: `m/44'/1'/0'`.
pub const DASH_BIP44_ACCOUNT_0_PATH_TESTNET: [ChildNumber; 3] = [
    ChildNumber::Hardened {
        index: BIP44_PURPOSE,
    },
    ChildNumber::Hardened {
        index: DASH_TESTNET_COIN_TYPE,
    },
    ChildNumber::Hardened { index: 0 },
];

/// Check if two networks use the same address format.
/// Testnet, Devnet, and Regtest all use testnet-style addresses.
pub(crate) fn networks_address_compatible(a: &Network, b: &Network) -> bool {
    matches!(
        (a, b),
        (Network::Mainnet, Network::Mainnet)
            | (
                Network::Testnet | Network::Devnet | Network::Regtest,
                Network::Testnet | Network::Devnet | Network::Regtest,
            )
    )
}

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
    fn is_bip32(&self) -> bool;
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

pub(crate) fn is_bip44_path(path: &DerivationPath, network: Network) -> bool {
    let coin_type = match network {
        Network::Mainnet => 5,
        _ => 1,
    };
    let components = path.as_ref();
    components.len() >= 4
        && components[0] == ChildNumber::Hardened { index: 44 }
        && components[1] == ChildNumber::Hardened { index: coin_type }
}

impl DerivationPathHelpers for DerivationPath {
    fn is_bip44(&self, network: Network) -> bool {
        is_bip44_path(self, network)
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

    fn is_bip32(&self) -> bool {
        let components = self.as_ref();
        matches!(components.len(), 2..=3) && components[0] == ChildNumber::Hardened { index: 0 }
    }

    fn is_asset_lock_funding(&self, network: Network) -> bool {
        let coin_type = match network {
            Network::Mainnet => 5,
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
            Network::Mainnet => 5,
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
            Network::Mainnet => 5,
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
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::hashes::Hash;
use zeroize::Zeroize;

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
#[derive(Debug, Clone)]
pub struct WalletArcRef {
    pub wallet: Arc<RwLock<Wallet>>,
    pub wallet_id: WalletId,
}

impl From<Arc<RwLock<Wallet>>> for WalletArcRef {
    fn from(wallet: Arc<RwLock<Wallet>>) -> Self {
        let wallet_id = wallet
            .read()
            .map(|w| w.wallet_id())
            .unwrap_or_else(|poisoned| {
                tracing::warn!("Wallet lock poisoned during WalletArcRef conversion");
                poisoned.into_inner().wallet_id()
            });
        Self { wallet, wallet_id }
    }
}

impl PartialEq for WalletArcRef {
    fn eq(&self, other: &Self) -> bool {
        self.wallet_id == other.wallet_id
    }
}

#[derive(Debug, Clone)]
pub struct Wallet {
    /// The platform wallet — `None` when the wallet is locked (encrypted seed).
    /// Set on creation (open wallet) or on unlock.
    pub platform_wallet: Option<Arc<crate::platform_wallet_bridge::PlatformWallet>>,
    pub wallet_seed: WalletSeed,
    pub uses_password: bool,
    pub master_bip44_ecdsa_extended_public_key: ExtendedPubKey,
    pub alias: Option<String>,
    pub is_main: bool,
    /// Dash Core wallet name for multi-wallet RPC calls
    pub core_wallet_name: Option<String>,
    /// Platform-wallet `WalletId` (`SHA256(root_pub_key || chain_code)`).
    /// Always populated — the wallet migration screen at first launch
    /// after v40 ensures every wallet has wallet_id before the main
    /// UI loads. Use this as the canonical wallet identifier for map
    /// keys, DB queries, and persistence.
    pub wallet_id: crate::platform_wallet_bridge::WalletId,
}

impl Wallet {
    /// Create a new HD wallet from a BIP39 seed.
    ///
    /// This is a pure construction method with no side effects — it does not
    /// touch the database or register the wallet anywhere. It derives the
    /// master BIP44 public key, computes the seed hash, optionally encrypts
    /// the seed, and populates the first receive address.
    ///
    /// Use [`AppContext::register_wallet()`] to persist and activate the wallet.
    pub fn new_from_seed(
        seed: [u8; 64],
        network: Network,
        alias: Option<String>,
        password: Option<&Secret>,
    ) -> Result<Self, TaskError> {
        // Encrypt seed or store plaintext
        let (encrypted_seed, salt, nonce, uses_password) = match password {
            Some(pw) if !pw.is_empty() => {
                let (enc, s, n) = ClosedKeyItem::encrypt_seed(&seed, pw.expose_secret())
                    .map_err(|e| TaskError::EncryptionError { detail: e })?;
                (enc, s, n, true)
            }
            _ => (seed.to_vec(), vec![], vec![], false),
        };

        let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);

        // Derive master BIP44 extended public key
        let master_priv = ExtendedPrivKey::new_master(network, &seed).map_err(|e| {
            TaskError::WalletKeyDerivationFailed {
                source: Box::new(e),
            }
        })?;
        let bip44_path = Self::bip44_account0_path(network);
        let secp = Secp256k1::new();
        let account_priv = master_priv.derive_priv(&secp, &bip44_path).map_err(|e| {
            TaskError::WalletKeyDerivationFailed {
                source: Box::new(e),
            }
        })?;
        let master_bip44_ecdsa_extended_public_key =
            ExtendedPubKey::from_priv(&secp, &account_priv);

        // Compute WalletId = SHA256(root_pub_key || chain_code) eagerly.
        // master_priv is at derivation path m (root), so its pubkey is
        // the root extended public key.
        let root_pub = ExtendedPubKey::from_priv(&secp, &master_priv);
        let wallet_id = Self::compute_wallet_id_from_root_pub(&root_pub);

        Ok(Wallet {
            platform_wallet: None,
            wallet_seed: WalletSeed::Open(OpenWalletSeed {
                seed,
                wallet_info: ClosedKeyItem {
                    seed_hash,
                    encrypted_seed,
                    salt,
                    nonce,
                    password_hint: None,
                },
            }),
            uses_password,
            master_bip44_ecdsa_extended_public_key,
            alias,
            is_main: true,
            core_wallet_name: None,
            wallet_id,
        })
    }

    /// Returns the BIP44 account 0 derivation path for the given network.
    fn bip44_account0_path(network: Network) -> DerivationPath {
        match network {
            Network::Mainnet => DerivationPath::from(DASH_BIP44_ACCOUNT_0_PATH_MAINNET.as_slice()),
            _ => DerivationPath::from(DASH_BIP44_ACCOUNT_0_PATH_TESTNET.as_slice()),
        }
    }

    /// Compute `WalletId = SHA256(root_pub_key.serialize() || chain_code)`
    /// from a root-level extended public key. This matches
    /// `key_wallet::Wallet::compute_wallet_id`.
    pub fn compute_wallet_id_from_root_pub(
        root_pub: &ExtendedPubKey,
    ) -> crate::platform_wallet_bridge::WalletId {
        use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};
        let mut data = Vec::with_capacity(33 + 32);
        data.extend_from_slice(&root_pub.public_key.serialize());
        data.extend_from_slice(&root_pub.chain_code[..]);
        sha256::Hash::hash(&data).to_byte_array()
    }

    /// The platform-wallet `WalletId`.
    pub fn wallet_id(&self) -> crate::platform_wallet_bridge::WalletId {
        self.wallet_id
    }
}

/// Transaction lifecycle status.
///
/// Tracks the progression: Unconfirmed → InstantSendLocked → Confirmed → ChainLocked.
/// Currently only Unconfirmed and Confirmed can be inferred from upstream data;
/// InstantSendLocked and ChainLocked require upstream changes (rust-dashcore#569).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TransactionStatus {
    /// In mempool, no InstantSend lock
    Unconfirmed = 0,
    /// InstantSend-locked but not yet mined (requires rust-dashcore#569)
    InstantSendLocked = 1,
    /// Mined in a block
    Confirmed = 2,
    /// In a chain-locked block (highest finality, requires rust-dashcore#569)
    ChainLocked = 3,
}

impl TransactionStatus {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Unconfirmed,
            1 => Self::InstantSendLocked,
            2 => Self::Confirmed,
            3 => Self::ChainLocked,
            _ => {
                tracing::warn!("Unknown TransactionStatus value {v}, defaulting to Unconfirmed");
                Self::Unconfirmed
            }
        }
    }

    /// Infer status from block height presence.
    /// This is a best-effort heuristic until upstream provides richer context.
    pub fn from_height(height: Option<u32>) -> Self {
        if height.is_some() {
            Self::Confirmed
        } else {
            Self::Unconfirmed
        }
    }

    /// User-facing label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unconfirmed => "Unconfirmed",
            Self::InstantSendLocked => "InstantSend",
            Self::Confirmed => "Confirmed",
            Self::ChainLocked => "ChainLocked",
        }
    }
}

impl std::fmt::Display for TransactionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
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
    pub status: TransactionStatus,
}

impl WalletTransaction {
    pub fn is_incoming(&self) -> bool {
        self.net_amount > 0
    }

    pub fn is_outgoing(&self) -> bool {
        self.net_amount < 0
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(
            self.status,
            TransactionStatus::Confirmed | TransactionStatus::ChainLocked
        )
    }

    pub fn amount_abs(&self) -> u64 {
        self.net_amount.unsigned_abs()
    }
}

/// Canonical wallet identifier type. `WalletId = [u8; 32]` =
/// `SHA256(root_pub_key || chain_code)`, matching platform-wallet's
/// `key_wallet_manager::WalletId`. Replaces the former
/// `WalletSeedHash` (which was `SHA256(seed_bytes)`).
pub use crate::platform_wallet_bridge::WalletId;

/// Legacy alias kept for `ClosedKeyItem.seed_hash` field type.
/// Both are `[u8; 32]` — the alias is purely for documentation.
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
    pub seed_hash: WalletId, // SHA-256 hash of the seed
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
                                format!("incorrect seed size, expected 64 bytes, got {}", e.len())
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
    /// Convert a Platform address to a canonical Core address representation for map keys.
    ///
    /// This ensures we always use the same `dashcore::Address` instance for a given Platform
    /// address, avoiding duplicate map entries caused by different internal representations.
    pub(crate) fn canonical_address(address: &Address, network: Network) -> Address {
        PlatformAddress::try_from(address.clone())
            .map(|pa| pa.to_address_with_network(network))
            .unwrap_or_else(|_| address.clone())
    }

    pub fn is_open(&self) -> bool {
        matches!(self.wallet_seed, WalletSeed::Open(_))
    }
    pub fn has_balance(&self) -> bool {
        self.platform_wallet
            .as_ref()
            .map(|pw| pw.core().balance().total() > 0)
            .unwrap_or(false)
    }

    /// Checks whether this wallet has any unused asset locks in the database
    /// (where identity_id IS NULL and identity_id_potentially_in_creation IS NULL).
    pub fn has_unused_asset_lock(&self, db: &Database, network: Network) -> bool {
        db.has_unused_asset_lock_transactions(&self.wallet_id(), network)
            .unwrap_or(false)
    }

    /// All addresses from PlatformWallet's CoreAddressInfo.
    /// Returns empty if the wallet is locked (no PlatformWallet) or
    /// the wallet manager lock is contended.
    pub fn all_addresses_info(&self) -> Vec<crate::platform_wallet_bridge::CoreAddressInfo> {
        self.platform_wallet
            .as_ref()
            .and_then(|pw| pw.try_state())
            .map(|info| {
                crate::platform_wallet_bridge::CoreAddressInfo::all_from_wallet_info(
                    &info.core_wallet,
                )
            })
            .unwrap_or_default()
    }

    /// Look up the derivation path for an address via PlatformWallet.
    /// Returns `None` if the wallet is locked, the lock is contended,
    /// or the address isn't known.
    ///
    /// O(accounts × pools-per-account) where each step is an O(1)
    /// `HashMap<Address, u32>` lookup on the per-pool reverse index.
    /// No address catalog rebuild, no per-call allocations beyond
    /// the `Vec<&ManagedCoreAccount>` from `all_accounts()` and the
    /// `clone()` of the matching `AddressInfo.path`.
    ///
    /// Holistic-review PC1 (performance-oracle): the previous
    /// implementation called `CoreAddressInfo::all_from_wallet_info`
    /// on every call, which walked every address in every pool and
    /// allocated a fresh `Vec<CoreAddressInfo>` per call. On a
    /// contact-heavy wallet the SPV hot path spent ~10k allocations
    /// per tx output there. This version goes straight to the
    /// existing per-pool `address_index` HashMap in key-wallet.
    ///
    /// Uses `try_state()` so calls from egui render callbacks (which
    /// run inside a tokio runtime context) never panic; returns
    /// `None` on lock contention and the next frame retries.
    pub fn derivation_path_for_address(&self, address: &Address) -> Option<DerivationPath> {
        let pw = self.platform_wallet.as_ref()?;
        let info = pw.try_state()?;
        for account in info.core_wallet.accounts.all_accounts() {
            if let Some(addr_info) = account.get_address_info(address) {
                return Some(addr_info.path);
            }
        }
        None
    }

    /// Async version of `derivation_path_for_address` — safe to call from async context.
    pub async fn derivation_path_for_address_async(
        &self,
        address: &Address,
    ) -> Option<DerivationPath> {
        let pw = self.platform_wallet.as_ref()?;
        let info = pw.state().await;
        for account in info.core_wallet.accounts.all_accounts() {
            if let Some(addr_info) = account.get_address_info(address) {
                return Some(addr_info.path);
            }
        }
        None
    }

    /// Check if an address belongs to this wallet via PlatformWallet.
    /// Returns `false` if the wallet is locked (no PlatformWallet) or
    /// the lock is contended (next frame will retry).
    ///
    /// Faster than `derivation_path_for_address(...).is_some()`
    /// because it uses `contains_address` which stays inside the
    /// per-pool HashMap and never clones the matching `AddressInfo`.
    pub fn has_address(&self, address: &Address) -> bool {
        let Some(pw) = self.platform_wallet.as_ref() else {
            return false;
        };
        let Some(info) = pw.try_state() else {
            return false;
        };
        info.core_wallet
            .accounts
            .all_accounts()
            .iter()
            .any(|a| a.contains_address(address))
    }

    /// Async version of `has_address` — safe to call from async context.
    pub async fn has_address_async(&self, address: &Address) -> bool {
        let Some(pw) = self.platform_wallet.as_ref() else {
            return false;
        };
        let info = pw.state().await;
        info.core_wallet
            .accounts
            .all_accounts()
            .iter()
            .any(|a| a.contains_address(address))
    }

    /// Per-address balance from PlatformWallet's pool state.
    /// Uses the per-pool `HashMap<Address, u32>` (see PC1 above)
    /// rather than rebuilding the full address catalog.
    /// Returns 0 if the wallet is locked or the lock is contended.
    pub fn address_balance(&self, address: &Address) -> u64 {
        let Some(pw) = self.platform_wallet.as_ref() else {
            return 0;
        };
        let Some(info) = pw.try_state() else {
            return 0;
        };
        for account in info.core_wallet.accounts.all_accounts() {
            if let Some(addr_info) = account.get_address_info(address) {
                return addr_info.balance;
            }
        }
        0
    }

    pub fn confirmed_balance_duffs(&self) -> u64 {
        self.platform_wallet
            .as_ref()
            .map(|pw| pw.core().balance().spendable())
            .unwrap_or(0)
    }

    /// Returns the SPV-reported confirmed balance, or `None` if the platform
    /// wallet is not available (locked). Callers that need certainty
    /// (e.g., test waiters) should use this and retry on `None`.
    pub fn spv_confirmed_balance(&self) -> Option<u64> {
        self.platform_wallet
            .as_ref()
            .map(|pw| pw.core().balance().spendable())
    }

    pub fn unconfirmed_balance_duffs(&self) -> u64 {
        self.platform_wallet
            .as_ref()
            .map(|pw| pw.core().balance().unconfirmed())
            .unwrap_or(0)
    }

    pub fn total_balance_duffs(&self) -> u64 {
        self.platform_wallet
            .as_ref()
            .map(|pw| pw.core().balance().total())
            .unwrap_or(0)
    }

    /// Read transaction history from the PlatformWallet's ManagedWalletInfo.
    ///
    /// Returns an empty vec when the wallet is locked or the wallet
    /// manager lock is contended.
    pub fn get_transactions(&self) -> Vec<WalletTransaction> {
        let Some(pw) = self.platform_wallet.as_ref() else {
            return Vec::new();
        };
        let Some(info) = pw.try_state() else {
            return Vec::new();
        };
        info.core_wallet
            .transaction_history()
            .into_iter()
            .map(|record| {
                let height = record.height();
                let status = TransactionStatus::from_height(height);
                let block_info = record.context.block_info();
                WalletTransaction {
                    txid: record.txid,
                    transaction: record.transaction.clone(),
                    timestamp: block_info.map(|bi| bi.timestamp() as u64).unwrap_or(0),
                    height,
                    block_hash: block_info.map(|bi| bi.block_hash()),
                    net_amount: record.net_amount,
                    fee: record.fee,
                    label: Some(record.label.clone()).filter(|s| !s.is_empty()),
                    is_ours: true,
                    status,
                }
            })
            .collect()
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
        wallet_seed_hash: WalletId,
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
        wallet_seed_hash: WalletId,
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
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        Ok(extended_private_key.to_priv())
    }

    pub fn private_key_for_address(
        &self,
        address: &Address,
        network: Network,
    ) -> Result<Option<PrivateKey>, String> {
        self.derivation_path_for_address(address)
            .map(|derivation_path| {
                derivation_path
                    .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
                    .map(|extended_private_key| extended_private_key.to_priv())
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())
            })
            .transpose()
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
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        Ok(extended_public_key.to_pub())
    }

    #[allow(clippy::type_complexity)]
    pub fn identity_authentication_ecdsa_public_keys_data_map(
        &mut self,
        app_context: &AppContext,
        register_addresses: bool,
        network: Network,
        identity_index: u32,
        key_index_range: Range<u32>,
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
                .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;

            let public_key = extended_public_key.to_pub();
            public_key_result_map.insert(
                extended_public_key.public_key.serialize().to_vec(),
                key_index,
            );
            public_key_hash_result_map.insert(public_key.pubkey_hash().to_byte_array(), key_index);
            if register_addresses {
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
        app_context: &AppContext,
        network: Network,
        identity_index: u32,
        key_index: u32,
    ) -> Result<(PrivateKey, DerivationPath), String> {
        let derivation_path = DerivationPath::identity_authentication_path(
            network,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        );
        tracing::debug!(
            identity_index = identity_index,
            key_index = key_index,
            path = %derivation_path,
            "Generated identity authentication ECDSA derivation path"
        );
        let extended_public_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .expect("derivation should not be able to fail");

        let private_key = extended_public_key.to_priv();
        self.register_address_from_private_key(
            &private_key,
            &derivation_path,
            DerivationPathType::SINGLE_USER_AUTHENTICATION,
            DerivationPathReference::BlockchainIdentities,
            app_context,
        )?;

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

        if app_context.core_backend_mode() == crate::spv::CoreBackendMode::Rpc {
            app_context.try_import_address(&address, self.core_wallet_name.as_deref(), None);
        }

        tracing::trace!(
            address = ?&address,
            network = &address.network().to_string(),
            "registered new address"
        );
        Ok(())
    }

    pub fn identity_top_up_ecdsa_private_key(
        &mut self,
        app_context: &AppContext,
        network: Network,
        identity_index: u32,
        top_up_index: u32,
    ) -> Result<PrivateKey, String> {
        let derivation_path =
            DerivationPath::identity_top_up_path(network, identity_index, top_up_index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .expect("derivation should not be able to fail");
        let private_key = extended_private_key.to_priv();

        self.register_address_from_private_key(
            &private_key,
            &derivation_path,
            DerivationPathType::CREDIT_FUNDING,
            DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
            app_context,
        )?;
        Ok(private_key)
    }

    /// Generate Core key for identity registration
    pub fn identity_registration_ecdsa_private_key(
        &mut self,
        app_context: &AppContext,
        network: Network,
        index: u32,
    ) -> Result<PrivateKey, String> {
        let derivation_path = DerivationPath::identity_registration_path(network, index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(self.seed_bytes()?, network)
            .expect("derivation should not be able to fail");
        let private_key = extended_private_key.to_priv();

        self.register_address_from_private_key(
            &private_key,
            &derivation_path,
            DerivationPathType::CREDIT_FUNDING,
            DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
            app_context,
        )?;
        Ok(private_key)
    }

    pub fn receive_address(
        &mut self,
        _network: Network,
        _force_new: bool,
        _register: Option<&AppContext>,
    ) -> Result<Address, String> {
        if let Some(pw) = &self.platform_wallet {
            return pw
                .core()
                .next_receive_address_blocking()
                .map_err(|e| e.to_string());
        }
        Err("Wallet is locked".to_string())
    }

    pub fn change_address(
        &mut self,
        _network: Network,
        _register: Option<&AppContext>,
    ) -> Result<Address, String> {
        if let Some(pw) = &self.platform_wallet {
            return pw
                .core()
                .next_change_address_blocking()
                .map_err(|e| e.to_string());
        }
        Err("Wallet is locked".to_string())
    }

    /// Derive the DIP-17 Platform payment address at the given HD index.
    ///
    /// Returns the P2PKH Core address at path
    /// `m/9'/<coin_type>'/17'/0'/0'/<index>`.
    ///
    /// The wallet must be unlocked (open seed). Returns an error if the wallet
    /// is locked or key derivation fails.
    pub fn platform_payment_address_at(
        &self,
        network: Network,
        index: u32,
    ) -> Result<Address, String> {
        let provider = WalletAddressProvider::new(self, network)?;
        provider.platform_payment_address_at(index)
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
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?
            .to_pub();
        Ok(Address::p2pkh(&public_key, network))
    }

    pub fn update_address_balance(
        &self,
        address: &Address,
        new_balance: Duffs,
        context: &AppContext,
    ) -> Result<(), String> {
        context
            .db
            .update_address_balance(&self.seed_hash(), address, new_balance)
            .map_err(|e| e.to_string())
    }

    fn recalculate_affected_address_balances_with_db(
        &self,
        used_utxos: &BTreeMap<OutPoint, (TxOut, Address)>,
        db: &Database,
    ) -> Result<(), String> {
        let seed_hash = self.seed_hash();
        let affected_addresses: BTreeSet<_> =
            used_utxos.values().map(|(_, addr)| addr.clone()).collect();
        for address in affected_addresses {
            let new_balance = self.address_balance(&address);
            db.update_address_balance(&seed_hash, &address, new_balance)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn update_address_total_received(
        &self,
        address: &Address,
        total_received: Duffs,
        context: &AppContext,
    ) -> Result<(), String> {
        context
            .db
            .update_address_total_received(&self.seed_hash(), address, total_received)
            .map_err(|e| e.to_string())
    }

    /// Get the private key for a Platform address
    #[allow(clippy::result_large_err)]
    pub fn get_platform_address_private_key(
        &self,
        platform_address: &PlatformAddress,
        network: Network,
    ) -> Result<PrivateKey, ProtocolError> {
        // Convert to Core address for lookup
        let core_address = platform_address.to_address_with_network(network);

        // Find the derivation path via PlatformWallet
        let derivation_path = self
            .derivation_path_for_address(&core_address)
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

/// Default gap limit for HD wallet address scanning
const DEFAULT_GAP_LIMIT: AddressIndex = 20;

/// Provider for wallet Platform addresses that implements AddressProvider for SDK address sync.
///
/// This struct tracks the state needed for the SDK's privacy-preserving address balance
/// synchronization. It can derive new Platform addresses on-demand to support HD wallet
/// gap limit behavior.
///
/// # Usage
/// ```ignore
/// let mut provider = WalletAddressProvider::new(&wallet, network)?;
/// let result = sdk.sync_address_balances(&mut provider, None, None).await?;
/// provider.apply_results_to_wallet(&mut wallet);
/// ```
pub struct WalletAddressProvider {
    /// Network for address derivation
    network: Network,
    /// Gap limit for HD wallet scanning
    gap_limit: AddressIndex,
    /// Seed bytes for deriving new addresses (64 bytes)
    seed: [u8; 64],
    /// Account index for Platform payment addresses (default 0)
    account: u32,
    /// Key class for Platform payment addresses (default 0)
    key_class: u32,
    /// Map of index to (PlatformAddress, CoreAddress) for pending addresses
    pending: BTreeMap<AddressIndex, (PlatformAddress, Address)>,
    /// Set of indices that have been resolved (found or absent)
    resolved: BTreeSet<AddressIndex>,
    /// Highest index found with a non-zero balance
    highest_found: Option<AddressIndex>,
    /// Results: address -> balance for addresses found with balance
    found_balances: BTreeMap<Address, AddressFunds>,
    /// Known balances from previous sync for incremental catch-up
    stored_balances: Vec<(AddressIndex, PlatformAddress, AddressFunds)>,
    /// Last sync height from previous sync for incremental catch-up
    stored_sync_height: u64,
}

impl WalletAddressProvider {
    /// Create a new WalletAddressProvider from a wallet.
    ///
    /// This initializes the provider with Platform payment addresses up to the gap limit.
    /// The wallet must be open (unlocked) to access the seed for address derivation.
    ///
    /// # Errors
    /// Returns an error if the wallet is closed/locked.
    pub fn new(wallet: &Wallet, network: Network) -> Result<Self, String> {
        Self::with_gap_limit(wallet, network, DEFAULT_GAP_LIMIT)
    }

    /// Create a new WalletAddressProvider with a custom gap limit.
    ///
    /// # Errors
    /// Returns an error if the wallet is closed/locked.
    pub fn with_gap_limit(
        wallet: &Wallet,
        network: Network,
        gap_limit: AddressIndex,
    ) -> Result<Self, String> {
        let seed = *wallet.seed_bytes()?;

        let mut provider = Self {
            network,
            gap_limit,
            seed,
            account: 0,
            key_class: 0,
            pending: BTreeMap::new(),
            resolved: BTreeSet::new(),
            highest_found: None,
            found_balances: BTreeMap::new(),
            stored_balances: Vec::new(),
            stored_sync_height: 0,
        };

        // Bootstrap initial addresses (0 to gap_limit - 1)
        provider.ensure_addresses_up_to(gap_limit.saturating_sub(1))?;

        Ok(provider)
    }

    /// Get the network this provider was created for.
    pub fn network(&self) -> Network {
        self.network
    }

    /// Get the found balances after sync is complete.
    ///
    /// Returns a map of Core Address -> balance (in credits).
    pub fn found_balances(&self) -> &BTreeMap<Address, AddressFunds> {
        &self.found_balances
    }

    /// Get the found balances with their indices after sync is complete.
    ///
    /// Returns an iterator of (index, (&Address, &balance)) for addresses that were found with balance.
    /// The index can be used to reconstruct the derivation path.
    pub fn found_balances_with_indices(
        &self,
    ) -> impl Iterator<Item = (AddressIndex, (&Address, &AddressFunds))> {
        // Build a reverse lookup from address to index
        let address_to_index: BTreeMap<&Address, AddressIndex> = self
            .pending
            .iter()
            .map(|(idx, (_, addr))| (addr, *idx))
            .collect();

        self.found_balances
            .iter()
            .filter_map(move |(addr, balance)| {
                address_to_index
                    .get(addr)
                    .map(|&idx| (idx, (addr, balance)))
            })
    }

    /// Update a balance for an address (used for terminal balance updates).
    ///
    /// This allows applying balance changes discovered after the initial sync.
    pub fn update_balance(&mut self, address: &Address, balance: u64) {
        let canonical_address = Wallet::canonical_address(address, self.network);

        let nonce = self
            .found_balances
            .get(&canonical_address)
            .map(|funds| funds.nonce)
            .unwrap_or(0);

        self.found_balances
            .insert(canonical_address, AddressFunds { nonce, balance });
    }

    /// Populate stored balances and sync height from DB-provided state.
    ///
    /// Call this after construction to enable incremental catch-up.
    /// The SDK uses `current_balances()` as the baseline and `last_sync_height()`
    /// as the starting block for applying delta operations.
    ///
    /// `stored_info` contains `(Address, balance, nonce)` tuples loaded from the DB.
    pub fn with_stored_state(
        mut self,
        stored_info: &[(Address, u64, u32)],
        network: Network,
        last_sync_height: u64,
    ) -> Self {
        self.stored_sync_height = last_sync_height;

        // Populate stored_balances from DB-provided platform address info
        for (core_addr, balance, nonce) in stored_info {
            // Find the matching pending address to get the index and platform address
            for (index, (platform_addr, pending_addr)) in &self.pending {
                let canonical = Wallet::canonical_address(pending_addr, network);
                if &canonical == core_addr {
                    self.stored_balances.push((
                        *index,
                        *platform_addr,
                        AddressFunds {
                            balance: *balance,
                            nonce: *nonce,
                        },
                    ));
                    break;
                }
            }
        }

        self
    }

    /// Derive the DIP-17 Platform payment address at the given index.
    ///
    /// The address is derived from the wallet seed at path
    /// `m/9'/<coin_type>'/17'/0'/0'/<index>` and returned as a P2PKH Core address.
    pub fn platform_payment_address_at(&self, index: u32) -> Result<Address, String> {
        self.derive_address_at_index(index).map(|(_, addr)| addr)
    }

    /// Derive a Platform address at the given index.
    fn derive_address_at_index(
        &self,
        index: AddressIndex,
    ) -> Result<(PlatformAddress, Address), String> {
        let derivation_path = DerivationPath::platform_payment_path(
            self.network,
            self.account,
            self.key_class,
            index,
        );

        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&self.seed, self.network)
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;

        let secp = Secp256k1::new();
        let private_key = extended_private_key.to_priv();
        let public_key = private_key.public_key(&secp);

        // Create P2PKH address
        let address = Address::p2pkh(&public_key, self.network);

        let platform_addr = PlatformAddress::try_from(address.clone())
            .map_err(|e| format!("Failed to convert to PlatformAddress: {}", e))?;

        Ok((platform_addr, address))
    }

    /// Ensure we have addresses derived up to and including the given index.
    fn ensure_addresses_up_to(&mut self, max_index: AddressIndex) -> Result<(), String> {
        let current_max = self.pending.keys().max().copied();

        let start = current_max.map(|m| m + 1).unwrap_or(0);
        for index in start..=max_index {
            if !self.pending.contains_key(&index) && !self.resolved.contains(&index) {
                let (key, address) = self.derive_address_at_index(index)?;
                self.pending.insert(index, (key, address));
            }
        }

        Ok(())
    }

    /// Extend pending addresses based on gap limit after finding an address.
    fn extend_for_gap_limit(&mut self, found_index: AddressIndex) -> Result<(), String> {
        let new_end = found_index.saturating_add(self.gap_limit);
        self.ensure_addresses_up_to(new_end)
    }
}

impl AddressProvider for WalletAddressProvider {
    fn gap_limit(&self) -> AddressIndex {
        self.gap_limit
    }

    fn pending_addresses(&self) -> Vec<(AddressIndex, PlatformAddress)> {
        self.pending
            .iter()
            .filter(|(index, _)| !self.resolved.contains(index))
            .map(|(index, (platform_addr, _))| (*index, *platform_addr))
            .collect()
    }

    fn on_address_found(
        &mut self,
        index: AddressIndex,
        address: &PlatformAddress,
        funds: AddressFunds,
    ) {
        self.resolved.insert(index);

        // Log what the SDK is returning
        if let Some((_, core_address)) = self.pending.get(&index) {
            let platform_addr_str = address.to_bech32m_string(self.network);
            tracing::info!(
                "on_address_found: index={}, core_address={}, platform_address={}, balance={}, nonce={}",
                index,
                core_address,
                platform_addr_str,
                funds.balance,
                funds.nonce
            );
        } else {
            tracing::warn!(
                "on_address_found: index={} not in pending! balance={}",
                index,
                funds.balance
            );
        }

        if let Some((_, core_address)) = self.pending.get(&index) {
            let canonical_address = Wallet::canonical_address(core_address, self.network);
            self.found_balances.insert(canonical_address, funds);
        }

        if funds.balance > 0 {
            // Update highest found
            self.highest_found = Some(self.highest_found.map(|h| h.max(index)).unwrap_or(index));

            // Extend the address range based on gap limit
            if let Err(e) = self.extend_for_gap_limit(index) {
                tracing::warn!("Failed to extend addresses for gap limit: {}", e);
            }
        }
    }

    fn on_address_absent(&mut self, index: AddressIndex, _address: &PlatformAddress) {
        self.resolved.insert(index);
    }

    fn has_pending(&self) -> bool {
        self.pending
            .keys()
            .any(|index| !self.resolved.contains(index))
    }

    fn highest_found_index(&self) -> Option<AddressIndex> {
        self.highest_found
    }

    fn current_balances(&self) -> &[(AddressIndex, PlatformAddress, AddressFunds)] {
        &self.stored_balances
    }

    fn last_sync_height(&self) -> u64 {
        self.stored_sync_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::key_wallet::bip32::{ExtendedPrivKey, ExtendedPubKey};

    /// Helper: create a minimal open wallet for testing.
    /// Uses a deterministic 64-byte seed and derives the BIP44 master public key.
    fn test_wallet() -> Wallet {
        let seed = [42u8; 64];
        let network = Network::Testnet;
        let secp = Secp256k1::new();

        // Derive master private key from seed
        let master_private_key =
            ExtendedPrivKey::new_master(network, &seed).expect("master key derivation");

        // Derive BIP44 account 0 path: m/44'/1'/0'
        let bip44_account_path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
        ]);
        let bip44_account_private = master_private_key
            .derive_priv(&secp, &bip44_account_path)
            .expect("bip44 derivation");
        let master_bip44_ecdsa_extended_public_key =
            ExtendedPubKey::from_priv(&secp, &bip44_account_private);

        let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);
        let root_pub = ExtendedPubKey::from_priv(&secp, &master_private_key);
        let wallet_id = Wallet::compute_wallet_id_from_root_pub(&root_pub);

        Wallet {
            platform_wallet: None,
            wallet_seed: WalletSeed::Open(OpenWalletSeed {
                seed,
                wallet_info: ClosedKeyItem {
                    seed_hash,
                    encrypted_seed: seed.to_vec(),
                    salt: vec![],
                    nonce: vec![],
                    password_hint: None,
                },
            }),
            uses_password: false,
            master_bip44_ecdsa_extended_public_key,
            alias: Some("Test Wallet".to_string()),
            is_main: true,
            core_wallet_name: None,
            wallet_id,
        }
    }

    /// Helper: create a test address on Testnet
    fn test_address(index: u8) -> Address {
        use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
        let secp = Secp256k1::new();
        let mut sk_bytes = [0u8; 32];
        sk_bytes[0] = if index == 0 { 1 } else { index };
        let sk = SecretKey::from_slice(&sk_bytes).expect("valid secret key");
        let inner = dash_sdk::dpp::dashcore::secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey = PublicKey::from_slice(&inner.serialize()).expect("valid pubkey");
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// Helper: create an OutPoint with a deterministic txid
    fn test_outpoint(tx_index: u8, vout: u32) -> OutPoint {
        let mut txid_bytes = [0u8; 32];
        txid_bytes[0] = tx_index;
        OutPoint::new(Txid::from_slice(&txid_bytes).unwrap(), vout)
    }

    // ========================================================================
    // Balance calculation tests
    // ========================================================================

    #[test]
    fn test_balance_returns_zero_without_platform_wallet() {
        let wallet = test_wallet();
        // Without platform_wallet, all balance methods return 0
        assert_eq!(wallet.confirmed_balance_duffs(), 0);
        assert_eq!(wallet.unconfirmed_balance_duffs(), 0);
        assert_eq!(wallet.total_balance_duffs(), 0);
        assert!(!wallet.has_balance());
        assert_eq!(wallet.spv_confirmed_balance(), None);
    }

    // ========================================================================
    // WalletTransaction tests
    // ========================================================================

    #[test]
    fn test_wallet_transaction_incoming() {
        let tx = WalletTransaction {
            txid: Txid::from_slice(&[0u8; 32]).unwrap(),
            transaction: Transaction {
                version: 2,
                lock_time: 0,
                input: vec![],
                output: vec![],
                special_transaction_payload: None,
            },
            timestamp: 1000,
            height: Some(100),
            block_hash: None,
            net_amount: 50_000,
            fee: Some(226),
            label: None,
            is_ours: true,
            status: TransactionStatus::Confirmed,
        };

        assert!(tx.is_incoming());
        assert!(!tx.is_outgoing());
        assert!(tx.is_confirmed());
        assert_eq!(tx.amount_abs(), 50_000);
    }

    #[test]
    fn test_wallet_transaction_outgoing() {
        let tx = WalletTransaction {
            txid: Txid::from_slice(&[0u8; 32]).unwrap(),
            transaction: Transaction {
                version: 2,
                lock_time: 0,
                input: vec![],
                output: vec![],
                special_transaction_payload: None,
            },
            timestamp: 1000,
            height: None,
            block_hash: None,
            net_amount: -30_000,
            fee: Some(226),
            label: None,
            is_ours: true,
            status: TransactionStatus::Unconfirmed,
        };

        assert!(!tx.is_incoming());
        assert!(tx.is_outgoing());
        assert!(!tx.is_confirmed());
        assert_eq!(tx.amount_abs(), 30_000);
    }

    #[test]
    fn test_wallet_transaction_zero_amount() {
        let tx = WalletTransaction {
            txid: Txid::from_slice(&[0u8; 32]).unwrap(),
            transaction: Transaction {
                version: 2,
                lock_time: 0,
                input: vec![],
                output: vec![],
                special_transaction_payload: None,
            },
            timestamp: 1000,
            height: None,
            block_hash: None,
            net_amount: 0,
            fee: None,
            label: None,
            is_ours: false,
            status: TransactionStatus::Unconfirmed,
        };

        assert!(!tx.is_incoming());
        assert!(!tx.is_outgoing());
        assert_eq!(tx.amount_abs(), 0);
    }

    // ========================================================================
    // Wallet state tests
    // ========================================================================

    #[test]
    fn test_wallet_is_open() {
        let wallet = test_wallet();
        assert!(wallet.is_open());
    }

    #[test]
    fn test_wallet_seed_hash_consistent() {
        let wallet = test_wallet();
        let hash1 = wallet.seed_hash();
        let hash2 = wallet.seed_hash();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_wallet_seed_bytes_available_when_open() {
        let wallet = test_wallet();
        assert!(wallet.seed_bytes().is_ok());
        assert_eq!(wallet.seed_bytes().unwrap().len(), 64);
    }

    // ========================================================================
    // Derivation path helpers tests
    // ========================================================================

    #[test]
    fn test_is_bip44_mainnet() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 5 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);
        assert!(path.is_bip44(Network::Mainnet));
        assert!(!path.is_bip44(Network::Testnet));
    }

    #[test]
    fn test_is_bip44_testnet() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);
        assert!(path.is_bip44(Network::Testnet));
        assert!(path.is_bip44(Network::Devnet));
        assert!(!path.is_bip44(Network::Mainnet));
    }

    #[test]
    fn test_is_bip44_external() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 }, // external
            ChildNumber::Normal { index: 5 },
        ]);
        assert!(path.is_bip44_external(Network::Testnet));
        assert!(!path.is_bip44_change(Network::Testnet));
    }

    #[test]
    fn test_is_bip44_change() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 1 }, // change
            ChildNumber::Normal { index: 3 },
        ]);
        assert!(!path.is_bip44_external(Network::Testnet));
        assert!(path.is_bip44_change(Network::Testnet));
    }

    #[test]
    fn test_is_asset_lock_funding() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 9 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 5 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Normal { index: 0 },
        ]);
        assert!(path.is_asset_lock_funding(Network::Testnet));
        assert!(!path.is_asset_lock_funding(Network::Mainnet));
    }

    #[test]
    fn test_is_platform_payment() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 9 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 17 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);
        assert!(path.is_platform_payment(Network::Testnet));
        assert!(!path.is_platform_payment(Network::Mainnet));
    }

    #[test]
    fn test_platform_payment_path_construction() {
        let path = DerivationPath::platform_payment_path(Network::Testnet, 0, 0, 5);
        assert!(path.is_platform_payment(Network::Testnet));

        let components = path.as_ref();
        assert_eq!(components.len(), 6);
        assert_eq!(components[0], ChildNumber::Hardened { index: 9 });
        assert_eq!(components[1], ChildNumber::Hardened { index: 1 }); // testnet coin_type
        assert_eq!(components[2], ChildNumber::Hardened { index: 17 });
        assert_eq!(components[3], ChildNumber::Hardened { index: 0 }); // account
        assert_eq!(components[4], ChildNumber::Hardened { index: 0 }); // key_class
        assert_eq!(components[5], ChildNumber::Normal { index: 5 }); // index
    }

    #[test]
    fn test_bip44_account_index() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 7 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);
        assert_eq!(path.bip44_account_index(), Some(7));
    }

    #[test]
    fn test_bip44_address_index() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 42 },
        ]);
        assert_eq!(path.bip44_address_index(), Some(42));
    }

    // ========================================================================
    // DerivationPathReference tests
    // ========================================================================

    #[test]
    fn test_derivation_path_reference_try_from_valid() {
        assert_eq!(
            DerivationPathReference::try_from(0u32).unwrap(),
            DerivationPathReference::Unknown
        );
        assert_eq!(
            DerivationPathReference::try_from(2u32).unwrap(),
            DerivationPathReference::BIP44
        );
        assert_eq!(
            DerivationPathReference::try_from(16u32).unwrap(),
            DerivationPathReference::PlatformPayment
        );
        assert_eq!(
            DerivationPathReference::try_from(255u32).unwrap(),
            DerivationPathReference::Root
        );
    }

    #[test]
    fn test_derivation_path_reference_try_from_invalid() {
        assert!(DerivationPathReference::try_from(17u32).is_err());
        assert!(DerivationPathReference::try_from(100u32).is_err());
        assert!(DerivationPathReference::try_from(254u32).is_err());
    }

    // ========================================================================
    // networks_address_compatible tests
    // ========================================================================

    #[test]
    fn test_networks_address_compatible() {
        assert!(networks_address_compatible(
            &Network::Mainnet,
            &Network::Mainnet
        ));
        assert!(networks_address_compatible(
            &Network::Testnet,
            &Network::Testnet
        ));
        assert!(networks_address_compatible(
            &Network::Testnet,
            &Network::Devnet
        ));
        assert!(networks_address_compatible(
            &Network::Devnet,
            &Network::Regtest
        ));
        assert!(!networks_address_compatible(
            &Network::Mainnet,
            &Network::Testnet
        ));
        assert!(!networks_address_compatible(
            &Network::Testnet,
            &Network::Mainnet
        ));
    }

    // ========================================================================
    // Wallet address derivation tests
    // ========================================================================

    #[test]
    fn test_derive_bip44_address_deterministic() {
        let wallet = test_wallet();
        let addr1 = wallet
            .derive_bip44_address(Network::Testnet, false, 0)
            .unwrap();
        let addr2 = wallet
            .derive_bip44_address(Network::Testnet, false, 0)
            .unwrap();
        assert_eq!(addr1, addr2, "Same derivation should produce same address");
    }

    #[test]
    fn test_derive_bip44_address_different_indices() {
        let wallet = test_wallet();
        let addr0 = wallet
            .derive_bip44_address(Network::Testnet, false, 0)
            .unwrap();
        let addr1 = wallet
            .derive_bip44_address(Network::Testnet, false, 1)
            .unwrap();
        assert_ne!(
            addr0, addr1,
            "Different indices should produce different addresses"
        );
    }

    #[test]
    fn test_derive_bip44_address_external_vs_change() {
        let wallet = test_wallet();
        let external = wallet
            .derive_bip44_address(Network::Testnet, false, 0)
            .unwrap();
        let change = wallet
            .derive_bip44_address(Network::Testnet, true, 0)
            .unwrap();
        assert_ne!(
            external, change,
            "External and change addresses should differ"
        );
    }

    #[test]
    fn test_receive_address_returns_error_when_locked() {
        let mut wallet = test_wallet();
        // Without PlatformWallet, receive_address should return Err
        let result = wallet.receive_address(Network::Testnet, false, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Wallet is locked");
    }

    #[test]
    fn test_change_address_returns_error_when_locked() {
        let mut wallet = test_wallet();
        // Without PlatformWallet, change_address should return Err
        let result = wallet.change_address(Network::Testnet, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Wallet is locked");
    }

    // ========================================================================
    // WalletSeed tests
    // ========================================================================

    #[test]
    fn test_wallet_seed_open_already_open() {
        let mut wallet = test_wallet();
        // Already open, should succeed with no-op
        assert!(wallet.wallet_seed.open("any_password").is_ok());
    }

    #[test]
    fn test_wallet_seed_close_and_reopen() {
        let mut wallet = test_wallet();
        let original_hash = wallet.seed_hash();

        wallet.wallet_seed.close();
        assert!(!wallet.is_open());

        // After closing, seed_bytes should fail
        assert!(wallet.seed_bytes().is_err());

        // Reopen without password (test wallet has no encryption)
        wallet.wallet_seed.open_no_password().unwrap();
        assert!(wallet.is_open());
        assert_eq!(wallet.seed_hash(), original_hash);
    }

    // ========================================================================
    // WalletArcRef tests
    // ========================================================================

    #[test]
    fn test_wallet_arc_ref_equality() {
        let wallet = test_wallet();
        let seed_hash = wallet.seed_hash();
        let arc1 = Arc::new(RwLock::new(wallet.clone()));
        let arc2 = Arc::new(RwLock::new(wallet));

        let ref1 = WalletArcRef::from(arc1);
        let ref2 = WalletArcRef::from(arc2);

        // Same wallet_id means equal
        assert_eq!(ref1, ref2);
        assert_eq!(ref1.wallet_id, ref1.wallet.read().unwrap().wallet_id());
    }

    // ========================================================================
    // find_in_arc_rw_lock_slice tests
    // ========================================================================

    #[test]
    fn test_find_in_arc_rw_lock_slice_found() {
        let wallet = test_wallet();
        let seed_hash = wallet.seed_hash();
        let arc = Arc::new(RwLock::new(wallet));
        let slice = vec![arc];

        let result = Wallet::find_in_arc_rw_lock_slice(&slice, seed_hash);
        assert!(result.is_some());
    }

    #[test]
    fn test_find_in_arc_rw_lock_slice_not_found() {
        let wallet = test_wallet();
        let arc = Arc::new(RwLock::new(wallet));
        let slice = vec![arc];

        let result = Wallet::find_in_arc_rw_lock_slice(&slice, [0u8; 32]);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_in_arc_rw_lock_slice_empty() {
        let result = Wallet::find_in_arc_rw_lock_slice(&[], [0u8; 32]);
        assert!(result.is_none());
    }
}
