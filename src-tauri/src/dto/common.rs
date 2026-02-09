//! Common DTO types shared across domains.

use dash_sdk::dpp::dashcore::Network;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Network identifier matching Dash SDK's Network enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NetworkDto {
    Dash,
    Testnet,
    Devnet,
    Regtest,
}

impl NetworkDto {
    /// Convert from the Dash SDK `Network` enum.
    pub fn from_network(network: Network) -> Self {
        match network {
            Network::Dash => Self::Dash,
            Network::Testnet => Self::Testnet,
            Network::Devnet => Self::Devnet,
            Network::Regtest => Self::Regtest,
            _ => Self::Dash, // Fallback for any future variants
        }
    }

    /// Convert to the Dash SDK `Network` enum.
    pub fn to_network(self) -> Network {
        match self {
            Self::Dash => Network::Dash,
            Self::Testnet => Network::Testnet,
            Self::Devnet => Network::Devnet,
            Self::Regtest => Network::Regtest,
        }
    }
}

/// A key-value pair for displaying structured data.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyValuePair {
    pub key: String,
    pub value: String,
}

/// Identifier represented as a hex string (32-byte platform identifier).
/// On the Rust side, `dash_sdk::platform::Identifier` is a 32-byte array.
/// For JSON/IPC we encode it as a hex string.
pub type IdentifierDto = String;

/// A wallet seed hash (SHA-256 of the wallet seed), hex-encoded.
/// On the Rust side this is `[u8; 32]`.
pub type WalletSeedHashDto = String;

/// A single-key wallet hash (SHA-256 of the private key), hex-encoded.
/// On the Rust side this is `[u8; 32]`.
pub type SingleKeyHashDto = String;

/// Credits (u64 on the Rust side, represented as number in TypeScript).
/// tauri-specta maps u64 to number with BigIntExportBehavior::Number.
pub type CreditsDto = u64;

/// Token amount (u128 on the Rust side).
/// Since u128 is not directly supported by JSON/specta, we encode as a string.
pub type TokenAmountDto = String;
