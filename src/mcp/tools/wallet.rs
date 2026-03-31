//! Wallet-related MCP tools: address generation, balances, sending funds.

use std::borrow::Cow;

use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
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
use crate::model::address::AddressKind;
use crate::model::send_routing::{SendRequest, SendResult, SendSource};

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

        if ctx.spv_manager.wallet_id_for_seed(seed_hash).is_none() {
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

        let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
        let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());

        Ok(WalletBalancesOutput {
            alias: wallet.alias.clone(),
            total_duffs: wallet.total_balance_duffs(),
            confirmed_duffs: wallet.confirmed_balance_duffs(),
            unconfirmed_duffs: wallet.unconfirmed_balance_duffs(),
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

// ---------------------------------------------------------------------------
// WalletFundsSend — unified send via send_routing
// ---------------------------------------------------------------------------

fn default_core_source() -> String {
    "core".to_string()
}

/// Send funds from any source (Core wallet, Platform addresses, shielded pool,
/// or identity balance) to any supported destination address type.
/// Uses unified send routing to determine the correct backend operation.
pub struct WalletFundsSend;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct WalletFundsSendParams {
    /// Wallet alias or 64-char hex seed hash (sender)
    pub wallet_id: String,
    /// Destination address: Core (X.../y...), Platform (dash1.../tdash1...),
    /// Shielded (dash1z.../tdash1z...), or Identity ID (Base58)
    pub address: String,
    /// Amount in duffs (1 DASH = 100,000,000 duffs). For non-Core sources the
    /// tool converts to credits internally (1 duff = 1,000 credits).
    pub amount_duffs: u64,
    /// Required: mainnet, testnet, devnet, local
    pub network: String,
    /// Source type: "core" (default), "platform", "shielded", "identity"
    #[serde(default = "default_core_source")]
    pub source: String,
    /// Identity ID in Base58 (required when source="identity")
    pub identity_id: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct WalletFundsSendOutput {
    /// Which route was taken (e.g. "core->core", "platform->shielded")
    route: String,
    /// Amount sent (in the source's native unit)
    amount: u64,
    /// Transaction ID if available (Core-to-Core sends)
    txid: Option<String>,
    /// Number of sequential tasks dispatched (>1 for multi-address Platform-to-Shielded)
    tasks_dispatched: u32,
}

impl ToolBase for WalletFundsSend {
    type Parameter = WalletFundsSendParams;
    type Output = WalletFundsSendOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "wallet_funds_send".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Send funds from any source to any destination address type. \
             Sources: core (default), platform, shielded, identity. \
             Destinations: Core address, Platform address, Shielded address, or Identity ID. \
             Amount is always specified in duffs (converted to credits internally for non-Core sources). \
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

impl AsyncTool<DashMcpService> for WalletFundsSend {
    async fn invoke(
        service: &DashMcpService,
        param: WalletFundsSendParams,
    ) -> Result<WalletFundsSendOutput, McpToolError> {
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

        // Validate amount
        resolve::validate_amount(param.amount_duffs)?;

        // Resolve wallet
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // SPV required for all wallet operations
        resolve::ensure_spv_synced(&ctx).await?;

        // Detect and validate destination address
        let dest_kind =
            AddressKind::detect(&param.address).ok_or_else(|| McpToolError::InvalidParam {
                message: format!(
                    "Could not detect address type for '{}'. \
                     Supported: Core (X/y/7/8), Platform (dash1/tdash1), \
                     Shielded (dash1z/tdash1z), Identity (Base58 ID).",
                    param.address
                ),
            })?;
        let destination = build_validated_address(&param.address, dest_kind, &ctx)
            .map_err(|msg| McpToolError::InvalidParam { message: msg })?;

        let source_str = param.source.to_lowercase();
        let source_label = source_str.as_str();

        // Build SendSource based on source param
        let (source, amount) = match source_label {
            "core" => {
                let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
                let balance = {
                    let w = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
                    w.confirmed_balance_duffs()
                };
                (
                    SendSource::CoreWallet {
                        wallet: wallet_arc,
                        balance_duffs: balance,
                        seed_hash,
                    },
                    param.amount_duffs,
                )
            }
            "platform" => {
                // Fetch platform address balances via backend task
                let task =
                    BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
                let result = dispatch_task(&ctx, task)
                    .await
                    .map_err(McpToolError::TaskFailed)?;

                let addresses = match result {
                    BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
                        platform_balances_to_address_tuples(&ctx, seed_hash, balances)?
                    }
                    other => {
                        return Err(McpToolError::Internal(format!(
                            "Unexpected task result: {other:?}"
                        )));
                    }
                };

                let amount_credits = duffs_to_credits(param.amount_duffs)?;

                (
                    SendSource::PlatformAddresses {
                        seed_hash,
                        addresses,
                    },
                    amount_credits,
                )
            }
            "shielded" => {
                let balance = shielded_balance(&ctx, seed_hash);
                let amount_credits = duffs_to_credits(param.amount_duffs)?;

                (
                    SendSource::Shielded {
                        seed_hash,
                        balance_credits: balance,
                    },
                    amount_credits,
                )
            }
            "identity" => {
                let identity_id_str =
                    param
                        .identity_id
                        .as_deref()
                        .ok_or_else(|| McpToolError::InvalidParam {
                            message:
                                "The 'identity_id' parameter is required when source is 'identity'."
                                    .to_owned(),
                        })?;
                let qi = resolve::qualified_identity(&ctx, identity_id_str)?;

                let amount_credits = duffs_to_credits(param.amount_duffs)?;

                (
                    SendSource::Identity {
                        qualified_identity: qi,
                    },
                    amount_credits,
                )
            }
            other => {
                return Err(McpToolError::InvalidParam {
                    message: format!(
                        "Unknown source '{other}'. Supported: core, platform, shielded, identity."
                    ),
                });
            }
        };

        // Resolve destination identity if needed
        let resolved_identity = match &destination {
            crate::model::address::ValidatedAddress::Identity { id, .. } => {
                Some(resolve::qualified_identity(
                    &ctx,
                    &id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                )?)
            }
            _ => None,
        };

        let route_label = format!(
            "{}->{}",
            source_label,
            dest_kind.short_label().to_lowercase()
        );

        // Build and dispatch the send request
        let request = SendRequest {
            source,
            destination,
            amount,
            resolved_identity,
            fee_context: None,
        };

        let send_result = crate::model::send_routing::resolve_send(request).map_err(|e| {
            McpToolError::InvalidParam {
                message: e.to_string(),
            }
        })?;

        match send_result {
            SendResult::Single(task) => {
                let result = dispatch_task(&ctx, task)
                    .await
                    .map_err(McpToolError::TaskFailed)?;

                let txid = extract_txid(&result);

                Ok(WalletFundsSendOutput {
                    route: route_label,
                    amount: param.amount_duffs,
                    txid,
                    tasks_dispatched: 1,
                })
            }
            SendResult::Sequential(tasks) => {
                let count = tasks.len() as u32;
                for task in tasks {
                    dispatch_task(&ctx, task)
                        .await
                        .map_err(McpToolError::TaskFailed)?;
                }

                Ok(WalletFundsSendOutput {
                    route: route_label,
                    amount: param.amount_duffs,
                    txid: None,
                    tasks_dispatched: count,
                })
            }
        }
    }
}

/// Convert duffs to credits (1 duff = 1,000 credits).
fn duffs_to_credits(duffs: u64) -> Result<u64, McpToolError> {
    duffs
        .checked_mul(CREDITS_PER_DUFF)
        .ok_or_else(|| McpToolError::InvalidParam {
            message: format!("Amount {duffs} duffs overflows when converting to credits."),
        })
}

/// Extract a transaction ID from a backend task result, if available.
fn extract_txid(result: &BackendTaskSuccessResult) -> Option<String> {
    match result {
        BackendTaskSuccessResult::WalletPayment { txid, .. } => Some(txid.clone()),
        _ => None,
    }
}

/// Convert platform balance map (from FetchPlatformAddressBalances) to the
/// tuple format expected by `SendSource::PlatformAddresses`.
fn platform_balances_to_address_tuples(
    ctx: &crate::context::AppContext,
    seed_hash: crate::model::wallet::WalletSeedHash,
    balances: std::collections::BTreeMap<dash_sdk::dpp::dashcore::Address, (u64, u32)>,
) -> Result<
    Vec<(
        dash_sdk::dpp::address_funds::PlatformAddress,
        dash_sdk::dpp::dashcore::Address,
        u64,
    )>,
    McpToolError,
> {
    let wallet_arc = resolve::wallet_arc(ctx, seed_hash)?;
    let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
    let platform_addrs = wallet.platform_addresses(ctx.network());

    let mut result = Vec::new();
    for (core_addr, platform_addr) in &platform_addrs {
        if let Some((balance, _nonce)) = balances.get(core_addr) {
            result.push((*platform_addr, core_addr.clone(), *balance));
        }
    }
    Ok(result)
}

/// Get the shielded balance for a wallet (0 if not initialized).
fn shielded_balance(
    ctx: &crate::context::AppContext,
    seed_hash: crate::model::wallet::WalletSeedHash,
) -> u64 {
    let states = ctx
        .shielded_states
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    states
        .get(&seed_hash)
        .map(|s| s.shielded_balance)
        .unwrap_or(0)
}

/// Build a `ValidatedAddress` from a raw address string and its detected kind.
fn build_validated_address(
    address: &str,
    kind: AddressKind,
    ctx: &crate::context::AppContext,
) -> Result<crate::model::address::ValidatedAddress, String> {
    use crate::model::address::ValidatedAddress;
    use dash_sdk::dpp::address_funds::PlatformAddress;
    use dash_sdk::dpp::dashcore::address::NetworkUnchecked;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    use dash_sdk::platform::Identifier;

    match kind {
        AddressKind::Core => {
            let unchecked: dash_sdk::dpp::dashcore::Address<NetworkUnchecked> = address
                .parse()
                .map_err(|e| format!("Invalid Core address '{address}': {e}"))?;
            let checked = unchecked
                .require_network(ctx.network())
                .map_err(|e| format!("Address network mismatch: {e}"))?;
            Ok(ValidatedAddress::Core(checked))
        }
        AddressKind::Platform => {
            let (platform_addr, _) = PlatformAddress::from_bech32m_string(address)
                .map_err(|e| format!("Invalid Platform address '{address}': {e}"))?;
            Ok(ValidatedAddress::Platform {
                address: platform_addr,
                bech32m: address.to_string(),
            })
        }
        AddressKind::Shielded => Ok(ValidatedAddress::Shielded(address.to_string())),
        AddressKind::Identity => {
            let id = Identifier::from_string(address, Encoding::Base58)
                .map_err(|e| format!("Invalid Identity ID '{address}': {e}"))?;
            Ok(ValidatedAddress::Identity {
                id,
                dpns_name: None,
            })
        }
    }
}
