use super::AppContext;
use super::get_transaction_info;
use crate::backend_task::error::TaskError;
use crate::database::is_unique_constraint_violation;
use crate::model::wallet::{
    AddressInfo as WalletAddressInfo, DerivationPathHelpers, DerivationPathReference,
    DerivationPathType, TransactionStatus, Wallet, WalletSeedHash, WalletTransaction,
};
use crate::spv::{AssetLockFinalityEvent, CoreBackendMode, SpvManager};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::{
    ManagedWalletInfo, wallet_info_interface::WalletInfoInterface,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub fn spv_manager(&self) -> &Arc<SpvManager> {
        &self.spv_manager
    }

    pub fn clear_spv_data(&self) -> Result<(), TaskError> {
        self.spv_manager
            .clear_data_dir()
            .map_err(|e| TaskError::SpvClearDataFailed { detail: e })
    }

    pub fn clear_network_database(&self) -> Result<(), TaskError> {
        self.db.clear_network_data(self.network)?;

        if let Ok(mut wallets) = self.wallets.write() {
            wallets.clear();
        }

        if let Ok(mut single_key_wallets) = self.single_key_wallets.write() {
            single_key_wallets.clear();
        }

        self.has_wallet.store(false, Ordering::Relaxed);

        Ok(())
    }

    pub fn start_spv(self: &Arc<Self>) -> Result<(), TaskError> {
        // Skip if SPV is already active — avoids orphaned listener tasks from
        // re-registering channels while existing handlers still hold old senders.
        if self.spv_manager.status().status.is_active() {
            return Ok(());
        }

        // Count wallets that will be loaded into SPV (open wallets with accessible seeds).
        // This is read synchronously so the SPV thread can wait for exactly this many.
        let expected_wallets = self
            .wallets
            .read()
            .map(|guard| {
                guard
                    .values()
                    .filter(|w| {
                        w.read()
                            .ok()
                            .is_some_and(|g| g.is_open() && g.seed_bytes().is_ok())
                    })
                    .count()
            })
            .unwrap_or(0);
        // Register reconcile channel BEFORE starting SPV so the event handlers
        // (spawned inside run_spv_loop) always capture a valid sender.
        self.spv_setup_reconcile_listener();
        self.spv_setup_finality_listener();
        self.spv_manager
            .start(expected_wallets)
            .map_err(|e| TaskError::SpvStartFailed { detail: e })?;
        // Immediately reflect the new SPV status in ConnectionStatus so the
        // UI sees the change on the next frame instead of waiting for the
        // next throttled trigger_refresh() cycle (2-10 seconds).
        self.connection_status
            .set_spv_status(self.spv_manager.status().status);
        self.connection_status.refresh_state();
        Ok(())
    }

    /// Persist a wallet to the database, register it in the in-memory map,
    /// save its known addresses, and load it into SPV if applicable.
    ///
    /// This is the single entry point for adding a wallet to the system.
    /// UI screens should call this after constructing a [`Wallet`] via
    /// [`Wallet::new_from_seed()`].
    pub fn register_wallet(
        self: &Arc<Self>,
        wallet: Wallet,
    ) -> Result<(WalletSeedHash, Arc<RwLock<Wallet>>), TaskError> {
        // 1. Persist wallet and known addresses atomically
        let addresses: Vec<_> = wallet
            .known_addresses
            .iter()
            .map(|(address, path)| {
                (
                    address,
                    path,
                    DerivationPathReference::BIP44,
                    DerivationPathType::CLEAR_FUNDS,
                )
            })
            .collect();

        self.db
            .store_wallet_with_addresses(&wallet, &self.network, &addresses)
            .map_err(|e| {
                if is_unique_constraint_violation(&e) {
                    TaskError::WalletAlreadyImported
                } else {
                    TaskError::Database { source: e }
                }
            })?;

        let seed_hash = wallet.seed_hash();

        // 2. Register in-memory
        let wallet_arc = Arc::new(RwLock::new(wallet));
        let mut wallets = self.wallets.write()?;
        wallets.insert(seed_hash, wallet_arc.clone());
        self.has_wallet.store(true, Ordering::Relaxed);
        drop(wallets);

        // 3. Bootstrap any additional addresses and load into SPV
        self.bootstrap_wallet_addresses(&wallet_arc);
        if self.core_backend_mode() == CoreBackendMode::Spv {
            self.handle_wallet_unlocked(&wallet_arc);
        }

        Ok((seed_hash, wallet_arc))
    }

    pub fn bootstrap_wallet_addresses(&self, wallet: &Arc<RwLock<Wallet>>) {
        if let Ok(mut guard) = wallet.write() {
            // Bootstrap when no addresses exist (fresh wallet) or when
            // platform payment addresses haven't been derived yet (wallet
            // created with only a Core address via new_from_seed).
            let has_platform_addresses = guard.watched_addresses.values().any(|info| {
                info.path_reference
                    == crate::model::wallet::DerivationPathReference::PlatformPayment
            });
            if guard.known_addresses.is_empty() || !has_platform_addresses {
                tracing::info!(wallet = %hex::encode(guard.seed_hash()), "Bootstrapping wallet addresses");
                guard.bootstrap_known_addresses(self);
            }
        }
    }

    pub fn handle_wallet_unlocked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        if let Some((seed_hash, seed_bytes)) = Self::wallet_seed_snapshot(wallet) {
            self.queue_spv_wallet_load(seed_hash, seed_bytes);
            // Note: Platform address sync is not done here.
            // Core UTXO refresh is handled at startup in bootstrap_loaded_wallets.

            // Initialize shielded wallet on a background thread to avoid
            // blocking the UI — ZIP32 key derivation and DB reads can stall.
            // After init completes, queue async SyncNotes -> CheckNullifiers.
            // This is the single init path — the UI never dispatches
            // InitializeShieldedWallet.
            self.queue_shielded_init_and_sync(seed_hash);
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

    /// Queue shielded wallet initialization on a blocking thread, then
    /// follow up with note sync + nullifier check. Tracked via `subtasks`
    /// so it participates in graceful shutdown and cancellation.
    fn queue_shielded_init_and_sync(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync("shielded_init", async move {
            let ctx2 = Arc::clone(&ctx);
            let init_result =
                tokio::task::spawn_blocking(move || ctx2.initialize_shielded_wallet(seed_hash))
                    .await;
            match init_result {
                Ok(Ok(_)) => {
                    tracing::trace!(
                        seed = %hex::encode(seed_hash),
                        "Shielded wallet state initialized on unlock"
                    );
                    ctx.run_shielded_sync(seed_hash).await;
                }
                Ok(Err(e)) => tracing::debug!(
                    seed = %hex::encode(seed_hash),
                    error = %e,
                    "Shielded wallet init skipped on unlock"
                ),
                Err(e) => tracing::debug!(
                    seed = %hex::encode(seed_hash),
                    error = %e,
                    "Shielded init task panicked"
                ),
            }
        });
    }

    /// Run SyncNotes -> CheckNullifiers sequence on a blocking thread.
    async fn run_shielded_sync(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let ctx = Arc::clone(self);
        let handle = tokio::runtime::Handle::current();
        let result = tokio::task::spawn_blocking(move || {
            handle.block_on(async {
                match ctx.sync_shielded_notes(seed_hash).await {
                    Ok(_) => {
                        if let Err(e) = ctx.check_nullifiers_task(seed_hash).await {
                            tracing::debug!(
                                seed = %hex::encode(seed_hash),
                                error = %e,
                                "Shielded nullifier check after init failed"
                            );
                        }
                    }
                    Err(e) => tracing::debug!(
                        seed = %hex::encode(seed_hash),
                        error = %e,
                        "Shielded note sync after init failed"
                    ),
                }
            })
        })
        .await;
        if let Err(e) = result {
            tracing::debug!(
                seed = %hex::encode(seed_hash),
                error = %e,
                "Shielded sync task panicked"
            );
        }
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
        self.subtasks.spawn_sync("spv_wallet_load", async move {
            if let Err(error) = spv.load_wallet_from_seed(seed_hash, seed_bytes).await {
                tracing::error!(seed = %hex::encode(seed_hash), %error, "Failed to load SPV wallet from seed");
            }
        });
    }

    fn queue_spv_wallet_unload(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let spv = Arc::clone(&self.spv_manager);
        self.subtasks.spawn_sync("spv_wallet_unload", async move {
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
        self.subtasks
            .spawn_sync("wallet_identity_discovery", async move {
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
            let guard = self.wallets.read().unwrap();
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
                self.subtasks
                    .spawn_sync("refresh_wallet_utxos", async move {
                        let result =
                            tokio::task::spawn_blocking(move || ctx.refresh_wallet_info(wallet))
                                .await;
                        match result {
                            Err(e) => tracing::warn!(
                                "Failed to auto-refresh wallet UTXOs on startup: {}",
                                e
                            ),
                            Ok(Err(e)) => tracing::warn!(
                                "Failed to auto-refresh wallet UTXOs on startup: {}",
                                e
                            ),
                            Ok(Ok(_)) => {}
                        }
                    });
            }

            let single_key_wallets: Vec<_> = {
                let guard = self.single_key_wallets.read().unwrap();
                guard.values().cloned().collect()
            };
            for wallet in single_key_wallets {
                let ctx = Arc::clone(self);
                self.subtasks
                    .spawn_sync("refresh_single_key_wallet_utxos", async move {
                        let result = tokio::task::spawn_blocking(move || {
                            ctx.refresh_single_key_wallet_info(wallet)
                        })
                        .await;
                        match result {
                            Err(e) => tracing::warn!(
                                "Failed to auto-refresh single key wallet UTXOs on startup: {}",
                                e
                            ),
                            Ok(Err(e)) => tracing::warn!(
                                "Failed to auto-refresh single key wallet UTXOs on startup: {}",
                                e
                            ),
                            Ok(Ok(())) => {}
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
    ) -> Result<(), TaskError> {
        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        let mut wallet = wallet_arc.write()?;

        for (platform_addr, maybe_info) in address_infos.iter() {
            if let Some(info) = maybe_info {
                // Convert PlatformAddress to core Address using the network
                let core_addr = platform_addr.to_address_with_network(self.network);

                // Update in-memory wallet state
                wallet.set_platform_address_info(core_addr.clone(), info.balance, info.nonce);

                // Update database
                if let Err(e) = self.db.set_platform_address_info(
                    &seed_hash,
                    &core_addr,
                    info.balance,
                    info.nonce,
                    &self.network,
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
    ) -> Result<bool, TaskError> {
        let mut guard = wallet.write()?;
        if guard.known_addresses.contains_key(&address) {
            return Ok(false);
        }

        let (path_reference, path_type) =
            self.classify_derivation_metadata(&derivation_path, path_reference, path_type);

        let seed_hash = guard.seed_hash();

        self.db.add_address_if_not_exists(
            &seed_hash,
            &address,
            &self.network,
            &derivation_path,
            path_reference,
            path_type,
            None,
        )?;

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
            Network::Mainnet => WalletNetwork::Mainnet,
            Network::Testnet => WalletNetwork::Testnet,
            Network::Devnet => WalletNetwork::Devnet,
            Network::Regtest => WalletNetwork::Regtest,
            other => {
                tracing::debug!(
                    ?other,
                    "Unknown Network variant, defaulting to Mainnet wallet key"
                );
                WalletNetwork::Mainnet
            }
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
            AccountType::ProviderVotingKeys => Some((
                DerivationPathReference::ProviderVotingKeys,
                DerivationPathType::CLEAR_FUNDS,
            )),
            AccountType::ProviderOwnerKeys => Some((
                DerivationPathReference::ProviderOwnerKeys,
                DerivationPathType::CLEAR_FUNDS,
            )),
            AccountType::ProviderOperatorKeys => Some((
                DerivationPathReference::ProviderOperatorKeys,
                DerivationPathType::CLEAR_FUNDS,
            )),
            AccountType::ProviderPlatformKeys => Some((
                DerivationPathReference::ProviderPlatformNodeKeys,
                DerivationPathType::CLEAR_FUNDS,
            )),
            // BlockchainIdentities addresses are bootstrapped by DET directly
            // (not via SDK WalletManager accounts) and registered with SPV
            // through register_spv_address() during wallet bootstrap. Other
            // account types (CoinJoin, DashPay, PlatformPayment, AssetLock*)
            // are either not yet supported or operate off-chain.
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

        if derivation_path.is_bip32() {
            return (DerivationPathReference::BIP32, default_type);
        }

        if derivation_path.is_bip44(self.network) {
            return (DerivationPathReference::BIP44, default_type);
        }

        (default_ref, default_type)
    }

    /// Listen for SPV instant lock / chain lock events and populate
    /// transactions_waiting_for_finality so identity registration can proceed.
    pub fn spv_setup_finality_listener(self: &Arc<Self>) {
        let rx = self.spv_manager.register_finality_channel();
        let ctx = Arc::clone(self);
        let cancel = self.subtasks.cancellation_token.clone();
        self.subtasks
            .spawn_sync("spv_finality_listener", async move {
                tokio::pin!(rx);
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        maybe = rx.recv() => {
                            let Some(event) = maybe else { break; };
                            // Wrap handler in select so cancellation can interrupt
                            // even when blocked on locks held by the SPV sync thread.
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                result = ctx.handle_spv_finality_event(event) => {
                                    if let Err(e) = result {
                                        tracing::debug!("SPV finality event error: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }
            });
    }

    async fn handle_spv_finality_event(
        &self,
        event: AssetLockFinalityEvent,
    ) -> Result<(), TaskError> {
        match event {
            AssetLockFinalityEvent::InstantLock { txid, instant_lock } => {
                // Check if this txid is pending in transactions_waiting_for_finality
                let is_pending = {
                    let transactions = self.transactions_waiting_for_finality.lock()?;
                    matches!(transactions.get(&txid), Some(None))
                };
                if !is_pending {
                    return Ok(());
                }

                // Retrieve the full transaction from the database
                let (tx, ..) = self
                    .db
                    .get_asset_lock_transaction(txid.as_byte_array())?
                    .ok_or(TaskError::AssetLockTransactionNotFoundInDatabase)?;

                self.received_asset_lock_finality(&tx, Some(*instant_lock), None)?;
            }
            AssetLockFinalityEvent::ChainLock {
                height: _height, ..
            } => {
                // Get all pending txids (where proof is None)
                let pending_txids: Vec<dash_sdk::dpp::dashcore::Txid> = {
                    let transactions = self.transactions_waiting_for_finality.lock()?;
                    transactions
                        .iter()
                        .filter_map(
                            |(txid, proof)| if proof.is_none() { Some(*txid) } else { None },
                        )
                        .collect()
                };
                if pending_txids.is_empty() {
                    return Ok(());
                }

                let sdk = self.sdk.load().as_ref().clone();

                for txid in pending_txids {
                    match get_transaction_info(&sdk, &txid).await {
                        Ok(tx_info) if tx_info.is_chain_locked && tx_info.height > 0 => {
                            if let Ok(Some((tx, ..))) =
                                self.db.get_asset_lock_transaction(txid.as_byte_array())
                            {
                                let _ = self.received_asset_lock_finality(
                                    &tx,
                                    None,
                                    Some(tx_info.height),
                                );
                            }
                        }
                        _ => {
                            // Transaction not yet chain-locked at this height, or DAPI
                            // lookup failed — will retry on next chain lock event.
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Subscribe to SPV reconcile signals and debounce updates.
    pub fn spv_setup_reconcile_listener(self: &Arc<Self>) {
        use tokio::time::{Duration, Instant, sleep};
        let rx = self.spv_manager.register_reconcile_channel();
        let ctx = Arc::clone(self);
        let cancel = self.subtasks.cancellation_token.clone();
        self.subtasks.spawn_sync("spv_reconcile_listener", async move {
            tokio::pin!(rx);
            let mut last = Instant::now();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    maybe = rx.recv() => {
                        if maybe.is_none() { break; }
                        // simple debounce window
                        if last.elapsed() > Duration::from_millis(300) {
                            // Wrap in select so cancellation can interrupt when
                            // blocked on locks held by the SPV sync thread.
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                result = ctx.reconcile_spv_wallets() => {
                                    if let Err(e) = result { tracing::debug!("SPV reconcile error: {}", e); }
                                }
                            }
                            last = Instant::now();
                        } else {
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                _ = sleep(Duration::from_millis(300)) => {}
                            }
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                result = ctx.reconcile_spv_wallets() => {
                                    if let Err(e) = result { tracing::debug!("SPV reconcile error: {}", e); }
                                }
                            }
                            last = Instant::now();
                        }
                    }
                }
            }
        });
    }

    /// Reconcile SPV wallet state into DET.
    pub async fn reconcile_spv_wallets(&self) -> Result<(), TaskError> {
        let wm_arc = self.spv_manager.wallet();
        let wm = wm_arc.read().await;
        let mapping = self.spv_manager.det_wallets_snapshot();

        // Take a snapshot of known addresses per wallet so we can scope DB updates
        let wallets_guard = self.wallets.read()?;

        for (seed_hash, wallet_id) in mapping.iter() {
            // Log total balance for visibility
            let balance = wm
                .get_wallet_balance(wallet_id)
                .map_err(|e| crate::spv::SpvError::WalletError(e.to_string()))?;
            tracing::debug!(wallet = %hex::encode(seed_hash), spendable = balance.spendable(), unconfirmed = balance.unconfirmed(), total = balance.total(), "SPV balance snapshot");

            let Some(wallet_info) = wm.get_wallet_info(wallet_id) else {
                continue;
            };

            let Some(wallet_arc) = wallets_guard.get(seed_hash).cloned() else {
                continue;
            };

            self.sync_spv_account_addresses(wallet_info, &wallet_arc);

            if let Ok(mut wallet) = wallet_arc.write() {
                wallet.update_spv_balances(
                    balance.spendable(),
                    balance.unconfirmed(),
                    balance.total(),
                );
                // Persist balances to database
                if let Err(e) = self.db.update_wallet_balances(
                    seed_hash,
                    balance.spendable(),
                    balance.unconfirmed(),
                    balance.total(),
                ) {
                    tracing::warn!(wallet = %hex::encode(seed_hash), error = %e, "Failed to persist wallet balances");
                }
            }

            // Get the wallet's known addresses (only update those to avoid cross-wallet churn)
            let mut known_addresses: std::collections::BTreeSet<Address> = {
                let w = wallet_arc.read()?;
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
                .map_err(|e| crate::spv::SpvError::WalletError(e.to_string()))?;

            let mut per_address_sum: std::collections::BTreeMap<Address, u64> = Default::default();
            // Build in-memory UTXO map to update wallet model
            let mut new_utxos: std::collections::HashMap<
                Address,
                std::collections::HashMap<
                    dash_sdk::dpp::dashcore::OutPoint,
                    dash_sdk::dpp::dashcore::TxOut,
                >,
            > = Default::default();

            for u in utxos {
                let outpoint = u.outpoint;
                let tx_out = u.txout.clone();

                // Derive address from script
                let address = match Address::from_script(&tx_out.script_pubkey, self.network) {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                // Always track the UTXO in the in-memory map for correct balance calculation
                new_utxos
                    .entry(address.clone())
                    .or_default()
                    .insert(outpoint, tx_out.clone());

                // Always count the UTXO value in per-address sum
                *per_address_sum.entry(address.clone()).or_default() += tx_out.value;

                // If address unknown to DET, try to register using SPV metadata
                if !known_addresses.contains(&address) {
                    let collection = wallet_info.accounts();
                    let mut registered = false;
                    for acc in collection.all_accounts() {
                        if let Some(ai) = acc.get_address_info(&address) {
                            let account_type = acc.account_type.to_account_type();
                            let (path_reference, path_type) =
                                Self::spv_account_metadata(&account_type).unwrap_or_else(|| {
                                    let default_ref = if ai.path.is_bip44(self.network) {
                                        DerivationPathReference::BIP44
                                    } else if ai.path.is_bip32() {
                                        DerivationPathReference::BIP32
                                    } else {
                                        tracing::warn!(
                                            path = %ai.path,
                                            "SPV address has unrecognized derivation path structure"
                                        );
                                        DerivationPathReference::Unknown
                                    };
                                    (default_ref, DerivationPathType::CLEAR_FUNDS)
                                });

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
                        tracing::debug!(
                            wallet = %hex::encode(seed_hash),
                            address = %address,
                            value = tx_out.value,
                            "SPV UTXO address not registered in DET (counted in balance but not in address table)"
                        );
                        // Still persist the UTXO to DB and delete stale entry first
                        let _ = self.db.execute(
                            "DELETE FROM utxos WHERE address = ? AND network = ?",
                            rusqlite::params![address.to_string(), self.network.to_string()],
                        );
                    }
                }

                // Insert UTXO row into DB
                self.db.insert_utxo(
                    outpoint.txid.as_ref(),
                    outpoint.vout,
                    &address,
                    tx_out.value,
                    &tx_out.script_pubkey.to_bytes(),
                    self.network,
                )?;
            }

            // Write per-address balances and UTXOs into wallet model
            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(mut w) = wref.write()
            {
                // Update in-memory UTXOs map
                w.utxos = new_utxos;

                // Zero out balances for known addresses that no longer have any UTXOs.
                // Without this, spent addresses retain stale non-zero balances because
                // per_address_sum only contains addresses with current UTXOs.
                for addr in &known_addresses {
                    if !w.utxos.contains_key(addr)
                        && let Err(e) = w.update_address_balance(addr, 0, self)
                    {
                        tracing::debug!(address = %addr, error = %e, "Failed to zero spent address balance");
                    }
                }

                for (addr, sum) in per_address_sum.into_iter() {
                    if let Err(e) = w.update_address_balance(&addr, sum, self) {
                        tracing::debug!(address = %addr, error = %e, "Failed to update address balance");
                    }
                }
            }

            let history = wm
                .wallet_transaction_history(wallet_id)
                .map_err(|e| crate::spv::SpvError::WalletError(e.to_string()))?;
            let wallet_transactions: Vec<WalletTransaction> = history
                .into_iter()
                .map(|record| {
                    let status = TransactionStatus::from_height(record.height);
                    WalletTransaction {
                        txid: record.txid,
                        transaction: record.transaction.clone(),
                        timestamp: record.timestamp,
                        height: record.height,
                        block_hash: record.block_hash,
                        net_amount: record.net_amount,
                        fee: record.fee,
                        label: record.label.clone(),
                        is_ours: spv_is_ours_override(record.is_ours, record.net_amount),
                        status,
                    }
                })
                .collect();

            tracing::info!(
                wallet = %hex::encode(seed_hash),
                spv_transactions = wallet_transactions.len(),
                spv_spendable = balance.spendable(),
                spv_total = balance.total(),
                "SPV reconcile summary"
            );

            // Only replace transactions if SPV returned some, to avoid wiping
            // previously persisted history when SPV hasn't populated history yet.
            if !wallet_transactions.is_empty() {
                self.db.replace_wallet_transactions(
                    seed_hash,
                    &self.network,
                    &wallet_transactions,
                )?;
            }

            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(mut wallet) = wref.write()
                && !wallet_transactions.is_empty()
            {
                wallet.set_transactions(wallet_transactions);
            }
        }

        Ok(())
    }

    pub fn stop_spv(&self) {
        self.spv_manager.stop();
        // Immediately reflect the new SPV status in ConnectionStatus so the
        // UI sees the change on the next frame instead of waiting for the
        // next throttled trigger_refresh() cycle (2-10 seconds).
        self.connection_status
            .set_spv_status(self.spv_manager.status().status);
        self.connection_status.refresh_state();
        // Reset the throttle timer so trigger_refresh() starts polling
        // at 200ms intervals and picks up the Stopped transition quickly.
        self.connection_status.reset_timer();
    }
}

/// SPV transaction history is per-wallet — all entries involve our addresses
/// (they passed bloom filter + `check_transaction()` address matching).
/// Upstream sets `is_ours` only for sends (`net_amount < 0`); we override
/// to `true` for all matched transactions since address ownership was
/// already verified by the SPV layer.
///
/// Bloom filter false positives are filtered by `check_transaction()` before
/// records reach this point, so the override is safe. Testing actual bloom
/// filter FP behavior would require mocking the SPV layer's bloom filter,
/// which is out of scope for unit tests.
fn spv_is_ours_override(upstream_is_ours: bool, net_amount: i64) -> bool {
    if !upstream_is_ours && net_amount >= 0 {
        tracing::debug!(
            net_amount,
            "SPV: overriding is_ours to true for receive transaction"
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ours_override_true_for_outgoing_already_ours() {
        assert!(spv_is_ours_override(true, -50_000));
    }

    #[test]
    fn is_ours_override_true_for_incoming_not_ours() {
        // Upstream marks receive transactions as !is_ours — we override.
        assert!(spv_is_ours_override(false, 100_000));
    }

    #[test]
    fn is_ours_override_true_for_zero_amount_not_ours() {
        // Edge case: net_amount == 0 (e.g. self-transfer minus fee)
        assert!(spv_is_ours_override(false, 0));
    }

    #[test]
    fn is_ours_override_true_for_outgoing_not_ours() {
        // Even if upstream says !is_ours for a send, we override.
        assert!(spv_is_ours_override(false, -10_000));
    }
}
