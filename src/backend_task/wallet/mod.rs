mod fetch_platform_address_balances;
mod fund_platform_address_from_asset_lock;
mod fund_platform_address_from_wallet_utxos;
mod generate_receive_address;
mod transfer_platform_credits;
mod withdraw_from_platform_address;

use crate::model::wallet::WalletId;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::prelude::AssetLockProof;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum WalletTask {
    GenerateReceiveAddress {
        wallet_id: WalletId,
    },
    /// Fetch Platform address balances and nonces from Platform for a wallet
    FetchPlatformAddressBalances {
        wallet_id: WalletId,
    },
    /// Transfer credits between Platform addresses
    TransferPlatformCredits {
        wallet_id: WalletId,
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
        wallet_id: WalletId,
        /// Asset lock proof
        asset_lock_proof: Box<AssetLockProof>,
        /// Address to fund (the asset lock address is the source)
        asset_lock_address: Address,
        /// Platform addresses and optional amounts to fund (None = distribute evenly)
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    },
    /// Withdraw from Platform addresses to Core
    WithdrawFromPlatformAddress {
        wallet_id: WalletId,
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
        wallet_id: WalletId,
        /// Amount in duffs to lock
        amount: u64,
        /// Destination platform address to fund
        destination: PlatformAddress,
        /// If true, fees are deducted from the output amount (recipient receives less).
        /// If false, fees are paid from extra wallet balance (recipient receives exact amount).
        fee_deduct_from_output: bool,
    },
    /// Load per-address info from the CoreWallet via the platform-wallet bridge
    LoadAddressInfo {
        wallet_id: WalletId,
    },
}
