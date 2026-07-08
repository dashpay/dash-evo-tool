use crate::app_dir::core_cookie_path;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::{TaskError, is_rpc_auth_error, is_rpc_connection_error};
use crate::config::{Config, NetworkConfig};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::wallet::single_key::SingleKeyWallet;
use dash_sdk::dashcore_rpc;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::{
    Address, Block, ChainLock, InstantLock, Network, OutPoint, Transaction, TxOut,
};
use dash_sdk::dpp::fee::Credits;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub enum CoreTask {
    #[allow(dead_code)] // May be used for getting single chain lock
    GetBestChainLock,
    GetBestChainLocks,
    /// Refresh wallet info from Core. The bool controls whether to also sync
    /// Platform address balances (true = sync Platform, false = Core only).
    RefreshWalletInfo(Arc<RwLock<Wallet>>, bool),
    RefreshSingleKeyWalletInfo(Arc<RwLock<SingleKeyWallet>>),
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
                };
                let active_rpc_error = if let Err(e) = active_result {
                    if let Some(task_err) =
                        Self::chain_lock_rpc_error(active_config, self.network, e)
                    {
                        return Err(task_err);
                    }
                    // Non-auth, non-connection error — show the actual error
                    // in the Networks page status display for debugging.
                    tracing::warn!(network = ?self.network, error = %e, "Chain lock query failed on active network");
                    Some(format!("RPC error: {e}"))
                } else {
                    // Successful chain lock fetch — clear any lingering RPC error
                    // so the connection status recovers after a transient outage.
                    self.connection_status.set_rpc_last_error(None);
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
                // Core wallet state (balances/UTXOs/transactions) is kept
                // current continuously by the upstream runtime and pushed via
                // the EventBridge — there is no explicit Core reconcile to
                // run. Ensure the backend exists, then optionally refresh the
                // DAPI-sourced Platform-address balances (retained DET path).
                self.wallet_backend()?;
                let seed_hash = {
                    let guard = wallet.read()?;
                    guard.seed_hash()
                };
                let warning = if sync_platform {
                    match self.fetch_platform_address_balances(seed_hash).await {
                        Ok(_) => None,
                        Err(e) => {
                            tracing::warn!("Failed to fetch Platform address balances: {}", e);
                            Some(format!("Platform sync failed: {e}"))
                        }
                    }
                } else {
                    None
                };
                Ok(BackendTaskSuccessResult::RefreshedWallet { warning })
            }
            // Single-key send/refresh unsupported this release — by design (single-key-mock.md, Decision #7).
            // TODO(PROJ-007 T3 refresh — PARKED on upstream): implementing balance/UTXO
            //   refresh for a bare imported P2PKH key needs UTXO discovery, which has no
            //   DET-local path. Per the F1 spike it requires (a) a key-wallet single-address
            //   pool/account helper (e.g. `AddressPool::with_single_address`) and (b) a public
            //   platform-wallet constructor `PlatformWalletManager::register_watch_only_wallet`
            //   that runs the existing private `register_wallet` body. Once those land, register
            //   the key as a degenerate watch-only wallet keyed by
            //   `seed_hash = SHA-256(SINGLE_KEY_NAMESPACE_BYTES ‖ addr)` and project
            //   `wallet_balance`/`utxos` into the `SingleKeyWallet` display fields.
            CoreTask::RefreshSingleKeyWalletInfo(_wallet) => {
                Err(TaskError::SingleKeyWalletsUnsupported)
            }
            CoreTask::CreateRegistrationAssetLock(wallet, amount, identity_index) => {
                let backend = self.wallet_backend()?;
                let seed_hash = wallet.read()?.seed_hash();
                let amount_duffs = amount / CREDITS_PER_DUFF;
                let (_, _, txid) = backend
                    .create_asset_lock_proof(
                        &seed_hash,
                        amount_duffs,
                        platform_wallet::AssetLockFundingType::IdentityRegistration,
                        identity_index,
                    )
                    .await?;
                Ok(BackendTaskSuccessResult::Message(format!(
                    "Asset lock transaction broadcast successfully. TX ID: {txid}"
                )))
            }
            CoreTask::CreateTopUpAssetLock(wallet, amount, identity_index, _top_up_index) => {
                let backend = self.wallet_backend()?;
                let seed_hash = wallet.read()?.seed_hash();
                let amount_duffs = amount / CREDITS_PER_DUFF;
                let (_, _, txid) = backend
                    .create_asset_lock_proof(
                        &seed_hash,
                        amount_duffs,
                        platform_wallet::AssetLockFundingType::IdentityTopUp,
                        identity_index,
                    )
                    .await?;
                Ok(BackendTaskSuccessResult::Message(format!(
                    "Asset lock transaction broadcast successfully. TX ID: {txid}"
                )))
            }
            CoreTask::SendWalletPayment { wallet, request } => {
                // `WalletBackend::send_payment` builds via the upstream
                // key-wallet `TransactionBuilder` with an internally-computed fee
                // and a fixed coin-selection strategy. It exposes only a fee
                // *rate*, not the absolute `override_fee` DET passes for a
                // min-relay retry. Rather than silently ignore that option —
                // which would send at a different fee than requested — reject it
                // with a typed error.
                if request.override_fee.is_some() {
                    return Err(TaskError::WalletPaymentOptionUnsupported);
                }
                // Backend-authoritative input validation: reject an empty
                // recipient list and any zero-amount recipient before building
                // a transaction (model validator is the single source of truth).
                let amounts: Vec<u64> = request.recipients.iter().map(|r| r.amount_duffs).collect();
                crate::model::wallet::validate_payment_recipients(&amounts)?;
                let backend = self.wallet_backend()?;
                let seed_hash = {
                    let guard = wallet.read()?;
                    guard.seed_hash()
                };
                let mut recipients = Vec::with_capacity(request.recipients.len());
                for r in &request.recipients {
                    let parsed = Address::from_str(&r.address).map_err(|source| {
                        TaskError::InvalidRecipientAddress {
                            address: r.address.clone(),
                            source,
                        }
                    })?;
                    let addr = parsed
                        .require_network(self.network)
                        .map_err(|source| TaskError::AddressNetworkMismatch { source })?;
                    recipients.push((addr, r.amount_duffs));
                }
                let total_amount: u64 = request.recipients.iter().map(|r| r.amount_duffs).sum();
                let result_recipients: Vec<(String, u64)> = request
                    .recipients
                    .iter()
                    .map(|r| (r.address.clone(), r.amount_duffs))
                    .collect();
                let txid = backend.send_payment(&seed_hash, recipients).await?;
                Ok(BackendTaskSuccessResult::WalletPayment {
                    txid: txid.to_string(),
                    recipients: result_recipients,
                    total_amount,
                })
            }
            // Single-key send/refresh unsupported this release — by design (single-key-mock.md, Decision #7).
            // TODO(PROJ-007 T4/T5 send — PARKED on upstream): broadcast itself is already wired
            //   and F1-independent (`WalletBackend::broadcast_transaction` →
            //   `SpvBroadcaster`). What is missing is coin selection over the imported key's
            //   UTXOs, which depends on the same UTXO-discovery upstream change as T3 (the
            //   key-wallet single-address pool helper + the platform-wallet
            //   `register_watch_only_wallet` constructor). Once UTXOs are discoverable, build a
            //   P2PKH tx from `utxos(seed_hash)`, sign via `DetSigner::SingleKey`, and broadcast.
            //   The T5 UI re-point (drop the dead `is_rpc_mode` gating in
            //   `single_key_send_screen.rs`, route fee math through `model/fee_estimation.rs`,
            //   and replace the string-parsed min-relay error with a typed `TaskError` variant)
            //   lands with this. Do NOT touch the parked `single_key_send_screen.rs` fee math.
            CoreTask::SendSingleKeyWalletPayment {
                wallet: _,
                request: _,
            } => Err(TaskError::SingleKeyWalletsUnsupported),
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

                let _ = wallet;
                let mined_count = mined.len() as u64;
                // Balances refresh via the EventBridge once sync observes the
                // mined block.
                Ok(BackendTaskSuccessResult::MineBlocksSuccess(mined_count))
            }
        }
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
            network_config.rpc_host(),
            network_config.rpc_port(network)
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
                tracing::trace!(
                    "Cookie auth unavailable at {:?}, using user/pass",
                    cookie_path
                );
                match Client::new(
                    &addr,
                    Auth::UserPass(
                        network_config.core_rpc_user.clone().unwrap_or_default(),
                        network_config.core_rpc_password.clone().unwrap_or_default(),
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
        network: Network,
        e: &dashcore_rpc::Error,
    ) -> Option<TaskError> {
        if is_rpc_auth_error(e) {
            return Some(TaskError::CoreRpcAuthFailed);
        }
        if is_rpc_connection_error(e) {
            let url = config
                .as_ref()
                .map(|c| format!("{}:{}", c.rpc_host(), c.rpc_port(network)))
                .unwrap_or_else(|| "unknown".to_string());
            return Some(TaskError::CoreRpcConnectionFailed { url, source: None });
        }
        None
    }
}

/// `override_fee` must NOT be silently ignored on `SendWalletPayment`.
/// Upstream `WalletBackend::send_payment` cannot express an absolute fee
/// override (only a fee rate), so the handler rejects it with a typed error
/// rather than sending at a different fee than requested. This lane proves
/// the rejection happens (no silent ignore).
#[cfg(test)]
mod send_payment_unsupported_options {
    use super::*;
    use crate::context::AppContext;
    use crate::model::wallet::Wallet;
    use dash_sdk::dpp::dashcore::Network;

    fn network_free_ctx(tmp: &std::path::Path) -> std::sync::Arc<AppContext> {
        crate::app_dir::ensure_env_file(tmp);
        let db_file = tmp.join("data.db");
        let db = std::sync::Arc::new(crate::database::Database::new(&db_file).expect("db"));
        // Force legacy wallet-family schema for tests — `initialize`
        // gates these out for truly-fresh installs post-T-DEV-01.
        db.create_tables(true).expect("create tables");
        db.set_default_version().expect("set version");
        let app_kv = AppContext::open_app_kv(tmp).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(tmp).expect("open secret store");
        AppContext::new(
            tmp.to_path_buf(),
            Network::Testnet,
            db,
            Default::default(),
            Default::default(),
            egui::Context::default(),
            app_kv,
            secret_store,
        )
        .expect("AppContext")
    }

    fn wallet_arc() -> Arc<RwLock<Wallet>> {
        let w = Wallet::new_from_seed([5u8; 64], Network::Testnet, Some("send-opts".into()), None)
            .expect("wallet");
        Arc::new(RwLock::new(w))
    }

    fn req(override_fee: Option<u64>) -> WalletPaymentRequest {
        WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: "yMLhEsf1bbDqM5p9LyrPHgM7g4Pvqp1Fbb".to_string(),
                amount_duffs: 10_000,
            }],
            override_fee,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn override_fee_is_rejected_not_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = network_free_ctx(tmp.path());
        let err = ctx
            .run_core_task(CoreTask::SendWalletPayment {
                wallet: wallet_arc(),
                request: req(Some(5_000)),
            })
            .await
            .expect_err("override_fee must be rejected");
        assert!(
            matches!(err, TaskError::WalletPaymentOptionUnsupported),
            "expected WalletPaymentOptionUnsupported, got {err:?}"
        );
    }
}
