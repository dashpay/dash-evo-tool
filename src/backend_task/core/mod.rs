mod create_asset_lock;
mod recover_asset_locks;
mod refresh_single_key_wallet_info;
mod refresh_wallet_info;
mod send_single_key_wallet_payment;
mod start_dash_qt;

use crate::app_dir::core_cookie_path;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::wallet::WalletResult;
use crate::config::{Config, NetworkConfig};
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::fee_estimation::estimate_p2pkh_tx_size;
use crate::model::wallet::Wallet;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::spv::CoreBackendMode;
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
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeLevel;
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

use crate::backend_task::wallet::PlatformSyncMode;

#[derive(Debug, Clone)]
pub enum CoreTask {
    #[allow(dead_code)] // May be used for getting single chain lock
    GetBestChainLock,
    GetBestChainLocks,
    /// Refresh wallet info from Core. The optional PlatformSyncMode controls whether
    /// and how to sync Platform address balances:
    /// - None: Skip Platform sync entirely (Core only)
    /// - Some(mode): Sync Platform with the specified mode
    RefreshWalletInfo(Arc<RwLock<Wallet>>, Option<PlatformSyncMode>),
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

#[derive(Debug, Clone, PartialEq)]
pub enum CoreResult {
    Item(CoreItem),
}

impl AppContext {
    pub async fn run_core_task(
        self: &Arc<Self>,
        task: CoreTask,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            CoreTask::GetBestChainLock => self
                .core_client
                .read_or_recover()
                .get_best_chain_lock()
                .map(|chain_lock| {
                    BackendTaskSuccessResult::Core(CoreResult::Item(CoreItem::ChainLock(
                        chain_lock,
                        self.network,
                    )))
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

                // If all three failed, bail out with an error
                if mainnet_chainlock.is_none()
                    && testnet_chainlock.is_none()
                    && devnet_chainlock.is_none()
                    && local_chainlock.is_none()
                {
                    return Err(
                        "Failed to get best chain lock for mainnet, testnet, devnet, and local network"
                            .to_string(),
                    );
                }

                // Otherwise, return the successes we have
                Ok(BackendTaskSuccessResult::Core(CoreResult::Item(
                    CoreItem::ChainLocks(
                        mainnet_chainlock,
                        testnet_chainlock,
                        devnet_chainlock,
                        local_chainlock,
                    ),
                )))
            }
            CoreTask::RefreshWalletInfo(wallet, platform_sync_mode) => {
                // Get wallet seed hash for Platform balance refresh
                let seed_hash = {
                    let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
                    wallet_guard.seed_hash()
                };

                if self.core_backend_mode() == crate::spv::CoreBackendMode::Spv {
                    self.reconcile_spv_wallets()
                        .await
                        .map_err(|e| format!("Error refreshing wallet via SPV: {}", e))?;
                } else {
                    // Run blocking RPC calls on a dedicated thread pool to avoid freezing the UI
                    let ctx = self.clone();
                    tokio::task::spawn_blocking(move || ctx.refresh_wallet_info(wallet))
                        .await
                        .map_err(|e| format!("Task join error: {}", e))?
                        .map_err(|e| format!("Error refreshing wallet: {}", e))?;
                }

                // Also refresh Platform address balances if a sync mode is specified
                let warning = if let Some(sync_mode) = platform_sync_mode {
                    match self
                        .fetch_platform_address_balances(seed_hash, sync_mode)
                        .await
                    {
                        Ok(_) => None,
                        Err(e) => {
                            tracing::warn!("Failed to fetch Platform address balances: {}", e);
                            Some(format!("Platform sync failed: {}", e))
                        }
                    }
                } else {
                    None
                };

                Ok(BackendTaskSuccessResult::Wallet(WalletResult::Refreshed {
                    warning,
                }))
            }
            CoreTask::RefreshSingleKeyWalletInfo(wallet) => {
                // Run blocking RPC calls on a dedicated thread pool to avoid freezing the UI
                let ctx = self.clone();
                tokio::task::spawn_blocking(move || ctx.refresh_single_key_wallet_info(wallet))
                    .await
                    .map_err(|e| format!("Task join error: {}", e))?
                    .map_err(|e| format!("Error refreshing wallet: {}", e))?;
                Ok(BackendTaskSuccessResult::Wallet(WalletResult::Refreshed {
                    warning: None,
                }))
            }
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| e.to_string())
                .map(|_| BackendTaskSuccessResult::None),
            CoreTask::CreateRegistrationAssetLock(wallet, amount, identity_index) => self
                .create_registration_asset_lock(wallet, amount, true, identity_index)
                .map_err(|e| format!("Error creating asset lock: {}", e)),
            CoreTask::CreateTopUpAssetLock(wallet, amount, identity_index, top_up_index) => self
                .create_top_up_asset_lock(wallet, amount, true, identity_index, top_up_index)
                .map_err(|e| format!("Error creating top up asset lock: {}", e)),
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
                self.network,
                &parsed_recipients,
                DEFAULT_TX_FEE,
                request.subtract_fee_from_amount,
                Some(self),
            )?
        };

        let txid = self
            .core_client
            .read_or_recover()
            .send_raw_transaction(&tx)
            .map_err(|e| format!("Failed to broadcast transaction: {e}"))?;

        let total_amount: u64 = request.recipients.iter().map(|r| r.amount_duffs).sum();
        let recipients_result: Vec<(String, u64)> = request
            .recipients
            .iter()
            .map(|r| (r.address.clone(), r.amount_duffs))
            .collect();

        Ok(BackendTaskSuccessResult::Wallet(WalletResult::Payment {
            txid: txid.to_string(),
            recipients: recipients_result,
            total_amount,
        }))
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

        Ok(BackendTaskSuccessResult::Wallet(WalletResult::Payment {
            txid: tx.txid().to_string(),
            recipients: recipients_result,
            total_amount,
        }))
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
        let current_height = wm.current_height();
        let total_amount: u64 = recipients.iter().map(|(_, amt)| *amt).sum();
        let mut scale_factor = 1.0f64;
        let mut attempted_fallback = false;

        loop {
            let scaled_recipients: Vec<(Address, u64)> = recipients
                .iter()
                .map(|(addr, amt)| (addr.clone(), (*amt as f64 * scale_factor) as u64))
                .collect();

            match wm.create_unsigned_payment_transaction(
                wallet_id,
                DEFAULT_BIP44_ACCOUNT_INDEX,
                Some(AccountTypePreference::BIP44),
                scaled_recipients,
                FeeLevel::Normal,
                current_height,
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

        let estimated_size = estimate_p2pkh_tx_size(spendable_inputs, 1);
        let fee = FeeLevel::Normal.fee_rate().calculate_fee(estimated_size);
        Ok(spendable_total.saturating_sub(fee))
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
}
