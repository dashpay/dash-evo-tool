//! Wallet-related MCP tools: address generation, balances, sending funds.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::error::TaskError;
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
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

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
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
        let alias = wallet_arc
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .alias
            .clone();

        // Balances come from the display-only WalletBackend snapshot (P4a);
        // upstream owns chain UTXO/balance bookkeeping.
        let balance = ctx.snapshot_balance(&seed_hash);

        Ok(WalletBalancesOutput {
            alias,
            total_duffs: balance.total,
            confirmed_duffs: balance.confirmed,
            unconfirmed_duffs: balance.unconfirmed,
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
        let ctx = service.tool_ctx().await?;

        // Network is mandatory for destructive operations.
        resolve::require_network(&ctx, Some(&param.network))?;

        // Validate inputs before dispatching
        resolve::validate_positive_amount(param.amount_duffs, "duffs")?;
        resolve::validate_address(&param.address)?;

        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        resolve::ensure_spv_synced(&ctx).await?;

        let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;

        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: param.address,
                amount_duffs: param.amount_duffs,
            }],
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
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // SPV is required: DAPI proof verification needs quorum/masternode list
        // data from the synced chain.  When a second client is running, SPV falls
        // back to a tempdir and must sync before platform queries can succeed.
        resolve::ensure_spv_synced(&ctx).await?;

        let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
                let network = ctx.network();
                let entries = balances
                    .into_iter()
                    .map(|(addr, (balance, nonce))| PlatformAddressBalance {
                        address: addr.to_bech32m_string(network),
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
// ImportWallet (BIP-39 mnemonic -> registered wallet)
// ---------------------------------------------------------------------------

/// Import a wallet from a BIP-39 recovery phrase.
pub struct ImportWallet;

#[derive(Deserialize, schemars::JsonSchema, Default)]
pub struct ImportWalletParams {
    /// BIP-39 recovery phrase (12 or 24 words, space-separated)
    pub mnemonic: String,
    /// Expected network (required so addresses are derived for the right chain)
    pub network: String,
    /// Optional human-readable wallet name
    #[serde(default)]
    pub alias: Option<String>,
}

// Hand-written so the recovery phrase can never reach a log sink. A derived
// `Debug` would print the mnemonic verbatim, and the BIP-39 phrase is the
// highest-value secret in the app (full, irreversible wallet compromise).
impl std::fmt::Debug for ImportWalletParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportWalletParams")
            .field("mnemonic", &"<redacted>")
            .field("network", &self.network)
            .field("alias", &self.alias)
            .finish()
    }
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ImportWalletOutput {
    seed_hash: String,
    alias: Option<String>,
    /// True when this seed was already present (the import was a no-op).
    already_imported: bool,
}

impl ToolBase for ImportWallet {
    type Parameter = ImportWalletParams;
    type Output = ImportWalletOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_wallet_import".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Import a wallet from a BIP-39 recovery phrase and register it on the \
             active network, returning its seed hash. Imports unprotected (no \
             passphrase) for headless use. Idempotent: re-importing the same \
             phrase is a no-op that returns the existing seed hash. \
             The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(false),
        )
    }
}

impl AsyncTool<DashMcpService> for ImportWallet {
    async fn invoke(
        service: &DashMcpService,
        param: ImportWalletParams,
    ) -> Result<ImportWalletOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;

        resolve::require_network(&ctx, Some(&param.network))?;

        // Hold the phrase in a zeroizing buffer so the cleartext seed words are
        // scrubbed from memory on drop rather than lingering in a freed String.
        // `bip39` is built with its `zeroize` feature, so the parsed `Mnemonic`
        // scrubs its word indices on drop too.
        let mnemonic_phrase = zeroize::Zeroizing::new(param.mnemonic);
        let mnemonic = bip39::Mnemonic::parse_normalized(mnemonic_phrase.trim()).map_err(|e| {
            McpToolError::InvalidParam {
                message: format!("The recovery phrase is not valid: {e}"),
            }
        })?;
        // The derived 64-byte HD seed is the spend secret; keep it zeroizing so
        // it never outlives this call in freed heap/stack memory.
        let seed = zeroize::Zeroizing::new(mnemonic.to_seed(""));

        let alias = param
            .alias
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned);

        let wallet =
            crate::model::wallet::Wallet::new_from_seed(*seed, ctx.network(), alias.clone(), None)
                .map_err(|e| McpToolError::TaskFailed(e.into()))?;
        // Capture the seed hash before `register_wallet` consumes the wallet so
        // the already-imported branch can still report it.
        let seed_hash = wallet.seed_hash();

        match ctx.register_wallet(
            wallet,
            &seed,
            crate::model::wallet::birth_height::WalletOrigin::Imported,
        ) {
            Ok((hash, _)) => Ok(ImportWalletOutput {
                seed_hash: hex::encode(hash),
                alias,
                already_imported: false,
            }),
            Err(TaskError::WalletAlreadyImported) => Ok(ImportWalletOutput {
                seed_hash: hex::encode(seed_hash),
                alias,
                already_imported: true,
            }),
            Err(e) => Err(McpToolError::TaskFailed(e)),
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
        let ctx = service.tool_ctx().await?;
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
