use super::AppContext;
use super::get_transaction_info;
use crate::backend_task::error::TaskError;
use crate::database::is_unique_constraint_violation;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData as QIPrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::{
    AddressInfo as WalletAddressInfo, DerivationPathHelpers, DerivationPathReference,
    DerivationPathType, TransactionStatus, Wallet, WalletSeedHash, WalletTransaction,
};
use crate::platform_wallet_bridge::{
    ManagedDpnsNameInfo, ManagedIdentityStatus, ManagedKeyStorage, ManagedPrivateKeyData,
    PlatformWallet,
};
use crate::spv::{AssetLockFinalityEvent, CoreBackendMode, SpvManager};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::dpp::key_wallet::WalletCoreBalance;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::{
    ManagedWalletInfo, wallet_info_interface::WalletInfoInterface,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zeroize::Zeroizing;

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

        if let Ok(mut mapping) = self.wallet_id_mapping.lock() {
            *mapping = crate::platform_wallet_bridge::WalletIdMapping::new();
        }

        self.has_wallet.store(false, Ordering::Relaxed);

        Ok(())
    }

    /// Get a `PlatformWallet` by `WalletSeedHash`.
    ///
    /// Returns `None` if the wallet doesn't exist or is locked (no platform_wallet).
    pub(crate) fn get_platform_wallet(&self, seed_hash: &WalletSeedHash) -> Option<PlatformWallet> {
        self.wallets
            .read()
            .ok()
            .and_then(|wallets| wallets.get(seed_hash).cloned())
            .and_then(|w| w.read().ok().and_then(|g| g.platform_wallet.clone()))
    }

    /// Get any available `PlatformWallet`.
    ///
    /// Useful when the caller needs SDK access but doesn't care which wallet
    /// instance is used (e.g. DPNS resolution, identity fetches where the
    /// wallet derivation index is irrelevant).
    pub(crate) fn first_available_platform_wallet(&self) -> Option<PlatformWallet> {
        self.wallets
            .read()
            .ok()
            .and_then(|wallets| {
                wallets.values().find_map(|w| {
                    w.read().ok().and_then(|g| g.platform_wallet.clone())
                })
            })
    }

    /// Get a `PlatformWallet` by seed hash, or return `TaskError::WalletNotFound`.
    ///
    /// Convenience wrapper for backend tasks that need the platform wallet.
    pub(crate) fn require_platform_wallet(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<PlatformWallet, TaskError> {
        self.get_platform_wallet(seed_hash)
            .ok_or(TaskError::WalletNotFound)
    }

    /// Get a `PlatformWallet` for the given `QualifiedIdentity`.
    ///
    /// Resolves the wallet seed hash from the identity's key derivation paths,
    /// then looks it up in the platform wallet bridge map.
    pub(crate) fn platform_wallet_for_identity(
        &self,
        identity: &crate::model::qualified_identity::QualifiedIdentity,
    ) -> Result<PlatformWallet, TaskError> {
        let (seed_hash, _) = identity
            .determine_wallet_info()
            .map_err(|e| {
                tracing::error!("Failed to determine wallet info: {}", e);
                TaskError::WalletNotFound
            })?
            .ok_or(TaskError::WalletNotFound)?;
        self.require_platform_wallet(&seed_hash)
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
            // INTENTIONAL(CODE-006): Bootstrap checks only PlatformPayment address type.
            // Other platform address types may trigger redundant re-derivation, but
            // bootstrap_known_addresses() is idempotent so this is safe.
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
            // Also register with the new PlatformWalletManager (Phase 2 bridge)
            self.register_with_platform_wallet_manager(seed_hash, seed_bytes);
            // Note: Platform address sync is not done here.
            // Core UTXO refresh is handled at startup in bootstrap_loaded_wallets.

            // Initialize shielded wallet state only when the network supports it
            // (all shielded state transitions present in the platform version).
            // On mainnet (which doesn't support shielded transactions yet), skip
            // entirely to avoid unnecessary sync attempts and log noise.
            if crate::model::feature_gate::FeatureGate::Shielded.is_available(self) {
                match self.initialize_shielded_wallet(seed_hash) {
                    Ok(_) => {
                        tracing::trace!(
                            seed = %hex::encode(seed_hash),
                            "Shielded wallet state initialized on unlock"
                        );
                        self.queue_shielded_sync(seed_hash);
                    }
                    Err(e) => tracing::debug!(
                        seed = %hex::encode(seed_hash),
                        error = %e,
                        "Shielded wallet init skipped on unlock"
                    ),
                }
            }
        }
    }

    /// Register an open wallet with the `PlatformWalletManager` and populate
    /// the `wallet_id_mapping`.
    ///
    /// This bridges the old `Wallet` type to the new `PlatformWallet` type
    /// during migration. If the wallet is already registered (e.g. from a
    /// previous unlock), this is a no-op.
    pub(crate) fn register_with_platform_wallet_manager(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        seed_bytes: [u8; 64],
    ) {
        // Check if already registered
        if let Ok(mapping) = self.wallet_id_mapping.lock()
            && mapping.wallet_id_for_seed(&seed_hash).is_some()
        {
            return;
        }

        let kw_network = self.wallet_network_key();
        let sdk = self.sdk.load().as_ref().clone();
        let options = WalletAccountCreationOptions::default();

        // Create a PlatformWallet to discover its WalletId (SHA256 of root pubkey).
        // This is a lightweight, synchronous operation (no network I/O).
        match PlatformWallet::from_seed_bytes(sdk, kw_network, seed_bytes, options) {
            Ok(platform_wallet) => {
                let wallet_id = platform_wallet.wallet_id();

                // Update the bidirectional mapping
                if let Ok(mut mapping) = self.wallet_id_mapping.lock() {
                    mapping.insert(seed_hash, wallet_id);
                }

                // Store on the Wallet struct itself
                if let Ok(wallets) = self.wallets.read() {
                    if let Some(wallet_arc) = wallets.get(&seed_hash) {
                        if let Ok(mut wallet) = wallet_arc.write() {
                            wallet.platform_wallet = Some(platform_wallet.clone());
                        }
                    }
                }

                tracing::info!(
                    seed = %hex::encode(seed_hash),
                    wallet_id = %hex::encode(wallet_id),
                    "Registered wallet with PlatformWallet bridge"
                );
            }
            Err(e) => {
                tracing::warn!(
                    seed = %hex::encode(seed_hash),
                    error = %e,
                    "Failed to create PlatformWallet from seed bytes for bridge"
                );
            }
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

        // Clear platform wallet from the Wallet struct
        if let Ok(mut guard) = wallet.write() {
            guard.platform_wallet = None;
        }

        if let Ok(mut mapping) = self.wallet_id_mapping.lock() {
            mapping.remove_by_seed_hash(&seed_hash);
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
    fn queue_shielded_sync(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync("shielded_sync", async move {
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
        });
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

        // Sync all DB identities to platform-wallet IdentityManagers so they
        // are available for DashPay and other platform-wallet operations.
        self.sync_all_identities_to_platform_wallets();

        // Register DashPay contact accounts in ManagedWalletInfo so SPV
        // monitors incoming payment addresses for established contacts.
        self.bootstrap_dashpay_contact_accounts();

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
        if guard.has_address(&address) {
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

            // Sync balance to PlatformWallet's ManagedWalletInfo so it stays
            // in sync with SPV and can serve as the canonical read source.
            // Uses try_wallet_info_mut() to avoid holding the std::sync
            // wallets_guard across an await point. The WalletInfoWriteGuard
            // auto-refreshes WalletBalance on drop.
            if let Some(pw) = self.get_platform_wallet(seed_hash) {
                if let Some(mut pw_info) = pw.core().try_wallet_info_mut() {
                    pw_info.balance = WalletCoreBalance::new(
                        balance.spendable(),
                        balance.unconfirmed(),
                        0, // immature
                        0, // locked
                    );
                }
            }

            // Persist balances to database
            if let Err(e) = self.db.update_wallet_balances(
                seed_hash,
                balance.spendable(),
                balance.unconfirmed(),
                balance.total(),
            ) {
                tracing::warn!(wallet = %hex::encode(seed_hash), error = %e, "Failed to persist wallet balances");
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

            // Persist per-address balances to DB (UTXOs are tracked by
            // ManagedWalletInfo — no need to copy to Wallet.utxos).
            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(w) = wref.read()
            {
                for addr in &known_addresses {
                    if !per_address_sum.contains_key(addr) {
                        if let Err(e) = w.update_address_balance(addr, 0, self) {
                            tracing::debug!(address = %addr, error = %e, "Failed to zero spent address balance");
                        }
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
                    let height = record.height();
                    let status = TransactionStatus::from_height(height);
                    let block_info = record.context.block_info();
                    WalletTransaction {
                        txid: record.txid,
                        transaction: record.transaction.clone(),
                        timestamp: block_info.map(|bi| bi.timestamp() as u64).unwrap_or(0),
                        height,
                        block_hash: block_info.map(|bi| bi.block_hash()),
                        net_amount: record.net_amount,
                        fee: record.fee,
                        label: record.label.clone(),
                        is_ours: true,
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
    pub(crate) fn sync_identity_to_platform_wallet(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) {
        // 1. Resolve the platform wallet for this identity
        let (seed_hash, _wallet_index) = match qualified_identity.determine_wallet_info() {
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

        let platform_wallet = match self.get_platform_wallet(&seed_hash) {
            Some(pw) => pw,
            None => {
                tracing::trace!(
                    identity = %qualified_identity.identity.id(),
                    seed = %hex::encode(seed_hash),
                    "Skipping platform-wallet sync: platform wallet not registered"
                );
                return;
            }
        };

        // 2. Access the identity_manager (tokio RwLock, use try_write)
        let identity_wallet = platform_wallet.identity();
        let mut manager = match identity_wallet.try_identity_manager_mut() {
            Some(guard) => guard,
            None => {
                tracing::debug!(
                    identity = %qualified_identity.identity.id(),
                    "Skipping platform-wallet sync: identity_manager lock contended"
                );
                return;
            }
        };

        let identity_id = qualified_identity.identity.id();
        let identity_index = qualified_identity.wallet_index.unwrap_or(0);

        // 3. Convert QualifiedIdentity data for the ManagedIdentity
        let mi_key_storage = Self::convert_key_storage(
            &qualified_identity.private_keys,
            &seed_hash,
        );
        let mi_dpns_names = Self::convert_dpns_names(&qualified_identity.dpns_names);
        let mi_status = Self::convert_identity_status(qualified_identity.status);

        // 4. Add or update the identity in the manager
        if let Some(managed) = manager.managed_identity_mut(&identity_id) {
            // Update existing managed identity
            managed.identity = qualified_identity.identity.clone();
            managed.key_storage = mi_key_storage;
            managed.dpns_names = mi_dpns_names;
            managed.status = mi_status;
            managed.wallet_seed_hash = Some(seed_hash);
            managed.top_ups = qualified_identity.top_ups.clone();
            if let Some(alias) = &qualified_identity.alias {
                managed.label = Some(alias.clone());
            }
            tracing::debug!(
                identity = %identity_id,
                "Updated identity in platform-wallet IdentityManager"
            );
        } else {
            // Add new identity
            match manager.add_identity(
                qualified_identity.identity.clone(),
                identity_index,
            ) {
                Ok(()) => {
                    // Now set extra fields on the newly added managed identity
                    if let Some(managed) = manager.managed_identity_mut(&identity_id) {
                        managed.key_storage = mi_key_storage;
                        managed.dpns_names = mi_dpns_names;
                        managed.status = mi_status;
                        managed.wallet_seed_hash = Some(seed_hash);
                        managed.top_ups = qualified_identity.top_ups.clone();
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
            let (seed_hash, _) = match identity.determine_wallet_info() {
                Ok(Some(info)) => info,
                _ => continue,
            };
            let pw = match self.get_platform_wallet(&seed_hash) {
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

                // Derive xpub and add account to key-wallet's Wallet (key store)
                let account = {
                    let mut wallet = pw.core().blocking_wallet_mut();
                    let path = match account_type.derivation_path(kw_network) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::debug!(contact = %contact_id, error = %e, "Failed to derive contact path");
                            continue;
                        }
                    };
                    let account_xpub = match wallet.derive_extended_public_key(&path) {
                        Ok(xpub) => xpub,
                        Err(e) => {
                            tracing::debug!(contact = %contact_id, error = %e, "Failed to derive contact xpub");
                            continue;
                        }
                    };
                    let account = Account {
                        parent_wallet_id: Some(wallet.wallet_id),
                        account_type,
                        network: kw_network,
                        account_xpub,
                        is_watch_only: false,
                    };
                    let _ = wallet.accounts.insert(account.clone());
                    account
                };

                // Add managed wrapper to ManagedWalletInfo (address pools)
                let managed = ManagedCoreAccount::from_account(&account);
                if let Some(mut info) = pw.core().try_wallet_info_mut() {
                    if let Err(e) = info.accounts.insert(managed) {
                        tracing::debug!(contact = %contact_id, error = %e, "Failed to insert contact account");
                    } else {
                        registered += 1;
                    }
                }
            }
        }

        if registered > 0 {
            tracing::info!(count = registered, "Registered DashPay contact accounts");
        }
    }

    /// Convert QualifiedIdentity's `KeyStorage` to ManagedIdentity's `KeyStorage`.
    ///
    /// Only keys with `PrivateKeyTarget::PrivateKeyOnMainIdentity` are converted.
    /// Voter/operator keys and encrypted keys are skipped.
    fn convert_key_storage(
        qi_keys: &crate::model::qualified_identity::encrypted_key_storage::KeyStorage,
        _seed_hash: &WalletSeedHash,
    ) -> ManagedKeyStorage {
        let mut result = ManagedKeyStorage::new();

        for ((target, key_id), (qualified_pub_key, private_key_data)) in
            qi_keys.private_keys.iter()
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
                    wallet_seed_hash,
                    derivation_path,
                }) => ManagedPrivateKeyData::AtWalletDerivationPath {
                    wallet_seed_hash: *wallet_seed_hash,
                    derivation_path: derivation_path.clone(),
                },
                QIPrivateKeyData::Encrypted(_) => {
                    // Cannot use encrypted keys without a password; skip
                    continue;
                }
            };

            result.insert(
                *key_id,
                (qualified_pub_key.identity_public_key.clone(), mi_private_key),
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
