use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::prelude::AssetLockProof;
use std::collections::BTreeMap;

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
    },
    /// Fund a platform address directly from wallet UTXOs
    /// Creates asset lock, broadcasts, waits for proof, then funds platform address
    FundPlatformAddressFromWalletUtxos {
        seed_hash: WalletSeedHash,
        /// Amount in duffs to lock
        amount: u64,
        /// Destination platform address to fund
        destination: PlatformAddress,
    },
    /// Fund multiple Platform addresses at once (for testing/benchmarking)
    /// Derives N new Platform addresses and funds each with the specified amount
    FundMultiplePlatformAddresses {
        seed_hash: WalletSeedHash,
        /// Number of addresses to create and fund
        count: u32,
        /// Amount in duffs per address (total locked = count * amount_per_address)
        amount_per_address: u64,
    },
}
