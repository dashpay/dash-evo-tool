mod create_asset_lock;
mod recover_asset_locks;
mod refresh_single_key_wallet_info;
mod refresh_wallet_info;
mod send_single_key_wallet_payment;
mod start_dash_qt;

use crate::app_dir::core_cookie_path;
use crate::backend_task::BackendTaskSuccessResult;
use crate::config::{Config, NetworkConfig};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::spv::CoreBackendMode;
use dash_sdk::dash_spv::sync::ProgressPercentage;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1};
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::{
    Address, Block, ChainLock, InstantLock, Network, OutPoint, PrivateKey, Transaction, TxOut,
};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::{FeeLevel, FeeRate};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::{
    BuilderError, TransactionBuilder,
};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_building::AccountTypePreference;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use dash_sdk::dpp::key_wallet_manager::wallet_manager::{WalletError, WalletId, WalletManager};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

const DEFAULT_BIP44_ACCOUNT_INDEX: u32 = 0;

/// Check if two networks use the same address format.
/// Testnet, Devnet, and Regtest all use testnet-style addresses.
fn networks_address_compatible(a: &Network, b: &Network) -> bool {
    matches!(
        (a, b),
        (Network::Dash, Network::Dash)
            | (
                Network::Testnet | Network::Devnet | Network::Regtest,
                Network::Testnet | Network::Devnet | Network::Regtest,
            )
    )
}

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
    ), // Mainnet, Testnet, Devnet, Local
    ChainLockedBlock(Block, ChainLock),
}

impl AppContext {
    pub async fn run_core_task(
        self: &Arc<Self>,
        task: CoreTask,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            CoreTask::GetBestChainLock => self
                .core_client
                .read()
                .expect("Core client lock was poisoned")
                .get_best_chain_lock()
                .map(|chain_lock| {
                    BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                        chain_lock,
                        self.network,
                    ))
                })
                .map_err(|e| e.to_string()),
            CoreTask::GetBestChainLocks => {
                // Load configs
                let config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;

                let maybe_mainnet_config = config.config_for_network(Network::Dash);
                let maybe_testnet_config = config.config_for_network(Network::Testnet);
                let maybe_devnet_config = config.config_for_network(Network::Devnet);
                let maybe_local_config = config.config_for_network(Network::Regtest);

                let mainnet_result = Self::get_best_chain_lock(maybe_mainnet_config, Network::Dash);
                let testnet_result =
                    Self::get_best_chain_lock(maybe_testnet_config, Network::Testnet);
                let devnet_result = Self::get_best_chain_lock(maybe_devnet_config, Network::Devnet);
                let local_result = Self::get_best_chain_lock(maybe_local_config, Network::Regtest);

                // Convert each to Option<ChainLock>
                let mainnet_chainlock = mainnet_result.ok();
                let testnet_chainlock = testnet_result.ok();
                let devnet_chainlock = devnet_result.ok();
                let local_chainlock = local_result.ok();

                // Return whatever we have — even all-None is valid.
                Ok(BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                    mainnet_chainlock,
                    testnet_chainlock,
                    devnet_chainlock,
                    local_chainlock,
                )))
            }
            CoreTask::RefreshWalletInfo(wallet, sync_platform) => {
                // Get wallet seed hash and first address for Platform balance refresh
                // and potential Core wallet auto-detection
                let (seed_hash, wallet_seed_hash_hex, first_addr) = {
                    let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
                    let sh = wallet_guard.seed_hash();
                    let addr = wallet_guard.known_addresses.keys().next().cloned();
                    (sh, hex::encode(sh), addr)
                };

                if self.core_backend_mode() == crate::spv::CoreBackendMode::Spv {
                    self.reconcile_spv_wallets()
                        .await
                        .map_err(|e| format!("Error refreshing wallet via SPV: {}", e))?;
                } else {
                    // Run blocking RPC calls on a dedicated thread pool to avoid freezing the UI
                    let ctx = self.clone();
                    let wsh = wallet_seed_hash_hex.clone();
                    if let Err(e) =
                        tokio::task::spawn_blocking(move || ctx.refresh_wallet_info(wallet))
                            .await
                            .map_err(|e| format!("Task join error: {}", e))?
                    {
                        let msg = format!("Error refreshing wallet: {}", e);
                        if let Some(result) =
                            self.check_wallet_not_specified(&msg, &wsh, first_addr.as_ref())
                        {
                            return result;
                        }
                        return Err(msg);
                    }
                }

                // Also refresh Platform address balances if requested
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
                // Run blocking RPC calls on a dedicated thread pool to avoid freezing the UI
                let ctx = self.clone();
                tokio::task::spawn_blocking(move || ctx.refresh_single_key_wallet_info(wallet))
                    .await
                    .map_err(|e| format!("Task join error: {}", e))?
                    .map_err(|e| format!("Error refreshing wallet: {}", e))?;
                Ok(BackendTaskSuccessResult::RefreshedWallet { warning: None })
            }
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| e.to_string())
                .map(|_| BackendTaskSuccessResult::None),
            CoreTask::CreateRegistrationAssetLock(wallet, amount, identity_index) => {
                let (wsh, first_addr) = {
                    let g = wallet.read().map_err(|e| e.to_string())?;
                    (
                        hex::encode(g.seed_hash()),
                        g.known_addresses.keys().next().cloned(),
                    )
                };
                match self
                    .create_registration_asset_lock(wallet, amount, true, identity_index)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        let msg = format!("Error creating asset lock: {}", e);
                        if let Some(result) =
                            self.check_wallet_not_specified(&msg, &wsh, first_addr.as_ref())
                        {
                            return result;
                        }
                        Err(msg)
                    }
                }
            }
            CoreTask::CreateTopUpAssetLock(wallet, amount, identity_index, top_up_index) => {
                let (wsh, first_addr) = {
                    let g = wallet.read().map_err(|e| e.to_string())?;
                    (
                        hex::encode(g.seed_hash()),
                        g.known_addresses.keys().next().cloned(),
                    )
                };
                match self
                    .create_top_up_asset_lock(wallet, amount, true, identity_index, top_up_index)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        let msg = format!("Error creating top up asset lock: {}", e);
                        if let Some(result) =
                            self.check_wallet_not_specified(&msg, &wsh, first_addr.as_ref())
                        {
                            return result;
                        }
                        Err(msg)
                    }
                }
            }
            CoreTask::SendWalletPayment { wallet, request } => {
                self.send_wallet_payment(wallet, request).await
            }
            CoreTask::SendSingleKeyWalletPayment { wallet, request } => {
                self.send_single_key_wallet_payment(wallet, request).await
            }
            CoreTask::RecoverAssetLocks(wallet) => {
                // Run blocking RPC calls on a dedicated thread pool to avoid freezing the UI
                let ctx = self.clone();
                tokio::task::spawn_blocking(move || ctx.recover_asset_locks(wallet))
                    .await
                    .map_err(|e| format!("Task join error: {}", e))?
            }
            CoreTask::MineBlocks {
                block_count,
                address,
                wallet,
            } => {
                if !matches!(self.network, Network::Regtest | Network::Devnet) {
                    return Err("Mining is only available on Regtest and Devnet".to_string());
                }
                let ctx = self.clone();
                let mined = tokio::task::spawn_blocking(move || {
                    ctx.core_client
                        .read()
                        .map_err(|e| format!("Core client lock was poisoned: {}", e))?
                        .generate_to_address(block_count, &address)
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| format!("Task join error: {}", e))??;

                let mined_count = mined.len() as u64;

                // Refresh wallet balances via RPC so the UI reflects the new coins
                let refresh_ctx = self.clone();
                tokio::task::spawn_blocking(move || refresh_ctx.refresh_wallet_info(wallet))
                    .await
                    .map_err(|e| format!("Task join error: {}", e))?
                    .map_err(|e| format!("Error refreshing wallet after mining: {}", e))?;

                Ok(BackendTaskSuccessResult::MineBlocksSuccess(mined_count))
            }
        }
    }

    /// If error indicates "Wallet file not specified" (Core error -19),
    /// try to auto-detect the correct Core wallet by address, falling back
    /// to listing loaded wallets and returning `CoreWalletSelectionNeeded`.
    fn check_wallet_not_specified(
        &self,
        error_msg: &str,
        wallet_seed_hash: &str,
        first_address: Option<&Address>,
    ) -> Option<Result<BackendTaskSuccessResult, String>> {
        if !error_msg.contains("Wallet file not specified") {
            return None;
        }
        tracing::info!("RPC error -19: wallet not specified, attempting auto-detection");

        // Try auto-detection first
        if let Some(address) = first_address
            && let Some(seed_hash_bytes) = hex::decode(wallet_seed_hash)
                .ok()
                .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
        {
            match self.try_detect_core_wallet_for_address(address) {
                Ok(Some(wallet_name)) => {
                    // Persist to DB
                    let _ = self
                        .db
                        .set_wallet_core_wallet_name(&seed_hash_bytes, Some(&wallet_name));
                    // Update in-memory
                    if let Ok(wallets) = self.wallets.read()
                        && let Some(w) = wallets.get(&seed_hash_bytes)
                        && let Ok(mut g) = w.write()
                    {
                        g.core_wallet_name = Some(wallet_name.clone());
                    }
                    tracing::info!("Auto-detected Core wallet '{}'", wallet_name);
                    return Some(Ok(BackendTaskSuccessResult::CoreWalletAutoDetected {
                        wallet_name,
                    }));
                }
                Ok(None) => {
                    tracing::info!("Auto-detection inconclusive, manual selection needed");
                }
                Err(e) => tracing::warn!("Auto-detection failed: {}", e),
            }
        }

        match self.list_core_wallets() {
            Ok(wallets) if wallets.is_empty() => {
                Some(Err("No wallets loaded in Dash Core".to_string()))
            }
            Ok(wallets) => Some(Ok(BackendTaskSuccessResult::CoreWalletSelectionNeeded {
                wallet_seed_hash: wallet_seed_hash.to_string(),
                core_wallets: wallets,
            })),
            Err(e) => Some(Err(format!("Failed to list Dash Core wallets: {e}"))),
        }
    }

    fn get_best_chain_lock(
        config: &Option<NetworkConfig>,
        network: Network,
    ) -> Result<ChainLock, String> {
        if let Some(network_config) = config {
            let addr = format!(
                "http://{}:{}",
                network_config.core_host, network_config.core_rpc_port
            );

            let cookie_path = core_cookie_path(network, &network_config.devnet_name)
                .map_err(|e| format!("Failed to get core cookie path: {}", e))?;

            // Try cookie authentication first
            let client = match Client::new(&addr, Auth::CookieFile(cookie_path.clone())) {
                Ok(client) => Ok(client),
                Err(_) => {
                    tracing::info!(
                        "Failed to authenticate using .cookie file at {:?}, falling back to user/pass",
                        cookie_path
                    );
                    Client::new(
                        &addr,
                        Auth::UserPass(
                            network_config.core_rpc_user.to_string(),
                            network_config.core_rpc_password.to_string(),
                        ),
                    )
                }
            }
                .map_err(|_| format!("Failed to create {} client", network))?;

            client
                .get_best_chain_lock()
                .map_err(|e| format!("Failed to get best chain lock for {}: {}", network, e))
        } else {
            Err(format!("{} config not found", network))
        }
    }

    async fn send_wallet_payment(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, String> {
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
    ) -> Result<BackendTaskSuccessResult, String> {
        let parsed_recipients = self.parse_recipients(&request)?;

        const DEFAULT_TX_FEE: u64 = 1_000;

        let tx = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            if !wallet_guard.is_open() {
                return Err("Wallet must be unlocked".to_string());
            }
            wallet_guard.build_multi_recipient_payment_transaction(
                self,
                self.network,
                &parsed_recipients,
                DEFAULT_TX_FEE,
                request.subtract_fee_from_amount,
            )?
        };

        let txid = self
            .core_client
            .read()
            .expect("Core client lock was poisoned")
            .send_raw_transaction(&tx)
            .map_err(|e| format!("Failed to broadcast transaction: {e}"))?;

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
    ) -> Result<BackendTaskSuccessResult, String> {
        self.reconcile_spv_wallets()
            .await
            .map_err(|e| format!("Unable to sync wallet before send: {}", e))?;

        let parsed_recipients = self.parse_recipients(&request)?;
        let seed_hash = {
            let guard = wallet.read().map_err(|e| e.to_string())?;
            if !guard.is_open() {
                return Err("Wallet must be unlocked".to_string());
            }
            guard.seed_hash()
        };

        let wallet_id = self
            .spv_manager
            .wallet_id_for_seed(seed_hash)
            .ok_or_else(|| "Wallet not loaded into SPV".to_string())?;

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
            .map_err(|e| format!("Broadcast failed: {e}"))?;

        self.reconcile_spv_wallets()
            .await
            .map_err(|e| format!("Failed to refresh wallet after send: {}", e))?;

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
    ) -> Result<Vec<(Address, u64)>, String> {
        if request.recipients.is_empty() {
            return Err("No recipients specified".to_string());
        }

        let mut parsed = Vec::with_capacity(request.recipients.len());
        for recipient in &request.recipients {
            if recipient.amount_duffs == 0 {
                return Err(format!(
                    "Amount must be greater than zero for address {}",
                    recipient.address
                ));
            }

            let addr = Address::from_str(&recipient.address)
                .map_err(|e| format!("Invalid address {}: {e}", recipient.address))?
                .assume_checked();

            if !networks_address_compatible(addr.network(), &self.network) {
                return Err(format!(
                    "Recipient address {} uses {} but wallet network is {}",
                    recipient.address,
                    addr.network(),
                    self.network
                ));
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
    ) -> Result<Transaction, String> {
        const FALLBACK_STEP: u64 = 100;

        let network = self.wallet_network_key();
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
            .ok_or("Cannot build transaction: SPV sync height is not yet known")?;
        const MAX_FEE_ITERATIONS: usize = 50;

        let total_amount: u64 = recipients.iter().map(|(_, amt)| *amt).sum();
        let mut scale_factor = 1.0f64;
        let mut attempted_fallback = false;

        // Obtain change address once before the retry loop to avoid marking
        // multiple addresses as used on failed fee-adjustment attempts.
        let change_result = wm
            .get_change_address(
                wallet_id,
                DEFAULT_BIP44_ACCOUNT_INDEX,
                AccountTypePreference::BIP44,
                true,
            )
            .map_err(|e| format!("Failed to get change address: {e}"))?;
        let change_address = change_result
            .address
            .ok_or_else(|| "No change address generated".to_string())?;

        for _ in 0..MAX_FEE_ITERATIONS {
            let scaled_recipients: Vec<(Address, u64)> = recipients
                .iter()
                .map(|(addr, amt)| (addr.clone(), (*amt as f64 * scale_factor) as u64))
                .collect();

            match Self::build_unsigned_payment_tx(
                wm,
                wallet_id,
                DEFAULT_BIP44_ACCOUNT_INDEX,
                scaled_recipients,
                current_height,
                &change_address,
            ) {
                Ok(tx) => return Ok(tx),
                Err(WalletError::InsufficientFunds) if request.subtract_fee_from_amount => {
                    let next_scale = if !attempted_fallback {
                        attempted_fallback = true;
                        let fallback_amount = self.estimate_fallback_amount(
                            wm,
                            wallet_id,
                            network,
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
                        return Err("Insufficient funds".to_string());
                    }
                    scale_factor = next_scale;
                }
                Err(err) => {
                    return Err(format!("Failed to build transaction: {err}"));
                }
            }
        }

        Err("Could not build transaction after maximum fee adjustment attempts".to_string())
    }

    fn estimate_fallback_amount(
        &self,
        wm: &mut WalletManager<ManagedWalletInfo>,
        wallet_id: &WalletId,
        _network: WalletNetwork,
        account_index: u32,
        current_height: u32,
    ) -> Result<u64, String> {
        let managed_info = wm
            .get_wallet_info(wallet_id)
            .ok_or_else(|| "Wallet info unavailable".to_string())?;
        let collection = managed_info.accounts();
        let account = collection
            .standard_bip44_accounts
            .get(&account_index)
            .ok_or_else(|| "BIP44 account missing".to_string())?;

        let mut spendable_total = 0u64;
        let mut spendable_inputs = 0usize;
        for utxo in account.utxos.values() {
            if (*utxo).is_spendable(current_height) {
                spendable_total = spendable_total.saturating_add(utxo.value());
                spendable_inputs += 1;
            }
        }

        if spendable_total == 0 || spendable_inputs == 0 {
            return Err("No spendable funds available".to_string());
        }

        let estimated_size = Self::estimate_p2pkh_tx_size(spendable_inputs, 1);
        let fee = FeeRate::normal().calculate_fee(estimated_size);
        Ok(spendable_total.saturating_sub(fee))
    }

    /// Build an unsigned payment transaction using TransactionBuilder.
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
            .set_fee_level(FeeLevel::Normal)
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
    ) -> Result<Transaction, String> {
        let wallet = wm
            .get_wallet(wallet_id)
            .ok_or_else(|| "Wallet object not found".to_string())?;
        let managed_info = wm
            .get_wallet_info(wallet_id)
            .ok_or_else(|| "Wallet info unavailable".to_string())?;
        let accounts = managed_info.accounts();
        let account = accounts
            .standard_bip44_accounts
            .get(&DEFAULT_BIP44_ACCOUNT_INDEX)
            .ok_or_else(|| "BIP44 account missing".to_string())?;

        let secp = Secp256k1::new();
        let mut tx_signed = tx;
        let cache = SighashCache::new(&tx_signed);

        let signing_data = tx_signed
            .input
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let utxo = account
                    .utxos
                    .get(&input.previous_output)
                    .ok_or_else(|| "Missing UTXO for signing".to_string())?;
                let sighash = cache
                    .legacy_signature_hash(index, &utxo.txout.script_pubkey, 1)
                    .map_err(|e| format!("Failed to compute signature hash: {e}"))?;
                Ok((sighash, utxo.address.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;

        for (input, (sighash, address)) in tx_signed.input.iter_mut().zip(signing_data.into_iter())
        {
            let digest: [u8; 32] = sighash.into();
            let message = Message::from_digest(digest);

            let addr_info = account
                .get_address_info(&address)
                .ok_or_else(|| "Address metadata missing".to_string())?;
            let secret_key = wallet
                .derive_private_key(&addr_info.path)
                .map_err(|e| format!("Failed to derive private key: {e}"))?;
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
