//! Identity-related MCP tools: top-up, transfer, withdraw.

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
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_amount(param.amount_duffs)?;

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
                    actual_fee: fee_result.actual_fee,
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
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;

        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
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
            balances.sort_by(|a, b| b.1.cmp(&a.1));
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
                    actual_fee: fee_result.actual_fee,
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
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
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
                    actual_fee: fee_result.actual_fee,
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
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        resolve::validate_address(&param.to_address)?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
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
                    actual_fee: fee_result.actual_fee,
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
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        // INTENTIONAL: no SPV sync needed — this tool only dispatches Platform state transitions,
        // not Core UTXO spends
        let _seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        let qi = resolve::qualified_identity(&ctx, &param.identity_id)?;

        let (platform_addr, _network) =
            dash_sdk::dpp::address_funds::PlatformAddress::from_bech32m_string(&param.to_address)
                .map_err(|e| McpToolError::InvalidParam {
                message: format!("Invalid Platform address: {e}"),
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
                    actual_fee: fee_result.actual_fee,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityMasternodeLoad (load a masternode/evonode identity by ProTxHash)
// ---------------------------------------------------------------------------

/// Load a masternode/evonode identity headlessly from its ProTxHash and keys.
pub struct IdentityMasternodeLoad;

#[derive(Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityMasternodeLoadParams {
    /// ProTxHash of the masternode/evonode (its identity ID). Accepts hex (the
    /// canonical encoding) or Base58.
    pub pro_tx_hash: String,
    /// Node type: "masternode" or "evonode".
    pub node_type: String,
    /// Owner private key (WIF or 64-char hex). Bound as the OWNER key. At least
    /// one of the owner or payout private key is required.
    #[serde(default)]
    pub owner_private_key: String,
    /// Voting private key (WIF or 64-char hex). Optional; binds the voter
    /// identity. Does not enable a withdrawal on its own.
    #[serde(default)]
    pub voting_private_key: String,
    /// Payout/transfer private key (WIF or 64-char hex). Bound as the TRANSFER
    /// key. At least one of the owner or payout private key is required.
    #[serde(default)]
    pub payout_private_key: String,
    /// Optional human-readable name; trimmed, empty falls back to a DPNS name.
    #[serde(default)]
    pub alias: String,
    /// Expected network (required so keys and addresses bind to the right chain).
    pub network: String,
}

// Hand-written so the three private keys can never reach a log sink or an MCP
// error `data` payload. A derived `Debug` would print the key material verbatim
// — these keys can move the node's full Platform credit balance. Mirrors
// `ImportWalletParams` (wallet.rs). The non-secret fields stay readable.
impl std::fmt::Debug for IdentityMasternodeLoadParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityMasternodeLoadParams")
            .field("pro_tx_hash", &self.pro_tx_hash)
            .field("node_type", &self.node_type)
            .field("owner_private_key", &"<redacted>")
            .field("voting_private_key", &"<redacted>")
            .field("payout_private_key", &"<redacted>")
            .field("alias", &self.alias)
            .field("network", &self.network)
            .finish()
    }
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityMasternodeLoadOutput {
    identity_id: String,
    node_type: String,
    alias: Option<String>,
    owner_key_loaded: bool,
    voting_key_loaded: bool,
    payout_key_loaded: bool,
    /// Withdrawal key modes this identity supports ("owner" / "transfer").
    available_withdrawal_keys: Vec<String>,
    /// Registered payout address (the OWNER-mode withdrawal destination), if any.
    payout_address: Option<String>,
    dpns_names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Tool A param Debug redaction (TC-MN-010) ──────────────────────────
    //
    // The single most important security test: the params `Debug` must never
    // surface any of the three private keys, because `McpToolError::TaskFailed`
    // serializes `{task_err:?}` into the MCP error `data` payload.

    #[test]
    fn load_params_debug_redacts_every_private_key() {
        let params = IdentityMasternodeLoadParams {
            pro_tx_hash: "PROTX_HASH_VALUE".to_owned(),
            node_type: "evonode".to_owned(),
            owner_private_key: "OWNER_SECRET_VALUE".to_owned(),
            voting_private_key: "VOTING_SECRET_VALUE".to_owned(),
            payout_private_key: "PAYOUT_SECRET_VALUE".to_owned(),
            alias: "my-node".to_owned(),
            network: "testnet".to_owned(),
        };

        let debug = format!("{params:?}");

        // No key sentinel may appear.
        assert!(
            !debug.contains("OWNER_SECRET_VALUE"),
            "owner key leaked: {debug}"
        );
        assert!(
            !debug.contains("VOTING_SECRET_VALUE"),
            "voting key leaked: {debug}"
        );
        assert!(
            !debug.contains("PAYOUT_SECRET_VALUE"),
            "payout key leaked: {debug}"
        );
        // Each key field renders as the redaction marker.
        assert_eq!(
            debug.matches("<redacted>").count(),
            3,
            "all three key fields redacted: {debug}"
        );
        // Non-secret fields stay readable.
        assert!(
            debug.contains("PROTX_HASH_VALUE"),
            "pro_tx_hash visible: {debug}"
        );
        assert!(debug.contains("evonode"), "node_type visible: {debug}");
        assert!(debug.contains("my-node"), "alias visible: {debug}");
        assert!(debug.contains("testnet"), "network visible: {debug}");
    }

    // ── Key-format policy delegation (TC-MN-011) ──────────────────────────
    //
    // The tool feeds raw key strings straight into `Secret` -> `IdentityInputToLoad`
    // -> the backend `verify_key_input`, which is the single source of truth for
    // the length policy (64-hex, 51/52-WIF, 0 -> none, else error). The tool adds
    // no competing length check of its own, so wrong-length keys are rejected by
    // the backend as `KeyInputValidationFailed`, role-named, value never echoed.
    //
    // This is a table-of-record assertion: the params struct accepts any string
    // for the key fields without pre-validating its length.
    #[test]
    fn load_params_accept_any_key_length_without_local_check() {
        for key in ["", "tooshort", &"a".repeat(63), &"f".repeat(64)] {
            let params = IdentityMasternodeLoadParams {
                pro_tx_hash: "x".to_owned(),
                node_type: "masternode".to_owned(),
                owner_private_key: key.to_owned(),
                network: "testnet".to_owned(),
                ..Default::default()
            };
            // Construction never validates key length — that is verify_key_input's job.
            assert_eq!(params.owner_private_key, key);
        }
    }
}
