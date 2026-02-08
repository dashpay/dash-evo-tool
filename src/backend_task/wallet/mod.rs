mod fetch_platform_address_balances;
mod fund_platform_address_from_asset_lock;
mod fund_platform_address_from_wallet_utxos;
mod generate_receive_address;
mod transfer_platform_credits;
mod withdraw_from_platform_address;

use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::dashcore::address::NetworkChecked;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::prelude::AssetLockProof;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum WalletResult {
    Payment {
        txid: String,
        /// List of (address, amount) pairs for each recipient
        recipients: Vec<(String, u64)>,
        total_amount: u64,
    },
    Refreshed {
        /// Optional warning message (e.g., Platform sync failed but Core refresh succeeded)
        warning: Option<String>,
    },
    RecoveredAssetLocks {
        recovered_count: usize,
        total_amount: u64,
    },
    GeneratedReceiveAddress {
        seed_hash: WalletSeedHash,
        address: String,
    },
    /// Platform address balances fetched from Platform
    PlatformAddressBalances {
        seed_hash: WalletSeedHash,
        /// Map of address to (balance, nonce)
        balances: BTreeMap<Address<NetworkChecked>, (u64, u32)>,
    },
    /// Platform credits transferred between addresses
    PlatformCreditsTransferred { seed_hash: WalletSeedHash },
    /// Platform address funded from asset lock
    PlatformAddressFunded { seed_hash: WalletSeedHash },
    /// Withdrawal from Platform address to Core initiated
    PlatformAddressWithdrawal { seed_hash: WalletSeedHash },
}

/// Controls how Platform address balance sync is performed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlatformSyncMode {
    /// Automatically decide based on time since last full sync
    #[default]
    Auto,
    /// Force a full sync (queries all addresses)
    ForceFull,
    /// Only do terminal sync using stored checkpoint (fails if no checkpoint exists)
    TerminalOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WalletTask {
    GenerateReceiveAddress {
        seed_hash: WalletSeedHash,
    },
    /// Fetch Platform address balances and nonces from Platform for a wallet
    FetchPlatformAddressBalances {
        seed_hash: WalletSeedHash,
        sync_mode: PlatformSyncMode,
    },
    /// Transfer credits between Platform addresses
    TransferPlatformCredits {
        seed_hash: WalletSeedHash,
        /// Source addresses with amounts to transfer
        inputs: BTreeMap<PlatformAddress, Credits>,
        /// Destination addresses with amounts
        outputs: BTreeMap<PlatformAddress, Credits>,
        /// Index of the input to deduct fees from (in BTreeMap order).
        /// Should be the input with the highest balance to ensure sufficient funds for fees.
        fee_payer_index: u16,
    },
    /// Fund Platform addresses from an asset lock
    FundPlatformAddressFromAssetLock {
        seed_hash: WalletSeedHash,
        /// Asset lock proof
        asset_lock_proof: Box<AssetLockProof>,
        /// Address to fund (the asset lock address is the source)
        asset_lock_address: Address,
        /// Platform addresses and optional amounts to fund (None = distribute evenly)
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    },
    /// Withdraw from Platform addresses to Core
    WithdrawFromPlatformAddress {
        seed_hash: WalletSeedHash,
        /// Platform addresses and amounts to withdraw
        inputs: BTreeMap<PlatformAddress, Credits>,
        /// Core script to receive the withdrawal (e.g., P2PKH script)
        output_script: CoreScript,
        /// Core fee per byte
        core_fee_per_byte: u32,
        /// Index of the input to deduct fees from (in BTreeMap order).
        fee_payer_index: u16,
    },
    /// Fund a platform address directly from wallet UTXOs
    /// Creates asset lock, broadcasts, waits for proof, then funds platform address
    FundPlatformAddressFromWalletUtxos {
        seed_hash: WalletSeedHash,
        /// Amount in duffs to lock
        amount: u64,
        /// Destination platform address to fund
        destination: PlatformAddress,
        /// If true, fees are deducted from the output amount (recipient receives less).
        /// If false, fees are paid from extra wallet balance (recipient receives exact amount).
        fee_deduct_from_output: bool,
    },
}
