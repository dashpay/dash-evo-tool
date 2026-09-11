//! Identity-related MCP tools: listing, top-up, transfer, withdraw.

use std::borrow::Cow;
use std::collections::BTreeMap;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::DashMcpService;
use crate::mcp::tools::NetworkParams;

// ---------------------------------------------------------------------------
// ListIdentitiesTool
// ---------------------------------------------------------------------------

/// List the identities persisted locally for the active network.
pub struct ListIdentitiesTool;

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityEntry {
    /// Base58-encoded identity ID.
    id: String,
    alias: Option<String>,
    /// `User`, `Masternode` or `Evonode`.
    identity_type: String,
    /// Last known Platform status, e.g. `Active` or `Unknown`.
    status: String,
    balance_credits: u64,
    /// DPNS names as last persisted locally; empty until a name refresh ran.
    dpns_names: Vec<String>,
    /// HD index this identity was registered at, when it belongs to a wallet.
    wallet_index: Option<u32>,
    /// Hex seed hashes of the wallets this identity is bound to.
    wallet_seed_hashes: Vec<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ListIdentitiesOutput {
    identities: Vec<IdentityEntry>,
}

impl ToolBase for ListIdentitiesTool {
    type Parameter = NetworkParams;
    type Output = ListIdentitiesOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_list".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "List the identities saved for the active network, with their DPNS names, \
             balances and wallet bindings. Reads persisted state only — it makes no \
             network calls and never refreshes from Platform."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(false))
    }
}

impl AsyncTool<DashMcpService> for ListIdentitiesTool {
    async fn invoke(
        service: &DashMcpService,
        param: NetworkParams,
    ) -> Result<ListIdentitiesOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        // Opens this network's storage so the identity read has a store to read
        // from. No SPV gate: every field below comes from persisted state, so
        // this reports what is on disk rather than what the chain currently says.
        resolve::ensure_wallets_hydrated(&ctx).await?;

        let identities =
            ctx.load_local_qualified_identities()
                .map_err(McpToolError::TaskFailed)?
                .into_iter()
                .map(|qi| IdentityEntry {
                    id: qi.identity.id().to_string(
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                    ),
                    identity_type: qi.identity_type.to_string(),
                    status: qi.status.to_string(),
                    balance_credits: qi.identity.balance(),
                    alias: qi.alias,
                    dpns_names: qi.dpns_names.into_iter().map(|name| name.name).collect(),
                    wallet_index: qi.wallet_index,
                    wallet_seed_hashes: qi.associated_wallets.keys().map(hex::encode).collect(),
                })
                .collect();

        Ok(ListIdentitiesOutput { identities })
    }
}

// ---------------------------------------------------------------------------
// IdentityCreditsTopup (Core -> Identity via asset lock)
// ---------------------------------------------------------------------------

/// Top up an identity with DASH from the wallet (creates asset lock).
pub struct IdentityCreditsTopup;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityTopupParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Base58-encoded identity ID to top up
    pub identity_id: String,
    /// Amount in duffs (1 DASH = 100,000,000 duffs)
    pub amount_duffs: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityTopupOutput {
    identity_id: String,
    amount_duffs: u64,
    estimated_fee: u64,
    actual_fee: u64,
}

impl ToolBase for IdentityCreditsTopup {
    type Parameter = IdentityTopupParams;
    type Output = IdentityTopupOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_credits_topup".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Top up an identity with DASH from the wallet. Creates an asset lock, \
             waits for proof, then broadcasts the top-up transition. \
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

impl AsyncTool<DashMcpService> for IdentityCreditsTopup {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityTopupParams,
    ) -> Result<IdentityTopupOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_duffs, "duffs")?;

        resolve::ensure_wallets_hydrated(&ctx).await?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;
        let qi = resolve::qualified_identity(&ctx, &param.identity_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;

        let identity_index = qi.wallet_index.unwrap_or(0);
        let top_up_index = qi.top_ups.len() as u32;
        let identity_id_str = qi
            .identity
            .id()
            .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

        let task = BackendTask::IdentityTask(IdentityTask::TopUpIdentity(IdentityTopUpInfo {
            qualified_identity: qi,
            wallet: wallet_arc,
            identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                param.amount_duffs,
                identity_index,
                top_up_index,
            ),
        }));

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ToppedUpIdentity(_identity, fee_result) => {
                Ok(IdentityTopupOutput {
                    identity_id: identity_id_str,
                    amount_duffs: param.amount_duffs,
                    estimated_fee: fee_result.estimated_fee,
                    actual_fee: fee_result.actual_fee.unwrap_or(fee_result.estimated_fee),
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityCreditsTopupFromPlatform (Platform -> Identity)
// ---------------------------------------------------------------------------

/// Top up an identity from Platform address balances.
pub struct IdentityCreditsTopupFromPlatform;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityTopupFromPlatformParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Base58-encoded identity ID to top up
    pub identity_id: String,
    /// Amount in credits to top up
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityTopupFromPlatformOutput {
    identity_id: String,
    amount_credits: u64,
    estimated_fee: u64,
    actual_fee: u64,
}

impl ToolBase for IdentityCreditsTopupFromPlatform {
    type Parameter = IdentityTopupFromPlatformParams;
    type Output = IdentityTopupFromPlatformOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_credits_topup_from_platform".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Top up an identity from Platform address balances. \
             Auto-allocates from highest-balance addresses. \
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

impl AsyncTool<DashMcpService> for IdentityCreditsTopupFromPlatform {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityTopupFromPlatformParams,
    ) -> Result<IdentityTopupFromPlatformOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;

        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        resolve::ensure_wallets_hydrated(&ctx).await?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;
        let qi = resolve::qualified_identity(&ctx, &param.identity_id)?;
        let identity_id_str = qi
            .identity
            .id()
            .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

        // Read platform addresses from wallet state and allocate inputs (scoped to avoid Send issues)
        let platform_balances = {
            let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
            let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
            let mut balances: Vec<_> = wallet
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
                .collect();
            balances.sort_by_key(|a| std::cmp::Reverse(a.1));
            balances
        };

        let mut inputs = BTreeMap::new();
        let mut remaining = param.amount_credits;

        for (platform_addr, balance) in &platform_balances {
            if remaining == 0 {
                break;
            }
            let use_amount = remaining.min(*balance);
            if use_amount > 0 {
                inputs.insert(*platform_addr, use_amount);
                remaining = remaining.saturating_sub(use_amount);
            }
        }

        if remaining > 0 {
            return Err(McpToolError::InvalidParam {
                message: format!("Insufficient platform balance. Short by {remaining} credits."),
            });
        }

        let task = BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses {
            identity: qi,
            inputs,
            wallet_seed_hash: seed_hash,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::ToppedUpIdentity(_identity, fee_result) => {
                Ok(IdentityTopupFromPlatformOutput {
                    identity_id: identity_id_str,
                    amount_credits: param.amount_credits,
                    estimated_fee: fee_result.estimated_fee,
                    actual_fee: fee_result.actual_fee.unwrap_or(fee_result.estimated_fee),
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityCreditsTransfer (Identity -> Identity)
// ---------------------------------------------------------------------------

/// Transfer credits from one identity to another.
pub struct IdentityCreditsTransfer;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityTransferParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Base58-encoded source identity ID
    pub from_identity_id: String,
    /// Base58-encoded destination identity ID
    pub to_identity_id: String,
    /// Amount in credits to transfer
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityTransferOutput {
    from_identity_id: String,
    to_identity_id: String,
    amount_credits: u64,
    estimated_fee: u64,
    actual_fee: u64,
}

impl ToolBase for IdentityCreditsTransfer {
    type Parameter = IdentityTransferParams;
    type Output = IdentityTransferOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_credits_transfer".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Transfer credits from one identity to another. \
             Both identities must be loaded locally. \
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

impl AsyncTool<DashMcpService> for IdentityCreditsTransfer {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityTransferParams,
    ) -> Result<IdentityTransferOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        resolve::ensure_wallets_hydrated(&ctx).await?;
        let _seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let from_qi = resolve::qualified_identity(&ctx, &param.from_identity_id)?;
        let to_identifier = dash_sdk::dpp::prelude::Identifier::from_string(
            &param.to_identity_id,
            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
        )
        .map_err(|_| McpToolError::InvalidParam {
            message: format!("Invalid destination identity ID: {}", param.to_identity_id),
        })?;

        let task = BackendTask::IdentityTask(IdentityTask::Transfer(
            from_qi,
            to_identifier,
            param.amount_credits,
            None,
        ));

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::TransferredCredits(fee_result) => {
                Ok(IdentityTransferOutput {
                    from_identity_id: param.from_identity_id,
                    to_identity_id: param.to_identity_id,
                    amount_credits: param.amount_credits,
                    estimated_fee: fee_result.estimated_fee,
                    actual_fee: fee_result.actual_fee.unwrap_or(fee_result.estimated_fee),
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityCreditsWithdraw (Identity -> Core)
// ---------------------------------------------------------------------------

/// Withdraw credits from an identity to a Core address.
pub struct IdentityCreditsWithdraw;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityWithdrawParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Base58-encoded identity ID to withdraw from
    pub identity_id: String,
    /// Dash Core address to receive the withdrawal (X.../y...)
    pub to_address: String,
    /// Amount in credits to withdraw
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityWithdrawOutput {
    identity_id: String,
    to_address: String,
    amount_credits: u64,
    estimated_fee: u64,
    actual_fee: u64,
}

impl ToolBase for IdentityCreditsWithdraw {
    type Parameter = IdentityWithdrawParams;
    type Output = IdentityWithdrawOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_credits_withdraw".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Withdraw credits from an identity to a Core address. \
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

impl AsyncTool<DashMcpService> for IdentityCreditsWithdraw {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityWithdrawParams,
    ) -> Result<IdentityWithdrawOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        resolve::validate_address(&param.to_address)?;

        // The backend calls `Identity::fetch_by_identifier` and reads the identity nonce —
        // both require a synced chain. Gate before `dispatch_task` so the withdrawal
        // never races ahead of chain sync.
        resolve::ensure_spv_synced(&ctx).await?;

        let _seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;
        let qi = resolve::qualified_identity(&ctx, &param.identity_id)?;

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

        let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
            qi,
            Some(core_address),
            param.amount_credits,
            None,
        ));

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) => {
                Ok(IdentityWithdrawOutput {
                    identity_id: param.identity_id,
                    to_address: param.to_address,
                    amount_credits: param.amount_credits,
                    estimated_fee: fee_result.estimated_fee,
                    actual_fee: fee_result.actual_fee.unwrap_or(fee_result.estimated_fee),
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityCreditsToAddress (Identity -> Platform address)
// ---------------------------------------------------------------------------

/// Transfer credits from an identity to a Platform address.
pub struct IdentityCreditsToAddress;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityToAddressParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
    /// Base58-encoded identity ID to transfer from
    pub identity_id: String,
    /// Bech32m Platform address to receive credits (dash1.../tdash1...)
    pub to_address: String,
    /// Amount in credits to transfer
    pub amount_credits: u64,
    /// Expected network (required for destructive operations)
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityToAddressOutput {
    identity_id: String,
    to_address: String,
    amount_credits: u64,
    estimated_fee: u64,
    actual_fee: u64,
}

impl ToolBase for IdentityCreditsToAddress {
    type Parameter = IdentityToAddressParams;
    type Output = IdentityToAddressOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_credits_to_address".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Transfer credits from an identity to a Platform address (bech32m). \
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

impl AsyncTool<DashMcpService> for IdentityCreditsToAddress {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityToAddressParams,
    ) -> Result<IdentityToAddressOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        resolve::ensure_wallets_hydrated(&ctx).await?;
        let _seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let qi = resolve::qualified_identity(&ctx, &param.identity_id)?;

        let platform_addr =
            dash_sdk::dpp::address_funds::PlatformAddress::from_bech32m_string(&param.to_address)
                .map_err(|e| McpToolError::InvalidParam {
                message: format!("Invalid Platform address: {e}"),
            })?;

        // Gate the destination against the ACTIVE network. The `network` param
        // only pins which network is active; it does NOT validate the
        // destination, so a mainnet `dash1…` address could otherwise be paid on
        // testnet (or vice-versa), misdirecting credits. Checked after parsing
        // so a malformed address still reports "invalid", not "wrong network".
        crate::model::address::validate_platform_address_for_network(
            &param.to_address,
            ctx.network(),
        )
        .map_err(|e| McpToolError::InvalidParam {
            message: e.to_string(),
        })?;

        let mut outputs = BTreeMap::new();
        outputs.insert(platform_addr, param.amount_credits);

        let task = BackendTask::IdentityTask(IdentityTask::TransferToAddresses {
            identity: qi,
            outputs,
            key_id: None,
        });

        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::TransferredCredits(fee_result) => {
                Ok(IdentityToAddressOutput {
                    identity_id: param.identity_id,
                    to_address: param.to_address,
                    amount_credits: param.amount_credits,
                    estimated_fee: fee_result.estimated_fee,
                    actual_fee: fee_result.actual_fee.unwrap_or(fee_result.estimated_fee),
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}
