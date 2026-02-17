pub mod bundle;
pub mod nullifiers;
pub mod sync;

use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;

#[derive(Debug, Clone, PartialEq)]
pub enum ShieldedTask {
    /// Initialize shielded state for a wallet (ZIP32 key derivation, load tree from DB)
    InitializeShieldedWallet { seed_hash: WalletSeedHash },

    /// Sync encrypted notes from platform (trial decrypt, update tree)
    SyncNotes { seed_hash: WalletSeedHash },

    /// Shield credits from platform address into the shielded pool (Type 15)
    ShieldCredits {
        seed_hash: WalletSeedHash,
        amount: u64,
        from_address: PlatformAddress,
    },

    /// Private transfer within the shielded pool (Type 16)
    ShieldedTransfer {
        seed_hash: WalletSeedHash,
        amount: u64,
        /// Serialized Orchard PaymentAddress bytes
        recipient_address_bytes: Vec<u8>,
    },

    /// Unshield credits to a platform address (Type 17)
    UnshieldCredits {
        seed_hash: WalletSeedHash,
        amount: u64,
        to_platform_address: PlatformAddress,
    },

    /// Check nullifiers to detect spent notes
    CheckNullifiers { seed_hash: WalletSeedHash },

    /// Warm up the proving key in background (~30s)
    WarmUpProvingKey,
}
