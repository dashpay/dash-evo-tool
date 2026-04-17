//! Wallet-related MCP tools: address generation, balances, sending funds,
//! and wallet lifecycle (import, delete).

use std::borrow::Cow;
use std::fmt;

use bip39::Mnemonic;
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
use crate::mcp::server::{DashMcpService, network_display_name};
use crate::mcp::tools::{NetworkParams, WalletIdParams};
use crate::model::secret::Secret;
use crate::model::wallet::Wallet;
use crate::model::wallet::mnemonic::sanitize_mnemonic_input;
use crate::spv::CoreBackendMode;

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
// ImportWalletTool
// ---------------------------------------------------------------------------

/// BIP-39 word counts that correspond to valid entropy sizes (128–256 bits).
const VALID_MNEMONIC_WORD_COUNTS: &[usize] = &[12, 15, 18, 21, 24];

/// Import a wallet from a BIP39 mnemonic.
pub struct ImportWalletTool;

/// Parameters for `core_wallet_import`.
///
/// This struct deliberately does **not** derive `Debug`. The hand-rolled impl
/// below redacts every secret field so logs that print the params — or a
/// future macro that captures the whole struct — cannot leak the mnemonic.
/// `Default` is required by the rmcp tool trait; the generated empty mnemonic
/// will fail the word-count pre-check if ever dispatched.
#[derive(Default, Deserialize, schemars::JsonSchema)]
pub struct ImportWalletParams {
    /// BIP39 mnemonic seed phrase, separated by spaces. Extra whitespace,
    /// digits, and punctuation are tolerated — the tool sanitizes the input
    /// before parsing. This is the only recovery material for the wallet:
    /// back it up before calling this tool.
    pub mnemonic: Secret,

    /// Optional BIP39 passphrase — the "25th word" that modifies the seed
    /// derivation. Distinct from `encryption_password`; conflating them will
    /// corrupt seed access. Pass the exact string used when the mnemonic was
    /// originally generated (commonly empty).
    #[serde(default)]
    pub passphrase: Option<Secret>,

    /// Optional password used to encrypt the wallet seed at rest on this
    /// device. If the app already has a main password configured, this value
    /// must match — providing a different password is rejected. Does **not**
    /// change the app's main password.
    #[serde(default)]
    pub encryption_password: Option<Secret>,

    /// Human-friendly alias shown in the wallet list. Auto-generated if
    /// omitted.
    #[serde(default)]
    pub alias: Option<String>,

    /// Dash Core wallet name for multi-wallet RPC installations. Ignored when
    /// the app is running in SPV-only mode; supplying it in SPV mode is an
    /// error so that the caller is aware the value would be discarded.
    #[serde(default)]
    pub core_wallet_name: Option<String>,

    /// Expected active network (`"mainnet"`, `"testnet"`, `"devnet"`,
    /// `"local"`). The tool rejects the request if it does not match.
    pub network: String,
}

impl fmt::Debug for ImportWalletParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportWalletParams")
            .field("mnemonic", &"***")
            .field("passphrase", &self.passphrase.as_ref().map(|_| "***"))
            .field(
                "encryption_password",
                &self.encryption_password.as_ref().map(|_| "***"),
            )
            .field("alias", &self.alias)
            .field("core_wallet_name", &self.core_wallet_name)
            .field("network", &self.network)
            .finish()
    }
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ImportWalletOutput {
    /// Hex-encoded SHA-256 hash of the BIP39 seed. Stable across networks
    /// for the same mnemonic.
    pub seed_hash: String,
    /// Effective alias stored on disk (matches `alias` when supplied,
    /// otherwise auto-generated).
    pub alias: String,
    /// Network the wallet was imported onto.
    pub network: String,
}

impl ToolBase for ImportWalletTool {
    type Parameter = ImportWalletParams;
    type Output = ImportWalletOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_wallet_import".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Import a wallet using a BIP39 mnemonic. \
             Stores the wallet locally and adds it to the SPV monitoring set. \
             The mnemonic is the only recovery material — back it up before calling this tool. \
             Re-importing the same mnemonic returns an error. \
             The optional `encryption_password` encrypts the wallet seed at rest and \
             must match the app's existing main password if one is already set."
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

/// Generate a fallback wallet alias when the caller did not provide one.
///
/// Mirrors the GUI convention in `import_mnemonic_screen::save_wallet` so
/// wallets imported via MCP look the same as GUI-imported wallets.
fn default_wallet_alias(ctx: &crate::context::AppContext) -> String {
    let existing_wallet_count = ctx.wallets.read().map(|w| w.len()).unwrap_or(0);
    format!("Wallet {}", existing_wallet_count + 1)
}

impl AsyncTool<DashMcpService> for ImportWalletTool {
    async fn invoke(
        service: &DashMcpService,
        param: ImportWalletParams,
    ) -> Result<ImportWalletOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        // Network is mandatory — importing onto the wrong network corrupts
        // the caller's expectation of which keys derive which addresses.
        if param.network.is_empty() {
            return Err(McpToolError::InvalidParam {
                message: "The 'network' parameter must not be empty. \
                          Use \"mainnet\", \"testnet\", \"devnet\", or \"local\"."
                    .to_owned(),
            });
        }
        resolve::require_network(&ctx, Some(&param.network))?;

        // Core wallet name is only meaningful when talking to an RPC-backed
        // Dash Core. Rejecting it up-front in SPV mode surfaces the mismatch
        // to the caller instead of silently discarding the value.
        let rpc_mode = ctx.core_backend_mode() == CoreBackendMode::Rpc;
        if param.core_wallet_name.is_some() && !rpc_mode {
            return Err(McpToolError::InvalidParam {
                message: "core_wallet_name is only supported in Dash Core RPC mode. \
                          Remove the field or switch the app to RPC-backed Core."
                    .to_owned(),
            });
        }

        // 1. Sanitize the mnemonic input.
        let sanitized = sanitize_mnemonic_input(param.mnemonic.expose_secret());

        // 2. Word-count pre-check. Must run before BIP39 parsing so we do not
        //    include the sanitized phrase in the error message.
        let word_count = sanitized.split_whitespace().count();
        if !VALID_MNEMONIC_WORD_COUNTS.contains(&word_count) {
            return Err(McpToolError::InvalidParam {
                message: format!("Expected 12, 15, 18, 21, or 24 words; got {word_count}."),
            });
        }

        // 3. Parse the mnemonic. Do not propagate bip39::Error details —
        //    they can echo pieces of the sanitized phrase.
        let mnemonic =
            Mnemonic::parse_normalized(&sanitized).map_err(|_| McpToolError::InvalidParam {
                message: "Invalid mnemonic: word list, spelling, or checksum rejected.".into(),
            })?;

        // 4. Derive the seed. Pass the BIP39 passphrase (if any) through
        //    verbatim; it is a distinct secret from encryption_password.
        let passphrase = param
            .passphrase
            .as_ref()
            .map(|s| s.expose_secret())
            .unwrap_or("");
        let seed = mnemonic.to_seed(passphrase);

        // 5. SEC-010 main-password guard — if the app already has a main
        //    password configured, the supplied encryption_password must
        //    match it. We never touch `update_main_password`: MCP callers
        //    cannot change the app-wide password through this tool.
        if let Some(ref enc_pw) = param.encryption_password
            && ctx.has_main_password()
        {
            ctx.verify_main_password(enc_pw)
                .map_err(McpToolError::TaskFailed)?;
        }

        // 6. Build the wallet.
        let alias_value = param
            .alias
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default_wallet_alias(&ctx));

        let mut wallet = Wallet::new_from_seed(
            seed,
            ctx.network(),
            Some(alias_value.clone()),
            param.encryption_password.as_ref(),
        )
        .map_err(McpToolError::TaskFailed)?;

        if rpc_mode {
            wallet.core_wallet_name = param.core_wallet_name.clone();
        }

        // 7. Register with the app context.
        let (seed_hash, _wallet_arc) = ctx.register_wallet(wallet).map_err(|e| match e {
            // Preserve the distinct WalletAlreadyImported signal; callers
            // commonly want to match on it to offer "already there" UX.
            TaskError::WalletAlreadyImported => McpToolError::TaskFailed(e),
            other => McpToolError::TaskFailed(other),
        })?;

        tracing::info!(
            seed = %hex::encode(seed_hash),
            alias = %alias_value,
            network = %param.network,
            "wallet imported via MCP"
        );

        Ok(ImportWalletOutput {
            seed_hash: hex::encode(seed_hash),
            alias: alias_value,
            network: network_display_name(ctx.network()).to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------
// DeleteWalletTool
// ---------------------------------------------------------------------------

/// Delete a wallet from the local device.
pub struct DeleteWalletTool;

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct DeleteWalletParams {
    /// Wallet alias or 64-char hex seed hash.
    pub wallet_id: String,
    /// Expected active network. Required for destructive operations.
    pub network: String,
    /// Hex seed hash of the wallet to confirm deletion. Must exactly match
    /// the resolved wallet's `seed_hash` — this avoids alias collisions
    /// silently deleting the wrong wallet.
    pub confirm_seed_hash: String,
    /// Allow deletion even when the wallet still holds a non-zero balance.
    /// Defaults to `false` — balance-carrying wallets are kept unless the
    /// caller explicitly opts in.
    #[serde(default)]
    pub allow_delete_with_balance: bool,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct DeleteWalletOutput {
    /// Hex seed hash of the deleted wallet.
    pub seed_hash: String,
    /// Alias the wallet had on disk, if any.
    pub alias: Option<String>,
    /// Network from which the wallet was removed.
    pub network: String,
    /// Count of local identities linked to the wallet that were cascaded out.
    pub orphaned_identities: u32,
    /// Other networks where the same mnemonic is still imported. The caller
    /// may want to repeat the deletion there.
    pub other_networks_with_same_seed: Vec<String>,
}

impl ToolBase for DeleteWalletTool {
    type Parameter = DeleteWalletParams;
    type Output = DeleteWalletOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "core_wallet_delete".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "DELETE a wallet from this device. \
             Removes local metadata (addresses, UTXOs, identity links) but leaves \
             on-chain funds untouched — however, you will lose access unless you \
             have the original mnemonic. \
             Operation is irreversible and scoped to the selected network. \
             Requires `confirm_seed_hash` to exactly match the target wallet's \
             hex seed_hash; refuses deletion of wallets with non-zero balance \
             unless `allow_delete_with_balance=true`."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(false),
        )
    }
}

impl AsyncTool<DashMcpService> for DeleteWalletTool {
    async fn invoke(
        service: &DashMcpService,
        param: DeleteWalletParams,
    ) -> Result<DeleteWalletOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        if param.network.is_empty() {
            return Err(McpToolError::InvalidParam {
                message: "The 'network' parameter must not be empty. \
                          Use \"mainnet\", \"testnet\", \"devnet\", or \"local\"."
                    .to_owned(),
            });
        }
        resolve::require_network(&ctx, Some(&param.network))?;

        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // Strict hex seed-hash confirmation — prevents "oops, wrong alias"
        // from deleting the wrong wallet.
        if !param
            .confirm_seed_hash
            .eq_ignore_ascii_case(&hex::encode(seed_hash))
        {
            return Err(McpToolError::InvalidParam {
                message:
                    "confirm_seed_hash must exactly match the resolved wallet's hex seed_hash. \
                          Call core_wallets_list to look it up."
                        .to_owned(),
            });
        }

        // Snapshot alias + balance before deletion.
        let wallet_arc = resolve::wallet_arc(&ctx, seed_hash)?;
        let (alias, total_balance_duffs) = {
            let w = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
            (w.alias.clone(), w.total_balance_duffs())
        };
        drop(wallet_arc);

        if total_balance_duffs > 0 && !param.allow_delete_with_balance {
            return Err(McpToolError::InvalidParam {
                message: format!(
                    "Wallet has non-zero balance ({total_balance_duffs} duffs). \
                     Pass allow_delete_with_balance=true to proceed."
                ),
            });
        }

        // Identity count and other-network presence (for the response snapshot).
        let orphaned_identities = ctx
            .db
            .identity_count_for_wallet(&seed_hash, &ctx.network())
            .map_err(|e| McpToolError::TaskFailed(TaskError::from(e)))?;

        let current_network_name = network_display_name(ctx.network()).to_owned();
        let other_networks_with_same_seed = ctx
            .db
            .wallet_seed_hash_networks(&seed_hash)
            .map_err(|e| McpToolError::TaskFailed(TaskError::from(e)))?
            .into_iter()
            .filter(|n| n != &current_network_name)
            .collect::<Vec<_>>();

        // Perform the deletion.
        ctx.remove_wallet(&seed_hash).map_err(|e| match e {
            TaskError::WalletNotFound => McpToolError::WalletNotFound {
                id: hex::encode(seed_hash),
            },
            other => McpToolError::TaskFailed(other),
        })?;

        tracing::info!(
            seed = %hex::encode(seed_hash),
            alias = ?alias,
            network = %current_network_name,
            orphaned_identities,
            "wallet deleted via MCP"
        );

        Ok(DeleteWalletOutput {
            seed_hash: hex::encode(seed_hash),
            alias,
            network: current_network_name,
            orphaned_identities,
            other_networks_with_same_seed,
        })
    }
}
