//! Wallet-related MCP tools: address generation, balances, sending funds.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::DashMcpService;
use crate::mcp::tools::{NetworkParams, WalletIdParams};

// ---------------------------------------------------------------------------
// GenerateReceiveAddress
// ---------------------------------------------------------------------------

/// Generate a new receive address for a wallet.
pub struct GenerateReceiveAddress;

#[derive(Serialize, schemars::JsonSchema)]
pub struct GenerateReceiveAddressOutput {
    address: String,
}

impl ToolBase for GenerateReceiveAddress {
    type Parameter = WalletIdParams;
    type Output = GenerateReceiveAddressOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_address_create".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Generate a new receive address for a wallet. \
             Pass wallet alias or hex seed hash."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(false)
                .open_world(false),
        )
    }
}

impl AsyncTool<DashMcpService> for GenerateReceiveAddress {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<GenerateReceiveAddressOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        // Verify the wallet has a PlatformWallet (required for SPV operations)
        if ctx.get_platform_wallet(&seed_hash).is_none() {
            return Err(McpToolError::Internal(
                "Wallet is not loaded into SPV. Please retry in a moment.".to_string(),
            ));
        }

        let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => {
                Ok(GenerateReceiveAddressOutput { address })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// WalletBalancesQuery
// ---------------------------------------------------------------------------

/// Show wallet balances (total, confirmed, unconfirmed) in duffs.
pub struct WalletBalancesQuery;

#[derive(Serialize, schemars::JsonSchema)]
pub struct WalletBalancesOutput {
    alias: Option<String>,
    total_duffs: u64,
    confirmed_duffs: u64,
    unconfirmed_duffs: u64,
}

impl ToolBase for WalletBalancesQuery {
    type Parameter = WalletIdParams;
    type Output = WalletBalancesOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_balances_get".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Show wallet balances (total, confirmed, unconfirmed) in duffs. \
             Pass wallet alias or hex seed hash."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(true))
    }
}

impl AsyncTool<DashMcpService> for WalletBalancesQuery {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<WalletBalancesOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        // Read alias from evo-tool Wallet (still the owner of metadata).
        let alias = resolve::wallet_arc(&ctx, seed_hash)
            .ok()
            .and_then(|arc| arc.read().ok().and_then(|w| w.alias.clone()));

        // Read balances from PlatformWallet's lock-free atomics — no RwLock
        // needed, instant read.
        let pw = resolve::platform_wallet(&ctx, seed_hash)?;
        let bal = pw.core().balance();

        Ok(WalletBalancesOutput {
            alias,
            total_duffs: bal.total(),
            confirmed_duffs: bal.spendable(),
            unconfirmed_duffs: bal.unconfirmed(),
        })
    }
}

// ---------------------------------------------------------------------------
// SendCoreFunds
// ---------------------------------------------------------------------------

/// Send DASH from a wallet to an address.
pub struct SendCoreFunds;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct SendFundsParams {
    /// Wallet alias or 64-char hex seed hash (sender)
    pub wallet_id: String,
    /// Recipient address (Dash address string)
    pub address: String,
    /// Amount to send in duffs (1 DASH = 100,000,000 duffs)
    pub amount_duffs: u64,
    /// Expected network (e.g. "mainnet", "testnet"). Required for send operations
    /// to prevent accidental cross-network transfers.
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RecipientOutput {
    address: String,
    amount_duffs: u64,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct SendFundsOutput {
    txid: String,
    recipients: Vec<RecipientOutput>,
    total_amount_duffs: u64,
}

impl ToolBase for SendCoreFunds {
    type Parameter = SendFundsParams;
    type Output = SendFundsOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_funds_send".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Send DASH from a wallet to an address. \
             Amount is in duffs (1 DASH = 100,000,000 duffs). \
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

impl AsyncTool<DashMcpService> for SendCoreFunds {
    async fn invoke(
        service: &DashMcpService,
        param: SendFundsParams,
    ) -> Result<SendFundsOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        // Network is mandatory for destructive operations
        if param.network.is_empty() {
            return Err(McpToolError::InvalidParam {
                message: "The 'network' parameter must not be empty. \
                          Use \"mainnet\", \"testnet\", \"devnet\", or \"local\"."
                    .to_owned(),
            });
        }
        resolve::require_network(&ctx, Some(&param.network))?;

        // Validate inputs before dispatching
        resolve::validate_amount(param.amount_duffs)?;
        resolve::validate_address(&param.address)?;

        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;

        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: param.address,
                amount_duffs: param.amount_duffs,
            }],
            subtract_fee_from_amount: false,
            memo: None,
            override_fee: None,
        };

        let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: wallet_arc,
            request,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::WalletPayment {
                txid,
                recipients,
                total_amount,
            } => Ok(SendFundsOutput {
                txid,
                recipients: recipients
                    .iter()
                    .map(|(addr, amt)| RecipientOutput {
                        address: addr.clone(),
                        amount_duffs: *amt,
                    })
                    .collect(),
                total_amount_duffs: total_amount,
            }),
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// FetchPlatformBalances
// ---------------------------------------------------------------------------

/// Fetch platform address balances for a wallet.
pub struct FetchPlatformBalances;

#[derive(Serialize, schemars::JsonSchema)]
pub struct PlatformAddressBalance {
    address: String,
    balance: u64,
    nonce: u32,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct PlatformBalancesOutput {
    balances: Vec<PlatformAddressBalance>,
}

impl ToolBase for FetchPlatformBalances {
    type Parameter = WalletIdParams;
    type Output = PlatformBalancesOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "platform_addresses_list".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Fetch platform address balances (credits and nonces) for a wallet. \
             Pass wallet alias or hex seed hash."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(true))
    }
}

impl AsyncTool<DashMcpService> for FetchPlatformBalances {
    async fn invoke(
        service: &DashMcpService,
        param: WalletIdParams,
    ) -> Result<PlatformBalancesOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
                let entries = balances
                    .into_iter()
                    .map(|(addr, (balance, nonce))| PlatformAddressBalance {
                        address: addr.to_string(),
                        balance,
                        nonce,
                    })
                    .collect();
                Ok(PlatformBalancesOutput { balances: entries })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ListWalletsTool
// ---------------------------------------------------------------------------

/// List wallet names currently loaded in the application.
pub struct ListWalletsTool;

#[derive(Serialize, schemars::JsonSchema)]
pub struct WalletEntry {
    seed_hash: String,
    alias: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ListWalletsOutput {
    wallets: Vec<WalletEntry>,
}

impl ToolBase for ListWalletsTool {
    type Parameter = NetworkParams;
    type Output = ListWalletsOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_wallets_list".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("List wallet names currently loaded in the application".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(false))
    }
}

impl AsyncTool<DashMcpService> for ListWalletsTool {
    async fn invoke(
        service: &DashMcpService,
        param: NetworkParams,
    ) -> Result<ListWalletsOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
        let entries: Vec<WalletEntry> = wallets
            .iter()
            .map(|(hash, wallet_arc)| {
                let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
                WalletEntry {
                    seed_hash: hex::encode(hash),
                    alias: wallet.alias.clone(),
                }
            })
            .collect();
        Ok(ListWalletsOutput { wallets: entries })
    }
}
