//! Wallet-related DTOs.
//!
//! Replaces `Arc<RwLock<Wallet>>` and `Arc<RwLock<SingleKeyWallet>>` with
//! serializable, owned types. Wallets are referenced by their seed hash
//! across the IPC boundary.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::common::{CreditsDto, NetworkDto, WalletSeedHashDto};

/// Serializable summary of an HD wallet, suitable for list views.
/// Replaces `Wallet` for IPC. Does NOT include private key material.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WalletDto {
    /// SHA-256 hex hash of the wallet seed (unique identifier).
    pub seed_hash: WalletSeedHashDto,
    /// Whether this wallet requires a password to unlock.
    pub uses_password: bool,
    /// User-assigned alias (e.g., "My Main Wallet").
    pub alias: Option<String>,
    /// Whether this is the main wallet.
    pub is_main: bool,
    /// Confirmed balance in duffs.
    pub confirmed_balance: u64,
    /// Unconfirmed balance in duffs.
    pub unconfirmed_balance: u64,
    /// Total balance in duffs (confirmed + unconfirmed).
    pub total_balance: u64,
    /// Known addresses with their string representation and balance.
    pub addresses: Vec<WalletAddressDto>,
    /// Transactions associated with this wallet.
    pub transactions: Vec<WalletTransactionDto>,
    /// Unused asset locks available for identity creation/top-up.
    pub unused_asset_locks: Vec<AssetLockDto>,
    /// Platform address info (DIP-17).
    pub platform_addresses: Vec<PlatformAddressDto>,
    /// Identity indexes registered from this wallet.
    pub identity_indexes: Vec<u32>,
    /// Password hint (if set).
    pub password_hint: Option<String>,
}

/// An address within an HD wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WalletAddressDto {
    /// The address string (e.g., "XpYv3N...").
    pub address: String,
    /// Current balance in duffs.
    pub balance: u64,
    /// Total received over lifetime in duffs.
    pub total_received: u64,
    /// Derivation path as string (e.g., "m/44'/5'/0'/0/0").
    pub derivation_path: String,
}

/// A wallet transaction.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WalletTransactionDto {
    /// Transaction ID as hex string.
    pub txid: String,
    /// Unix timestamp.
    pub timestamp: u64,
    /// Block height (None if unconfirmed).
    pub height: Option<u32>,
    /// Block hash as hex string (None if unconfirmed).
    pub block_hash: Option<String>,
    /// Net amount change in duffs (positive = incoming, negative = outgoing).
    pub net_amount: i64,
    /// Transaction fee in duffs (if known).
    pub fee: Option<u64>,
    /// User-assigned label.
    pub label: Option<String>,
    /// Whether all inputs are ours.
    pub is_ours: bool,
}

/// An unused asset lock available for identity operations.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssetLockDto {
    /// Transaction ID as hex string.
    pub txid: String,
    /// Address holding the locked funds.
    pub address: String,
    /// Amount locked in credits.
    pub amount: CreditsDto,
    /// Whether an instant lock has been received.
    pub has_instant_lock: bool,
    /// Whether an asset lock proof has been generated.
    pub has_asset_lock_proof: bool,
    /// Proof details (None when no proof is available yet).
    pub proof_details: Option<AssetLockProofDetailsDto>,
    /// Serialized proof as hex (JSON bytes → hex encoded). None when no proof.
    pub proof_hex: Option<String>,
}

/// Detailed proof information for an asset lock.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AssetLockProofDetailsDto {
    /// Instant Send proof.
    #[serde(rename_all = "camelCase")]
    InstantSend {
        /// The InstantLock's transaction ID.
        instant_lock_txid: String,
        /// The output index in the transaction.
        output_index: u32,
    },
    /// Chain Lock proof.
    #[serde(rename_all = "camelCase")]
    ChainLock {
        /// The height at which the Core chain was locked.
        core_chain_locked_height: u32,
        /// The outpoint transaction ID.
        out_point_txid: String,
        /// The outpoint output index.
        out_point_vout: u32,
    },
}

/// Platform address info (DIP-17).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlatformAddressDto {
    /// The Core address string.
    pub address: String,
    /// Balance in credits.
    pub balance: CreditsDto,
    /// Current nonce.
    pub nonce: u64,
}

/// Serializable summary of a single-key wallet.
/// Replaces `SingleKeyWallet` for IPC. Does NOT include private key material.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SingleKeyWalletDto {
    /// SHA-256 hex hash of the private key (unique identifier).
    pub key_hash: String,
    /// Whether this wallet requires a password to unlock.
    pub uses_password: bool,
    /// The public key as hex string.
    pub public_key: String,
    /// The P2PKH address string.
    pub address: String,
    /// User-assigned alias.
    pub alias: Option<String>,
    /// Confirmed balance in duffs.
    pub confirmed_balance: u64,
    /// Unconfirmed balance in duffs.
    pub unconfirmed_balance: u64,
    /// Total balance in duffs.
    pub total_balance: u64,
    /// Number of UTXOs.
    pub utxo_count: usize,
    /// The UTXOs themselves.
    pub utxos: Vec<UtxoDto>,
}

/// A UTXO (unspent transaction output).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UtxoDto {
    /// Transaction ID as hex string.
    pub txid: String,
    /// Output index within the transaction.
    pub vout: u32,
    /// Amount in duffs.
    pub amount: u64,
}

/// Unified wallet reference — either HD or single-key.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum WalletRefDto {
    /// HD wallet, referenced by seed hash.
    #[serde(rename_all = "camelCase")]
    Hd { seed_hash: WalletSeedHashDto },
    /// Single-key wallet, referenced by key hash.
    #[serde(rename_all = "camelCase")]
    SingleKey { key_hash: String },
}

/// Summary of all wallets for the wallet list screen.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WalletListDto {
    pub hd_wallets: Vec<WalletDto>,
    pub single_key_wallets: Vec<SingleKeyWalletDto>,
    /// Which wallet is currently selected (if any).
    pub selected: Option<WalletRefDto>,
}

/// Result of a wallet payment operation.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WalletPaymentResultDto {
    /// Transaction ID as hex string.
    pub txid: String,
    /// List of (address, amount) pairs.
    pub recipients: Vec<PaymentRecipientDto>,
    /// Total amount sent in duffs.
    pub total_amount: u64,
}

/// A payment recipient.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRecipientDto {
    pub address: String,
    pub amount: u64,
}

/// Result of recovering asset locks.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredAssetLocksDto {
    pub recovered_count: usize,
    pub total_amount: u64,
}

/// Response from generating a new receive address.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenerateReceiveAddressResponseDto {
    pub address: String,
}

/// Network identifier for the wallet context.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WalletNetworkInfoDto {
    pub network: NetworkDto,
    /// Number of HD wallets loaded.
    pub hd_wallet_count: usize,
    /// Number of single-key wallets loaded.
    pub single_key_wallet_count: usize,
}
