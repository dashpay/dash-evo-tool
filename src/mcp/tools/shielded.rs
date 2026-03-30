//! Shielded-related MCP tools: shield, transfer, unshield, withdraw.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::DashMcpService;

// ---------------------------------------------------------------------------
// ShieldedShieldFromCore (Core -> Shielded via asset lock)
// ---------------------------------------------------------------------------

/// Shield DASH from Core wallet into the shielded pool via asset lock.
pub struct ShieldedShieldFromCore;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ShieldFromCoreParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Amount in duffs to shield (1 DASH = 100,000,000 duffs)
    pub amount_duffs: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldFromCoreOutput {
    amount_duffs: u64,
    shielded_credits: u64,
}

impl ToolBase for ShieldedShieldFromCore {
    type Parameter = ShieldFromCoreParams;
    type Output = ShieldFromCoreOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_shield_from_core".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Shield DASH from Core wallet into the shielded pool. \
             Creates an asset lock, waits for proof (~30s), then shields. \
             Amount is in duffs. The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedShieldFromCore {
    async fn invoke(
        service: &DashMcpService,
        param: ShieldFromCoreParams,
    ) -> Result<ShieldFromCoreOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_amount(param.amount_duffs)?;

        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;
        resolve::ensure_spv_synced(&ctx).await?;

        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
            seed_hash,
            amount_duffs: param.amount_duffs,
            source_address: None,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ShieldedFromAssetLock { amount, .. } => {
                Ok(ShieldFromCoreOutput {
                    amount_duffs: param.amount_duffs,
                    shielded_credits: amount,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ShieldedShieldFromPlatform (Platform -> Shielded)
// ---------------------------------------------------------------------------

/// Shield credits from a Platform address into the shielded pool.
pub struct ShieldedShieldFromPlatform;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ShieldFromPlatformParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Amount in credits to shield
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldFromPlatformOutput {
    amount_credits: u64,
}

impl ToolBase for ShieldedShieldFromPlatform {
    type Parameter = ShieldFromPlatformParams;
    type Output = ShieldFromPlatformOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_shield_from_platform".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Shield credits from a Platform address into the shielded pool. \
             Auto-selects the highest-balance Platform address. \
             The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedShieldFromPlatform {
    async fn invoke(
        service: &DashMcpService,
        param: ShieldFromPlatformParams,
    ) -> Result<ShieldFromPlatformOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;

        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // Auto-select highest-balance platform address and verify sufficient balance
        let from_address = {
            let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
            let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
            let best = wallet
                .platform_address_info
                .iter()
                .filter_map(|(addr, info)| {
                    if info.balance > 0 {
                        dash_sdk::dpp::address_funds::PlatformAddress::try_from(addr.clone())
                            .ok()
                            .map(|pa| (pa, info.balance))
                    } else {
                        None
                    }
                })
                .max_by_key(|(_, balance)| *balance)
                .ok_or_else(|| McpToolError::InvalidParam {
                    message: "No Platform addresses with balance found".to_owned(),
                })?;

            if best.1 < param.amount_credits {
                return Err(McpToolError::InvalidParam {
                    message: format!(
                        "Insufficient platform balance. Highest address has {} credits but {} required.",
                        best.1, param.amount_credits
                    ),
                });
            }

            best.0
        };

        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldCredits {
            seed_hash,
            amount: param.amount_credits,
            from_address,
            nonce_override: None,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ShieldedCreditsShielded { .. } => {
                Ok(ShieldFromPlatformOutput {
                    amount_credits: param.amount_credits,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ShieldedTransfer (Shielded -> Shielded)
// ---------------------------------------------------------------------------

/// Private transfer within the shielded pool.
pub struct ShieldedTransferTool;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ShieldedTransferParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Shielded address (dash1z.../tdash1z...)
    pub to_address: String,
    /// Amount in credits to transfer
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedTransferOutput {
    amount_credits: u64,
}

impl ToolBase for ShieldedTransferTool {
    type Parameter = ShieldedTransferParams;
    type Output = ShieldedTransferOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_transfer".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Send a private shielded transfer to another shielded address. \
             The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedTransferTool {
    async fn invoke(
        service: &DashMcpService,
        param: ShieldedTransferParams,
    ) -> Result<ShieldedTransferOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let recipient_bytes =
            dash_sdk::dpp::address_funds::OrchardAddress::from_bech32m_string(&param.to_address)
                .map(|(addr, _)| addr.to_raw_bytes().to_vec())
                .map_err(|e| McpToolError::InvalidParam {
                    message: format!("Invalid shielded address: {e}"),
                })?;

        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldedTransfer {
            seed_hash,
            amount: param.amount_credits,
            recipient_address_bytes: recipient_bytes,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ShieldedTransferComplete { .. } => {
                Ok(ShieldedTransferOutput {
                    amount_credits: param.amount_credits,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ShieldedUnshield (Shielded -> Platform)
// ---------------------------------------------------------------------------

/// Unshield credits to a Platform address.
pub struct ShieldedUnshield;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ShieldedUnshieldParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Bech32m Platform address (dash1.../tdash1...)
    pub to_address: String,
    /// Amount in credits to unshield
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedUnshieldOutput {
    amount_credits: u64,
    to_address: String,
}

impl ToolBase for ShieldedUnshield {
    type Parameter = ShieldedUnshieldParams;
    type Output = ShieldedUnshieldOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_unshield".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Unshield credits to a Platform address. \
             The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedUnshield {
    async fn invoke(
        service: &DashMcpService,
        param: ShieldedUnshieldParams,
    ) -> Result<ShieldedUnshieldOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let (platform_addr, _network) =
            dash_sdk::dpp::address_funds::PlatformAddress::from_bech32m_string(&param.to_address)
                .map_err(|e| McpToolError::InvalidParam {
                message: format!("Invalid Platform address: {e}"),
            })?;

        let task = BackendTask::ShieldedTask(ShieldedTask::UnshieldCredits {
            seed_hash,
            amount: param.amount_credits,
            to_platform_address: platform_addr,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ShieldedCreditsUnshielded { .. } => {
                Ok(ShieldedUnshieldOutput {
                    amount_credits: param.amount_credits,
                    to_address: param.to_address,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ShieldedWithdraw (Shielded -> Core)
// ---------------------------------------------------------------------------

/// Withdraw from shielded pool to a Core address.
pub struct ShieldedWithdrawTool;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ShieldedWithdrawParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Dash Core address to receive the withdrawal (X.../y...)
    pub to_address: String,
    /// Amount in credits to withdraw
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedWithdrawOutput {
    amount_credits: u64,
    to_address: String,
}

impl ToolBase for ShieldedWithdrawTool {
    type Parameter = ShieldedWithdrawParams;
    type Output = ShieldedWithdrawOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_withdraw".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Withdraw from shielded pool to a Core address. \
             The withdrawal is queued on Platform and settles after confirmation. \
             The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedWithdrawTool {
    async fn invoke(
        service: &DashMcpService,
        param: ShieldedWithdrawParams,
    ) -> Result<ShieldedWithdrawOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        resolve::validate_address(&param.to_address)?;
        // INTENTIONAL: no SPV sync needed — this tool dispatches a Platform state transition
        // (withdrawal is queued on Platform and settles after confirmation)
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let core_address: dash_sdk::dashcore_rpc::dashcore::Address<
            dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked,
        > = param
            .to_address
            .parse()
            .map_err(|e| McpToolError::InvalidParam {
                message: format!("Invalid Core address: {e}"),
            })?;
        let core_address = core_address.assume_checked();

        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldedWithdrawal {
            seed_hash,
            amount: param.amount_credits,
            to_core_address: core_address,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ShieldedWithdrawalComplete { .. } => {
                Ok(ShieldedWithdrawOutput {
                    amount_credits: param.amount_credits,
                    to_address: param.to_address,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}
