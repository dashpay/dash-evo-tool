mod derive_key_for_display;
mod fetch_platform_address_balances;
mod fund_platform_address_from_asset_lock;
mod fund_platform_address_from_wallet_utxos;
mod generate_receive_address;
mod transfer_platform_credits;
mod withdraw_from_platform_address;

use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum WalletTask {
    GenerateReceiveAddress {
        seed_hash: WalletSeedHash,
    },
    /// Derive a private key for on-screen display/export. The HD seed is
    /// fetched just-in-time through the JIT chokepoint, the key is derived in
    /// the backend, and only the WIF (wrapped in `Secret`) is returned — the
    /// seed never crosses into the UI layer.
    DeriveKeyForDisplay {
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
    },
    /// Fetch Platform address balances and nonces from Platform for a wallet
    FetchPlatformAddressBalances {
        seed_hash: WalletSeedHash,
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
    /// Fund Platform addresses from a tracked asset lock identified by its
    /// credit-output outpoint. The proof and credit-output key are recovered
    /// from the upstream `AssetLockManager` and the wallet's funding
    /// account; DET no longer stages the asset lock itself.
    FundPlatformAddressFromAssetLock {
        seed_hash: WalletSeedHash,
        /// Credit-output outpoint of the tracked asset lock.
        out_point: OutPoint,
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
