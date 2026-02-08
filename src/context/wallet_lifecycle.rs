use crate::lock_helper::RwLockExt;
use crate::model::wallet::{
    AddressInfo as WalletAddressInfo, DerivationPathReference, DerivationPathType, Wallet,
    WalletSeedHash, WalletTransaction,
};
use crate::spv::{CoreBackendMode, SpvManager};
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::{
    ManagedWalletInfo, wallet_info_interface::WalletInfoInterface,
};
use std::sync::{Arc, RwLock};

use super::AppContext;

impl AppContext {
    pub fn spv_manager(&self) -> &Arc<SpvManager> {
        &self.spv_manager
    }

    pub fn clear_spv_data(&self) -> Result<(), String> {
        self.spv_manager.clear_data_dir()
    }

    pub fn clear_network_database(&self) -> Result<(), String> {
        self.db
            .clear_network_data(self.network)
            .map_err(|e| e.to_string())?;

        if let Ok(mut wallets) = self.wallets.write() {
            wallets.clear();
        }

        if let Ok(mut single_key_wallets) = self.single_key_wallets.write() {
            single_key_wallets.clear();
        }

        self.has_wallet
            .store(false, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    pub fn start_spv(self: &Arc<Self>) -> Result<(), String> {
        self.spv_manager.start()?;
        self.spv_setup_reconcile_listener();
        Ok(())
    }

    pub fn bootstrap_wallet_addresses(&self, wallet: &Arc<RwLock<Wallet>>) {
        if let Ok(mut guard) = wallet.write()
            && guard.known_addresses.is_empty()
        {
            tracing::info!(wallet = %hex::encode(guard.seed_hash()), "Bootstrapping wallet addresses");
            guard.bootstrap_known_addresses(self);
        }
    }

    pub fn handle_wallet_unlocked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        if let Some((seed_hash, seed_bytes)) = Self::wallet_seed_snapshot(wallet) {
            self.queue_spv_wallet_load(seed_hash, seed_bytes);
            // Note: Platform address sync is not done here.
            // Core UTXO refresh is handled at startup in bootstrap_loaded_wallets.
        }
    }

    pub fn handle_wallet_locked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        let seed_hash = match wallet.read() {
            Ok(guard) => guard.seed_hash(),
            Err(err) => {
                tracing::warn!(error = %err, "Unable to read wallet during lock handling");
                return;
            }
        };
        self.queue_spv_wallet_unload(seed_hash);
    }

    fn wallet_seed_snapshot(wallet: &Arc<RwLock<Wallet>>) -> Option<(WalletSeedHash, [u8; 64])> {
        let guard = wallet.read().ok()?;
        if !guard.is_open() {
            return None;
        }
        let seed_bytes = match guard.seed_bytes() {
            Ok(bytes) => *bytes,
            Err(err) => {
                tracing::warn!(error = %err, wallet = %hex::encode(guard.seed_hash()), "Unable to snapshot wallet seed for SPV load");
                return None;
            }
        };
        Some((guard.seed_hash(), seed_bytes))
    }

    fn queue_spv_wallet_load(self: &Arc<Self>, seed_hash: WalletSeedHash, seed_bytes: [u8; 64]) {
        let spv = Arc::clone(&self.spv_manager);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = spv.load_wallet_from_seed(seed_hash, seed_bytes).await {
                tracing::error!(seed = %hex::encode(seed_hash), %error, "Failed to load SPV wallet from seed");
            }
        });
    }

    fn queue_spv_wallet_unload(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let spv = Arc::clone(&self.spv_manager);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = spv.unload_wallet(seed_hash).await {
                tracing::error!(seed = %hex::encode(seed_hash), %error, "Failed to unload SPV wallet");
            }
        });
    }

    /// Queue automatic discovery of identities derived from a wallet.
    /// Checks identity indices 0 through max_identity_index for existing identities on the network.
    pub fn queue_wallet_identity_discovery(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
    ) {
        let ctx = Arc::clone(self);
        let wallet_clone = Arc::clone(wallet);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = ctx
                .discover_identities_from_wallet(&wallet_clone, max_identity_index)
                .await
            {
                tracing::warn!(
                    %error,
                    "Failed to discover identities from wallet"
                );
            }
        });
    }

    pub fn bootstrap_loaded_wallets(self: &Arc<Self>) {
        let wallets: Vec<_> = {
            let guard = self.wallets.read_or_recover();
            guard.values().cloned().collect()
        };

        for wallet in wallets.iter() {
            self.bootstrap_wallet_addresses(wallet);
            self.handle_wallet_unlocked(wallet);
        }

        // Auto-refresh UTXOs from Core on startup so balances are current
        // without requiring the user to manually click Refresh (fixes GH#522).
        // Only in RPC mode — SPV mode handles UTXO loading via reconciliation.
        if self.core_backend_mode() == CoreBackendMode::Rpc {
            for wallet in wallets {
                let ctx = Arc::clone(self);
                self.subtasks.spawn_sync(async move {
                    if let Err(e) =
                        tokio::task::spawn_blocking(move || ctx.refresh_wallet_info(wallet))
                            .await
                            .map_err(|e| format!("Task join error: {}", e))
                            .and_then(|r| r.map(|_| ()))
                    {
                        tracing::warn!("Failed to auto-refresh wallet UTXOs on startup: {}", e);
                    }
                });
            }

            let single_key_wallets: Vec<_> = {
                let guard = self.single_key_wallets.read_or_recover();
                guard.values().cloned().collect()
            };
            for wallet in single_key_wallets {
                let ctx = Arc::clone(self);
                self.subtasks.spawn_sync(async move {
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        ctx.refresh_single_key_wallet_info(wallet)
                    })
                    .await
                    .map_err(|e| format!("Task join error: {}", e))
                    .and_then(|r| r)
                    {
                        tracing::warn!(
                            "Failed to auto-refresh single key wallet UTXOs on startup: {}",
                            e
                        );
                    }
                });
            }
        }
    }

    /// Update wallet platform address info from SDK-returned AddressInfos.
    /// This uses the proof-verified data from SDK operations rather than fetching.
    pub(crate) fn update_wallet_platform_address_info_from_sdk(
        &self,
        seed_hash: WalletSeedHash,
        address_infos: &dash_sdk::query_types::AddressInfos,
    ) -> Result<(), String> {
        let wallet_arc = {
            let wallets = self.wallets.read_or_recover();
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

        for (platform_addr, maybe_info) in address_infos.iter() {
            if let Some(info) = maybe_info {
                // Convert PlatformAddress to core Address using the network
                let core_addr = platform_addr.to_address_with_network(self.network);

                // Update in-memory wallet state
                wallet.set_platform_address_info(core_addr.clone(), info.balance, info.nonce);

                // Update database (not a sync operation - preserve last_full_sync_balance
                // so the next terminal sync can correctly apply any pending AddToCredits)
                if let Err(e) = self.db.set_platform_address_info(
                    &seed_hash,
                    &core_addr,
                    info.balance,
                    info.nonce,
                    &self.network,
                    false, // Not a sync operation
                ) {
                    tracing::warn!("Failed to store Platform address info in database: {}", e);
                }

                tracing::debug!(
                    "Updated platform address {} balance={} nonce={} from SDK response",
                    core_addr,
                    info.balance,
                    info.nonce
                );
            }
        }

        Ok(())
    }

    pub(crate) fn register_spv_address(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        address: Address,
        derivation_path: DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
    ) -> Result<bool, String> {
        let mut guard = wallet.write().map_err(|e| e.to_string())?;
        if guard.known_addresses.contains_key(&address) {
            return Ok(false);
        }

        let (path_reference, path_type) =
            self.classify_derivation_metadata(&derivation_path, path_reference, path_type);

        let seed_hash = guard.seed_hash();

        self.db
            .add_address_if_not_exists(
                &seed_hash,
                &address,
                &self.network,
                &derivation_path,
                path_reference,
                path_type,
                None,
            )
            .map_err(|e| e.to_string())?;

        guard
            .known_addresses
            .insert(address.clone(), derivation_path.clone());
        guard.watched_addresses.insert(
            derivation_path,
            WalletAddressInfo {
                address,
                path_type,
                path_reference,
            },
        );

        Ok(true)
    }

    pub(crate) fn wallet_network_key(&self) -> WalletNetwork {
        match self.network {
            Network::Dash => WalletNetwork::Dash,
            Network::Testnet => WalletNetwork::Testnet,
            Network::Devnet => WalletNetwork::Devnet,
            Network::Regtest => WalletNetwork::Regtest,
            _ => WalletNetwork::Dash,
        }
    }

    fn sync_spv_account_addresses(
        &self,
        wallet_info: &ManagedWalletInfo,
        wallet_arc: &Arc<RwLock<Wallet>>,
    ) {
        let collection = wallet_info.accounts();

        let mut inserted = 0u32;
        for account in collection.all_accounts() {
            let account_type = account.account_type.to_account_type();
            if matches!(account_type, AccountType::Standard { .. }) {
                continue;
            }
            let Some((path_reference, path_type)) = Self::spv_account_metadata(&account_type)
            else {
                continue;
            };

            for address in account.account_type.all_addresses() {
                if let Some(info) = account.get_address_info(&address)
                    && let Ok(true) = self.register_spv_address(
                        wallet_arc,
                        address.clone(),
                        info.path.clone(),
                        path_type,
                        path_reference,
                    )
                {
                    inserted += 1;
                }
            }
        }

        if inserted > 0 {
            tracing::debug!(added = inserted, "Registered SPV-managed addresses");
        }
    }

    fn spv_account_metadata(
        account_type: &AccountType,
    ) -> Option<(DerivationPathReference, DerivationPathType)> {
        match account_type {
            AccountType::IdentityRegistration => Some((
                DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                DerivationPathType::CREDIT_FUNDING,
            )),
            AccountType::IdentityInvitation => Some((
                DerivationPathReference::BlockchainIdentityCreditInvitationFunding,
                DerivationPathType::CREDIT_FUNDING,
            )),
            AccountType::IdentityTopUp { .. } | AccountType::IdentityTopUpNotBoundToIdentity => {
                Some((
                    DerivationPathReference::BlockchainIdentityCreditTopupFunding,
                    DerivationPathType::CREDIT_FUNDING,
                ))
            }
            AccountType::Standard { .. } => Some((
                DerivationPathReference::BIP44,
                DerivationPathType::CLEAR_FUNDS,
            )),
            _ => None,
        }
    }

    fn classify_derivation_metadata(
        &self,
        derivation_path: &DerivationPath,
        default_ref: DerivationPathReference,
        default_type: DerivationPathType,
    ) -> (DerivationPathReference, DerivationPathType) {
        let components = derivation_path.as_ref();
        if components.len() >= 5
            && matches!(components[0], ChildNumber::Hardened { index: 9 })
            && matches!(components[2], ChildNumber::Hardened { index: 5 })
            && matches!(components[3], ChildNumber::Hardened { .. })
        {
            let hardened_leaf = matches!(components.last(), Some(ChildNumber::Hardened { .. }));
            if !hardened_leaf {
                return (
                    DerivationPathReference::BlockchainIdentities,
                    DerivationPathType::SINGLE_USER_AUTHENTICATION,
                );
            }
        }

        (default_ref, default_type)
    }

    /// Subscribe to SPV reconcile signals and debounce updates.
    pub fn spv_setup_reconcile_listener(self: &Arc<Self>) {
        use tokio::time::{Duration, Instant, sleep};
        let rx = self.spv_manager.register_reconcile_channel();
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync(async move {
            tokio::pin!(rx);
            let mut last = Instant::now();
            loop {
                tokio::select! {
                    maybe = rx.recv() => {
                        if maybe.is_none() { break; }
                        // simple debounce window
                        if last.elapsed() > Duration::from_millis(300) {
                            if let Err(e) = ctx.reconcile_spv_wallets().await { tracing::debug!("SPV reconcile error: {}", e); }
                            last = Instant::now();
                        } else {
                            sleep(Duration::from_millis(300)).await;
                            if let Err(e) = ctx.reconcile_spv_wallets().await { tracing::debug!("SPV reconcile error: {}", e); }
                            last = Instant::now();
                        }
                    }
                }
            }
        });
    }

    /// Reconcile SPV wallet state into DET.
    pub async fn reconcile_spv_wallets(&self) -> Result<(), String> {
        let wm_arc = self.spv_manager.wallet();
        let wm = wm_arc.read().await;
        let mapping = self.spv_manager.det_wallets_snapshot();

        // Take a snapshot of known addresses per wallet so we can scope DB updates
        let wallets_guard = self.wallets.read_or_recover();

        for (seed_hash, wallet_id) in mapping.iter() {
            // Log total balance for visibility
            let balance = wm
                .get_wallet_balance(wallet_id)
                .map_err(|e| format!("get_wallet_balance failed: {e}"))?;
            tracing::debug!(wallet = %hex::encode(seed_hash), spendable = balance.spendable(), unconfirmed = balance.unconfirmed, total = balance.total, "SPV balance snapshot");

            let Some(wallet_info) = wm.get_wallet_info(wallet_id) else {
                continue;
            };

            let Some(wallet_arc) = wallets_guard.get(seed_hash).cloned() else {
                continue;
            };

            self.sync_spv_account_addresses(wallet_info, &wallet_arc);

            if let Ok(mut wallet) = wallet_arc.write() {
                wallet.update_spv_balances(balance.spendable(), balance.unconfirmed, balance.total);
                // Persist balances to database
                if let Err(e) = self.db.update_wallet_balances(
                    seed_hash,
                    balance.spendable(),
                    balance.unconfirmed,
                    balance.total,
                ) {
                    tracing::warn!(wallet = %hex::encode(seed_hash), error = %e, "Failed to persist wallet balances");
                }
            }

            // Get the wallet's known addresses (only update those to avoid cross-wallet churn)
            let mut known_addresses: std::collections::BTreeSet<dash_sdk::dpp::dashcore::Address> = {
                let w = wallet_arc.read_or_recover();
                w.known_addresses.keys().cloned().collect()
            };

            // Clear existing UTXOs for these addresses in this network
            for addr in &known_addresses {
                let _ = self.db.execute(
                    "DELETE FROM utxos WHERE address = ? AND network = ?",
                    rusqlite::params![addr.to_string(), self.network.to_string()],
                );
            }

            // Read current UTXOs from SPV and re-insert, registering unknown addresses if derivation metadata is available
            let utxos = wm
                .wallet_utxos(wallet_id)
                .map_err(|e| format!("wallet_utxos failed: {e}"))?;

            use dash_sdk::dpp::dashcore::Address as CoreAddress;
            // no-op

            let mut per_address_sum: std::collections::BTreeMap<CoreAddress, u64> =
                Default::default();

            for u in utxos {
                // Best-effort accessors for outpoint/txout; adjust if API differs
                // Try field access (common struct layout): `outpoint` + `txout`
                let outpoint = u.outpoint;
                let tx_out = u.txout.clone();

                // Derive address from script
                let address = match CoreAddress::from_script(&tx_out.script_pubkey, self.network) {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                // If address unknown to DET, try to register using SPV metadata
                if !known_addresses.contains(&address) {
                    let collection = wallet_info.accounts();
                    let mut registered = false;
                    for acc in collection.all_accounts() {
                        if let Some(ai) = acc.get_address_info(&address) {
                            let account_type = acc.account_type.to_account_type();
                            let (path_reference, path_type) =
                                Self::spv_account_metadata(&account_type).unwrap_or((
                                    DerivationPathReference::BIP44,
                                    DerivationPathType::CLEAR_FUNDS,
                                ));

                            if let Ok(inserted) = self.register_spv_address(
                                &wallet_arc,
                                address.clone(),
                                ai.path.clone(),
                                path_type,
                                path_reference,
                            ) {
                                if inserted {
                                    known_addresses.insert(address.clone());
                                }
                                registered = true;
                            }
                            break;
                        }
                    }
                    if !registered {
                        continue;
                    }
                }

                // Insert UTXO row
                self.db
                    .insert_utxo(
                        outpoint.txid.as_ref(),
                        outpoint.vout,
                        &address,
                        tx_out.value,
                        &tx_out.script_pubkey.to_bytes(),
                        self.network,
                    )
                    .map_err(|e| e.to_string())?;

                // Sum per address for balance update
                *per_address_sum.entry(address).or_default() += tx_out.value;
            }

            // Write per-address balances into DB and wallet model
            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(mut w) = wref.write()
            {
                for (addr, sum) in per_address_sum.into_iter() {
                    // Update wallet and DB through model helper
                    let _ = w.update_address_balance(&addr, sum, self);
                }
            }

            let history = wm
                .wallet_transaction_history(wallet_id)
                .map_err(|e| format!("wallet_transaction_history failed: {e}"))?;
            let wallet_transactions: Vec<WalletTransaction> = history
                .into_iter()
                .map(|record| WalletTransaction {
                    txid: record.txid,
                    transaction: record.transaction.clone(),
                    timestamp: record.timestamp,
                    height: record.height,
                    block_hash: record.block_hash,
                    net_amount: record.net_amount,
                    fee: record.fee,
                    label: record.label.clone(),
                    is_ours: record.is_ours,
                })
                .collect();

            self.db
                .replace_wallet_transactions(seed_hash, &self.network, &wallet_transactions)
                .map_err(|e| e.to_string())?;

            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(mut wallet) = wref.write()
            {
                wallet.set_transactions(wallet_transactions.clone());
            }
        }

        Ok(())
    }

    pub fn stop_spv(&self) {
        self.spv_manager.stop();
    }
}
