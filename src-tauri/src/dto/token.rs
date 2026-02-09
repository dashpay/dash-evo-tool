//! Token-related DTOs.
//!
//! Replaces `TokenInfo`, `IdentityTokenBalance`, `TokenConfiguration`,
//! and related SDK types.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::common::{IdentifierDto, TokenAmountDto};
use super::fee::FeeResultDto;

/// Serializable version of `TokenInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfoDto {
    /// Token identifier as hex string.
    pub token_id: IdentifierDto,
    /// Token display name.
    pub token_name: String,
    /// Parent data contract identifier as hex string.
    pub data_contract_id: IdentifierDto,
    /// Position within the data contract's token array.
    pub token_position: u16,
    /// Token description (if set).
    pub description: Option<String>,
    /// Token configuration details.
    pub configuration: TokenConfigurationDto,
}

/// Serializable token configuration (subset of SDK's TokenConfiguration).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenConfigurationDto {
    /// Number of decimal places.
    pub decimals: u8,
    /// Maximum supply (None = unlimited). Encoded as string for u128.
    pub max_supply: Option<TokenAmountDto>,
    /// Whether the token is currently paused.
    pub is_paused: bool,
    /// Whether the token keeps history.
    pub keeps_history: bool,
    /// Whether transfers to frozen identities are allowed.
    pub allows_transfers_to_frozen_identities: bool,
    /// Base supply (initial mint amount). Encoded as string for u128.
    pub base_supply: TokenAmountDto,
    /// Token conventions (name, symbol, etc.) as JSON.
    pub conventions: serde_json::Value,
}

/// Token balance for an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentityTokenBalanceDto {
    /// Token identifier as hex string.
    pub token_id: IdentifierDto,
    /// Token display name or alias.
    pub token_alias: String,
    /// Identity that owns the tokens.
    pub identity_id: IdentifierDto,
    /// Current balance encoded as string (u128).
    pub balance: TokenAmountDto,
    /// Estimated unclaimed distribution rewards (if applicable).
    pub estimated_unclaimed_rewards: Option<TokenAmountDto>,
    /// Parent data contract identifier.
    pub data_contract_id: IdentifierDto,
    /// Token position in the contract.
    pub token_position: u16,
    /// Decimals for display formatting.
    pub decimals: u8,
    /// Available actions for this identity on this token.
    pub available_actions: IdentityTokenAvailableActionsDto,
}

/// Available token actions for an identity (determines which buttons to show).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentityTokenAvailableActionsDto {
    pub can_claim: bool,
    pub can_estimate: bool,
    pub can_mint: bool,
    pub can_burn: bool,
    pub can_freeze: bool,
    pub can_unfreeze: bool,
    pub can_destroy: bool,
    pub can_do_emergency_action: bool,
    pub can_maybe_purchase: bool,
    pub can_set_price: bool,
    pub can_transfer: bool,
    pub can_update_config: bool,
}

/// Token identifier combining identity and token IDs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentityTokenIdentifierDto {
    pub identity_id: IdentifierDto,
    pub token_id: IdentifierDto,
}

/// Token result for operations that return a fee result.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenOperationResultDto {
    /// Type of operation that was performed.
    pub operation: String,
    /// Fee information.
    pub fee_result: FeeResultDto,
}

/// Token pricing information.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenPricingDto {
    /// Token identifier.
    pub token_id: IdentifierDto,
    /// Pricing schedule as JSON (complex SDK type).
    pub pricing_schedule: Option<serde_json::Value>,
}

/// Distribution reward estimation result.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DistributionRewardEstimationDto {
    /// The identity-token pair.
    pub identity_token: IdentityTokenIdentifierDto,
    /// Estimated reward amount.
    pub estimated_amount: TokenAmountDto,
    /// Explanation of the interval evaluation.
    pub explanation: serde_json::Value,
}
