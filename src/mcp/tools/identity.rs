//! Identity-related MCP tools: top-up, transfer, withdraw.

use std::borrow::Cow;
use std::collections::BTreeMap;

use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::identity::{
    IdentityInputToLoad, IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod,
};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::DashMcpService;
use crate::model::masternode_input::{self, KeyMode};
use crate::model::secret::Secret;

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

impl ToolBase for IdentityMasternodeLoad {
    type Parameter = IdentityMasternodeLoadParams;
    type Output = IdentityMasternodeLoadOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_masternode_load".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Load a masternode or evonode identity by ProTxHash, binding its \
             owner/voting/payout private keys (WIF or hex) and persisting it \
             locally for the withdraw tool. The output reports which keys \
             loaded, the available withdrawal modes, and the registered payout \
             address. The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for IdentityMasternodeLoad {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityMasternodeLoadParams,
    ) -> Result<IdentityMasternodeLoadOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        // Cheap validation first, before the SPV wait: network match, node type,
        // and the at-least-one-signing-key rule all reject without touching the
        // network (TC-MN-012/013/014).
        resolve::require_network(&ctx, Some(&param.network))?;
        let identity_type = masternode_input::parse_node_type(&param.node_type)?;
        masternode_input::require_at_least_one_signing_key(
            &param.owner_private_key,
            &param.payout_private_key,
        )?;

        // Load fetches the identity (and, with a voting key, the voter identity)
        // over DAPI; proof verification needs a synced chain.
        resolve::ensure_spv_synced(&ctx).await?;

        let input = IdentityInputToLoad {
            identity_id_input: param.pro_tx_hash,
            identity_type,
            alias_input: param.alias.trim().to_owned(),
            voting_private_key_input: Secret::new(param.voting_private_key),
            owner_private_key_input: Secret::new(param.owner_private_key),
            payout_address_private_key_input: Secret::new(param.payout_private_key),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
        };

        let task = BackendTask::IdentityTask(IdentityTask::LoadIdentity(input));
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        let BackendTaskSuccessResult::LoadedIdentity(qi) = result else {
            return Err(McpToolError::Internal(format!(
                "Unexpected task result: {result:?}"
            )));
        };

        let mut owner_key_loaded = false;
        let mut payout_key_loaded = false;
        let mut available_withdrawal_keys = Vec::new();
        for key in qi.available_withdrawal_keys() {
            match key.identity_public_key.purpose() {
                Purpose::OWNER => {
                    owner_key_loaded = true;
                    available_withdrawal_keys.push("owner".to_owned());
                }
                Purpose::TRANSFER => {
                    payout_key_loaded = true;
                    available_withdrawal_keys.push("transfer".to_owned());
                }
                _ => {}
            }
        }

        Ok(IdentityMasternodeLoadOutput {
            identity_id: qi.identity.id().to_string(Encoding::Base58),
            node_type: identity_type.to_string().to_ascii_lowercase(),
            alias: qi.alias.clone(),
            owner_key_loaded,
            voting_key_loaded: qi.associated_voter_identity.is_some(),
            payout_key_loaded,
            available_withdrawal_keys,
            payout_address: qi
                .masternode_payout_address(ctx.network())
                .map(|addr| addr.to_string()),
            dpns_names: qi.dpns_names.iter().map(|d| d.name.clone()).collect(),
        })
    }
}

// ---------------------------------------------------------------------------
// IdentityMasternodeCreditsWithdraw (masternode-aware Identity -> Core)
// ---------------------------------------------------------------------------

/// Withdraw a masternode/evonode identity's Platform credits, honoring the
/// two key modes (owner -> payout-forced, transfer -> any Core address).
pub struct IdentityMasternodeCreditsWithdraw;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct IdentityMasternodeWithdrawParams {
    /// Base58 identity ID of the loaded masternode/evonode identity.
    pub identity_id: String,
    /// Key mode: "owner" (destination forced to the payout address) or
    /// "transfer" (withdraw to any Core address).
    pub key_mode: String,
    /// Core address to receive the withdrawal. Required for "transfer" mode;
    /// forbidden for "owner" mode (the destination is the registered payout
    /// address).
    #[serde(default)]
    pub to_address: String,
    /// Amount in credits to withdraw (must be greater than zero).
    pub amount_credits: u64,
    /// Expected network (required for destructive operations).
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct IdentityMasternodeWithdrawOutput {
    identity_id: String,
    key_mode: String,
    /// The Core address the funds were actually sent to (the payout address in
    /// owner mode, the caller's address in transfer mode).
    to_address: String,
    amount_credits: u64,
    estimated_fee: u64,
    actual_fee: u64,
}

/// Reject a caller-supplied address in owner mode (TC-MN-033).
///
/// An owner-key withdrawal always goes to the registered payout address, so a
/// supplied `to_address` is a contradiction surfaced as an error rather than
/// silently ignored.
fn reject_owner_address_contradiction(to_address: &str) -> Result<(), McpToolError> {
    if !to_address.trim().is_empty() {
        return Err(McpToolError::InvalidParam {
            message: "An owner-key withdrawal always goes to the registered payout address. \
                      Remove 'to_address', or use key_mode=transfer to choose an address."
                .to_owned(),
        });
    }
    Ok(())
}

/// Parse and validate a transfer-mode destination address (format only).
///
/// Rejects an empty address (TC-MN-034), a Platform bech32m address via
/// `is_platform_address_string` — NOT the weaker first-char check (TC-MN-031 /
/// TC-MN-046) — and an address that does not parse as Core (TC-MN-035). The
/// network match is enforced by the caller via `require_network` on the
/// returned `NetworkUnchecked` address (TC-MN-047).
fn parse_transfer_core_address(
    to_address: &str,
) -> Result<
    dash_sdk::dashcore_rpc::dashcore::Address<
        dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked,
    >,
    McpToolError,
> {
    let trimmed = to_address.trim();
    if trimmed.is_empty() {
        return Err(McpToolError::InvalidParam {
            message: "A transfer withdrawal needs a Core address. \
                      Provide 'to_address' with a Core address."
                .to_owned(),
        });
    }
    if crate::model::address::is_platform_address_string(trimmed) {
        return Err(McpToolError::InvalidParam {
            message: "Enter a valid Core address — Platform addresses cannot receive withdrawals."
                .to_owned(),
        });
    }
    trimmed
        .parse::<dash_sdk::dashcore_rpc::dashcore::Address<
            dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked,
        >>()
        .map_err(|_| McpToolError::InvalidParam {
            message: "Enter a valid Core address — that address could not be read.".to_owned(),
        })
}

impl ToolBase for IdentityMasternodeCreditsWithdraw {
    type Parameter = IdentityMasternodeWithdrawParams;
    type Output = IdentityMasternodeWithdrawOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "identity_masternode_credits_withdraw".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Withdraw a loaded masternode/evonode identity's Platform credits to \
             Core. With key_mode=owner the destination is forced to the \
             registered payout address; with key_mode=transfer you choose any \
             Core address. The 'network' parameter is required."
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

impl AsyncTool<DashMcpService> for IdentityMasternodeCreditsWithdraw {
    async fn invoke(
        service: &DashMcpService,
        param: IdentityMasternodeWithdrawParams,
    ) -> Result<IdentityMasternodeWithdrawOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        // Cheap validation first, before the SPV wait.
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_credits(param.amount_credits)?;
        let key_mode = masternode_input::parse_key_mode(&param.key_mode)?;

        // Surface the owner+address contradiction before resolving the identity
        // so it fires even for a not-yet-loaded identity (TC-MN-033/042).
        if key_mode == KeyMode::Owner {
            reject_owner_address_contradiction(&param.to_address)?;
        }

        // Resolve the loaded identity. A not-found points at the load tool
        // (FR-B5, TC-MN-040); a malformed ID is reported separately.
        let identity_id = masternode_input::decode_identity_id(&param.identity_id)?;
        let qi = ctx
            .get_identity_by_id(&identity_id)
            .map_err(|e| McpToolError::Internal(e.to_string()))?
            .ok_or_else(|| McpToolError::InvalidParam {
                message: format!(
                    "This identity is not loaded yet: {}. \
                     Run identity-masternode-load with the ProTxHash and keys first.",
                    param.identity_id
                ),
            })?;

        // Resolve the signing key for the requested mode from the identity's
        // available withdrawal keys (TC-MN-044/054).
        let purpose = match key_mode {
            KeyMode::Owner => Purpose::OWNER,
            KeyMode::Transfer => Purpose::TRANSFER,
        };
        let key_id = qi
            .available_withdrawal_keys()
            .into_iter()
            .find(|k| k.identity_public_key.purpose() == purpose)
            .map(|k| k.identity_public_key.id())
            .ok_or_else(|| McpToolError::InvalidParam {
                message: format!(
                    "The {} key needed for this withdrawal is not loaded. \
                     Re-run identity-masternode-load and include it.",
                    match key_mode {
                        KeyMode::Owner => "owner",
                        KeyMode::Transfer => "payout",
                    }
                ),
            })?;

        // Resolve the destination per mode.
        let to_address = match key_mode {
            KeyMode::Owner => {
                Some(qi.masternode_payout_address(ctx.network()).ok_or_else(|| {
                    McpToolError::InvalidParam {
                        message: "This identity has no registered payout address, so an owner-key \
                         withdrawal has no destination. Use key_mode=transfer with a Core address."
                            .to_owned(),
                    }
                })?)
            }
            KeyMode::Transfer => {
                let parsed = parse_transfer_core_address(&param.to_address)?;
                let checked = parsed.require_network(ctx.network()).map_err(|_| {
                    McpToolError::InvalidParam {
                        message: "The Core address does not match the active network. \
                                  Use an address for the active network."
                            .to_owned(),
                    }
                })?;
                Some(checked)
            }
        };

        // A withdrawal does proof-verified Platform reads — gate on SPV (NFR-P2,
        // OQ-4 option b: add the gate, diverging from the sibling
        // identity_credits_withdraw which historically skips it).
        resolve::ensure_spv_synced(&ctx).await?;

        let dispatched_address = to_address.clone();
        let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
            qi,
            to_address,
            param.amount_credits,
            Some(key_id),
        ));
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        let BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) = result else {
            return Err(McpToolError::Internal(format!(
                "Unexpected task result: {result:?}"
            )));
        };

        Ok(IdentityMasternodeWithdrawOutput {
            identity_id: param.identity_id,
            key_mode: match key_mode {
                KeyMode::Owner => "owner".to_owned(),
                KeyMode::Transfer => "transfer".to_owned(),
            },
            // Echo the address actually used (the resolved payout address in
            // owner mode), so the caller always learns where the funds went.
            to_address: dispatched_address
                .map(|a| a.to_string())
                .unwrap_or_default(),
            amount_credits: param.amount_credits,
            estimated_fee: fee_result.estimated_fee,
            actual_fee: fee_result.actual_fee,
        })
    }
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

    // ── Discoverability & schema (TC-MN-015) ──────────────────────────────
    //
    // The router is built without an AppContext, so tool registration,
    // annotations, and the input schema are assertable with no network.

    fn schema_property_names(tool: &rmcp::model::Tool) -> Vec<String> {
        tool.input_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn load_tool_registered_with_expected_annotations_and_schema() {
        let router = DashMcpService::tool_router();
        let tool = router
            .get("identity_masternode_load")
            .expect("identity_masternode_load must be registered in tool_router");

        let ann = tool
            .annotations
            .as_ref()
            .expect("tool must carry annotations");
        assert_eq!(ann.read_only_hint, Some(false));
        assert_eq!(ann.destructive_hint, Some(false));
        assert_eq!(ann.idempotent_hint, Some(false));
        assert_eq!(ann.open_world_hint, Some(true));

        let props = schema_property_names(tool);
        for expected in [
            "pro_tx_hash",
            "node_type",
            "owner_private_key",
            "voting_private_key",
            "payout_private_key",
            "alias",
            "network",
        ] {
            assert!(
                props.iter().any(|p| p == expected),
                "schema must expose '{expected}', got {props:?}"
            );
        }
    }

    // ── Tool B pure pre-flight checks (TC-MN-031/032/033/034/035) ─────────

    #[test]
    fn withdraw_amount_zero_rejected() {
        // TC-MN-032 — delegated to the shared resolve::validate_credits.
        let err = resolve::validate_credits(0).unwrap_err();
        assert!(err.to_string().contains("greater than zero"), "got: {err}");
        assert!(resolve::validate_credits(1).is_ok());
    }

    #[test]
    fn owner_mode_supplied_address_rejected() {
        // TC-MN-033 — a non-empty address in owner mode is a contradiction.
        let err =
            reject_owner_address_contradiction("yQ9JNCT4S9zVHaKYbr1FUY4YkUMYxSzWAj").unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(
            err.to_string().contains("registered payout address"),
            "got: {err}"
        );
        // Empty / whitespace-only address is the expected owner-mode input.
        assert!(reject_owner_address_contradiction("").is_ok());
        assert!(reject_owner_address_contradiction("   ").is_ok());
    }

    #[test]
    fn transfer_mode_missing_address_rejected() {
        // TC-MN-034 — transfer mode requires an address.
        for empty in ["", "   "] {
            let err = parse_transfer_core_address(empty).unwrap_err();
            assert!(matches!(err, McpToolError::InvalidParam { .. }));
            assert!(err.to_string().contains("Core address"), "got: {err}");
        }
    }

    #[test]
    fn transfer_mode_invalid_address_rejected() {
        // TC-MN-035 — the address is actually parsed, not first-char checked.
        let err = parse_transfer_core_address("not-an-address").unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(err.to_string().contains("could not be read"), "got: {err}");
    }

    #[test]
    fn transfer_mode_platform_address_rejected_via_guard() {
        // TC-MN-031 / TC-MN-046 — Platform bech32m addresses are rejected by
        // is_platform_address_string, NOT the weaker first-char check. A
        // dash1…/tdash1… string must never slip through as Core.
        for platform in ["dash1qwer1234", "tdash1qwer1234"] {
            let err = parse_transfer_core_address(platform).unwrap_err();
            assert!(matches!(err, McpToolError::InvalidParam { .. }));
            assert!(
                err.to_string()
                    .contains("Platform addresses cannot receive"),
                "for {platform}: {err}"
            );
        }
    }

    /// Derive a real, checksum-valid testnet P2PKH address from a fixed key.
    fn sample_core_address() -> String {
        use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dashcore_rpc::dashcore::{Address, Network, PrivateKey, PublicKey};

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[7u8; 32]).expect("valid secret key");
        let privkey = PrivateKey::new(sk, Network::Testnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        Address::p2pkh(&pubkey, Network::Testnet).to_string()
    }

    #[test]
    fn transfer_mode_valid_core_address_accepted() {
        // A well-formed Core address parses (network match is enforced later
        // via require_network on the NetworkUnchecked address).
        let addr = sample_core_address();
        assert!(
            parse_transfer_core_address(&addr).is_ok(),
            "derived address {addr} should parse"
        );
    }

    // ── Tool B discoverability & schema (TC-MN-043) ───────────────────────

    #[test]
    fn withdraw_tool_registered_with_expected_annotations_and_schema() {
        let router = DashMcpService::tool_router();
        let tool = router
            .get("identity_masternode_credits_withdraw")
            .expect("identity_masternode_credits_withdraw must be registered in tool_router");

        let ann = tool
            .annotations
            .as_ref()
            .expect("tool must carry annotations");
        // Identical to identity_credits_withdraw: a fund-moving, destructive op.
        assert_eq!(ann.read_only_hint, Some(false));
        assert_eq!(ann.destructive_hint, Some(true));
        assert_eq!(ann.idempotent_hint, Some(false));
        assert_eq!(ann.open_world_hint, Some(true));

        let props = schema_property_names(tool);
        for expected in [
            "identity_id",
            "key_mode",
            "to_address",
            "amount_credits",
            "network",
        ] {
            assert!(
                props.iter().any(|p| p == expected),
                "schema must expose '{expected}', got {props:?}"
            );
        }
    }

    // ── TaskFailed data payload never leaks key material (TC-MN-061) ──────
    //
    // McpToolError::TaskFailed serializes `{task_err:?}` into the MCP error
    // `data` payload. Because keys live in `Secret` (Debug = "Secret(***)") and
    // KeyInputValidationFailed carries only a role name + a backend detail (no
    // key bytes), neither the Display message nor the Debug `data` chain can
    // contain the key WIF/hex.

    #[test]
    fn secret_debug_never_reveals_plaintext() {
        let secret = Secret::new("OWNER_SECRET_WIF_VALUE");
        let debug = format!("{secret:?}");
        assert_eq!(debug, "Secret(***)");
        assert!(!debug.contains("OWNER_SECRET_WIF_VALUE"));
    }

    #[test]
    fn task_failed_data_payload_excludes_key_material() {
        use crate::backend_task::error::TaskError;
        use rmcp::ErrorData as McpError;

        // The backend never puts key bytes in this variant — only a role name
        // and a length/format detail.
        let err = McpToolError::TaskFailed(TaskError::KeyInputValidationFailed {
            key_name: "Owner".to_owned(),
            detail: "key is of incorrect size".to_owned(),
        });

        let mcp: McpError = err.into();
        let message = mcp.message.to_string();
        let data = mcp.data.map(|v| v.to_string()).unwrap_or_default();

        // A representative key sentinel must appear in neither surface.
        const KEY_SENTINEL: &str = "OWNER_SECRET_WIF_VALUE";
        assert!(!message.contains(KEY_SENTINEL), "message: {message}");
        assert!(!data.contains(KEY_SENTINEL), "data: {data}");
        // The role-named, value-free detail is present in the data chain.
        assert!(
            data.contains("KeyInputValidationFailed"),
            "data should carry the typed variant: {data}"
        );
    }
}
