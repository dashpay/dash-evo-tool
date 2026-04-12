//! Bridge module for platform-wallet integration.
//!
//! Re-exports types from `platform-wallet` and provides conversion helpers
//! between old wallet types and new platform wallet types.
//!
//! # Migration plan for AppContext
//!
//! The current `AppContext` stores wallets as:
//! ```ignore
//! wallets: RwLock<BTreeMap<WalletId, Arc<RwLock<Wallet>>>>
//! ```
//! where `WalletId = [u8; 32]` is SHA-256 of the wallet seed.
//!
//! In the new platform-wallet model, the equivalent is:
//! - `WalletId = [u8; 32]` (SHA-256 of the root public key, from `key-wallet`)
//! - `PlatformWalletManager` manages multiple `PlatformWallet` instances
//!   internally via `RwLock<BTreeMap<WalletId, PlatformWallet>>`
//!
//! Key differences:
//! - `WalletId` is derived from the seed; `WalletId` is derived from
//!   the root public key. Both are `[u8; 32]` and serve as unique wallet identifiers.
//!   They will produce different values for the same wallet.
//! - The old model uses `Arc<RwLock<Wallet>>` for interior mutability;
//!   the new model uses `PlatformWallet` which is cheaply cloneable (all Arc fields).
//! - `PlatformWallet` subsumes the old `Wallet` by composing `CoreWallet`,
//!   `IdentityWallet`, `DashPayWallet`, and `PlatformAddressWallet`.

// ── Primary types ──────────────────────────────────────────────────────

pub use platform_wallet::IdentityManager;
pub use platform_wallet::ManagedIdentity;
pub use platform_wallet::PlatformEventHandler;
pub use platform_wallet::PlatformWallet;
pub use platform_wallet::PlatformWalletError;
pub use platform_wallet::PlatformWalletManager;

// ── Sub-wallet types ───────────────────────────────────────────────────

pub use platform_wallet::CoreWallet;
pub use platform_wallet::TokenWallet;
pub use platform_wallet::WalletBalance;
pub use platform_wallet::wallet::WalletId;

// ── Identity sub-types ────────────────────────────────────────────────

pub use platform_wallet::DpnsNameInfo as ManagedDpnsNameInfo;
pub use platform_wallet::IdentityFunding;
pub use platform_wallet::IdentityStatus as ManagedIdentityStatus;
pub use platform_wallet::KeyStorage as ManagedKeyStorage;
pub use platform_wallet::PrivateKeyData as ManagedPrivateKeyData;

// ── Supporting types ───────────────────────────────────────────────────

pub use platform_wallet::BlockTime;
pub use platform_wallet::ContactRequest;
pub use platform_wallet::EstablishedContact;

// ── Address info (moved from platform-wallet — UI-only type) ─────────

use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;

/// Per-address info for UI consumption.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreAddressInfo {
    /// The address itself.
    pub address: Address,
    /// Full HD derivation path for this address.
    pub derivation_path: DerivationPath,
    /// Current balance held at this address (in satoshis).
    pub balance: u64,
    /// Total amount ever received by this address (in satoshis).
    pub total_received: u64,
    /// Number of UTXOs currently held at this address.
    pub utxo_count: usize,
    /// Whether this address has ever been used in a transaction.
    pub is_used: bool,
    /// Index within its address pool.
    pub index: u32,
    /// Account index this address belongs to, if applicable.
    pub account_index: Option<u32>,
}

impl CoreAddressInfo {
    /// Build a `CoreAddressInfo` list for every address across all accounts.
    pub fn all_from_wallet_info(info: &ManagedWalletInfo) -> Vec<Self> {
        let mut result = Vec::new();

        for account in info.accounts.all_accounts() {
            let account_index = account.index();

            let mut utxo_counts: std::collections::BTreeMap<Address, usize> =
                std::collections::BTreeMap::new();
            for utxo in account.utxos.values() {
                *utxo_counts.entry(utxo.address.clone()).or_default() += 1;
            }

            for pool in account.account_type.address_pools() {
                for addr_info in pool.addresses.values() {
                    result.push(CoreAddressInfo {
                        address: addr_info.address.clone(),
                        derivation_path: addr_info.path.clone(),
                        balance: addr_info.balance,
                        total_received: addr_info.total_received,
                        utxo_count: utxo_counts.get(&addr_info.address).copied().unwrap_or(0),
                        is_used: addr_info.used,
                        index: addr_info.index,
                        account_index,
                    });
                }
            }
        }

        result
    }
}

