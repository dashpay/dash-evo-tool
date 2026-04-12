use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::database::is_unique_constraint_violation;
use crate::model::feature_gate::FeatureGate;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData as QIPrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::{
    DerivationPathHelpers, DerivationPathReference, DerivationPathType, Wallet, WalletId,
};
use crate::platform_wallet_bridge::{
    ManagedDpnsNameInfo, ManagedIdentityStatus, ManagedKeyStorage, ManagedPrivateKeyData,
    PlatformWallet,
};
use crate::spv::CoreBackendMode;
use crate::spv::event_bridge::SpvEventBridge;
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zeroize::Zeroizing;

impl AppContext {
    pub fn spv_event_bridge(&self) -> &Arc<SpvEventBridge> {
        &self.spv_event_bridge
    }

    pub fn clear_spv_data(&self) -> Result<(), TaskError> {
        // Delete the SPV data directory directly (it's just files on disk).
        let spv_dir = self.data_dir.join("spv");
        if spv_dir.exists() {
            std::fs::remove_dir_all(&spv_dir).map_err(|e| TaskError::SpvClearDataFailed {
                detail: format!("Failed to remove SPV data directory: {e}"),
            })?;
        }
        std::fs::create_dir_all(&spv_dir).map_err(|e| TaskError::SpvClearDataFailed {
            detail: format!("Failed to re-create SPV data directory: {e}"),
        })?;
        Ok(())
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

    /// Get a `PlatformWallet` by wallet map key.
    ///
    /// The map is keyed by `WalletId` (post-v40). Returns `None` if the
    /// wallet doesn't exist or is locked (no platform_wallet).
    pub(crate) fn get_platform_wallet(
        &self,
        wallet_key: &WalletId,
    ) -> Option<Arc<PlatformWallet>> {
        self.wallets
            .read()
            .ok()
            .and_then(|wallets| wallets.get(wallet_key).cloned())
            .and_then(|w| w.read().ok().and_then(|g| g.platform_wallet.clone()))
    }

    /// Get any available `PlatformWallet`.
    ///
    /// Useful when the caller needs SDK access but doesn't care which wallet
    /// instance is used (e.g. DPNS resolution, identity fetches where the
    /// wallet derivation index is irrelevant).
    pub(crate) fn first_available_platform_wallet(&self) -> Option<Arc<PlatformWallet>> {
        self.wallets.read().ok().and_then(|wallets| {
            wallets
                .values()
                .find_map(|w| w.read().ok().and_then(|g| g.platform_wallet.clone()))
        })
    }

    /// Get a `PlatformWallet` by wallet map key, or return `TaskError::WalletNotFound`.
    ///
    /// Convenience wrapper for backend tasks that need the platform wallet.
    pub(crate) fn require_platform_wallet(
        &self,
        wallet_key: &WalletId,
    ) -> Result<Arc<PlatformWallet>, TaskError> {
        self.get_platform_wallet(wallet_key)
            .ok_or(TaskError::WalletNotFound)
    }

    /// Flush queued changesets from a `PlatformWallet` to SQLite.
    ///
    /// Calls [`PlatformWallet::flush_persist`] which delegates to the
    /// attached persister's `flush()`. If no persister is attached or nothing
    /// is queued the call is a no-op. Persistence failures are logged but
    /// never propagated — the in-memory state remains authoritative.
    ///
    /// With [`FlushStrategy::Immediate`](crate::changeset::FlushStrategy::Immediate)
    /// (the default), each `queue()` call auto-flushes, making explicit calls
    /// here unnecessary for most code paths. This method remains available for
    /// batch operations that use [`FlushStrategy::Manual`](crate::changeset::FlushStrategy::Manual).
    #[allow(dead_code)]
    pub(crate) fn flush_wallet_persistence(&self, platform_wallet: &PlatformWallet) {
        if let Err(e) = platform_wallet.flush_persist() {
            tracing::warn!(
                error = %e,
                "Failed to flush wallet persistence"
            );
        }
    }

    /// Get a `PlatformWallet` for the given `QualifiedIdentity`.
    ///
    /// Resolves the wallet seed hash from the identity's key derivation paths,
    /// then looks it up in the platform wallet bridge map.
    pub(crate) fn platform_wallet_for_identity(
        &self,
        identity: &crate::model::qualified_identity::QualifiedIdentity,
    ) -> Result<Arc<PlatformWallet>, TaskError> {
        let (wallet_id, _) = identity
            .determine_wallet_info()
            .map_err(|e| {
                tracing::error!("Failed to determine wallet info: {}", e);
                TaskError::WalletNotFound
            })?
            .ok_or(TaskError::WalletNotFound)?;
        self.require_platform_wallet(&wallet_id)
    }

    /// Reset SPV filter_committed_height to force a rescan from birth_height.
    ///
    /// Call before `start_spv()` when wallet state isn't persisted yet.
    pub async fn reset_spv_filter_committed_height(&self) {
        // TODO: re-wire after SpvRuntime exposes reset_filter_committed_height
        tracing::debug!(
            "reset_spv_filter_committed_height: not yet implemented in new SpvRuntime API"
        );
    }

    pub fn start_spv(self: &Arc<Self>) -> Result<(), TaskError> {
        // Skip if SPV is already running.
        if self.wallet_manager.spv().is_started() {
            tracing::info!("start_spv: SPV already started, skipping");
            return Ok(());
        }

        tracing::info!("start_spv: building SPV config...");
        let config = self.build_spv_config().map_err(|e| {
            tracing::error!("start_spv: failed to build config: {}", e);
            TaskError::SpvStartFailed { detail: e }
        })?;
        tracing::info!("start_spv: config built, starting SPV...");

        // Events now flow through PlatformEventHandler trait directly
        // (SpvEventBridge registered as handler in PlatformWalletManager::new).
        // No broadcast channel or run-loop needed.

        // Wire up the reconcile listener (debounces reconcile signals from
        // the event bridge and writes wallet state back to DET).
        // The reconcile channel is created fresh on each start so stale
        // signals from a previous session don't leak.
        // TODO: Re-enable reconcile listener once lock contention is resolved.
        // The reconcile listener acquires WalletManager read lock during heavy
        // work, which blocks SPV's write lock for process_block, causing the
        // sync pipeline to stall.
        // let reconcile_rx = self.spv_event_bridge.new_reconcile_channel();
        // self.spv_setup_reconcile_listener(reconcile_rx);

        // Spawn SPV sync via PlatformWalletManager's SpvRuntime.
        let cancel = self.subtasks.cancellation_token.child_token();
        // Store the cancel token so stop_spv() can cancel it.
        if let Ok(mut guard) = self.spv_cancel_token.lock() {
            *guard = Some(cancel.clone());
        }
        let wm = Arc::clone(&self.wallet_manager);
        let conn_status = Arc::clone(&self.connection_status);
        self.subtasks.spawn_sync("spv_main_loop", async move {
            if let Err(e) = wm.spv().run(config, cancel).await {
                tracing::error!(error = %e, "SPV runtime failed");
                conn_status.set_spv_last_error(Some(e.to_string()));
                conn_status.set_spv_status(crate::spv::SpvStatus::Error);
                conn_status.refresh_state();
            }
        });

        // Immediately reflect SPV Starting in ConnectionStatus.
        self.connection_status
            .set_spv_status(crate::spv::SpvStatus::Starting);
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
    ) -> Result<(WalletId, Arc<RwLock<Wallet>>), TaskError> {
        // 1. Persist wallet (no legacy address maps)
        self.db
            .store_wallet_with_addresses(&wallet, &self.network, &[])
            .map_err(|e| {
                if is_unique_constraint_violation(&e) {
                    TaskError::WalletAlreadyImported
                } else {
                    TaskError::Database { source: e }
                }
            })?;

        let map_key = wallet.wallet_id();

        // 2. Register in-memory
        let wallet_arc = Arc::new(RwLock::new(wallet));
        let mut wallets = self.wallets.write()?;
        wallets.insert(map_key, wallet_arc.clone());
        self.has_wallet.store(true, Ordering::Relaxed);
        drop(wallets);

        // 3. Create PlatformWallet and load into SPV
        self.handle_wallet_unlocked(&wallet_arc);

        Ok((map_key, wallet_arc))
    }

    pub fn bootstrap_wallet_addresses(&self, _wallet: &Arc<RwLock<Wallet>>) {
        // No-op: PlatformWallet's ManagedWalletInfo has all account types
        // with address pools from the seed. Locked wallets show no addresses
        // (address visibility requires authentication).
    }

    pub fn handle_wallet_unlocked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        if let Some((wallet_id, seed_bytes)) = Self::wallet_seed_snapshot(wallet) {
            // Register with the PlatformWalletManager (creates PlatformWallet
            // and wires SPV event channel). NOTE: this may re-key the wallets
            // map entry from wallet_id to wallet_id.
            self.register_with_platform_wallet_manager(wallet_id, seed_bytes);

            // After registration, use wallet_id for all subsequent lookups —
            // the map entry was re-keyed from wallet_id to wallet_id.
            let wallet_id = wallet
                .read()
                .ok()
                .map(|g| g.wallet_id())
                .unwrap_or(wallet_id);

            if FeatureGate::Shielded.is_available(self) {
                match self.initialize_shielded_wallet(wallet_id) {
                    Ok(_) => {
                        tracing::trace!(
                            wallet_id = %hex::encode(wallet_id),
                            "Shielded wallet state initialized on unlock"
                        );
                        self.queue_shielded_sync(wallet_id);
                    }
                    Err(e) => tracing::debug!(
                        wallet_id = %hex::encode(wallet_id),
                        error = %e,
                        "Shielded wallet init skipped on unlock"
                    ),
                }
            }
        }
    }

    /// Register an open wallet with the `PlatformWalletManager`.
    ///
    /// Creates a `PlatformWallet`, persists the computed `wallet_id` to
    /// the DB, and re-keys the `AppContext.wallets` map entry from
    /// `wallet_id` to `wallet_id`. If the wallet is already registered (e.g. from a
    /// previous unlock), this is a no-op.
    pub(crate) fn register_with_platform_wallet_manager(
        self: &Arc<Self>,
        wallet_id: WalletId,
        seed_bytes: [u8; 64],
    ) {
        // Check if already registered by looking at whether the wallet
        // already has a PlatformWallet attached.
        if let Ok(wallets) = self.wallets.read() {
            let already_registered = wallets.values().any(|w| {
                w.read()
                    .ok()
                    .map(|g| g.wallet_id() == wallet_id && g.platform_wallet.is_some())
                    .unwrap_or(false)
            });
            if already_registered {
                return;
            }
        }

        let kw_network = self.wallet_network_key();

        // Create a PlatformWallet via the manager — this wires the shared
        // SPV event channel so IS-lock/ChainLock events reach AssetLockManager.
        // The manager also initializes persisted state and registers the wallet
        // for SPV processing in one call.
        match tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.wallet_manager.create_wallet_from_seed_bytes(
                    kw_network,
                    seed_bytes,
                    Default::default(),
                ),
            )
        }) {
            Ok(platform_wallet) => {
                let wallet_id = platform_wallet.wallet_id();

                // Persist wallet_id to DB (no-op if already set).
                if let Err(e) = self.db.set_wallet_id(&wallet_id, &wallet_id) {
                    tracing::warn!(
                        error = %e,
                        seed = %hex::encode(wallet_id),
                        "Failed to persist wallet_id to database"
                    );
                }

                // Store platform_wallet on the Wallet struct. The map
                // is already keyed by wallet_id (populated at load time
                // or at creation; the migration screen ensures this).
                if let Ok(wallets) = self.wallets.read() {
                    if let Some(wallet_arc) = wallets.get(&wallet_id) {
                        if let Ok(mut wallet) = wallet_arc.write() {
                            wallet.platform_wallet = Some(Arc::clone(&platform_wallet));
                        }
                    }
                }

                tracing::info!(
                    seed = %hex::encode(wallet_id),
                    wallet_id = %hex::encode(wallet_id),
                    "Registered wallet with PlatformWallet bridge"
                );
            }
            Err(e) => {
                tracing::warn!(
                    seed = %hex::encode(wallet_id),
                    error = %e,
                    "Failed to create PlatformWallet from seed bytes for bridge"
                );
            }
        }
    }

    pub fn handle_wallet_locked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        let _map_key = match wallet.read() {
            Ok(guard) => guard.wallet_id(),
            Err(err) => {
                tracing::warn!(error = %err, "Unable to read wallet during lock handling");
                return;
            }
        };

        // Clear platform wallet from the Wallet struct
        if let Ok(mut guard) = wallet.write() {
            guard.platform_wallet = None;
        }

        // Note: we do NOT remove the wallet from the AppContext.wallets
        // map here — locking a wallet keeps it visible in the UI, just
        // without platform_wallet access. The map entry stays at its
        // current key (wallet_id or wallet_id fallback).
    }

    /// Initialize shielded state for unlocked wallets that were skipped
    /// because the protocol version wasn't known at unlock time.
    /// Called when the protocol version first crosses the shielded threshold.
    pub(crate) fn init_missing_shielded_wallets(self: &Arc<Self>) {
        // Collect candidate seed hashes while holding locks, then release
        // before calling initialize_shielded_wallet (which re-acquires both).
        let candidates: Vec<crate::model::wallet::WalletId> = (|| {
            let wallets = self.wallets.read().ok()?;
            let existing = self.shielded_states.lock().ok()?;
            Some(
                wallets
                    .iter()
                    .filter(|(hash, wallet_arc)| {
                        !existing.contains_key(*hash)
                            && wallet_arc.read().ok().map(|w| w.is_open()).unwrap_or(false)
                    })
                    .map(|(hash, _)| *hash)
                    .collect(),
            )
        })()
        .unwrap_or_default();

        for wallet_id in candidates {
            match self.initialize_shielded_wallet(wallet_id) {
                Ok(_) => {
                    tracing::info!(
                        seed = %hex::encode(wallet_id),
                        "Shielded wallet initialized after protocol version update"
                    );
                    self.queue_shielded_sync(wallet_id);
                }
                Err(e) => tracing::debug!(
                    seed = %hex::encode(wallet_id),
                    error = %e,
                    "Shielded wallet init failed after protocol version update"
                ),
            }
        }

    }

    /// Queue async SyncNotes -> CheckNullifiers for an already-initialized
    /// shielded wallet. Tracked via `subtasks` so it participates in graceful
    /// shutdown and cancellation.
    ///
    /// Uses `spawn_blocking(block_on(...))` because the async methods on
    /// `Arc<Self>` produce futures that borrow `self`, which the compiler
    /// cannot prove are `'static` (rust-lang/rust#100013). The trampoline
    /// resolves the futures synchronously on a blocking thread, satisfying
    /// the `'static` bound required by `spawn_sync`.
    fn queue_shielded_sync(self: &Arc<Self>, wallet_id: WalletId) {
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync("shielded_sync", async move {
            let handle = tokio::runtime::Handle::current();
            let result = tokio::task::spawn_blocking(move || {
                handle.block_on(async {
                    match ctx.sync_shielded_notes(wallet_id).await {
                        Ok(_) => {
                            if let Err(e) = ctx.check_nullifiers_task(wallet_id).await {
                                tracing::debug!(
                                    seed = %hex::encode(wallet_id),
                                    error = %e,
                                    "Shielded nullifier check after init failed"
                                );
                            }
                        }
                        Err(e) => tracing::debug!(
                            seed = %hex::encode(wallet_id),
                            error = %e,
                            "Shielded note sync after init failed"
                        ),
                    }
                })
            })
            .await;
            if let Err(e) = result {
                tracing::debug!(
                    seed = %hex::encode(wallet_id),
                    error = %e,
                    "Shielded sync task panicked"
                );
            }
        });
    }

    fn wallet_seed_snapshot(wallet: &Arc<RwLock<Wallet>>) -> Option<(WalletId, [u8; 64])> {
        let guard = wallet.read().ok()?;
        if !guard.is_open() {
            return None;
        }
        let seed_bytes = match guard.seed_bytes() {
            Ok(bytes) => *bytes,
            Err(err) => {
                tracing::warn!(error = %err, wallet = %hex::encode(guard.wallet_id()), "Unable to snapshot wallet seed for SPV load");
                return None;
            }
        };
        Some((guard.wallet_id(), seed_bytes))
    }

    // queue_spv_wallet_load and queue_spv_wallet_unload removed —
    // wallet registration is handled by PlatformWalletManager, and
    // SpvRuntime's WalletAdapter reads from the shared wallets map.

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
            // Create PlatformWallet first — it populates ManagedWalletInfo with
            // all account types and address pools from the seed. Then bootstrap
            // only if PlatformWallet wasn't created (locked wallet).
            self.handle_wallet_unlocked(wallet);
            self.bootstrap_wallet_addresses(wallet);
        }

        // Sync all DB identities to platform-wallet IdentityManagers so they
        // are available for DashPay and other platform-wallet operations.
        self.sync_all_identities_to_platform_wallets();

        // Register DashPay contact accounts in ManagedWalletInfo so SPV
        // monitors incoming payment addresses for established contacts.
        self.bootstrap_dashpay_contact_accounts();

        // Phase 10 6b: replay persisted account state
        // (`wallet_account_pool_state` rows + `utxos.is_instant_locked`)
        // now that DashPay contact accounts exist. The initial
        // `load_persisted` that runs during `PlatformWallet` creation
        // only hydrates state for accounts that existed at construction
        // time — DashPay contact accounts didn't exist yet. Re-applying
        // after bootstrap is idempotent (monotonic MAX on highest_used,
        // set-union on utxos_instant_locked) so standard accounts
        // already hydrated in pass 1 don't regress.
        self.replay_persisted_state_after_bootstrap();

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

    /// Update platform address info in DB from SDK-returned AddressInfos.
    /// This uses the proof-verified data from SDK operations rather than fetching.
    pub(crate) fn update_wallet_platform_address_info_from_sdk(
        &self,
        wallet_id: WalletId,
        address_infos: &dash_sdk::query_types::AddressInfos,
    ) -> Result<(), TaskError> {
        // Verify the wallet exists
        {
            let wallets = self.wallets.read()?;
            if !wallets.contains_key(&wallet_id) {
                return Err(TaskError::WalletNotFound);
            }
        }

        for (platform_addr, maybe_info) in address_infos.iter() {
            if let Some(info) = maybe_info {
                // Convert PlatformAddress to core Address using the network
                let core_addr = platform_addr.to_address_with_network(self.network);

                // Update database
                if let Err(e) = self.db.set_platform_address_info(
                    &wallet_id,
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

    pub(crate) async fn register_spv_address(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        address: Address,
        derivation_path: DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
    ) -> Result<bool, TaskError> {
        // Extract what we need from the wallet under a short-lived sync lock,
        // then drop the guard before any async work.
        let (platform_wallet, wallet_id) = {
            let guard = wallet.read()?;
            (guard.platform_wallet.clone(), guard.wallet_id())
        };

        // Check address ownership via PlatformWallet's async state
        // lock. Uses the per-pool O(1) `contains_address` (PC1 from
        // the performance review) rather than rebuilding the full
        // address catalog via `all_from_wallet_info`.
        if let Some(pw) = &platform_wallet {
            let info = pw.state().await;
            let has_it = info
                .core_wallet
                .accounts
                .all_accounts()
                .iter()
                .any(|a| a.contains_address(&address));
            if has_it {
                return Ok(false);
            }
        }

        let (path_reference, path_type) =
            self.classify_derivation_metadata(&derivation_path, path_reference, path_type);

        self.db.add_address_if_not_exists(
            &wallet_id,
            &address,
            &self.network,
            &derivation_path,
            path_reference,
            path_type,
            None,
        )?;

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

    async fn sync_spv_account_addresses(
        &self,
        wallet_info: &ManagedWalletInfo,
        wallet_arc: &Arc<RwLock<Wallet>>,
    ) {
        let collection = &wallet_info.accounts;

        let mut inserted = 0u32;
        for account in collection.all_accounts() {
            let account_type = account.account_type.to_account_type();
            let Some((path_reference, path_type)) = Self::spv_account_metadata(&account_type)
            else {
                continue;
            };

            for address in account.account_type.all_addresses() {
                if let Some(info) = account.get_address_info(&address)
                    && let Ok(true) = self
                        .register_spv_address(
                            wallet_arc,
                            address.clone(),
                            info.path.clone(),
                            path_type,
                            path_reference,
                        )
                        .await
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

    /// Subscribe to SPV reconcile signals and debounce updates.
    fn spv_setup_reconcile_listener(self: &Arc<Self>, rx: tokio::sync::mpsc::Receiver<()>) {
        use tokio::time::{Duration, Instant, sleep};
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
    ///
    /// Currently a no-op. Addresses and balances are managed by PlatformWallet
    /// (source of truth). Persistence will be handled by PlatformWalletPersistence
    /// in a future PR.
    pub async fn reconcile_spv_wallets(&self) -> Result<(), TaskError> {
        Ok(())
    }

    // -----------------------------------------------------------------
    // Platform-wallet IdentityManager synchronization
    // -----------------------------------------------------------------

    /// Sync a `QualifiedIdentity` to the platform-wallet's `IdentityManager`.
    ///
    /// Called after every insert/update so that the in-memory IdentityManager
    /// stays in sync with the SQLite database. This is best-effort: if the
    /// platform wallet is not available (e.g. external import, wallet locked)
    /// or the lock cannot be acquired, the error is logged and the caller
    /// is not affected.
    pub(crate) fn sync_identity_to_platform_wallet(&self, qualified_identity: &QualifiedIdentity) {
        // 1. Resolve the platform wallet for this identity
        let (wallet_id, _wallet_index) = match qualified_identity.determine_wallet_info() {
            Ok(Some(info)) => info,
            Ok(None) => {
                // No wallet association — external import or no derivation path
                tracing::trace!(
                    identity = %qualified_identity.identity.id(),
                    "Skipping platform-wallet sync: no wallet association"
                );
                return;
            }
            Err(e) => {
                tracing::debug!(
                    identity = %qualified_identity.identity.id(),
                    error = %e,
                    "Skipping platform-wallet sync: cannot determine wallet info"
                );
                return;
            }
        };

        let platform_wallet = match self.get_platform_wallet(&wallet_id) {
            Some(pw) => pw,
            None => {
                tracing::trace!(
                    identity = %qualified_identity.identity.id(),
                    seed = %hex::encode(wallet_id),
                    "Skipping platform-wallet sync: platform wallet not registered"
                );
                return;
            }
        };

        // 2. Access the identity_manager (tokio RwLock, use try_write)
        let mut wm_guard = match platform_wallet.wallet_manager().try_write() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!(
                    identity = %qualified_identity.identity.id(),
                    "Skipping platform-wallet sync: identity_manager lock contended"
                );
                return;
            }
        };

        let wallet_id = platform_wallet.wallet_id();
        let manager = match wm_guard.get_wallet_info_mut(&wallet_id) {
            Some(info) => info,
            None => {
                tracing::debug!(
                    identity = %qualified_identity.identity.id(),
                    "Skipping platform-wallet sync: wallet info not found"
                );
                return;
            }
        };

        let identity_id = qualified_identity.identity.id();
        let identity_index = qualified_identity.wallet_index.unwrap_or(0);

        // 3. Convert QualifiedIdentity data for the ManagedIdentity
        let mi_key_storage =
            Self::convert_key_storage(&qualified_identity.private_keys, &wallet_id);
        let mi_dpns_names = Self::convert_dpns_names(&qualified_identity.dpns_names);
        let mi_status = Self::convert_identity_status(qualified_identity.status);

        // 3b. Hydrate DashPay state (profile + payment history) from
        // SQL. The persister writes `dashpay_profiles` and
        // `dashpay_payments` on flush (Phase 9b-1, 9b-2), but
        // `SqliteWalletPersister::load()` is a no-op because it
        // can't construct a full `IdentityEntry` without access to
        // the identity blob. Instead, we rehydrate the DashPay
        // subset here, where we already have the identity and are
        // about to write it into the ManagedIdentity. This closes
        // the write-only persistence gap (data-integrity C1).
        let (mi_dashpay_profile, mi_dashpay_payments) =
            self.load_dashpay_state_for_identity(&identity_id);

        // 3c. Hydrate established contacts (Item 7c). Reads
        // `dashpay_contact_requests` rows with full DIP-15 crypto,
        // joins outgoing+incoming pairs, builds full
        // `ContactRequest` + `EstablishedContact` instances. Rows
        // with NULL crypto are skipped — those are either legacy
        // (pre-v38) or in-flight; the next background
        // `DashPayContactRequests` sync will repopulate them.
        let mi_established_contacts = self.load_established_contacts_for_identity(&identity_id);

        // 4. Add or update the identity in the manager
        if let Some(managed) = manager.identity_manager.managed_identity_mut(&identity_id) {
            // Update existing managed identity
            managed.identity = qualified_identity.identity.clone();
            managed.key_storage = mi_key_storage;
            managed.dpns_names = mi_dpns_names;
            managed.status = mi_status;
            managed.wallet_id = Some(wallet_id);
            managed.top_ups = qualified_identity.top_ups.clone();
            managed.dashpay_profile = mi_dashpay_profile;
            managed.dashpay_payments = mi_dashpay_payments;
            // Established contacts: extend rather than replace so
            // live-mutation entries (auto-established during this
            // session) aren't clobbered by a mid-session resync.
            for (contact_id, contact) in mi_established_contacts.clone() {
                managed
                    .established_contacts
                    .entry(contact_id)
                    .or_insert(contact);
            }
            if let Some(alias) = &qualified_identity.alias {
                managed.label = Some(alias.clone());
            }
            tracing::debug!(
                identity = %identity_id,
                "Updated identity in platform-wallet IdentityManager"
            );
        } else {
            // Add new identity
            // TODO(Phase 9a-5d): forward the returned changeset to the persister
            // instead of relying on the in-memory mutation alone.
            match manager
                .identity_manager
                .add_identity(qualified_identity.identity.clone(), identity_index)
            {
                Ok(_cs) => {
                    // Now set extra fields on the newly added managed identity
                    if let Some(managed) =
                        manager.identity_manager.managed_identity_mut(&identity_id)
                    {
                        managed.key_storage = mi_key_storage;
                        managed.dpns_names = mi_dpns_names;
                        managed.status = mi_status;
                        managed.wallet_id = Some(wallet_id);
                        managed.top_ups = qualified_identity.top_ups.clone();
                        managed.dashpay_profile = mi_dashpay_profile;
                        managed.dashpay_payments = mi_dashpay_payments;
                        managed.established_contacts = mi_established_contacts;
                        if let Some(alias) = &qualified_identity.alias {
                            managed.label = Some(alias.clone());
                        }
                    }
                    tracing::debug!(
                        identity = %identity_id,
                        index = identity_index,
                        "Added identity to platform-wallet IdentityManager"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        identity = %identity_id,
                        error = %e,
                        "Failed to add identity to platform-wallet IdentityManager"
                    );
                }
            }
        }
    }

    /// Hydrate the DashPay subset (`dashpay_profile` +
    /// `dashpay_payments`) for an identity from SQL. Called by
    /// `sync_identity_to_platform_wallet` to rebuild the state that
    /// the persister wrote on the previous run. Returns
    /// `(None, empty map)` if no rows exist or the SQL read fails.
    ///
    /// The persister writes these fields via `write_identity_dashpay_subset`
    /// on flush. `SqliteWalletPersister::load()` intentionally does
    /// NOT read them — the persister layer doesn't have access to
    /// the full `Identity` blob needed to construct an `IdentityEntry`,
    /// so the read happens here where the `QualifiedIdentity` is
    /// already in scope.
    fn load_dashpay_state_for_identity(
        &self,
        identity_id: &dash_sdk::platform::Identifier,
    ) -> (
        Option<platform_wallet::wallet::dashpay::DashPayProfile>,
        std::collections::BTreeMap<String, platform_wallet::wallet::dashpay::PaymentEntry>,
    ) {
        use platform_wallet::wallet::dashpay::{
            DashPayProfile, PaymentDirection, PaymentEntry, PaymentStatus,
        };

        let network_str = self.network.to_string();

        // Profile: straightforward map from StoredProfile to DashPayProfile.
        let profile = match self.db.load_dashpay_profile(identity_id, &network_str) {
            Ok(Some(stored)) => Some(DashPayProfile {
                display_name: stored.display_name,
                bio: stored.bio,
                avatar_url: stored.avatar_url,
                avatar_bytes: stored.avatar_bytes,
                public_message: stored.public_message,
            }),
            Ok(None) => None,
            Err(e) => {
                tracing::debug!(
                    identity = %identity_id,
                    error = %e,
                    "load_dashpay_state: profile read failed — leaving empty"
                );
                None
            }
        };

        // Payment history: convert StoredPayment rows to a
        // BTreeMap<tx_id, PaymentEntry>. `from_identity_id` vs
        // `to_identity_id` determines direction from the owner's
        // perspective.
        let mut payments = std::collections::BTreeMap::new();
        const LOAD_LIMIT: u32 = 10_000;
        let identity_bytes = identity_id.to_buffer();
        match self.db.load_payment_history(identity_id, LOAD_LIMIT) {
            Ok(stored_payments) => {
                for sp in stored_payments {
                    let (direction, counterparty_bytes) = if sp.from_identity_id == identity_bytes {
                        (PaymentDirection::Sent, &sp.to_identity_id)
                    } else {
                        (PaymentDirection::Received, &sp.from_identity_id)
                    };
                    let Ok(counterparty_id) =
                        dash_sdk::platform::Identifier::from_bytes(counterparty_bytes)
                    else {
                        tracing::debug!(
                            identity = %identity_id,
                            tx_id = %sp.tx_id,
                            "load_dashpay_state: invalid counterparty id — skipping payment"
                        );
                        continue;
                    };
                    let status = match sp.status.as_str() {
                        "confirmed" => PaymentStatus::Confirmed,
                        "failed" => PaymentStatus::Failed,
                        _ => PaymentStatus::Pending,
                    };
                    payments.insert(
                        sp.tx_id,
                        PaymentEntry {
                            counterparty_id,
                            amount_duffs: sp.amount as u64,
                            memo: sp.memo,
                            direction,
                            status,
                        },
                    );
                }
            }
            Err(e) => {
                tracing::debug!(
                    identity = %identity_id,
                    error = %e,
                    "load_dashpay_state: payment history read failed — leaving empty"
                );
            }
        }

        (profile, payments)
    }

    /// Item 7c: Hydrate `managed.established_contacts` from SQL.
    ///
    /// Loads every `dashpay_contact_requests` row for this identity
    /// where the DIP-15 crypto columns are populated (status =
    /// 'accepted'), groups them by `(from, to)` pair, and for each
    /// pair where BOTH directions exist builds a full
    /// `EstablishedContact` with real `ContactRequest` crypto. Rows
    /// with NULL crypto (legacy or mid-sync) are silently excluded
    /// by the SQL WHERE clause — the next background
    /// `DashPayContactRequests` sync repopulates them.
    ///
    /// Returns `BTreeMap<contact_identity_id, EstablishedContact>`
    /// ready to be assigned to `ManagedIdentity.established_contacts`.
    ///
    /// Does NOT touch key-wallet's receival-account state — that's
    /// separately bootstrapped by `bootstrap_dashpay_contact_accounts`.
    fn load_established_contacts_for_identity(
        &self,
        identity_id: &dash_sdk::platform::Identifier,
    ) -> std::collections::BTreeMap<
        dash_sdk::platform::Identifier,
        platform_wallet::wallet::dashpay::EstablishedContact,
    > {
        use platform_wallet::wallet::dashpay::{ContactRequest, EstablishedContact};

        let network_str = self.network.to_string();
        let rows = match self
            .db
            .load_contact_request_crypto_rows(identity_id, &network_str)
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::debug!(
                    identity = %identity_id,
                    error = %e,
                    "load_established_contacts: SQL read failed — returning empty map"
                );
                return std::collections::BTreeMap::new();
            }
        };

        // Group rows by `(from, to)` pair so we can find matching
        // outgoing + incoming for each contact.
        let mut by_pair: std::collections::BTreeMap<
            (
                dash_sdk::platform::Identifier,
                dash_sdk::platform::Identifier,
            ),
            ContactRequest,
        > = std::collections::BTreeMap::new();

        for (
            from_id,
            to_id,
            sender_key_index,
            recipient_key_index,
            account_reference,
            encrypted_public_key,
            encrypted_account_label_bytes,
            auto_accept_proof,
            core_height_created_at,
            platform_created_at_ms,
        ) in rows
        {
            let mut request = ContactRequest::new(
                from_id,
                to_id,
                sender_key_index,
                recipient_key_index,
                account_reference,
                encrypted_public_key,
                core_height_created_at,
                // The `platform_created_at_ms` column is already in
                // milliseconds (matching `ContactRequest.created_at:
                // TimestampMillis`). No conversion. See Item 7 review M2.
                platform_created_at_ms,
            );
            request.encrypted_account_label = encrypted_account_label_bytes;
            request.auto_accept_proof = auto_accept_proof;

            by_pair.insert((from_id, to_id), request);
        }

        // Join outgoing (owner → contact) with incoming (contact → owner).
        let mut result = std::collections::BTreeMap::new();
        for ((from_id, to_id), outgoing) in &by_pair {
            // Only process pairs where `from_id == identity_id`
            // (outgoing). The sibling incoming entry will be keyed
            // `(contact_id, identity_id)`.
            if *from_id != *identity_id {
                continue;
            }
            // S1: self-contact edge case. A malformed platform
            // contact request with `from == to == identity` would
            // otherwise build `EstablishedContact::new(owner, req,
            // req)` — semantically wrong (you can't be your own
            // contact). Skip.
            if *from_id == *to_id {
                tracing::debug!(
                    identity = %identity_id,
                    "load_established_contacts: self-contact row — skipping"
                );
                continue;
            }
            let contact_id = *to_id;
            let Some(incoming) = by_pair.get(&(contact_id, *identity_id)) else {
                tracing::debug!(
                    identity = %identity_id,
                    contact = %contact_id,
                    "load_established_contacts: outgoing without matching incoming — skipping"
                );
                continue;
            };
            result.insert(
                contact_id,
                EstablishedContact::new(contact_id, outgoing.clone(), incoming.clone()),
            );
        }

        if !result.is_empty() {
            tracing::debug!(
                identity = %identity_id,
                count = result.len(),
                "load_established_contacts: reconstructed from SQL"
            );
        }

        result
    }

    /// Sync all locally stored identities to platform-wallet IdentityManagers.
    ///
    /// Called during wallet bootstrap so identities loaded from the database
    /// are available to DashPay and other platform-wallet operations.
    pub fn sync_all_identities_to_platform_wallets(&self) {
        match self.load_local_qualified_identities() {
            Ok(identities) => {
                let count = identities.len();
                for identity in &identities {
                    self.sync_identity_to_platform_wallet(identity);
                }
                tracing::info!(
                    count,
                    "Synced local identities to platform-wallet IdentityManagers"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to load local identities for platform-wallet sync"
                );
            }
        }
    }

    /// Register DashPay contact accounts in ManagedWalletInfo for all
    /// established contacts loaded from the database.
    ///
    /// Called during wallet bootstrap (after identities are synced) so that
    /// SPV monitors incoming payment addresses for existing contacts.
    fn bootstrap_dashpay_contact_accounts(&self) {
        let network_str = self.network.to_string();

        // Load all identities to find their contacts
        let identities = match self.load_local_qualified_identities() {
            Ok(ids) => ids,
            Err(e) => {
                tracing::debug!(error = %e, "Skipping contact account bootstrap");
                return;
            }
        };

        let mut registered = 0u32;

        for identity in &identities {
            let identity_id = identity.identity.id();

            // Load contacts for this identity from DB
            let contacts = match self.db.load_dashpay_contacts(&identity_id, &network_str) {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(identity = %identity_id, error = %e, "Failed to load contacts");
                    continue;
                }
            };

            // Find the PlatformWallet for this identity's wallet
            let (wallet_id, _) = match identity.determine_wallet_info() {
                Ok(Some(info)) => info,
                _ => continue,
            };
            let pw = match self.get_platform_wallet(&wallet_id) {
                Some(pw) => pw,
                None => continue,
            };

            for contact in &contacts {
                if contact.contact_status != "accepted" {
                    continue;
                }

                let contact_id = match dash_sdk::dpp::prelude::Identifier::from_bytes(
                    &contact.contact_identity_id,
                ) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                // Register the contact account synchronously using blocking locks.
                // This creates a DashpayReceivingFunds account in ManagedWalletInfo
                // so SPV monitors incoming payment addresses for this contact.
                let account_type = AccountType::DashpayReceivingFunds {
                    index: 0,
                    user_identity_id: identity_id.to_buffer(),
                    friend_identity_id: contact_id.to_buffer(),
                };

                use dash_sdk::dpp::key_wallet::Account;
                use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreAccount;

                let kw_network = self.wallet_network_key();

                // Derive xpub and add account to key-wallet's Wallet (key store),
                // then add managed wrapper to ManagedWalletInfo (address pools).
                // Both live inside the single PlatformWalletInfo guard.
                if let Ok(mut wm_guard) = pw.wallet_manager().try_write() {
                    let wallet_id = pw.wallet_id();
                    let path = match account_type.derivation_path(kw_network) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::debug!(contact = %contact_id, error = %e, "Failed to derive contact path");
                            continue;
                        }
                    };
                    let account_xpub = match wm_guard
                        .get_wallet(&wallet_id)
                        .and_then(|w| w.derive_extended_public_key(&path).ok())
                    {
                        Some(xpub) => xpub,
                        None => {
                            tracing::debug!(contact = %contact_id, "Failed to derive contact xpub");
                            continue;
                        }
                    };
                    let account = Account {
                        parent_wallet_id: Some(wallet_id),
                        account_type,
                        network: kw_network,
                        account_xpub,
                        is_watch_only: false,
                    };
                    // TODO: re-wire wallet_mut().accounts.insert() after WalletManager
                    //       exposes mutable wallet access in the new API.

                    let managed = ManagedCoreAccount::from_account(&account);
                    if let Some(info) = wm_guard.get_wallet_info_mut(&wallet_id) {
                        if let Err(e) = info.core_wallet.accounts.insert(managed) {
                            tracing::debug!(contact = %contact_id, error = %e, "Failed to insert contact account");
                        } else {
                            registered += 1;
                        }
                    }
                }
            }
        }

        if registered > 0 {
            tracing::info!(count = registered, "Registered DashPay contact accounts");
        }
    }

    /// Phase 10 6b: replay persisted account state for every
    /// registered platform wallet after DashPay contact accounts
    /// have been bootstrapped.
    ///
    /// Reason: `PlatformWallet` construction calls `load_persisted`
    /// + `apply_changeset` once, but at that point only the base
    /// HD accounts derived from the seed exist. DashPay contact
    /// accounts (`DashpayReceivingFunds { user_id, friend_id }`)
    /// are registered later by `bootstrap_dashpay_contact_accounts`,
    /// so any persisted `wallet_account_pool_state` rows keyed on
    /// those account types are dropped with a warning during the
    /// initial apply (the bucket has no owning account to route
    /// into). Re-applying after bootstrap picks them up.
    ///
    /// The apply path is idempotent — monotonic MAX on
    /// `highest_used`, set-union on `utxos_instant_locked` — so
    /// standard accounts whose state was already applied in pass 1
    /// don't regress.
    fn replay_persisted_state_after_bootstrap(&self) {
        let platform_wallets: Vec<_> = {
            let Ok(wallets) = self.wallets.read() else {
                return;
            };
            wallets
                .values()
                .filter_map(|w| w.read().ok().and_then(|g| g.platform_wallet.clone()))
                .collect()
        };

        for pw in platform_wallets {
            let pw_clone = Arc::clone(&pw);
            self.subtasks
                .spawn_sync("replay_persisted_state", async move {
                    if let Err(e) = pw_clone.load_and_apply_persisted().await {
                        tracing::warn!(
                            error = %e,
                            "Phase 10 6b replay failed — persisted pool state not \
                             hydrated for this wallet; SPV replay will re-establish \
                             state from blocks forward"
                        );
                    }
                });
        }
    }

    /// Convert QualifiedIdentity's `KeyStorage` to ManagedIdentity's `KeyStorage`.
    ///
    /// Only keys with `PrivateKeyTarget::PrivateKeyOnMainIdentity` are converted.
    /// Voter/operator keys and encrypted keys are skipped.
    fn convert_key_storage(
        qi_keys: &crate::model::qualified_identity::encrypted_key_storage::KeyStorage,
        _wallet_id: &WalletId,
    ) -> ManagedKeyStorage {
        let mut result = ManagedKeyStorage::new();

        for ((target, key_id), (qualified_pub_key, private_key_data)) in qi_keys.private_keys.iter()
        {
            // Only convert main identity keys
            if *target != PrivateKeyTarget::PrivateKeyOnMainIdentity {
                continue;
            }

            let mi_private_key = match private_key_data {
                QIPrivateKeyData::Clear(bytes) | QIPrivateKeyData::AlwaysClear(bytes) => {
                    ManagedPrivateKeyData::Clear(Zeroizing::new(*bytes))
                }
                QIPrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_id,
                    derivation_path,
                }) => ManagedPrivateKeyData::AtWalletDerivationPath {
                    wallet_id: *wallet_id,
                    derivation_path: derivation_path.clone(),
                },
                QIPrivateKeyData::Encrypted(_) => {
                    // Cannot use encrypted keys without a password; skip
                    continue;
                }
            };

            result.insert(
                *key_id,
                (
                    qualified_pub_key.identity_public_key.clone(),
                    mi_private_key,
                ),
            );
        }

        result
    }

    /// Convert QualifiedIdentity's `DPNSNameInfo` vec to ManagedIdentity's `DpnsNameInfo` vec.
    fn convert_dpns_names(
        qi_names: &[crate::model::qualified_identity::DPNSNameInfo],
    ) -> Vec<ManagedDpnsNameInfo> {
        qi_names
            .iter()
            .map(|n| ManagedDpnsNameInfo {
                label: n.name.clone(),
                acquired_at: Some(n.acquired_at),
            })
            .collect()
    }

    /// Convert QualifiedIdentity's `IdentityStatus` to ManagedIdentity's `IdentityStatus`.
    fn convert_identity_status(
        qi_status: crate::model::qualified_identity::IdentityStatus,
    ) -> ManagedIdentityStatus {
        match qi_status {
            crate::model::qualified_identity::IdentityStatus::Unknown => {
                ManagedIdentityStatus::Unknown
            }
            crate::model::qualified_identity::IdentityStatus::PendingCreation => {
                ManagedIdentityStatus::PendingCreation
            }
            crate::model::qualified_identity::IdentityStatus::Active => {
                ManagedIdentityStatus::Active
            }
            crate::model::qualified_identity::IdentityStatus::NotFound => {
                ManagedIdentityStatus::NotFound
            }
            crate::model::qualified_identity::IdentityStatus::FailedCreation => {
                ManagedIdentityStatus::FailedCreation
            }
        }
    }

    pub fn stop_spv(&self) {
        // Cancel the SPV cancel token. This triggers the SpvRuntime::run()
        // future to exit (which stops the SPV client) and cascades to the
        // event bridge and reconcile listener (they exit when their channels
        // close or the global cancellation token fires).
        if let Ok(mut guard) = self.spv_cancel_token.lock() {
            if let Some(token) = guard.take() {
                token.cancel();
            }
        }

        // Immediately reflect the new SPV status in ConnectionStatus so the
        // UI sees the change on the next frame instead of waiting for the
        // next throttled trigger_refresh() cycle (2-10 seconds).
        self.connection_status
            .set_spv_status(crate::spv::SpvStatus::Stopped);
        self.connection_status.refresh_state();
        // Reset the throttle timer so trigger_refresh() starts polling
        // at 200ms intervals and picks up the Stopped transition quickly.
        self.connection_status.reset_timer();
    }

    /// Build a [`ClientConfig`] for the SPV runtime.
    ///
    /// Mirrors the logic from the former `SpvManager::build_client()`.
    fn build_spv_config(&self) -> Result<dash_sdk::dash_spv::ClientConfig, String> {
        use dash_sdk::dash_spv::ClientConfig;
        use dash_sdk::dash_spv::client::config::MempoolStrategy;
        use dash_sdk::dash_spv::types::ValidationMode;

        // Determine SPV data directory
        let cfg = self.config.read().map_err(|e| e.to_string())?;
        let data_dir = build_spv_data_dir(&self.data_dir, self.network, &cfg)?;
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| format!("Failed to create SPV data dir: {e}"))?;

        // Check if there are open wallets
        let has_wallets = self
            .wallets
            .read()
            .map(|g| {
                g.values()
                    .any(|w| w.read().ok().is_some_and(|g| g.is_open()))
            })
            .unwrap_or(false);

        let start_height = if has_wallets { 0 } else { u32::MAX };

        let mut config = ClientConfig::new(self.network)
            .with_storage_path(data_dir)
            .with_validation_mode(ValidationMode::Full)
            .with_start_height(start_height)
            .with_mempool_tracking(MempoolStrategy::BloomFilter);

        // Load user preference for local node
        let use_local_node = self.db.get_use_local_spv_node().unwrap_or(false);

        // Configure peer discovery based on network type and user preference.
        //
        // For devnet/regtest and local-node mode, use the configured Core
        // host as the single SPV peer.
        //
        // For mainnet/testnet with DNS discovery, ALSO seed the peer list
        // from the configured DAPI addresses (if set in .env). DAPI nodes
        // are masternodes that serve P2P on the standard port (9999 for
        // mainnet, 19999 for testnet). Using them avoids relying solely on
        // DNS seeds, which can resolve to stale nodes that don't support
        // compact block filters — causing CFHeaders timeouts and SPV sync
        // stalls.
        if self.network == Network::Devnet || self.network == Network::Regtest {
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
            }
        } else if use_local_node {
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
            }
        }

        // Seed SPV peers from DAPI addresses (all networks except local node).
        if !use_local_node {
            let p2p_port = match self.network {
                Network::Mainnet => 9999u16,
                Network::Testnet => 19999,
                _ => 19999,
            };
            if let Some(ref dapi_addrs) = cfg.dapi_addresses {
                for addr_str in dapi_addrs.split(',') {
                    // Parse "https://68.67.122.1:1443" → extract host IP
                    let trimmed = addr_str.trim();
                    if let Some(host) = trimmed
                        .strip_prefix("https://")
                        .or_else(|| trimmed.strip_prefix("http://"))
                    {
                        // Strip port (":1443") if present
                        let ip_str = host.split(':').next().unwrap_or(host);
                        if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                            config.add_peer(std::net::SocketAddr::new(ip, p2p_port));
                        }
                    }
                }
            }
        }

        Ok(config)
    }

    /// Resolve the primary peer socket address from config.
    fn primary_peer_socket(&self) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        let config = self.config.read().ok()?;
        let host = config.core_host.as_deref()?;
        let port = match self.network {
            Network::Mainnet => 9999,
            Network::Testnet => 19999,
            Network::Devnet => 20001,
            Network::Regtest => 19899,
            _ => 9999,
        };
        let addr = format!("{}:{}", host, port);
        addr.to_socket_addrs().ok()?.next()
    }
}

/// Build the SPV data directory path for the given network.
fn build_spv_data_dir(
    app_data_dir: &std::path::Path,
    network: Network,
    config: &crate::config::NetworkConfig,
) -> Result<std::path::PathBuf, String> {
    let mut base = app_data_dir.to_path_buf();
    base.push("spv");
    std::fs::create_dir_all(&base).map_err(|e| format!("Failed to create SPV base dir: {e}"))?;

    let network_dir = match network {
        Network::Mainnet => "mainnet".to_string(),
        Network::Testnet => "testnet".to_string(),
        Network::Devnet => {
            let name = config
                .devnet_name
                .clone()
                .unwrap_or_else(|| "devnet".to_string());
            let sanitized: String = name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if sanitized.is_empty() {
                "devnet".to_string()
            } else {
                sanitized
            }
        }
        Network::Regtest => "regtest".to_string(),
        other => format!("{other:?}"),
    };

    Ok(base.join(network_dir))
}
