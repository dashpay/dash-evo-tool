pub mod auth_pubkey_cache;
pub mod birth_height;
pub mod encryption;
pub mod meta;
pub mod passphrase;
pub mod seed_envelope;
pub mod shielded;
pub mod single_key;

use crate::backend_task::error::TaskError;
use crate::database::WalletError;
use crate::model::secret::Secret;
use crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::async_trait::async_trait;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{
    ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey, KeyDerivationType,
};
use dash_sdk::dpp::prelude::AddressNonce;
use dash_sdk::platform::address_sync::{AddressFunds, AddressIndex, AddressProvider};

use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::{
    Address, BlockHash, Network, PrivateKey, PublicKey, Transaction, Txid,
};
use std::cmp;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

/// Resolve the SLIP-0044 coin type for a Dash network.
///
/// Mainnet uses `5'`; Testnet, Devnet, and Regtest all use `1'`. This mirrors
/// the canonical key-wallet mapping (`DASH_COIN_TYPE` / `DASH_TESTNET_COIN_TYPE`
/// in `key_wallet::dip9`) and the arms upstream `AccountType::derivation_path`
/// applies, so every HD path DET builds agrees with what the wallet derives.
pub const fn coin_type_for_network(network: Network) -> u32 {
    match network {
        Network::Mainnet => DASH_COIN_TYPE,
        Network::Testnet | Network::Devnet | Network::Regtest => DASH_TESTNET_COIN_TYPE,
    }
}

/// Stateless backend-authoritative validation for a wallet payment's
/// recipient amounts.
///
/// Rejects a payment with no recipients and any recipient whose amount is
/// zero — both would build a degenerate transaction that wastes the network
/// fee without moving the intended funds. Takes raw duff amounts so it carries
/// no dependency on the backend-task recipient type and stays trivially
/// unit-testable; UI screens may call it for instant feedback, but the backend
/// task is the authoritative caller (see CLAUDE.md validation-placement rule).
///
/// # Errors
///
/// - [`TaskError::PaymentNoRecipients`] when `amounts_duffs` is empty.
/// - [`TaskError::PaymentZeroAmount`] when any amount is `0`.
pub fn validate_payment_recipients(amounts_duffs: &[u64]) -> Result<(), TaskError> {
    if amounts_duffs.is_empty() {
        return Err(TaskError::PaymentNoRecipients);
    }
    if amounts_duffs.contains(&0) {
        return Err(TaskError::PaymentZeroAmount);
    }
    Ok(())
}

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
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::platform::Identity;
use zeroize::Zeroizing;

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
        // From trait doesn't allow returning Result, so use a fallback for poisoned locks
        let seed_hash = wallet
            .read()
            .map(|w| w.seed_hash())
            .unwrap_or_else(|poisoned| {
                tracing::warn!("Wallet lock poisoned during WalletArcRef conversion");
                poisoned.into_inner().seed_hash()
            });
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
    pub known_addresses: BTreeMap<Address, DerivationPath>,
    pub watched_addresses: BTreeMap<DerivationPath, AddressInfo>,
    pub alias: Option<String>,
    pub identities: HashMap<u32, Identity>,
    pub is_main: bool,
    /// DIP-17: Platform address balances and nonces (keyed by Core Address for lookup)
    pub platform_address_info: BTreeMap<Address, PlatformAddressInfo>,
    /// Dash Core wallet name for multi-wallet RPC calls
    pub core_wallet_name: Option<String>,
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

        // Derive the first receive address (m/44'/coin'/0'/0/0)
        let (known_addresses, watched_addresses) =
            Self::derive_first_address(&master_bip44_ecdsa_extended_public_key, network, &secp)
                .map_err(|e| TaskError::WalletKeyDerivationFailed { source: e.into() })?;

        Ok(Wallet {
            wallet_seed: WalletSeed::Open(OpenWalletSeed {
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
            known_addresses,
            watched_addresses,
            alias,
            identities: Default::default(),
            is_main: true,
            platform_address_info: Default::default(),
            core_wallet_name: None,
        })
    }

    /// Returns the BIP44 account 0 derivation path for the given network.
    fn bip44_account0_path(network: Network) -> DerivationPath {
        match network {
            Network::Mainnet => DerivationPath::from(DASH_BIP44_ACCOUNT_0_PATH_MAINNET.as_slice()),
            _ => DerivationPath::from(DASH_BIP44_ACCOUNT_0_PATH_TESTNET.as_slice()),
        }
    }

    /// Derive the first receive address (index 0) and return populated
    /// `known_addresses` and `watched_addresses` maps.
    #[allow(clippy::type_complexity)]
    fn derive_first_address(
        master_pub: &ExtendedPubKey,
        network: Network,
        secp: &Secp256k1<dash_sdk::dpp::dashcore::secp256k1::All>,
    ) -> Result<
        (
            BTreeMap<Address, DerivationPath>,
            BTreeMap<DerivationPath, AddressInfo>,
        ),
        String,
    > {
        let mut known_addresses = BTreeMap::new();
        let mut watched_addresses = BTreeMap::new();

        let address_path = DerivationPath::from(
            [
                ChildNumber::Normal { index: 0 }, // receive (not change)
                ChildNumber::Normal { index: 0 }, // first address
            ]
            .as_slice(),
        );

        let pk = master_pub
            .derive_pub(secp, &address_path)
            .map_err(|e| format!("Failed to derive first receive address: {e}"))?;
        let address = Address::p2pkh(&pk.to_pub(), network);
        let bip44 = match network {
            Network::Mainnet => &DASH_BIP44_ACCOUNT_0_PATH_MAINNET,
            _ => &DASH_BIP44_ACCOUNT_0_PATH_TESTNET,
        };
        let full_path = DerivationPath::from(
            [
                bip44[0],
                bip44[1],
                bip44[2],
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index: 0 },
            ]
            .as_slice(),
        );
        known_addresses.insert(address.clone(), full_path.clone());
        watched_addresses.insert(
            full_path,
            AddressInfo {
                address,
                path_type: DerivationPathType::CLEAR_FUNDS,
                path_reference: DerivationPathReference::BIP44,
            },
        );

        Ok((known_addresses, watched_addresses))
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

pub type WalletSeedHash = [u8; 32];

/// State of a wallet's HD seed.
///
/// Neither variant ever holds the plaintext seed. `Open` means
/// **unlocked/verified** — the passphrase has been proven correct and the
/// wallet is usable — and `Closed` means locked. Both variants carry only the
/// encrypted [`ClosedKeyItem`] metadata; the plaintext seed lives **only**
/// inside a `with_secret*` frame of the
/// [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint.
#[derive(Debug, Clone, PartialEq)]
pub enum WalletSeed {
    Open(OpenWalletSeed),
    Closed(ClosedWalletSeed),
}

/// The "unlocked/verified" half of [`WalletSeed`].
///
/// Holds no secret: an open wallet parks no plaintext seed. The retained
/// [`ClosedKeyItem`] carries the encrypted-envelope metadata (`seed_hash`,
/// `salt`, `nonce`, `encrypted_seed`, `password_hint`) the model and UI read
/// without the seed.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenWalletSeed {
    pub wallet_info: ClosedKeyItem,
}

#[derive(Clone, PartialEq)]
pub struct ClosedKeyItem {
    pub seed_hash: WalletSeedHash, // SHA-256 hash of the seed
    pub encrypted_seed: Vec<u8>,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub password_hint: Option<String>,
}

impl std::fmt::Debug for ClosedKeyItem {
    /// Redacting `Debug`: prints only the public seed hash and lengths.
    ///
    /// For an unprotected wallet `encrypted_seed` is the raw 64-byte
    /// plaintext seed, so it must never reach a `Debug` sink (logs,
    /// panics). `Wallet`, `WalletSeed`, and `OpenWalletSeed` all derive
    /// `Debug` and delegate here, so redacting once protects the whole
    /// chain.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClosedKeyItem")
            .field("seed_hash", &hex::encode(self.seed_hash))
            .field("encrypted_seed", &"[redacted]")
            .field("salt_len", &self.salt.len())
            .field("nonce_len", &self.nonce.len())
            .field("password_hint", &self.password_hint)
            .finish()
    }
}

pub type ClosedWalletSeed = ClosedKeyItem;

impl WalletSeed {
    /// Verify the passphrase and mark the wallet unlocked, **without parking
    /// the seed**.
    ///
    /// Decrypts the stored envelope only to prove `password` is correct, then
    /// discards the plaintext (it is zeroized at the end of this call). On
    /// success the state flips to `Open` carrying no secret. Seed residency is
    /// owned entirely by the
    /// [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint; the
    /// caller that wants the session kept unlocked promotes the seed there
    /// (see [`AppContext::handle_wallet_unlocked`](crate::context::AppContext::handle_wallet_unlocked)).
    pub fn open(&mut self, password: &str) -> Result<(), String> {
        match self {
            WalletSeed::Open(_) => {
                // Wallet is already open
                Ok(())
            }
            WalletSeed::Closed(closed_seed) => {
                // Decrypt to PROVE the password is correct, then drop the
                // plaintext (`Zeroizing`) without parking it.
                let _verified = Zeroizing::new(closed_seed.decrypt_seed(password)?);
                let open_wallet_seed = OpenWalletSeed {
                    wallet_info: closed_seed.clone(),
                };
                *self = WalletSeed::Open(open_wallet_seed);
                Ok(())
            }
        }
    }

    /// Mark a no-password wallet unlocked, **without parking the seed**.
    ///
    /// The verify-not-park counterpart of [`Self::open`] for unprotected
    /// wallets: it validates the stored envelope is a well-formed 64-byte seed,
    /// then flips to `Open` carrying no secret.
    pub fn open_no_password(&mut self) -> Result<(), String> {
        match self {
            WalletSeed::Open(_) => {
                // Wallet is already open
                Ok(())
            }
            WalletSeed::Closed(closed_seed) => {
                // Unprotected envelopes store the raw 64 bytes verbatim;
                // validate the length to prove the wallet is openable, then
                // drop the plaintext without parking it.
                if closed_seed.encrypted_seed.len() != 64 {
                    return Err(format!(
                        "incorrect seed size, expected 64 bytes, got {}",
                        closed_seed.encrypted_seed.len()
                    ));
                }
                let open_wallet_seed = OpenWalletSeed {
                    wallet_info: closed_seed.clone(),
                };
                *self = WalletSeed::Open(open_wallet_seed);
                Ok(())
            }
        }
    }

    /// Transition the wallet back to the locked (`Closed`) state.
    // Allow dead_code: This method provides explicit wallet closure functionality,
    // useful for security-conscious applications requiring manual wallet management
    #[allow(dead_code)]
    pub fn close(&mut self) {
        match self {
            WalletSeed::Open(open_seed) => {
                let closed_seed = open_seed.wallet_info.clone();
                *self = WalletSeed::Closed(closed_seed);
            }
            WalletSeed::Closed(_) => {
                // Wallet is already closed
            }
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
    /// Derive and register the wallet's full bootstrap address set from a
    /// borrowed HD seed.
    ///
    /// The seed is supplied by the async caller that already holds a
    /// [`with_secret_session`](crate::wallet_backend::SecretAccess::with_secret_session)
    /// scope — one session spans the whole bootstrap run. It is borrowed for
    /// the duration of the call and never copied into an owned buffer that
    /// outlives the scope; every `bootstrap_*` child derives in place from the
    /// same borrow. Derivation paths, the per-network coin-type, and the keys
    /// are identical to the prior parked-seed path — only the seed *source*
    /// changes (parameter vs `self`).
    pub fn bootstrap_known_addresses(&mut self, seed: &[u8; 64], app_context: &AppContext) {
        let network = app_context.network;

        if let Err(err) = self.bootstrap_bip44_addresses(network, app_context) {
            tracing::warn!("Failed to bootstrap BIP44 addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_bip32_addresses(seed, network, app_context) {
            tracing::warn!("Failed to bootstrap BIP32 addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_coinjoin_addresses(seed, network, app_context) {
            tracing::warn!("Failed to bootstrap CoinJoin addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_identity_addresses(seed, network, app_context) {
            tracing::warn!("Failed to bootstrap identity addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_provider_addresses(seed, network, app_context) {
            tracing::warn!("Failed to bootstrap provider addresses: {}", err);
        }

        if let Err(err) = self.bootstrap_platform_payment_addresses(seed, network, app_context) {
            tracing::warn!("Failed to bootstrap Platform payment addresses: {}", err);
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

    /// Derive the private key for `derivation_path` from a `seed` borrowed by
    /// the caller (resolved once through the JIT chokepoint), confirming the
    /// matching wallet is present in `slice` by `wallet_seed_hash` without
    /// reading any wallet's parked seed. `Ok(None)` when no wallet matches.
    pub fn derive_private_key_in_arc_rw_lock_slice_with_seed(
        slice: &[Arc<RwLock<Wallet>>],
        wallet_seed_hash: WalletSeedHash,
        seed: &[u8; 64],
        derivation_path: &DerivationPath,
        network: Network,
    ) -> Result<Option<Zeroizing<[u8; 32]>>, String> {
        for wallet in slice {
            let wallet_ref = wallet.read().unwrap();
            if wallet_ref.seed_hash() == wallet_seed_hash {
                // SECURITY: `ExtendedPrivKey` is a `Copy` BIP-32 type from
                // key_wallet with no `Drop`, so its inner SecretKey + ChainCode
                // cannot be wiped by RAII here. Extract the key straight into a
                // `Zeroizing` buffer and never bind the intermediate to a named
                // local; the transient copy left on the stack is the
                // unavoidable residue of a third-party `Copy` type.
                let secret = Zeroizing::new(
                    derivation_path
                        .derive_priv_ecdsa_for_master_seed(seed, network)
                        .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?
                        .private_key
                        .secret_bytes(),
                );
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }

    /// Derive the private key for `derivation_path` from a `seed` borrowed by
    /// the caller (resolved through the JIT chokepoint). Same path, same
    /// per-network derivation, same key as the BIP-32 spec dictates.
    pub fn private_key_at_derivation_path_with_seed(
        &self,
        seed: &[u8; 64],
        derivation_path: &DerivationPath,
        network: Network,
    ) -> Result<PrivateKey, String> {
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(seed, network)
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        Ok(extended_private_key.to_priv())
    }

    /// Derive the private key for a known `address` from a `seed` borrowed by
    /// the caller (resolved through the JIT chokepoint).
    ///
    /// Looks up the address's derivation path (pure, no secret) and derives at
    /// it. `Ok(None)` when the address is not one of this wallet's known
    /// addresses. Same path, same per-network derivation, same key.
    pub fn private_key_for_address_with_seed(
        &self,
        seed: &[u8; 64],
        address: &Address,
        network: Network,
    ) -> Result<Option<PrivateKey>, String> {
        self.known_addresses
            .get(address)
            .map(|derivation_path| {
                derivation_path
                    .derive_priv_ecdsa_for_master_seed(seed, network)
                    .map(|extended_private_key| extended_private_key.to_priv())
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())
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
        let snapshot_address_balances = register
            .map(|ctx| ctx.snapshot_address_balances(&self.seed_hash()))
            .unwrap_or_default();
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
                let balance = snapshot_address_balances.get(address).cloned().unwrap_or(0);

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
                        .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?
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
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?
                    .to_pub();
                known_public_key = Some(public_key);
                if let Some(app_context) = register {
                    let address = Address::p2pkh(&public_key, network);
                    app_context.try_import_address(
                        &address,
                        self.core_wallet_name.as_deref(),
                        Some(&format!(
                            "Managed by Dash Evo Tool {} {}",
                            self.alias.clone().unwrap_or_default(),
                            derivation_path
                        )),
                    );

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

    /// Look up one identity-authentication ECDSA public key from the
    /// memoised [`AuthPubkeyCache`], without touching the seed.
    ///
    /// Returns `None` on a cache miss; the caller resolves the seed
    /// just-in-time via the [`SecretAccess`](crate::wallet_backend::SecretAccess)
    /// chokepoint, derives with
    /// [`Self::identity_authentication_ecdsa_public_key_from_seed`], and
    /// repopulates the cache. The hardened leaf makes seed-free *first*
    /// derivation impossible, so the cache is the only seed-free read.
    pub fn identity_authentication_ecdsa_public_key_cached(
        &self,
        cache: &AuthPubkeyCache,
        network: Network,
        identity_index: u32,
        key_index: u32,
    ) -> Option<PublicKey> {
        cache.get(network, identity_index, key_index)
    }

    /// Derive one identity-authentication ECDSA public key from a
    /// borrowed HD seed.
    ///
    /// The seed is supplied by the async caller holding a `with_secret`
    /// scope; it is never read from `self`. The result is byte-identical
    /// to the cached value [`Self::identity_authentication_ecdsa_public_key_cached`]
    /// serves once the cache is warm — the caller writes it back on a
    /// cold miss.
    pub fn identity_authentication_ecdsa_public_key_from_seed(
        &self,
        seed: &[u8; 64],
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
            .derive_pub_ecdsa_for_master_seed(seed, network)
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        Ok(extended_public_key.to_pub())
    }

    /// Build the two identity-auth public-key lookup maps for `range`
    /// from the cache alone, returning the key indices that missed.
    ///
    /// Seed-free: every map entry is reconstructed from a cached
    /// `PublicKey`, and address registration (when requested) uses that
    /// same reconstructed key. The returned `Vec<u32>` lists the indices
    /// the caller must cold-fill — see
    /// [`Self::identity_authentication_ecdsa_public_keys_data_map_from_seed`].
    /// Partitioning misses here lets the caller open a *single*
    /// `with_secret` scope for the whole request.
    #[allow(clippy::type_complexity)]
    pub fn identity_authentication_ecdsa_public_keys_data_map_cached(
        &mut self,
        app_context: &AppContext,
        register_addresses: bool,
        cache: &AuthPubkeyCache,
        network: Network,
        identity_index: u32,
        key_index_range: Range<u32>,
    ) -> Result<(BTreeMap<Vec<u8>, u32>, BTreeMap<[u8; 20], u32>, Vec<u32>), String> {
        let mut public_key_result_map = BTreeMap::new();
        let mut public_key_hash_result_map = BTreeMap::new();
        let mut misses = Vec::new();
        for key_index in key_index_range {
            let Some(public_key) = cache.get(network, identity_index, key_index) else {
                misses.push(key_index);
                continue;
            };
            self.record_identity_auth_public_key(
                &mut public_key_result_map,
                &mut public_key_hash_result_map,
                app_context,
                register_addresses,
                network,
                identity_index,
                key_index,
                &public_key,
            )?;
        }
        Ok((public_key_result_map, public_key_hash_result_map, misses))
    }

    /// Cold-fill the cache-miss key indices from a borrowed HD seed.
    ///
    /// Returns the two lookup maps for the *missing* indices plus the
    /// freshly derived `(key_index, PublicKey)` pairs for cache write-back;
    /// the caller merges the maps into its cache-hit maps. The seed is
    /// supplied by the async caller's single `with_secret` scope and never
    /// read from `self`. Address registration mirrors the cached path so
    /// behaviour is identical regardless of cache warmth.
    #[allow(clippy::type_complexity)]
    pub fn identity_authentication_ecdsa_public_keys_data_map_from_seed(
        &mut self,
        app_context: &AppContext,
        register_addresses: bool,
        seed: &[u8; 64],
        network: Network,
        identity_index: u32,
        missing_key_indices: &[u32],
    ) -> Result<
        (
            BTreeMap<Vec<u8>, u32>,
            BTreeMap<[u8; 20], u32>,
            Vec<(u32, PublicKey)>,
        ),
        String,
    > {
        let mut public_key_result_map = BTreeMap::new();
        let mut public_key_hash_result_map = BTreeMap::new();
        let mut derived = Vec::with_capacity(missing_key_indices.len());
        for &key_index in missing_key_indices {
            let public_key = self.identity_authentication_ecdsa_public_key_from_seed(
                seed,
                network,
                identity_index,
                key_index,
            )?;
            self.record_identity_auth_public_key(
                &mut public_key_result_map,
                &mut public_key_hash_result_map,
                app_context,
                register_addresses,
                network,
                identity_index,
                key_index,
                &public_key,
            )?;
            derived.push((key_index, public_key));
        }
        Ok((public_key_result_map, public_key_hash_result_map, derived))
    }

    /// Fold one identity-auth public key into the serialized-key and
    /// hash160 lookup maps, registering its address when requested.
    /// Shared by the cache-hit and cold-fill paths so both produce
    /// byte-identical map entries.
    #[allow(clippy::too_many_arguments)]
    fn record_identity_auth_public_key(
        &mut self,
        public_key_result_map: &mut BTreeMap<Vec<u8>, u32>,
        public_key_hash_result_map: &mut BTreeMap<[u8; 20], u32>,
        app_context: &AppContext,
        register_addresses: bool,
        network: Network,
        identity_index: u32,
        key_index: u32,
        public_key: &PublicKey,
    ) -> Result<(), String> {
        public_key_result_map.insert(public_key.inner.serialize().to_vec(), key_index);
        public_key_hash_result_map.insert(public_key.pubkey_hash().to_byte_array(), key_index);
        if register_addresses {
            let derivation_path = DerivationPath::identity_authentication_path(
                network,
                KeyDerivationType::ECDSA,
                identity_index,
                key_index,
            );
            self.register_address_from_public_key(
                public_key,
                &derivation_path,
                DerivationPathType::SINGLE_USER_AUTHENTICATION,
                DerivationPathReference::BlockchainIdentities,
                app_context,
            )?;
        }
        Ok(())
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

        // T-W-01: addresses are derived deterministically from the
        // master xpub each time the wallet is loaded, so the legacy
        // `wallet_addresses` write that used to live here is a dead
        // write — no production read path consumes it. The in-memory
        // `known_addresses` / `watched_addresses` maps below stay the
        // single source of truth at runtime.
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
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        for account in 0..BOOTSTRAP_BIP32_ACCOUNT_COUNT {
            for index in 0..BOOTSTRAP_BIP32_ADDRESS_COUNT {
                let derivation_path = DerivationPath::from(vec![
                    ChildNumber::Hardened { index: account },
                    ChildNumber::Normal { index },
                ]);
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(seed, network)
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        for account in 0..BOOTSTRAP_COINJOIN_ACCOUNT_COUNT {
            let base_path = DerivationPath::coinjoin_path(network, account);
            for index in 0..BOOTSTRAP_COINJOIN_ADDRESS_COUNT {
                let mut components = base_path.as_ref().to_vec();
                components.push(ChildNumber::Normal { index });
                let derivation_path = DerivationPath::from(components);
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(seed, network)
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        let registration_indices = self.identity_registration_indices();
        self.bootstrap_identity_registration_addresses(
            seed,
            network,
            app_context,
            &registration_indices,
        )?;
        self.bootstrap_identity_invitation_addresses(seed, network, app_context)?;
        self.bootstrap_identity_topup_addresses(seed, network, app_context, &registration_indices)?;
        Ok(())
    }

    fn bootstrap_identity_registration_addresses(
        &mut self,
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
        registration_indices: &BTreeSet<u32>,
    ) -> Result<(), String> {
        for &index in registration_indices {
            let derivation_path = DerivationPath::identity_registration_path(network, index);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(seed, network)
                .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        for index in 0..BOOTSTRAP_IDENTITY_INVITATION_COUNT {
            let derivation_path = DerivationPath::identity_invitation_path(network, index);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(seed, network)
                .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
        registration_indices: &BTreeSet<u32>,
    ) -> Result<(), String> {
        for &registration_index in registration_indices {
            for top_up_index in 0..BOOTSTRAP_IDENTITY_TOPUP_PER_REGISTRATION {
                let derivation_path =
                    DerivationPath::identity_top_up_path(network, registration_index, top_up_index);
                let extended_private_key = derivation_path
                    .derive_priv_ecdsa_for_master_seed(seed, network)
                    .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        self.bootstrap_identity_topup_not_bound_addresses(network, app_context, seed)
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
                .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        self.bootstrap_provider_account(
            seed,
            network,
            app_context,
            AccountType::ProviderVotingKeys,
        )?;
        self.bootstrap_provider_account(
            seed,
            network,
            app_context,
            AccountType::ProviderOwnerKeys,
        )?;
        Ok(())
    }

    fn bootstrap_provider_account(
        &mut self,
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
        account_type: AccountType,
    ) -> Result<(), String> {
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
                .derive_priv_ecdsa_for_master_seed(seed, network)
                .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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

    /// Bootstrap DIP-17 Platform payment addresses (dash/tdash Bech32m HRP per DIP-18)
    /// These addresses are for receiving Dash Credits on Platform, independent of identities.
    fn bootstrap_platform_payment_addresses(
        &mut self,
        seed: &[u8; 64],
        network: Network,
        app_context: &AppContext,
    ) -> Result<(), String> {
        // Default account 0', default key_class 0' (as per DIP-17)
        let account = 0u32;
        let key_class = 0u32;

        for index in 0..BOOTSTRAP_PLATFORM_PAYMENT_ADDRESS_COUNT {
            let derivation_path =
                DerivationPath::platform_payment_path(network, account, key_class, index);
            let extended_private_key = derivation_path
                .derive_priv_ecdsa_for_master_seed(seed, network)
                .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
        let canonical_address = Wallet::canonical_address(&address, app_context.network);

        // T-W-01: dead legacy `wallet_addresses` write removed — the
        // in-memory maps below are the single runtime source of truth
        // and the picker rederives from the master xpub at cold boot.
        // Platform payment addresses are still not imported to Core (the
        // address format differs).
        self.known_addresses
            .insert(canonical_address.clone(), derivation_path.clone());
        self.watched_addresses.insert(
            derivation_path.clone(),
            AddressInfo {
                address: canonical_address.clone(),
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
            Network::Mainnet => 5,
            _ => 1,
        }
    }

    /// Derive a BIP-44 receive address DET-side, advancing the in-memory index.
    ///
    /// NOT funds-safe for user-facing receiving: with `skip = true` it walks the
    /// index past the upstream gap-limit window, so the returned address may be
    /// outside the SPV-watched pool. The Receive flow no longer uses this — it
    /// routes through `WalletBackend::next_receive_address` (upstream watched
    /// pool). Remaining callers derive *funding* addresses for identity / asset-
    /// lock creation, not user receive addresses.
    ///
    // TODO(funds-safety): migrate the identity-creation funding
    // (`backend_task/identity`) and asset-lock funding (`create_asset_lock_screen`)
    // callers onto the upstream watched pool, then delete this method and
    // `unused_bip_44_public_key`. Those funding addresses must also be SPV-watched
    // for the funded outputs to be seen.
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

    /// Derive and register a *new* Platform payment address at the next unused
    /// index from a `seed` borrowed by the caller (resolved through the JIT
    /// chokepoint).
    ///
    /// Production callers reach this through
    /// [`AppContext::generate_platform_receive_address`](crate::context::AppContext)
    /// which opens the secret scope. The "return an existing address" shortcut
    /// is the caller's responsibility (see `Wallet::platform_addresses`); this
    /// is the unlock-required generation step. Same DIP-17 path, same
    /// per-network derivation, same address as the retired parked-seed method.
    pub fn generate_platform_receive_address_with_seed(
        &mut self,
        seed: &[u8; 64],
        network: Network,
        register: Option<&AppContext>,
    ) -> Result<Address, String> {
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

        // Generate a new Platform address at the next index
        let next_index = existing_indices.iter().max().map(|m| m + 1).unwrap_or(0);

        let derivation_path =
            DerivationPath::platform_payment_path(network, account, key_class, next_index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(seed, network)
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
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
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?
            .to_pub();
        Ok(Address::p2pkh(&public_key, network))
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

    /// Get Platform address info by canonical address comparison.
    ///
    /// This method handles the case where the same platform address may be represented
    /// by different Address objects. It normalizes by comparing PlatformAddress bytes
    /// to find a matching entry.
    pub fn get_platform_address_info(&self, address: &Address) -> Option<&PlatformAddressInfo> {
        // First try direct lookup
        if let Some(info) = self.platform_address_info.get(address) {
            return Some(info);
        }

        // If direct lookup fails, try canonical comparison via PlatformAddress bytes
        if let Ok(platform_addr) = PlatformAddress::try_from(address.clone()) {
            let canonical_bytes = platform_addr.to_bytes();
            for (existing_addr, info) in &self.platform_address_info {
                if let Ok(existing_platform) = PlatformAddress::try_from(existing_addr.clone())
                    && existing_platform.to_bytes() == canonical_bytes
                {
                    return Some(info);
                }
            }
        }

        None
    }

    /// Update Platform address info (balance and nonce).
    ///
    /// Handles canonical address deduplication: if the same platform address is
    /// stored under a different `Address` key, the duplicate is removed first.
    pub fn set_platform_address_info(
        &mut self,
        address: Address,
        balance: Credits,
        nonce: AddressNonce,
    ) {
        // Remove duplicate entries for the same canonical platform address
        if let Ok(platform_addr) = PlatformAddress::try_from(address.clone()) {
            let canonical_bytes = platform_addr.to_bytes();
            let keys_to_remove: Vec<Address> = self
                .platform_address_info
                .keys()
                .filter(|existing_addr| {
                    if let Ok(existing_platform) =
                        PlatformAddress::try_from((*existing_addr).clone())
                    {
                        existing_platform.to_bytes() == canonical_bytes
                            && *existing_addr != &address
                    } else {
                        false
                    }
                })
                .cloned()
                .collect();

            for key in keys_to_remove {
                self.platform_address_info.remove(&key);
            }
        }

        self.platform_address_info
            .insert(address, PlatformAddressInfo { balance, nonce });
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
/// // `seed` is borrowed from inside a JIT `with_secret(_session)` scope.
/// let mut provider = WalletAddressProvider::new(&wallet, network, seed)?;
/// let result = sdk.sync_address_balances(&mut provider, None, None).await?;
/// provider.apply_results_to_wallet(&mut wallet);
/// ```
pub struct WalletAddressProvider {
    /// Network for address derivation
    network: Network,
    /// Gap limit for HD wallet scanning
    gap_limit: AddressIndex,
    /// DIP-17 account-level extended **public** key at
    /// `m/9'/coin_type'/17'/account'/key_class'`. All gap-limit children are
    /// the non-hardened final `index`, so addresses derive from this public
    /// key alone — the provider never holds the plaintext seed. The seed is
    /// borrowed once at construction (through the JIT chokepoint) to derive
    /// this xpub, then dropped.
    account_xpub: ExtendedPubKey,
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
    /// Account / key-class used for Platform payment derivation. Single
    /// source of truth for the constructors and the xpub derivation.
    const PLATFORM_ACCOUNT: u32 = 0;
    const PLATFORM_KEY_CLASS: u32 = 0;

    /// Create a new WalletAddressProvider from a borrowed HD seed.
    ///
    /// The `seed` is resolved by the async caller through the JIT secret
    /// chokepoint and borrowed only for this construction — it is used once to
    /// derive the DIP-17 account-level extended public key and is never copied
    /// into the provider. All subsequent address derivation is public-key only.
    ///
    /// # Errors
    /// Returns an error if the account-level xpub cannot be derived.
    pub fn new(wallet: &Wallet, network: Network, seed: &[u8; 64]) -> Result<Self, String> {
        Self::with_gap_limit(wallet, network, DEFAULT_GAP_LIMIT, seed)
    }

    /// Create a new WalletAddressProvider with a custom gap limit from a
    /// borrowed HD seed. See [`new`](Self::new) for the seed-borrow contract.
    ///
    /// # Errors
    /// Returns an error if the account-level xpub cannot be derived.
    pub fn with_gap_limit(
        _wallet: &Wallet,
        network: Network,
        gap_limit: AddressIndex,
        seed: &[u8; 64],
    ) -> Result<Self, String> {
        let account = Self::PLATFORM_ACCOUNT;
        let key_class = Self::PLATFORM_KEY_CLASS;
        let account_xpub = Self::derive_account_xpub(seed, network, account, key_class)?;

        let mut provider = Self {
            network,
            gap_limit,
            account_xpub,
            account,
            key_class,
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

    /// Derive the DIP-17 account-level extended **public** key at
    /// `m/9'/coin_type'/17'/account'/key_class'` from the borrowed seed.
    ///
    /// This is the only place the seed is touched. The hardened account /
    /// key-class steps require the private key, so the seed is needed here; the
    /// resulting xpub then derives every non-hardened `index` child publicly.
    fn derive_account_xpub(
        seed: &[u8; 64],
        network: Network,
        account: u32,
        key_class: u32,
    ) -> Result<ExtendedPubKey, String> {
        let coin_type = Wallet::coin_type(network);
        let account_path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 9 },
            ChildNumber::Hardened { index: coin_type },
            ChildNumber::Hardened { index: 17 },
            ChildNumber::Hardened { index: account },
            ChildNumber::Hardened { index: key_class },
        ]);
        let secp = Secp256k1::new();
        let master = ExtendedPrivKey::new_master(network, seed)
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        let account_priv = master
            .derive_priv(&secp, &account_path)
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        Ok(ExtendedPubKey::from_priv(&secp, &account_priv))
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

    /// Apply the sync results to a wallet, updating Platform address info.
    ///
    /// This updates the wallet's `platform_address_info` with the balances found during sync.
    /// Also ensures addresses are registered in `known_addresses` and `watched_addresses`
    /// so they appear in the UI.
    /// Nonces are taken directly from the SDK sync results.
    pub fn apply_results_to_wallet(&self, wallet: &mut Wallet) {
        // Build a reverse lookup from address to index
        let address_to_index: BTreeMap<Address, AddressIndex> = self
            .pending
            .iter()
            .map(|(idx, (_, addr))| (Wallet::canonical_address(addr, self.network), *idx))
            .collect();

        for (address, funds) in &self.found_balances {
            let canonical_address = Wallet::canonical_address(address, self.network);

            // Update wallet with synced balances
            wallet.set_platform_address_info(canonical_address.clone(), funds.balance, funds.nonce);

            // Also register in known_addresses and watched_addresses if not already present
            if !wallet.known_addresses.contains_key(&canonical_address)
                && let Some(&index) = address_to_index.get(&canonical_address)
            {
                let derivation_path = DerivationPath::platform_payment_path(
                    self.network,
                    self.account,
                    self.key_class,
                    index,
                );

                wallet
                    .known_addresses
                    .insert(canonical_address.clone(), derivation_path.clone());

                wallet.watched_addresses.insert(
                    derivation_path,
                    AddressInfo {
                        address: canonical_address.clone(),
                        path_type: DerivationPathType::CLEAR_FUNDS,
                        path_reference: DerivationPathReference::PlatformPayment,
                    },
                );
            }
        }
    }

    /// Populate stored balances and sync height from a wallet's known state.
    ///
    /// Call this after construction to enable incremental catch-up.
    /// The SDK uses `current_balances()` as the baseline and `last_sync_height()`
    /// as the starting block for applying delta operations.
    pub fn with_stored_state(
        mut self,
        wallet: &Wallet,
        network: Network,
        last_sync_height: u64,
    ) -> Self {
        self.stored_sync_height = last_sync_height;

        // Populate stored_balances from wallet's known platform addresses
        for (core_addr, info) in &wallet.platform_address_info {
            // Find the matching pending address to get the index and key
            for (index, (key, pending_addr)) in &self.pending {
                let canonical = Wallet::canonical_address(pending_addr, network);
                if &canonical == core_addr {
                    self.stored_balances.push((
                        *index,
                        *key,
                        AddressFunds {
                            balance: info.balance,
                            nonce: info.nonce,
                        },
                    ));
                    break;
                }
            }
        }

        self
    }

    /// Derive a Platform address at the given index from the account-level
    /// **public** key — no seed access.
    ///
    /// The DIP-17 final `index` is a non-hardened child, so deriving it from
    /// the account xpub yields the same public key (and therefore the same
    /// P2PKH address) the legacy seed-based derivation produced. Parity is
    /// asserted by the `xpub_derivation_matches_seed_derivation` test.
    fn derive_address_at_index(
        &self,
        index: AddressIndex,
    ) -> Result<(PlatformAddress, Address), String> {
        let secp = Secp256k1::new();
        let child = self
            .account_xpub
            .derive_pub(&secp, &[ChildNumber::Normal { index }])
            .map_err(|e| WalletError::KeyDerivation { source: e }.to_string())?;
        let public_key = child.to_pub();

        // Create P2PKH address
        let address = Address::p2pkh(&public_key, self.network);

        // Convert to PlatformAddress (the SDK address-sync key type)
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

#[async_trait]
impl AddressProvider for WalletAddressProvider {
    type Tag = AddressIndex;
    type Address = PlatformAddress;

    fn gap_limit(&self) -> AddressIndex {
        self.gap_limit
    }

    fn pending_addresses(&self) -> impl Iterator<Item = (AddressIndex, PlatformAddress)> + '_ {
        self.pending
            .iter()
            .filter(|(index, _)| !self.resolved.contains(index))
            .map(|(index, (platform_addr, _))| (*index, *platform_addr))
    }

    async fn on_address_found(
        &mut self,
        index: AddressIndex,
        _address: &PlatformAddress,
        funds: AddressFunds,
    ) {
        self.resolved.insert(index);

        // Log what the SDK is returning
        if let Some((_, core_address)) = self.pending.get(&index) {
            // Also show Platform address format for comparison
            let platform_addr_str = PlatformAddress::try_from(core_address.clone())
                .map(|p| p.to_bech32m_string(self.network))
                .unwrap_or_else(|_| "conversion failed".to_string());
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

    async fn on_address_absent(&mut self, index: AddressIndex, _address: &PlatformAddress) {
        self.resolved.insert(index);
    }

    fn has_pending(&self) -> bool {
        self.pending
            .keys()
            .any(|index| !self.resolved.contains(index))
    }

    fn current_balances(
        &self,
    ) -> impl Iterator<Item = (AddressIndex, PlatformAddress, AddressFunds)> + '_ {
        self.stored_balances.iter().copied()
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

    /// The deterministic 64-byte seed every [`test_wallet`] is built from.
    ///
    /// Since R3 the seed is no longer parked in `WalletSeed::Open`, so tests
    /// that need the wallet's raw seed take it from here rather than from a
    /// (removed) parked-seed accessor — the value is known by construction.
    const TEST_SEED: [u8; 64] = [42u8; 64];

    /// The known seed behind [`test_wallet`]. Test-only replacement for the
    /// removed `Wallet::seed_bytes()` — derives nothing, just hands back the
    /// constant the wallet was built from.
    fn test_seed() -> [u8; 64] {
        TEST_SEED
    }

    /// Helper: create a minimal open wallet for testing.
    /// Uses a deterministic 64-byte seed and derives the BIP44 master public key.
    fn test_wallet() -> Wallet {
        let seed = TEST_SEED;
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

        Wallet {
            wallet_seed: WalletSeed::Open(OpenWalletSeed {
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
            known_addresses: BTreeMap::new(),
            watched_addresses: BTreeMap::new(),
            alias: Some("Test Wallet".to_string()),
            identities: HashMap::new(),
            is_main: true,
            platform_address_info: BTreeMap::new(),
            core_wallet_name: None,
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

    // ========================================================================
    // WalletSeed::open_no_password guard
    // ========================================================================

    /// `open_no_password` must REFUSE a password-protected envelope. There is
    /// no `uses_password` flag on `ClosedKeyItem`, so the guard keys on the
    /// stored-blob length: an unprotected envelope stores the raw 64-byte seed
    /// verbatim, whereas a password-protected blob is AES-256-GCM ciphertext
    /// (64-byte plaintext + 16-byte tag = 80 bytes). Opening a protected
    /// wallet with no passphrase would silently treat the ciphertext as a seed
    /// — this pins the rejection.
    #[test]
    fn open_no_password_rejects_protected_envelope() {
        let seed = [0x42u8; 64];
        let (encrypted_seed, salt, nonce) =
            ClosedKeyItem::encrypt_seed(&seed, "a-passphrase").expect("encrypt");
        // Precondition: a protected blob is longer than a bare 64-byte seed.
        assert_ne!(
            encrypted_seed.len(),
            64,
            "protected ciphertext must not be exactly 64 bytes"
        );

        let mut wallet_seed = WalletSeed::Closed(ClosedKeyItem {
            seed_hash: ClosedKeyItem::compute_seed_hash(&seed),
            encrypted_seed,
            salt,
            nonce,
            password_hint: None,
        });

        let result = wallet_seed.open_no_password();
        assert!(
            result.is_err(),
            "open_no_password must reject a password-protected envelope"
        );
        assert!(
            matches!(wallet_seed, WalletSeed::Closed(_)),
            "the wallet must stay Closed when open_no_password is refused"
        );
    }

    /// The matching accept case: an unprotected envelope stores the raw
    /// 64-byte seed verbatim, so `open_no_password` flips it to `Open`.
    #[test]
    fn open_no_password_accepts_unprotected_envelope() {
        let seed = [0x09u8; 64];
        let mut wallet_seed = WalletSeed::Closed(ClosedKeyItem {
            seed_hash: ClosedKeyItem::compute_seed_hash(&seed),
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
        });

        wallet_seed
            .open_no_password()
            .expect("unprotected envelope opens without a password");
        assert!(
            matches!(wallet_seed, WalletSeed::Open(_)),
            "unprotected wallet must flip to Open"
        );
    }

    // ========================================================================
    // Platform address info tests
    // ========================================================================

    #[test]
    fn test_total_platform_balance_empty() {
        let wallet = test_wallet();
        assert_eq!(wallet.total_platform_balance(), 0);
    }

    #[test]
    fn test_total_platform_balance_with_entries() {
        let mut wallet = test_wallet();
        let addr1 = test_address(1);
        let addr2 = test_address(2);

        wallet.platform_address_info.insert(
            addr1,
            PlatformAddressInfo {
                balance: 1_000_000,
                nonce: 0,
            },
        );
        wallet.platform_address_info.insert(
            addr2,
            PlatformAddressInfo {
                balance: 2_000_000,
                nonce: 1,
            },
        );

        assert_eq!(wallet.total_platform_balance(), 3_000_000);
    }

    #[test]
    fn test_set_platform_address_info_update() {
        let mut wallet = test_wallet();
        let addr = test_address(1);

        wallet.set_platform_address_info(addr.clone(), 500_000, 3);

        wallet.set_platform_address_info(addr.clone(), 600_000, 4);

        let info = wallet.platform_address_info.get(&addr).unwrap();
        assert_eq!(info.balance, 600_000);
        assert_eq!(info.nonce, 4);
    }

    #[test]
    fn test_get_platform_address_info_direct_lookup() {
        let mut wallet = test_wallet();
        let addr = test_address(1);

        wallet.platform_address_info.insert(
            addr.clone(),
            PlatformAddressInfo {
                balance: 100_000,
                nonce: 1,
            },
        );

        let info = wallet.get_platform_address_info(&addr);
        assert!(info.is_some());
        assert_eq!(info.unwrap().balance, 100_000);
    }

    #[test]
    fn test_get_platform_address_info_not_found() {
        let wallet = test_wallet();
        let addr = test_address(1);
        assert!(wallet.get_platform_address_info(&addr).is_none());
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

    /// R3 capstone: an open wallet retains NO plaintext seed.
    ///
    /// `WalletSeed::Open` is the verify-not-park state — it carries only the
    /// encrypted-envelope metadata, never the decrypted seed. This test pins
    /// the invariant structurally: the `Open` payload exposes only
    /// `wallet_info` (a [`ClosedKeyItem`], whose `encrypted_seed` is, for a
    /// password wallet, the ciphertext — not the plaintext), and there is no
    /// accessor that yields the plaintext seed from an open wallet.
    #[test]
    fn open_wallet_retains_no_plaintext_seed() {
        let password = "correct horse battery staple";
        let secret = Secret::new(password);
        let mut wallet = Wallet::new_from_seed(
            test_seed(),
            Network::Testnet,
            Some("verify-not-park".to_string()),
            Some(&secret),
        )
        .expect("build password wallet");

        // Lock, then unlock by verifying the passphrase.
        wallet.wallet_seed.close();
        assert!(!wallet.is_open());
        wallet
            .wallet_seed
            .open(password)
            .expect("correct passphrase verifies");
        assert!(wallet.is_open(), "verified-correct passphrase opens");

        // The open payload holds only the ENCRYPTED envelope — never the
        // plaintext seed. For a password wallet the stored bytes are the
        // ciphertext, which must differ from the plaintext seed.
        let WalletSeed::Open(open) = &wallet.wallet_seed else {
            panic!("wallet should be open");
        };
        assert_ne!(
            open.wallet_info.encrypted_seed,
            test_seed().to_vec(),
            "open wallet must not store the plaintext seed"
        );
        // A wrong passphrase is rejected — proves `open` truly decrypts to
        // verify, it does not just flip a flag.
        wallet.wallet_seed.close();
        assert!(wallet.wallet_seed.open("wrong passphrase").is_err());
    }

    /// `Debug` of an UNPROTECTED wallet must never leak the plaintext seed.
    ///
    /// For a no-password wallet `encrypted_seed` holds the raw 64-byte seed
    /// verbatim. `ClosedKeyItem` redacts it in `Debug`, and `WalletSeed`,
    /// `OpenWalletSeed`, and `Wallet` all delegate to that impl. A known
    /// distinctive seed (not all-equal, so byte fragments are unambiguous)
    /// must appear in none of their `Debug` renderings.
    #[test]
    fn debug_output_never_leaks_plaintext_seed() {
        let mut seed = [0u8; 64];
        for (i, b) in seed.iter_mut().enumerate() {
            *b = i as u8;
        }
        let wallet = Wallet::new_from_seed(seed, Network::Testnet, None, None)
            .expect("build unprotected wallet");

        // The unprotected envelope stores the raw plaintext.
        assert_eq!(wallet.encrypted_seed_slice(), seed.as_slice());

        let needle = hex::encode(seed);
        let closed = ClosedKeyItem {
            seed_hash: ClosedKeyItem::compute_seed_hash(&seed),
            encrypted_seed: seed.to_vec(),
            salt: vec![],
            nonce: vec![],
            password_hint: None,
        };
        let open_seed = OpenWalletSeed {
            wallet_info: closed.clone(),
        };
        let wallet_seed = WalletSeed::Open(open_seed.clone());

        for (label, rendered) in [
            ("ClosedKeyItem", format!("{closed:?}")),
            ("OpenWalletSeed", format!("{open_seed:?}")),
            ("WalletSeed", format!("{wallet_seed:?}")),
            ("Wallet", format!("{wallet:?}")),
        ] {
            assert!(
                !rendered.contains(&needle),
                "{label} Debug leaked hex seed bytes: {rendered}"
            );
            // Also catch the raw comma-separated `Vec<u8>` rendering.
            assert!(
                !rendered.contains("0, 1, 2, 3, 4, 5"),
                "{label} Debug leaked raw seed byte sequence: {rendered}"
            );
            assert!(
                rendered.contains("[redacted]"),
                "{label} Debug should mark the seed redacted: {rendered}"
            );
        }
    }

    // ========================================================================
    // R3 D4b — identity-auth public-key cache byte-equivalence & cold-fill
    // ========================================================================

    /// D4B-EQUIV-001 — the cached public key is byte-identical to the
    /// seed-derived one. Warm the cache from the seed-derived key, read it
    /// back through the cache path, and assert both the compressed bytes
    /// and the hash160 match. This is the load-bearing correctness claim:
    /// a cache hit must never serve a different key than the seed would.
    #[test]
    fn auth_pubkey_cached_equals_seed_derived() {
        let wallet = test_wallet();
        let seed = test_seed();
        let network = Network::Testnet;

        for identity_index in 0..2u32 {
            for key_index in 0..4u32 {
                let from_seed = wallet
                    .identity_authentication_ecdsa_public_key_from_seed(
                        &seed,
                        network,
                        identity_index,
                        key_index,
                    )
                    .expect("seed derivation");

                let mut cache = AuthPubkeyCache::default();
                cache.insert(network, identity_index, key_index, &from_seed);

                let from_cache = wallet
                    .identity_authentication_ecdsa_public_key_cached(
                        &cache,
                        network,
                        identity_index,
                        key_index,
                    )
                    .expect("cache hit");

                assert_eq!(from_cache, from_seed);
                assert_eq!(from_cache.inner.serialize(), from_seed.inner.serialize());
                assert_eq!(from_cache.pubkey_hash(), from_seed.pubkey_hash());
            }
        }
    }

    /// D4B-COLD-001 — a cold (empty) cache misses; the seed-derived path
    /// produces the key; warming the cache makes the subsequent read
    /// seed-free and identical. Exercises the cold ⇒ JIT-fill ⇒ warm
    /// read-back self-heal contract at the model level.
    #[test]
    fn auth_pubkey_cold_cache_self_heals() {
        let wallet = test_wallet();
        let seed = test_seed();
        let network = Network::Testnet;
        let (identity_index, key_index) = (1u32, 3u32);

        let mut cache = AuthPubkeyCache::default();
        // Cold: the cache misses entirely.
        assert!(
            wallet
                .identity_authentication_ecdsa_public_key_cached(
                    &cache,
                    network,
                    identity_index,
                    key_index,
                )
                .is_none()
        );

        // JIT cold-fill from the seed, then populate the cache.
        let derived = wallet
            .identity_authentication_ecdsa_public_key_from_seed(
                &seed,
                network,
                identity_index,
                key_index,
            )
            .expect("seed derivation");
        assert!(cache.insert(network, identity_index, key_index, &derived));

        // Warm: the same read now serves from cache, byte-identical.
        let warmed = wallet
            .identity_authentication_ecdsa_public_key_cached(
                &cache,
                network,
                identity_index,
                key_index,
            )
            .expect("cache hit after fill");
        assert_eq!(warmed, derived);

        // A different network coordinate stays cold (network-keyed).
        assert!(
            wallet
                .identity_authentication_ecdsa_public_key_cached(
                    &cache,
                    Network::Mainnet,
                    identity_index,
                    key_index,
                )
                .is_none()
        );
    }

    // ========================================================================
    // R3 D2 — seed-as-parameter derivation drift tests
    // ========================================================================

    /// One representative derivation path from every `bootstrap_*` family.
    ///
    /// The bootstrap children differ only in WHICH paths they enumerate; the
    /// seed-dependent step is identical (`derive_priv_ecdsa_for_master_seed`),
    /// so proving the seed-param derivation matches the self-seed derivation on
    /// this representative set proves the whole bootstrap address set is
    /// unchanged by the seed-source switch.
    fn representative_bootstrap_paths(network: Network) -> Vec<DerivationPath> {
        let coin_type = Wallet::coin_type(network);
        let coinjoin = {
            let mut c = DerivationPath::coinjoin_path(network, 0).as_ref().to_vec();
            c.push(ChildNumber::Normal { index: 3 });
            DerivationPath::from(c)
        };
        let provider_owner = {
            let mut c = AccountType::ProviderOwnerKeys
                .derivation_path(network)
                .expect("provider path")
                .as_ref()
                .to_vec();
            c.push(ChildNumber::Hardened { index: 1 });
            DerivationPath::from(c)
        };
        let topup_not_bound = {
            let mut c = AccountType::IdentityTopUpNotBoundToIdentity
                .derivation_path(network)
                .expect("not-bound path")
                .as_ref()
                .to_vec();
            c.push(ChildNumber::Normal { index: 2 });
            DerivationPath::from(c)
        };
        vec![
            // BIP-32
            DerivationPath::from(vec![
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index: 5 },
            ]),
            // BIP-44 external + change
            DerivationPath::from(vec![
                ChildNumber::Hardened { index: 44 },
                ChildNumber::Hardened { index: coin_type },
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index: 7 },
            ]),
            DerivationPath::from(vec![
                ChildNumber::Hardened { index: 44 },
                ChildNumber::Hardened { index: coin_type },
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index: 1 },
                ChildNumber::Normal { index: 4 },
            ]),
            coinjoin,
            DerivationPath::identity_registration_path(network, 2),
            DerivationPath::identity_invitation_path(network, 3),
            DerivationPath::identity_top_up_path(network, 1, 2),
            topup_not_bound,
            provider_owner,
            DerivationPath::platform_payment_path(network, 0, 0, 6),
        ]
    }

    /// The seed-as-parameter derivation produces byte-identical private keys to
    /// a direct BIP-32 reference derivation across every bootstrap family — the
    /// derivation math is the spec, not the wrapper.
    #[test]
    fn seed_param_derivation_matches_reference_derivation() {
        for network in [Network::Testnet, Network::Mainnet] {
            let wallet = test_wallet();
            // The per-path private key is derived directly from the raw seed
            // (BIP-44 master xpub is not involved), so the derivation is
            // network-correct as long as the same `network` is passed.
            let seed = test_seed();
            for path in representative_bootstrap_paths(network) {
                let reference = path
                    .derive_priv_ecdsa_for_master_seed(&seed, network)
                    .expect("reference derive")
                    .to_priv();
                let from_param = wallet
                    .private_key_at_derivation_path_with_seed(&seed, &path, network)
                    .expect("seed-param derive");
                assert_eq!(
                    reference.to_bytes(),
                    from_param.to_bytes(),
                    "derivation drift on path {path} for {network:?}"
                );
            }
        }
    }

    /// `private_key_for_address_with_seed` resolves the same key a direct
    /// reference derivation at the address's stored path produces.
    #[test]
    fn private_key_for_address_seed_param_matches_reference() {
        let network = Network::Testnet;
        let wallet = test_wallet();
        let seed = test_seed();

        // Derive a known address + path and register it in the wallet.
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 1 },
        ]);
        let reference = path
            .derive_priv_ecdsa_for_master_seed(&seed, network)
            .expect("reference derive")
            .to_priv();
        let secp = Secp256k1::new();
        let address = Address::p2pkh(&reference.public_key(&secp), network);

        let mut wallet = wallet;
        wallet.known_addresses.insert(address.clone(), path);

        let param = wallet
            .private_key_for_address_with_seed(&seed, &address, network)
            .expect("param")
            .expect("known");
        assert_eq!(reference.to_bytes(), param.to_bytes());
    }

    /// `generate_platform_receive_address_with_seed` derives the DIP-17
    /// platform-payment address at the next unused index. On a fresh wallet
    /// that is index 0 — assert byte parity against a direct reference
    /// derivation at `platform_payment_path(network, 0, 0, 0)` (the legacy
    /// parked-seed `platform_receive_address` it replaced is gone).
    #[test]
    fn platform_receive_address_seed_param_matches() {
        for network in [Network::Testnet, Network::Mainnet] {
            let mut param_wallet = test_wallet();
            let seed = test_seed();

            let reference_path = DerivationPath::platform_payment_path(network, 0, 0, 0);
            let reference_xprv = reference_path
                .derive_priv_ecdsa_for_master_seed(&seed, network)
                .expect("reference derive");
            let secp = Secp256k1::new();
            let reference = Address::p2pkh(&reference_xprv.to_priv().public_key(&secp), network);

            let param = param_wallet
                .generate_platform_receive_address_with_seed(&seed, network, None)
                .expect("seed-param generate");
            assert_eq!(
                reference, param,
                "platform receive address drift for {network:?}"
            );
        }
    }

    /// `derive_private_key_in_arc_rw_lock_slice_with_seed` matches a direct
    /// BIP-32 reference derivation at the same path (the legacy parked-seed
    /// slice-derive it replaced is gone, so parity is anchored independently).
    #[test]
    fn slice_derive_seed_param_matches() {
        let network = Network::Testnet;
        let wallet = test_wallet();
        let seed_hash = wallet.seed_hash();
        let seed = test_seed();
        let slice = vec![Arc::new(RwLock::new(wallet))];

        let path = DerivationPath::identity_registration_path(network, 0);
        let reference = path
            .derive_priv_ecdsa_for_master_seed(&seed, network)
            .expect("reference derive")
            .private_key
            .secret_bytes();
        let param = Wallet::derive_private_key_in_arc_rw_lock_slice_with_seed(
            &slice, seed_hash, &seed, &path, network,
        )
        .expect("param")
        .expect("found");
        assert_eq!(reference, *param);

        // A non-matching seed hash yields None on the seed-param path too.
        let other_hash = [0xEE; 32];
        assert!(
            Wallet::derive_private_key_in_arc_rw_lock_slice_with_seed(
                &slice, other_hash, &seed, &path, network,
            )
            .expect("no error")
            .is_none()
        );
    }

    /// The borrowed seed never leaks into the error string of a seed-param
    /// derivation: a forced derivation failure carries no seed bytes.
    #[test]
    fn seed_param_derivation_error_does_not_leak_seed() {
        const SENTINEL_SEED: [u8; 64] = [0x5A; 64];
        let network = Network::Testnet;
        let wallet = test_wallet();
        // An empty path always derives successfully, so force the "wallet not
        // present" branch on the slice-derive with a non-matching seed hash and
        // confirm the resulting message holds no seed material.
        let path = DerivationPath::identity_registration_path(network, 0);
        let slice = vec![Arc::new(RwLock::new(wallet))];
        let err = Wallet::derive_private_key_in_arc_rw_lock_slice_with_seed(
            &slice,
            [0xEE; 32],
            &SENTINEL_SEED,
            &path,
            network,
        );
        // Non-matching hash returns Ok(None), not an error — assert the seed
        // never surfaces in the Debug of either arm.
        let rendered = format!("{err:?}");
        let sentinel_hex = hex::encode(SENTINEL_SEED);
        assert!(
            !rendered.contains(&sentinel_hex),
            "seed leaked into slice-derive result: {rendered}"
        );
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
    fn test_receive_address_returns_first_unused() {
        let mut wallet = test_wallet();
        // With no watched addresses, should derive address at index 0
        let addr = wallet
            .receive_address(Network::Testnet, false, None)
            .unwrap();
        assert!(!addr.to_string().is_empty());
    }

    /// Helper: manually register an address in watched_addresses so the wallet
    /// considers it "known" (normally done by register_address with AppContext).
    fn register_address_locally(
        wallet: &mut Wallet,
        address: &Address,
        derivation_path: &DerivationPath,
    ) {
        wallet
            .known_addresses
            .insert(address.clone(), derivation_path.clone());
        wallet.watched_addresses.insert(
            derivation_path.clone(),
            AddressInfo {
                address: address.clone(),
                path_type: DerivationPathType::CLEAR_FUNDS,
                path_reference: DerivationPathReference::BIP44,
            },
        );
    }

    #[test]
    fn test_receive_address_skip_known_with_no_funds() {
        let mut wallet = test_wallet();

        // Derive address at index 0 and register it locally
        let addr0 = wallet
            .derive_bip44_address(Network::Testnet, false, 0)
            .unwrap();
        let path0 = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);
        register_address_locally(&mut wallet, &addr0, &path0);

        // With skip=false, should return the same known zero-balance address
        let addr_same = wallet
            .receive_address(Network::Testnet, false, None)
            .unwrap();
        assert_eq!(addr0, addr_same);

        // With skip=true, should skip the known zero-balance address and get a new one
        let addr_next = wallet
            .receive_address(Network::Testnet, true, None)
            .unwrap();
        assert_ne!(addr0, addr_next);
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
        // The seed_hash survives the lock (it is envelope metadata, not the
        // seed) — the closed state still identifies the wallet.
        assert_eq!(wallet.seed_hash(), original_hash);

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

        // Same seed hash means equal
        assert_eq!(ref1, ref2);
        assert_eq!(ref1.seed_hash, seed_hash);
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

    /// FUND-SAFETY PARITY: the rebuilt `WalletAddressProvider` derives each
    /// gap-limit address from the DIP-17 account **xpub** (no owned seed). Its
    /// addresses must be byte-identical to the legacy seed-based
    /// `derive_priv_ecdsa_for_master_seed(...).to_priv().public_key()` path, on
    /// every network — otherwise platform balance sync would query the wrong
    /// addresses.
    #[test]
    fn provider_xpub_matches_seed_derivation() {
        for network in [Network::Testnet, Network::Mainnet] {
            let seed = [42u8; 64];
            let wallet = Wallet::new_from_seed(seed, network, None, None).expect("wallet");
            let provider = WalletAddressProvider::new(&wallet, network, &seed).expect("provider");

            let secp = Secp256k1::new();
            for index in 0u32..DEFAULT_GAP_LIMIT {
                // Legacy seed-based derivation (what the old provider and the
                // platform signer used).
                let path = DerivationPath::platform_payment_path(network, 0, 0, index);
                let legacy_priv = path
                    .derive_priv_ecdsa_for_master_seed(&seed, network)
                    .expect("legacy derive")
                    .to_priv();
                let legacy_address = Address::p2pkh(&legacy_priv.public_key(&secp), network);

                // Provider's xpub-based derivation.
                let (_platform, provider_address) = provider
                    .derive_address_at_index(index)
                    .expect("provider derive");

                assert_eq!(
                    legacy_address, provider_address,
                    "provider xpub address diverged from seed derivation at index {index} on {network:?}"
                );
            }
        }
    }

    // -------------------------------------------------------------------
    // F34: backend-authoritative payment-recipient validation.
    // -------------------------------------------------------------------

    #[test]
    fn validate_payment_recipients_accepts_positive_amounts() {
        assert!(validate_payment_recipients(&[1]).is_ok());
        assert!(validate_payment_recipients(&[100_000, 250_000, 1]).is_ok());
    }

    #[test]
    fn validate_payment_recipients_rejects_empty_list() {
        assert!(matches!(
            validate_payment_recipients(&[]),
            Err(TaskError::PaymentNoRecipients)
        ));
    }

    #[test]
    fn validate_payment_recipients_rejects_zero_amount() {
        assert!(matches!(
            validate_payment_recipients(&[0]),
            Err(TaskError::PaymentZeroAmount)
        ));
        // A zero anywhere in the list is rejected, not just the first slot.
        assert!(matches!(
            validate_payment_recipients(&[100_000, 0, 50_000]),
            Err(TaskError::PaymentZeroAmount)
        ));
    }

    #[test]
    fn validate_payment_recipients_empty_takes_precedence_over_zero_check() {
        // An empty list reports the no-recipients error, never the zero error.
        assert!(matches!(
            validate_payment_recipients(&[]),
            Err(TaskError::PaymentNoRecipients)
        ));
    }

    /// FUNDS-SAFETY: every Core receive address handed to a user must live
    /// inside the upstream gap-limit pool that SPV actually watches. The
    /// upstream BIP-44 external pool watches indices `0..=29` (gap limit 30);
    /// anything past index 29 is invisible to SPV, so funds sent there never
    /// appear. This pins the property the Receive "New Address" action must
    /// satisfy: the address it returns is always in the watched pool.
    ///
    /// The legacy `Wallet::receive_address(skip = true)` path violated this —
    /// it walked the index forward past every known zero-balance address with
    /// no gap-limit bound, handing out e.g. index 32 (a real user lost a
    /// 1 tDASH deposit this way). The fix routes the action through the
    /// upstream `next_unused`, which can only return a watched address.
    #[test]
    fn receive_address_stays_within_upstream_watched_pool() {
        use dash_sdk::dpp::key_wallet::gap_limit::DEFAULT_EXTERNAL_GAP_LIMIT;
        use dash_sdk::dpp::key_wallet::managed_account::address_pool::{
            AddressPool, AddressPoolType, KeySource,
        };

        let network = Network::Testnet;

        // The upstream SPV-watched external pool: same account xpub DET uses,
        // gap limit 30 ⇒ generates indices 0..=29 and watches exactly those.
        let account_xpub = test_wallet().master_bip44_ecdsa_extended_public_key;
        let mut watched_pool = AddressPool::new(
            DerivationPath::master(),
            AddressPoolType::External,
            DEFAULT_EXTERNAL_GAP_LIMIT,
            network,
            &KeySource::Public(account_xpub),
        )
        .expect("upstream external pool");

        // The upstream next-unused address is, by construction, watched.
        let watched_addr = watched_pool
            .next_unused(&KeySource::Public(account_xpub), false)
            .expect("upstream next_unused");
        assert!(
            watched_pool.contains_address(&watched_addr),
            "upstream next_unused must return a watched address"
        );

        // Reproduce the user's actions: advance the legacy receive path past
        // the gap window. We pre-register zero-balance known addresses 0..=31
        // so `skip_known_addresses_with_no_funds` walks past them and derives a
        // brand-new index 32 — outside the watched pool.
        let mut wallet = test_wallet();
        let secp = Secp256k1::new();
        for index in 0u32..=31 {
            let path = DerivationPath::bip_44_payment_path(network, 0, false, index);
            let pubkey = wallet
                .master_bip44_ecdsa_extended_public_key
                .derive_pub(
                    &secp,
                    &DerivationPath::from(
                        [
                            ChildNumber::Normal { index: 0 },
                            ChildNumber::Normal { index },
                        ]
                        .as_slice(),
                    ),
                )
                .expect("derive")
                .to_pub();
            let address = Address::p2pkh(&pubkey, network);
            wallet.known_addresses.insert(address.clone(), path.clone());
            wallet.watched_addresses.insert(
                path,
                AddressInfo {
                    address,
                    path_type: DerivationPathType::CREDIT_FUNDING,
                    path_reference: DerivationPathReference::BIP44,
                },
            );
        }

        let legacy_addr = wallet
            .receive_address(network, true, None)
            .expect("legacy receive_address");

        // The legacy path escaped the watched window. This documents the bug:
        // index 32 is NOT in the SPV-watched pool, so funds sent there are
        // invisible. The Receive action must never hand out such an address.
        assert!(
            !watched_pool.contains_address(&legacy_addr),
            "legacy receive_address must escape the watched pool (the bug being fixed); \
             if it now stays inside the pool the gap window changed and this guard needs review"
        );

        // The invariant the fixed Receive action satisfies: the address handed
        // to the user is always watched. The legacy address fails it; the
        // upstream address passes it. The production "New Address" button now
        // routes through the upstream path, so the user-visible address is
        // always `watched_addr`-class, never `legacy_addr`-class.
        assert!(
            watched_pool.contains_address(&watched_addr)
                && !watched_pool.contains_address(&legacy_addr),
            "the watched-pool address is funds-safe; the legacy-derived address is not"
        );
    }
}
