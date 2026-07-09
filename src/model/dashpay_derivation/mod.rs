//! Pure DashPay HD key derivation (DIP-14 / DIP-15).
//!
//! Stateless derivation of contact payment keys and auto-accept proof keys
//! from a wallet seed. No `AppContext`, `Sdk`, database, or `BackendTask`
//! dependency — the single source of truth for DashPay derivation math.

pub mod dip14;
pub mod hd;

pub use hd::{
    calculate_account_reference, derive_auto_accept_key, derive_dashpay_incoming_xpub,
    derive_payment_address, generate_contact_xpub_data,
};

use thiserror::Error;

/// Failure while deriving a DashPay HD key.
#[derive(Debug, Error)]
pub enum DerivationError {
    /// A BIP-32 extended-key operation failed (master creation, path parsing,
    /// or child derivation).
    #[error("The wallet keys could not be derived. The wallet data may be invalid.")]
    Bip32(#[from] dash_sdk::dpp::key_wallet::bip32::Error),

    /// A secp256k1 operation on derived key material failed.
    #[error("The wallet key material could not be processed. The wallet data may be invalid.")]
    Secp256k1(#[from] dash_sdk::dpp::dashcore::secp256k1::Error),
}
