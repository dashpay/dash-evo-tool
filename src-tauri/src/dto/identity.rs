//! Identity-related DTOs.
//!
//! Replaces `QualifiedIdentity`, `Identity`, `IdentityPublicKey` and related
//! SDK types with serializable equivalents.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::common::{CreditsDto, IdentifierDto, NetworkDto, WalletSeedHashDto};

/// Type of identity (User, Masternode, Evonode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum IdentityTypeDto {
    User,
    Masternode,
    Evonode,
}

/// Status of an identity on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum IdentityStatusDto {
    Unknown,
    PendingCreation,
    Active,
    NotFound,
    FailedCreation,
}

/// DPNS name info attached to an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DpnsNameInfoDto {
    /// The registered DPNS name (e.g., "alice.dash").
    pub name: String,
    /// Timestamp when the name was acquired.
    pub acquired_at: u64,
}

/// Contract bounds for a key.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ContractBoundsDto {
    /// Key is bound to a single contract.
    #[serde(rename_all = "camelCase")]
    SingleContract {
        /// Contract identifier (hex).
        contract_id: IdentifierDto,
    },
    /// Key is bound to a single contract and document type.
    #[serde(rename_all = "camelCase")]
    SingleContractDocumentType {
        /// Contract identifier (hex).
        contract_id: IdentifierDto,
        /// Document type name.
        document_type_name: String,
    },
}

/// A public key on an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentityKeyDto {
    /// Key ID within the identity.
    pub key_id: u32,
    /// Key type (ECDSA_SECP256K1, BLS12_381, ECDSA_HASH160, etc.).
    pub key_type: String,
    /// Key purpose (AUTHENTICATION, VOTING, TRANSFER, OWNER, etc.).
    pub purpose: String,
    /// Security level (MASTER, CRITICAL, HIGH, MEDIUM, etc.).
    pub security_level: String,
    /// Public key data as hex string.
    pub data: String,
    /// Whether this key is disabled.
    pub is_disabled: bool,
    /// Disabled at timestamp (if disabled).
    pub disabled_at: Option<u64>,
    /// Whether we have the corresponding private key in local storage.
    pub has_private_key: bool,
    /// Optional contract bounds restricting this key to a specific contract.
    pub contract_bounds: Option<ContractBoundsDto>,
}

/// Serializable version of `QualifiedIdentity`.
/// This is the primary identity DTO sent to the frontend for display and selection.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QualifiedIdentityDto {
    /// Platform identifier as hex string.
    pub id: IdentifierDto,
    /// Identity type (User, Masternode, Evonode).
    pub identity_type: IdentityTypeDto,
    /// User-assigned alias.
    pub alias: Option<String>,
    /// Current balance in credits.
    pub balance: CreditsDto,
    /// Public keys on this identity.
    pub keys: Vec<IdentityKeyDto>,
    /// DPNS names registered for this identity.
    pub dpns_names: Vec<DpnsNameInfoDto>,
    /// Seed hashes of associated wallets (hex-encoded).
    pub associated_wallet_hashes: Vec<WalletSeedHashDto>,
    /// The wallet index used during registration (if applicable).
    pub wallet_index: Option<u32>,
    /// Top-up history: map of index → amount.
    pub top_ups: Vec<TopUpEntryDto>,
    /// Current status on the platform.
    pub status: IdentityStatusDto,
    /// Network this identity belongs to.
    pub network: NetworkDto,
    /// Associated voter identity ID (if any, for masternodes/evonodes).
    pub voter_identity_id: Option<IdentifierDto>,
    /// Associated operator identity ID (if any).
    pub operator_identity_id: Option<IdentifierDto>,
    /// Masternode payout address (if applicable, for masternodes/evonodes).
    pub masternode_payout_address: Option<String>,
}

/// A top-up history entry.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopUpEntryDto {
    pub index: u32,
    pub amount: u64,
}

/// Minimal identity reference for dropdowns and selectors.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySummaryDto {
    /// Platform identifier as hex string.
    pub id: IdentifierDto,
    /// Display name: alias or truncated ID.
    pub display_name: String,
    /// Identity type.
    pub identity_type: IdentityTypeDto,
    /// Current balance in credits.
    pub balance: CreditsDto,
}
