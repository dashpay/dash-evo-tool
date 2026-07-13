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
use crate::mcp::tools::WalletIdParams;

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
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_duffs, "duffs")?;

        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;
        resolve::ensure_spv_synced(&ctx).await?;

        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
            seed_hash,
            amount_duffs: param.amount_duffs,
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
            "Shield credits from the wallet's Platform balance into the shielded pool. \
             The wallet selects the input addresses. \
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
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;

        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // Pre-flight: verify the wallet's total platform balance can cover the
        // amount. The upstream coordinator selects the actual input addresses —
        // DET no longer picks a single `from_address`.
        {
            let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
            let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
            let total: u64 = wallet
                .platform_address_info
                .values()
                .map(|info| info.balance)
                .sum();
            if total < param.amount_credits {
                return Err(McpToolError::InvalidParam {
                    message: format!(
                        "Insufficient platform balance. Total available is {} credits but {} required.",
                        total, param.amount_credits
                    ),
                });
            }
        }

        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromBalance {
            seed_hash,
            amount: param.amount_credits,
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
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let recipient_bytes =
            dash_sdk::dpp::address_funds::OrchardAddress::from_bech32m_string(&param.to_address)
                .map(|addr| addr.to_raw_bytes().to_vec())
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
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let platform_addr =
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
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        resolve::validate_address(&param.to_address)?;
        // INTENTIONAL: no SPV sync needed — this tool dispatches a Platform state transition
        // (withdrawal is queued on Platform and settles after confirmation)
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let core_address = param
            .to_address
            .parse::<dash_sdk::dashcore_rpc::dashcore::Address<
                dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked,
            >>()
            .map_err(|_| McpToolError::InvalidParam {
                message: "The Core address is invalid.".to_owned(),
            })?
            .require_network(ctx.network())
            .map_err(|_| McpToolError::InvalidParam {
                message: "The Core address does not match the active network.".to_owned(),
            })?;

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

// ---------------------------------------------------------------------------
// ShieldedInit (warm prover + bind shielded keys)
// ---------------------------------------------------------------------------

/// Warm the Orchard prover and bind shielded keys for a wallet.
pub struct ShieldedInit;

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedInitOutput {
    /// Whether the Halo2 proving key finished building (ready for spends).
    prover_ready: bool,
    /// Whether the wallet's Orchard keys are bound to the shielded coordinator.
    shielded_bound: bool,
}

impl ToolBase for ShieldedInit {
    type Parameter = WalletIdParams;
    type Output = ShieldedInitOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_init".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Prepare a wallet for shielded operations: bind its Orchard keys and \
             warm the proving key (~30s on first call). Idempotent — safe to call \
             repeatedly. Run this once before shielding or transferring."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedInit {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<ShieldedInitOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // Binding rehydrates from the local shielded store and does not need a
        // fully-synced chain, but the wallet backend must be wired so the
        // coordinator exists and the wallet resolves. `ensure_spv_synced` is the
        // single MCP chokepoint that wires it (idempotent on later calls).
        resolve::ensure_spv_synced(&ctx).await?;

        let backend = ctx.wallet_backend().map_err(McpToolError::TaskFailed)?;

        // Bind first (fast), then warm the proving key (~30s). A successful bind
        // guarantees the wallet is bound; report it directly.
        backend
            .ensure_shielded_bound_jit(&seed_hash)
            .await
            .map_err(McpToolError::TaskFailed)?;
        let prover_ready = backend.warm_shielded_prover().await;

        Ok(ShieldedInitOutput {
            prover_ready,
            shielded_bound: true,
        })
    }
}

// ---------------------------------------------------------------------------
// ShieldedSync (force a coordinator sync, return fresh balance)
// ---------------------------------------------------------------------------

/// Force an immediate shielded sync and return the post-sync balance.
pub struct ShieldedSync;

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedSyncOutput {
    shielded_credits: u64,
    shielded_duffs: u64,
}

impl ToolBase for ShieldedSync {
    type Parameter = WalletIdParams;
    type Output = ShieldedSyncOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_sync".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Force an immediate shielded sync pass and return the wallet's \
             post-sync shielded balance (credits and duffs). Use this to verify \
             a balance change after a shield, transfer, unshield, or withdraw."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for ShieldedSync {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<ShieldedSyncOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        let backend = ctx.wallet_backend().map_err(McpToolError::TaskFailed)?;
        // `sync_now` fires `on_shielded_sync_completed` synchronously, so the
        // push snapshot is fresh by the time this returns (Phase E writer).
        backend.sync_shielded_now(true).await;

        Ok(ShieldedSyncOutput {
            shielded_credits: ctx.shielded_balance_credits(&seed_hash),
            shielded_duffs: ctx.shielded_balance_duffs(&seed_hash),
        })
    }
}

// ---------------------------------------------------------------------------
// ShieldedBalanceGet (read the push snapshot, no sync)
// ---------------------------------------------------------------------------

/// Read the wallet's shielded balance from the push snapshot (no sync).
pub struct ShieldedBalanceGet;

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedBalanceGetOutput {
    shielded_credits: u64,
    shielded_duffs: u64,
}

impl ToolBase for ShieldedBalanceGet {
    type Parameter = WalletIdParams;
    type Output = ShieldedBalanceGetOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_balance_get".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Read the wallet's shielded balance (credits and duffs) from the last \
             synced snapshot without triggering a sync. Returns zero when the \
             wallet has never synced. Use shielded_sync to refresh first."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(false))
    }
}

impl AsyncTool<DashMcpService> for ShieldedBalanceGet {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<ShieldedBalanceGetOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // INTENTIONAL: a pure snapshot read — no SPV gate, no sync. The figure
        // reflects the last completed shielded sync (zero if none).
        Ok(ShieldedBalanceGetOutput {
            shielded_credits: ctx.shielded_balance_credits(&seed_hash),
            shielded_duffs: ctx.shielded_balance_duffs(&seed_hash),
        })
    }
}

// ---------------------------------------------------------------------------
// ShieldedAddressGet (default Orchard receive address)
// ---------------------------------------------------------------------------

/// Return the wallet's default shielded (Orchard) receive address.
pub struct ShieldedAddressGet;

#[derive(Serialize, schemars::JsonSchema)]
pub struct ShieldedAddressGetOutput {
    /// Bech32m Orchard address (dash1z.../tdash1z...).
    address: String,
}

impl ToolBase for ShieldedAddressGet {
    type Parameter = WalletIdParams;
    type Output = ShieldedAddressGetOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "shielded_address_get".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Return the wallet's default shielded (Orchard) receive address as a \
             bech32m string, usable as the recipient for a shielded transfer. \
             Run shielded_init first if the wallet is not yet bound."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(false))
    }
}

impl AsyncTool<DashMcpService> for ShieldedAddressGet {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<ShieldedAddressGetOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let backend = ctx.wallet_backend().map_err(McpToolError::TaskFailed)?;
        let raw = backend
            .shielded_default_address(&seed_hash, 0)
            .await
            .map_err(McpToolError::TaskFailed)?
            .ok_or_else(|| McpToolError::InvalidParam {
                message: "This wallet has no shielded address yet. \
                          Run shielded_init first to bind its shielded keys."
                    .to_owned(),
            })?;

        let address = crate::model::address::encode_shielded_address(&raw, ctx.network())
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        Ok(ShieldedAddressGetOutput { address })
    }
}
