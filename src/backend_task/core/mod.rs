mod create_asset_lock;
mod recover_asset_locks;
mod refresh_single_key_wallet_info;
mod refresh_wallet_info;
mod send_single_key_wallet_payment;
mod start_dash_qt;

use crate::app_dir::core_cookie_path;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::{TaskError, is_rpc_auth_error, is_rpc_connection_error};
use crate::config::{Config, NetworkConfig};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::wallet::networks_address_compatible;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::spv::CoreBackendMode;
use dash_sdk::dash_spv::sync::ProgressPercentage;
use dash_sdk::dashcore_rpc;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1};
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::{
    Address, Block, ChainLock, InstantLock, Network, OutPoint, PrivateKey, Transaction, TxOut,
};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::account::ECDSAAddressDerivation;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeRate;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::{
    BuilderError, TransactionBuilder,
};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use dash_sdk::dpp::key_wallet_manager::manager::{WalletError, WalletId, WalletManager};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

const DEFAULT_BIP44_ACCOUNT_INDEX: u32 = 0;

#[derive(Debug, Clone)]
pub enum CoreTask {
    #[allow(dead_code)] // May be used for getting single chain lock
    GetBestChainLock,
    GetBestChainLocks,
    /// Refresh wallet info from Core. The bool controls whether to also sync
    /// Platform address balances (true = sync Platform, false = Core only).
    RefreshWalletInfo(Arc<RwLock<Wallet>>, bool),
    RefreshSingleKeyWalletInfo(Arc<RwLock<SingleKeyWallet>>),
    StartDashQT(Network, PathBuf, bool),
    CreateRegistrationAssetLock(Arc<RwLock<Wallet>>, Credits, u32), // wallet, amount in credits, identity index
    CreateTopUpAssetLock(Arc<RwLock<Wallet>>, Credits, u32, u32), // wallet, amount in credits, identity index, top up index
    SendWalletPayment {
        wallet: Arc<RwLock<Wallet>>,
        request: WalletPaymentRequest,
    },
    SendSingleKeyWalletPayment {
        wallet: Arc<RwLock<SingleKeyWallet>>,
        request: WalletPaymentRequest,
    },
    RecoverAssetLocks(Arc<RwLock<Wallet>>),
    MineBlocks {
        block_count: u64,
        address: Address,
        wallet: Arc<RwLock<Wallet>>,
    },
    ListCoreWallets,
}
impl PartialEq for CoreTask {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (CoreTask::GetBestChainLock, CoreTask::GetBestChainLock)
                | (CoreTask::GetBestChainLocks, CoreTask::GetBestChainLocks)
                | (
                    CoreTask::RefreshWalletInfo(_, _),
                    CoreTask::RefreshWalletInfo(_, _)
                )
                | (
                    CoreTask::RefreshSingleKeyWalletInfo(_),
                    CoreTask::RefreshSingleKeyWalletInfo(_)
                )
                | (
                    CoreTask::StartDashQT(_, _, _),
                    CoreTask::StartDashQT(_, _, _)
                )
                | (
                    CoreTask::CreateRegistrationAssetLock(_, _, _),
                    CoreTask::CreateRegistrationAssetLock(_, _, _)
                )
                | (
                    CoreTask::CreateTopUpAssetLock(_, _, _, _),
                    CoreTask::CreateTopUpAssetLock(_, _, _, _)
                )
                | (
                    CoreTask::SendWalletPayment { .. },
                    CoreTask::SendWalletPayment { .. },
                )
                | (
                    CoreTask::SendSingleKeyWalletPayment { .. },
                    CoreTask::SendSingleKeyWalletPayment { .. },
                )
                | (
                    CoreTask::RecoverAssetLocks(_),
                    CoreTask::RecoverAssetLocks(_),
                )
                | (CoreTask::MineBlocks { .. }, CoreTask::MineBlocks { .. })
                | (CoreTask::ListCoreWallets, CoreTask::ListCoreWallets)
        )
    }
}

/// A single recipient in a payment request
#[derive(Debug, Clone)]
pub struct PaymentRecipient {
    pub address: String,
    pub amount_duffs: u64,
}

#[derive(Debug, Clone)]
pub struct WalletPaymentRequest {
    pub recipients: Vec<PaymentRecipient>,
    pub subtract_fee_from_amount: bool,
    pub memo: Option<String>,
    /// Override fee to use instead of calculated fee (for retry after min relay fee error)
    pub override_fee: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreItem {
    InstantLockedTransaction(Transaction, Vec<(OutPoint, TxOut, Address)>, InstantLock),
    ReceivedAvailableUTXOTransaction(Transaction, Vec<(OutPoint, TxOut, Address)>),
    ChainLock(ChainLock, Network),
    ChainLocks(
        Option<ChainLock>,
        Option<ChainLock>,
        Option<ChainLock>,
        Option<ChainLock>,
        Option<String>,
    ), // Mainnet, Testnet, Devnet, Local, active network RPC error
    ChainLockedBlock(Block, ChainLock),
}

impl AppContext {
    /// Extract the seed hash and first known address from an HD wallet.
    fn core_wallet_first_address(
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<([u8; 32], Option<Address>), TaskError> {
        let g = wallet.read()?;
        Ok((g.seed_hash(), g.known_addresses.keys().next().cloned()))
    }

    pub async fn run_core_task(
        self: &Arc<Self>,
        task: CoreTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            CoreTask::GetBestChainLock => self
                .core_client
                .read()?
                .get_best_chain_lock()
                .map(|chain_lock| {
                    BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                        chain_lock,
                        self.network,
                    ))
                })
                .map_err(|e| self.rpc_error_with_url(e)),
            CoreTask::GetBestChainLocks => {
                // Load configs
                let config = Config::load_from(&self.data_dir)?;

                let maybe_mainnet_config = config.config_for_network(Network::Mainnet);
                let maybe_testnet_config = config.config_for_network(Network::Testnet);
                let maybe_devnet_config = config.config_for_network(Network::Devnet);
                let maybe_local_config = config.config_for_network(Network::Regtest);

                let mainnet_result =
                    Self::get_best_chain_lock(maybe_mainnet_config, Network::Mainnet);
                let testnet_result =
                    Self::get_best_chain_lock(maybe_testnet_config, Network::Testnet);
                let devnet_result = Self::get_best_chain_lock(maybe_devnet_config, Network::Devnet);
                let local_result = Self::get_best_chain_lock(maybe_local_config, Network::Regtest);

                // Surface auth and connection errors on the active network
                // instead of silently degrading to "Disconnected".
                let (active_result, active_config) = match self.network {
                    Network::Mainnet => (&mainnet_result, maybe_mainnet_config),
                    Network::Testnet => (&testnet_result, maybe_testnet_config),
                    Network::Devnet => (&devnet_result, maybe_devnet_config),
                    Network::Regtest => (&local_result, maybe_local_config),
                    _ => (&mainnet_result, maybe_mainnet_config),
                };
                let active_rpc_error = if let Err(e) = active_result {
                    if let Some(task_err) = Self::chain_lock_rpc_error(active_config, e) {
                        return Err(task_err);
                    }
                    // Non-auth, non-connection error — log the raw error but show
                    // a sanitized message in the UI status display.
                    tracing::warn!(network = ?self.network, error = %e, "Chain lock query failed on active network");
                    Some("RPC error — check Dash Core status".to_string())
                } else {
                    None
                };

                // Convert each to Option<ChainLock> (flatten Ok(None) and Err into None)
                let mainnet_chainlock = mainnet_result.ok().flatten();
                let testnet_chainlock = testnet_result.ok().flatten();
                let devnet_chainlock = devnet_result.ok().flatten();
                let local_chainlock = local_result.ok().flatten();

                // Return whatever we have — even all-None is valid.
                Ok(BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                    mainnet_chainlock,
                    testnet_chainlock,
                    devnet_chainlock,
                    local_chainlock,
                    active_rpc_error,
                )))
            }
            CoreTask::RefreshWalletInfo(wallet, sync_platform) => {
                let (seed_hash, first_addr) = Self::core_wallet_first_address(&wallet)?;

                if self.core_backend_mode() == crate::spv::CoreBackendMode::Spv {
                    self.reconcile_spv_wallets().await?;
                } else {
                    let ctx = self.clone();
                    let wallet_for_retry = wallet.clone();
                    let result =
                        tokio::task::spawn_blocking(move || ctx.refresh_wallet_info(wallet))
                            .await?;
                    match self.with_wallet_recovery(&seed_hash, first_addr.as_ref(), false, result)
                    {
                        Err(TaskError::MustRetry(_)) => {
                            // Wallet was auto-configured; retry the refresh.
                            let ctx = self.clone();
                            tokio::task::spawn_blocking(move || {
                                ctx.refresh_wallet_info(wallet_for_retry)
                            })
                            .await??;
                        }
                        other => {
                            other?;
                        }
                    }
                }

                let warning = if sync_platform {
                    match self.fetch_platform_address_balances(seed_hash).await {
                        Ok(_) => None,
                        Err(e) => {
                            tracing::warn!("Failed to fetch Platform address balances: {}", e);
                            Some(format!("Platform sync failed: {}", e))
                        }
                    }
                } else {
                    None
                };

                Ok(BackendTaskSuccessResult::RefreshedWallet { warning })
            }
            CoreTask::RefreshSingleKeyWalletInfo(wallet) => {
                let (key_hash, address) = {
                    let g = wallet.read()?;
                    (g.key_hash, g.address.clone())
                };
                let wallet_for_retry = wallet.clone();
                let ctx = self.clone();
                let result =
                    tokio::task::spawn_blocking(move || ctx.refresh_single_key_wallet_info(wallet))
                        .await?
                        .map(|()| BackendTaskSuccessResult::RefreshedWallet { warning: None });
                match self.with_wallet_recovery(&key_hash, Some(&address), true, result) {
                    Err(TaskError::MustRetry(_)) => {
                        // Wallet was auto-configured; retry the refresh.
                        let ctx = self.clone();
                        tokio::task::spawn_blocking(move || {
                            ctx.refresh_single_key_wallet_info(wallet_for_retry)
                        })
                        .await??;
                        Ok(BackendTaskSuccessResult::RefreshedWallet { warning: None })
                    }
                    other => other,
                }
            }
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| TaskError::DashCoreStartError { source: e })
                .map(|_| BackendTaskSuccessResult::None),
            CoreTask::CreateRegistrationAssetLock(wallet, amount, identity_index) => {
                let (seed_hash, first_addr) = Self::core_wallet_first_address(&wallet)?;
                let result = self
                    .create_registration_asset_lock(wallet, amount, true, identity_index)
                    .await;
                self.with_wallet_recovery(&seed_hash, first_addr.as_ref(), false, result)
            }
            CoreTask::CreateTopUpAssetLock(wallet, amount, identity_index, top_up_index) => {
                let (seed_hash, first_addr) = Self::core_wallet_first_address(&wallet)?;
                let result = self
                    .create_top_up_asset_lock(wallet, amount, true, identity_index, top_up_index)
                    .await;
                self.with_wallet_recovery(&seed_hash, first_addr.as_ref(), false, result)
            }
            CoreTask::SendWalletPayment { wallet, request } => {
                let (seed_hash, first_addr) = Self::core_wallet_first_address(&wallet)?;
                let result = self.send_wallet_payment(wallet, request).await;
                self.with_wallet_recovery(&seed_hash, first_addr.as_ref(), false, result)
            }
            CoreTask::SendSingleKeyWalletPayment { wallet, request } => {
                let (key_hash, address) = {
                    let g = wallet.read()?;
                    (g.key_hash, g.address.clone())
                };
                let result = self.send_single_key_wallet_payment(wallet, request).await;
                self.with_wallet_recovery(&key_hash, Some(&address), true, result)
            }
            CoreTask::RecoverAssetLocks(wallet) => {
                let (seed_hash, first_addr) = Self::core_wallet_first_address(&wallet)?;
                let ctx = self.clone();
                let result =
                    tokio::task::spawn_blocking(move || ctx.recover_asset_locks(wallet)).await?;
                self.with_wallet_recovery(&seed_hash, first_addr.as_ref(), false, result)
            }
            CoreTask::MineBlocks {
                block_count,
                address,
                wallet,
            } => {
                if !matches!(self.network, Network::Regtest | Network::Devnet) {
                    return Err(TaskError::OperationNotAvailableOnNetwork {
                        operation: "Mining",
                        allowed_networks: "Regtest and Devnet",
                    });
                }
                let ctx = self.clone();
                let mined = tokio::task::spawn_blocking(move || {
                    ctx.core_client
                        .read()
                        .map_err(TaskError::from)?
                        .generate_to_address(block_count, &address)
                        .map_err(TaskError::from)
                })
                .await??;

                let mined_count = mined.len() as u64;

                // Refresh wallet balances via RPC so the UI reflects the new coins
                let refresh_ctx = self.clone();
                tokio::task::spawn_blocking(move || refresh_ctx.refresh_wallet_info(wallet))
                    .await??;

                Ok(BackendTaskSuccessResult::MineBlocksSuccess(mined_count))
            }
            CoreTask::ListCoreWallets => {
                let wallets = self.list_core_wallets()?;
                Ok(BackendTaskSuccessResult::CoreWalletsList(wallets))
            }
        }
    }

    /// If `result` is `Err(CoreWalletNotConfigured)`, attempt auto-detection
    /// of the correct Core wallet by address. On success returns
    /// `Err(MustRetry)` so callers can retry the original operation with
    /// the newly configured wallet. On failure returns the original
    /// `Err(CoreWalletNotConfigured)` so the wallets screen can show a
    /// selection dialog. Non-wallet errors pass through unchanged.
    fn with_wallet_recovery(
        &self,
        wallet_id: &[u8; 32],
        address: Option<&Address>,
        is_single_key: bool,
        result: Result<BackendTaskSuccessResult, TaskError>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let Err(TaskError::CoreWalletNotConfigured) = &result else {
            return result;
        };

        tracing::debug!(
            "RPC error -19{}: wallet not specified, attempting auto-detection",
            if is_single_key { " (single-key)" } else { "" }
        );

        if let Some(addr) = address {
            let detection_result =
                tokio::task::block_in_place(|| self.try_detect_core_wallet_for_address(addr));
            match detection_result {
                Ok(Some(wallet_name)) => {
                    if is_single_key {
                        if !self
                            .db
                            .set_single_key_wallet_core_wallet_name(wallet_id, Some(&wallet_name))?
                        {
                            return Err(TaskError::WalletDatabasePersistError);
                        }
                        if let Ok(skw) = self.single_key_wallets.read()
                            && let Some(w) = skw.get(wallet_id)
                            && let Ok(mut g) = w.write()
                        {
                            g.core_wallet_name = Some(wallet_name.clone());
                        }
                    } else {
                        if !self
                            .db
                            .set_wallet_core_wallet_name(wallet_id, Some(&wallet_name))?
                        {
                            return Err(TaskError::WalletDatabasePersistError);
                        }
                        if let Ok(wallets) = self.wallets.read()
                            && let Some(w) = wallets.get(wallet_id)
                            && let Ok(mut g) = w.write()
                        {
                            g.core_wallet_name = Some(wallet_name.clone());
                        }
                    }
                    tracing::info!("Auto-detected Core wallet '{}'", wallet_name);
                    return Err(TaskError::MustRetry(format!(
                        "Auto-detected Core wallet '{wallet_name}'"
                    )));
                }
                Ok(None) => {
                    tracing::debug!("Auto-detection inconclusive, manual selection needed");
                }
                Err(e) => tracing::warn!("Auto-detection failed: {}", e),
            }
        }

        Err(TaskError::CoreWalletNotConfigured)
    }

    fn get_best_chain_lock(
        config: &Option<NetworkConfig>,
        network: Network,
    ) -> Result<Option<ChainLock>, dashcore_rpc::Error> {
        let Some(network_config) = config else {
            return Ok(None);
        };

        let addr = format!(
            "http://{}:{}",
            network_config.core_host, network_config.core_rpc_port
        );

        let cookie_path = match core_cookie_path(network, &network_config.devnet_name) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get core cookie path for {network}: {e}");
                return Ok(None);
            }
        };

        // Try cookie authentication first
        let client = match Client::new(&addr, Auth::CookieFile(cookie_path.clone())) {
            Ok(client) => client,
            Err(_) => {
                tracing::debug!(
                    "Failed to authenticate using .cookie file at {:?}, falling back to user/pass",
                    cookie_path
                );
                match Client::new(
                    &addr,
                    Auth::UserPass(
                        network_config.core_rpc_user.to_string(),
                        network_config.core_rpc_password.to_string(),
                    ),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Failed to create {network} client: {e}");
                        return Ok(None);
                    }
                }
            }
        };

        client.get_best_chain_lock().map(Some)
    }

    /// Convert a `dashcore_rpc::Error` from `get_best_chain_lock` into a
    /// `TaskError`, enriching connection failures with host:port.
    fn chain_lock_rpc_error(
        config: &Option<NetworkConfig>,
        e: &dashcore_rpc::Error,
    ) -> Option<TaskError> {
        if is_rpc_auth_error(e) {
            return Some(TaskError::CoreRpcAuthFailed);
        }
        if is_rpc_connection_error(e) {
            let url = config
                .as_ref()
                .map(|c| format!("{}:{} ({})", c.core_host, c.core_rpc_port, e))
                .unwrap_or_else(|| "unknown".to_string());
            return Some(TaskError::CoreRpcConnectionFailed { url, source: None });
        }
        None
    }

    async fn send_wallet_payment(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match self.core_backend_mode() {
            CoreBackendMode::Spv => self.send_wallet_payment_via_spv(wallet, request).await,
            CoreBackendMode::Rpc => self.send_wallet_payment_via_rpc(wallet, request).await,
        }
    }
}

impl AppContext {
    async fn send_wallet_payment_via_rpc(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let parsed_recipients = self.parse_recipients(&request)?;

        const DEFAULT_TX_FEE: u64 = 1_000;

        let tx = {
            let mut wallet_guard = wallet.write()?;
            if !wallet_guard.is_open() {
                return Err(TaskError::WalletLocked);
            }
            wallet_guard
                .build_multi_recipient_payment_transaction(
                    self,
                    self.network,
                    &parsed_recipients,
                    DEFAULT_TX_FEE,
                    request.subtract_fee_from_amount,
                )
                .map_err(|e| TaskError::WalletPaymentFailed { detail: e })?
        };

        let txid = self
            .core_client
            .read()?
            .send_raw_transaction(&tx)
            .map_err(TaskError::from)?;

        let total_amount: u64 = request.recipients.iter().map(|r| r.amount_duffs).sum();
        let recipients_result: Vec<(String, u64)> = request
            .recipients
            .iter()
            .map(|r| (r.address.clone(), r.amount_duffs))
            .collect();

        Ok(BackendTaskSuccessResult::WalletPayment {
            txid: txid.to_string(),
            recipients: recipients_result,
            total_amount,
        })
    }

    async fn send_wallet_payment_via_spv(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.reconcile_spv_wallets().await?;

        let parsed_recipients = self.parse_recipients(&request)?;
        let seed_hash = {
            let guard = wallet.read()?;
            if !guard.is_open() {
                return Err(TaskError::WalletLocked);
            }
            guard.seed_hash()
        };

        let wallet_id = self
            .spv_manager
            .wallet_id_for_seed(seed_hash)
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "Wallet not loaded into SPV".to_string(),
            })?;

        let tx = {
            let wm_arc = self.spv_manager.wallet();
            let mut wm = wm_arc.write().await;
            let unsigned = self.build_spv_unsigned_transaction_multi(
                &mut wm,
                &wallet_id,
                &parsed_recipients,
                &request,
            )?;
            self.sign_spv_transaction(&mut wm, &wallet_id, unsigned)?
        };

        self.spv_manager
            .broadcast_transaction(&tx)
            .await
            .map_err(|e| TaskError::SpvBroadcastFailed { detail: e })?;

        self.reconcile_spv_wallets().await?;

        // Calculate actual amounts sent from the transaction outputs
        let recipients_result: Vec<(String, u64)> = request
            .recipients
            .iter()
            .zip(parsed_recipients.iter())
            .map(|(req, (addr, _))| {
                let actual_amount = Self::sum_outputs_to_script(&tx, &addr.script_pubkey())
                    .unwrap_or(req.amount_duffs);
                (req.address.clone(), actual_amount)
            })
            .collect();

        let total_amount: u64 = recipients_result.iter().map(|(_, amt)| *amt).sum();

        Ok(BackendTaskSuccessResult::WalletPayment {
            txid: tx.txid().to_string(),
            recipients: recipients_result,
            total_amount,
        })
    }

    fn parse_recipients(
        &self,
        request: &WalletPaymentRequest,
    ) -> Result<Vec<(Address, u64)>, TaskError> {
        if request.recipients.is_empty() {
            return Err(TaskError::WalletPaymentFailed {
                detail: "No recipients specified".to_string(),
            });
        }

        let mut parsed = Vec::with_capacity(request.recipients.len());
        for recipient in &request.recipients {
            if recipient.amount_duffs == 0 {
                return Err(TaskError::WalletPaymentFailed {
                    detail: format!(
                        "Amount must be greater than zero for address {}",
                        recipient.address
                    ),
                });
            }

            let addr = Address::from_str(&recipient.address)
                .map_err(|source| TaskError::InvalidRecipientAddress {
                    address: recipient.address.clone(),
                    source,
                })?
                .assume_checked();

            if !networks_address_compatible(addr.network(), &self.network) {
                return Err(TaskError::WalletPaymentFailed {
                    detail: format!(
                        "Recipient address {} uses {} but wallet network is {}",
                        recipient.address,
                        addr.network(),
                        self.network
                    ),
                });
            }

            parsed.push((addr, recipient.amount_duffs));
        }

        Ok(parsed)
    }

    fn build_spv_unsigned_transaction_multi(
        &self,
        wm: &mut WalletManager<ManagedWalletInfo>,
        wallet_id: &WalletId,
        recipients: &[(Address, u64)],
        request: &WalletPaymentRequest,
    ) -> Result<Transaction, TaskError> {
        const FALLBACK_STEP: u64 = 100;

        let _network = self.wallet_network_key();
        let current_height = self
            .spv_manager()
            .status()
            .sync_progress
            .and_then(|p| {
                p.headers()
                    .inspect_err(|e| {
                        tracing::debug!("SPV headers progress unavailable: {e}");
                    })
                    .ok()
                    .map(|h| h.current_height())
            })
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "Cannot build transaction: SPV sync height is not yet known".to_string(),
            })?;

        let total_amount: u64 = recipients.iter().map(|(_, amt)| *amt).sum();
        let mut scale_factor = 1.0f64;
        let mut attempted_fallback = false;

        // Get UTXOs and change address from the wallet account
        let (utxos, change_index) = {
            let managed_info =
                wm.get_wallet_info(wallet_id)
                    .ok_or_else(|| TaskError::WalletPaymentFailed {
                        detail: "Wallet info unavailable".to_string(),
                    })?;
            let account = managed_info
                .accounts()
                .standard_bip44_accounts
                .get(&DEFAULT_BIP44_ACCOUNT_INDEX)
                .ok_or_else(|| TaskError::WalletPaymentFailed {
                    detail: "BIP44 account missing".to_string(),
                })?;

            let utxos: Vec<_> = account.utxos.values().cloned().collect();
            let change_index = account.get_next_change_address_index().unwrap_or(0);
            (utxos, change_index)
        };

        let wallet = wm
            .get_wallet(wallet_id)
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "Wallet object not found".to_string(),
            })?;
        let wallet_account = wallet
            .accounts
            .standard_bip44_accounts
            .get(&DEFAULT_BIP44_ACCOUNT_INDEX)
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "BIP44 wallet account missing".to_string(),
            })?;
        let change_addr = wallet_account
            .derive_change_address(change_index)
            .map_err(|e| TaskError::WalletPaymentFailed {
                detail: format!("Failed to derive change address: {e}"),
            })?;

        loop {
            let scaled_recipients: Vec<(Address, u64)> = recipients
                .iter()
                .map(|(addr, amt)| (addr.clone(), (*amt as f64 * scale_factor) as u64))
                .collect();

            let build_result = (|| -> Result<Transaction, BuilderError> {
                let mut builder = TransactionBuilder::new()
                    .set_fee_rate(FeeRate::normal())
                    .set_change_address(change_addr.clone());

                for (addr, amt) in &scaled_recipients {
                    builder = builder.add_output(addr, *amt)?;
                }

                builder = builder.select_inputs(
                    &utxos,
                    SelectionStrategy::LargestFirst,
                    current_height,
                    |_| None, // No private keys for unsigned tx
                )?;

                builder.build()
            })();

            match build_result {
                Ok(tx) => return Ok(tx),
                Err(BuilderError::InsufficientFunds { .. }) if request.subtract_fee_from_amount => {
                    let next_scale = if !attempted_fallback {
                        attempted_fallback = true;
                        let fallback_amount = self.estimate_fallback_amount(
                            wm,
                            wallet_id,
                            _network,
                            DEFAULT_BIP44_ACCOUNT_INDEX,
                            current_height,
                        )?;
                        fallback_amount as f64 / total_amount as f64
                    } else {
                        let current_total = (total_amount as f64 * scale_factor) as u64;
                        let reduced = current_total.saturating_sub(FALLBACK_STEP);
                        reduced as f64 / total_amount as f64
                    };

                    if next_scale <= 0.0 || (next_scale - scale_factor).abs() < 0.0001 {
                        return Err(TaskError::WalletPaymentFailed {
                            detail: "Insufficient funds".to_string(),
                        });
                    }
                    scale_factor = next_scale;
                }
                Err(err) => {
                    return Err(TaskError::WalletPaymentFailed {
                        detail: format!("Failed to build transaction: {err}"),
                    });
                }
            }
        }
    }

    fn estimate_fallback_amount(
        &self,
        wm: &mut WalletManager<ManagedWalletInfo>,
        wallet_id: &WalletId,
        _network: WalletNetwork,
        account_index: u32,
        current_height: u32,
    ) -> Result<u64, TaskError> {
        let managed_info =
            wm.get_wallet_info(wallet_id)
                .ok_or_else(|| TaskError::WalletPaymentFailed {
                    detail: "Wallet info unavailable".to_string(),
                })?;
        let collection = managed_info.accounts();
        let account = collection
            .standard_bip44_accounts
            .get(&account_index)
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "BIP44 account missing".to_string(),
            })?;

        let mut spendable_total = 0u64;
        let mut spendable_inputs = 0usize;
        for utxo in account.utxos.values() {
            if (*utxo).is_spendable(current_height) {
                spendable_total = spendable_total.saturating_add(utxo.value());
                spendable_inputs += 1;
            }
        }

        if spendable_total == 0 || spendable_inputs == 0 {
            return Err(TaskError::WalletPaymentFailed {
                detail: "No spendable funds available".to_string(),
            });
        }

        let estimated_size = Self::estimate_p2pkh_tx_size(spendable_inputs, 1);
        let fee = FeeRate::normal().calculate_fee(estimated_size);
        Ok(spendable_total.saturating_sub(fee))
    }

    /// Build an unsigned payment transaction using TransactionBuilder.
    #[allow(dead_code)]
    fn build_unsigned_payment_tx(
        wm: &mut WalletManager<ManagedWalletInfo>,
        wallet_id: &WalletId,
        account_index: u32,
        recipients: Vec<(Address, u64)>,
        current_height: u32,
        change_address: &Address,
    ) -> Result<Transaction, WalletError> {
        // Get spendable UTXOs from the managed wallet info
        let managed_info = wm
            .get_wallet_info(wallet_id)
            .ok_or(WalletError::WalletNotFound(*wallet_id))?;
        let collection = managed_info.accounts();
        let account = collection
            .standard_bip44_accounts
            .get(&account_index)
            .ok_or(WalletError::AccountNotFound(account_index))?;

        let all_utxos: Vec<_> = account.utxos.values().cloned().collect();
        if all_utxos.is_empty() {
            return Err(WalletError::InsufficientFunds);
        }

        // Build the transaction using TransactionBuilder
        let mut builder = TransactionBuilder::new()
            .set_fee_rate(FeeRate::normal())
            .set_change_address(change_address.clone());

        for (address, amount) in recipients {
            builder = builder
                .add_output(&address, amount)
                .map_err(|e: BuilderError| WalletError::TransactionBuild(e.to_string()))?;
        }

        builder = builder
            .select_inputs(
                &all_utxos,
                SelectionStrategy::OptimalConsolidation,
                current_height,
                |_| None, // No private keys for unsigned transaction
            )
            // TODO(RUST-002): String-based error classification — see #660
            .map_err(|e: BuilderError| match e.to_string() {
                msg if msg.contains("Insufficient") => WalletError::InsufficientFunds,
                msg => WalletError::TransactionBuild(msg),
            })?;

        builder
            .build()
            .map_err(|e: BuilderError| WalletError::TransactionBuild(e.to_string()))
    }

    fn sign_spv_transaction(
        &self,
        wm: &mut WalletManager<ManagedWalletInfo>,
        wallet_id: &WalletId,
        tx: Transaction,
    ) -> Result<Transaction, TaskError> {
        let wallet = wm
            .get_wallet(wallet_id)
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "Wallet object not found".to_string(),
            })?;
        let managed_info =
            wm.get_wallet_info(wallet_id)
                .ok_or_else(|| TaskError::WalletPaymentFailed {
                    detail: "Wallet info unavailable".to_string(),
                })?;
        let accounts = managed_info.accounts();
        let account = accounts
            .standard_bip44_accounts
            .get(&DEFAULT_BIP44_ACCOUNT_INDEX)
            .ok_or_else(|| TaskError::WalletPaymentFailed {
                detail: "BIP44 account missing".to_string(),
            })?;

        let secp = Secp256k1::new();
        let mut tx_signed = tx;
        let cache = SighashCache::new(&tx_signed);

        let signing_data = tx_signed
            .input
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let utxo = account.utxos.get(&input.previous_output).ok_or_else(|| {
                    TaskError::WalletPaymentFailed {
                        detail: "Missing UTXO for signing".to_string(),
                    }
                })?;
                let sighash = cache
                    .legacy_signature_hash(index, &utxo.txout.script_pubkey, 1)
                    .map_err(|source| TaskError::SighashComputationFailed { source })?;
                Ok((sighash, utxo.address.clone()))
            })
            .collect::<Result<Vec<_>, TaskError>>()?;

        for (input, (sighash, address)) in tx_signed.input.iter_mut().zip(signing_data.into_iter())
        {
            let digest: [u8; 32] = sighash.into();
            let message = Message::from_digest(digest);

            let addr_info = account.get_address_info(&address).ok_or_else(|| {
                TaskError::WalletPaymentFailed {
                    detail: "Address metadata missing".to_string(),
                }
            })?;
            let secret_key = wallet.derive_private_key(&addr_info.path).map_err(|e| {
                TaskError::WalletPaymentFailed {
                    detail: format!("Failed to derive private key: {e}"),
                }
            })?;
            let private_key = PrivateKey {
                compressed: true,
                network: self.network,
                inner: secret_key,
            };

            let sig = secp.sign_ecdsa(&message, &private_key.inner);
            let mut serialized_sig = sig.serialize_der().to_vec();
            let mut script_sig = vec![serialized_sig.len() as u8 + 1];
            script_sig.append(&mut serialized_sig);
            script_sig.push(1);
            let mut serialized_pub_key = private_key.public_key(&secp).to_bytes();
            script_sig.push(serialized_pub_key.len() as u8);
            script_sig.append(&mut serialized_pub_key);
            input.script_sig = dash_sdk::dpp::dashcore::ScriptBuf::from_bytes(script_sig);
        }

        Ok(tx_signed)
    }

    fn sum_outputs_to_script(
        tx: &Transaction,
        script: &dash_sdk::dpp::dashcore::ScriptBuf,
    ) -> Option<u64> {
        let mut total = 0u64;
        for output in &tx.output {
            if &output.script_pubkey == script {
                total = total.saturating_add(output.value);
            }
        }
        if total == 0 { None } else { Some(total) }
    }

    fn estimate_p2pkh_tx_size(inputs: usize, outputs: usize) -> usize {
        fn varint_size(value: usize) -> usize {
            match value {
                0..=0xfc => 1,
                0xfd..=0xffff => 3,
                0x1_0000..=0xffff_ffff => 5,
                _ => 9,
            }
        }

        let mut size = 8; // version/type/lock_time
        size += varint_size(inputs);
        size += varint_size(outputs);
        size += inputs * 148;
        size += outputs * 34;
        size
    }
}
