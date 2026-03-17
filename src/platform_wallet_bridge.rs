//! Bridge module for platform-wallet integration.
//!
//! Re-exports types from `platform-wallet` and provides conversion helpers
//! between old wallet types and new platform wallet types.
//!
//! # Migration plan for AppContext
//!
//! The current `AppContext` stores wallets as:
//! ```ignore
//! wallets: RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>>
//! ```
//! where `WalletSeedHash = [u8; 32]` is SHA-256 of the wallet seed.
//!
//! In the new platform-wallet model, the equivalent is:
//! - `WalletId = [u8; 32]` (SHA-256 of the root public key, from `key-wallet`)
//! - `PlatformWalletManager` manages multiple `PlatformWallet` instances
//!   internally via `RwLock<BTreeMap<WalletId, PlatformWallet>>`
//! - External code accesses wallets through `WalletHandle` (cheap cloneable token)
//!
//! Key differences:
//! - `WalletSeedHash` is derived from the seed; `WalletId` is derived from
//!   the root public key. Both are `[u8; 32]` and serve as unique wallet identifiers.
//!   They will produce different values for the same wallet.
//! - The old model uses `Arc<RwLock<Wallet>>` for interior mutability;
//!   the new model uses `WalletHandle` which wraps an `Arc` internally.
//! - `PlatformWallet` subsumes the old `Wallet` by composing `CoreWallet`,
//!   `IdentityWallet`, `DashPayWallet`, and `PlatformAddressWallet`.
//!
//! ## Usage sites for `app_context.wallets`
//!
//! **context layer** (6 files):
//! - `context/mod.rs` (line 72): field declaration and DB loading at startup
//! - `context/wallet_lifecycle.rs`: add, remove, bootstrap wallets; 6 access sites
//! - `context/transaction_processing.rs`: read wallets for tx matching; 2 access sites
//! - `context/identity_db.rs`: read wallets for identity-to-wallet mapping; 5 access sites
//! - `context/contract_token_db.rs`: write-lock to update token balances on wallets
//!
//! **backend_task layer** (11 files):
//! - `backend_task/wallet/`: 6 files (transfer, receive-addr, fetch balances,
//!   fund-from-asset-lock, fund-from-utxos, withdraw)
//! - `backend_task/identity/`: 4 files (discover, load, load-by-dpns, register)
//! - `backend_task/core/mod.rs`: read wallets for chain-lock processing
//!
//! **UI layer** (8 files):
//! - `ui/wallets/wallets_screen/mod.rs`: wallet list, selection, display; ~10 access sites
//! - `ui/wallets/add_new_wallet_screen.rs`: wallet creation + insertion
//! - `ui/wallets/import_mnemonic_screen.rs`: wallet import + insertion
//! - `ui/wallets/send_screen.rs`: wallet selection for sending
//! - `ui/identities/`: 4 files (identities_screen, add_existing, add_new, top_up)
//! - `ui/tools/grovestark_screen.rs`: read wallets for grovestark
//! - `ui/network_chooser_screen.rs`: read wallets for network switching
//!
//! **other** (3 files):
//! - `spv/manager.rs`: `WalletSeedHash` for SPV wallet tracking
//! - `database/identities.rs`: `WalletSeedHash` in DB queries
//! - `database/tokens.rs`: read wallets for token balance display
//!
//! `WalletSeedHash` is used in 27 files total.
//!
//! ## Migration strategy (future PRs)
//!
//! The replacement of `wallets: RwLock<BTreeMap<WalletSeedHash, ...>>` with
//! `PlatformWalletManager` is deferred to follow-up PRs once the full
//! platform-wallet API surface is stable. The migration will need to:
//!
//! 1. Replace `WalletSeedHash` with `WalletId` as the wallet map key
//! 2. Replace `Arc<RwLock<Wallet>>` with `WalletHandle`
//! 3. Replace direct `Wallet` method calls with `PlatformWallet` sub-wallet calls
//!    (e.g., `wallet.balance()` -> `platform_wallet.core().balance()`)
//! 4. Update DB schema to store `WalletId` instead of `WalletSeedHash`
//! 5. Update `single_key_wallets` field similarly

// ── Primary types ──────────────────────────────────────────────────────

pub use platform_wallet::PlatformWallet;
pub use platform_wallet::PlatformWalletManager;
pub use platform_wallet::WalletHandle;
pub use platform_wallet::PlatformWalletError;
pub use platform_wallet::PlatformWalletEvent;
pub use platform_wallet::IdentityManager;
pub use platform_wallet::ManagedIdentity;
pub use platform_wallet::PlatformWalletInfo;

// ── Sub-wallet types ───────────────────────────────────────────────────

pub use platform_wallet::CoreWallet;
pub use platform_wallet::wallet::WalletId;

// ── Supporting types ───────────────────────────────────────────────────

pub use platform_wallet::BlockTime;
pub use platform_wallet::ContactRequest;
pub use platform_wallet::EstablishedContact;
