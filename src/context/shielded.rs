use std::sync::OnceLock;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::{TaskError, shielded_build_error};
use crate::backend_task::shielded::ShieldedTask;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::{ShieldedNote, ShieldedWalletState, derive_orchard_keys};
use dash_sdk::grovedb_commitment_tree::{
    ClientPersistentCommitmentTree, Nullifier, Position, ProvingKey,
};
use std::sync::Arc;

static PROVING_KEY: OnceLock<ProvingKey> = OnceLock::new();

/// Get or build the Halo 2 ProvingKey (cached for app lifetime).
///
/// The first call takes ~30 seconds to build the key. Subsequent calls return
/// immediately from the cache. Use `warm_up_proving_key()` on a background
/// thread at app startup to avoid blocking the user's first shielded operation.
pub fn get_proving_key() -> &'static ProvingKey {
    PROVING_KEY.get_or_init(ProvingKey::build)
}

/// Whether the proving key has already been built and cached.
pub fn is_proving_key_ready() -> bool {
    PROVING_KEY.get().is_some()
}

impl AppContext {
    /// Run a shielded pool task.
    pub async fn run_shielded_task(
        self: &Arc<Self>,
        task: ShieldedTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            ShieldedTask::WarmUpProvingKey => {
                let _ = get_proving_key();
                Ok(BackendTaskSuccessResult::ProvingKeyReady)
            }

            ShieldedTask::InitializeShieldedWallet { seed_hash } => {
                self.initialize_shielded_wallet(seed_hash)
            }

            ShieldedTask::SyncNotes { seed_hash } => self.sync_shielded_notes(seed_hash).await,

            ShieldedTask::ShieldCredits {
                seed_hash,
                amount,
                from_address,
                nonce_override,
            } => {
                self.shield_credits_task(seed_hash, amount, from_address, nonce_override)
                    .await
            }

            ShieldedTask::ShieldedTransfer {
                seed_hash,
                amount,
                recipient_address_bytes,
            } => {
                self.shielded_transfer_task(seed_hash, amount, recipient_address_bytes)
                    .await
            }

            ShieldedTask::UnshieldCredits {
                seed_hash,
                amount,
                to_platform_address,
            } => {
                self.unshield_credits_task(seed_hash, amount, to_platform_address)
                    .await
            }

            ShieldedTask::CheckNullifiers { seed_hash } => {
                self.check_nullifiers_task(seed_hash).await
            }

            ShieldedTask::ShieldFromAssetLock {
                seed_hash,
                amount_duffs,
            } => {
                self.shield_from_asset_lock_task(seed_hash, amount_duffs)
                    .await
            }

            ShieldedTask::ShieldedWithdrawal {
                seed_hash,
                amount,
                to_core_address,
            } => {
                self.shielded_withdrawal_task(seed_hash, amount, to_core_address)
                    .await
            }
        }
    }

    /// Increment the stored nonce for a platform address after a successful state transition.
    ///
    /// Updates both the in-memory wallet and the persisted DB record so that
    /// subsequent operations (single-shot or batch) read the correct next nonce
    /// without needing a full platform-address sync.
    pub fn bump_platform_address_nonce(
        &self,
        seed_hash: &WalletSeedHash,
        from_address: &dash_sdk::dpp::address_funds::PlatformAddress,
    ) {
        let wallets = self.wallets.read().unwrap();
        let wallet_arc = match wallets.get(seed_hash) {
            Some(w) => w.clone(),
            None => return,
        };
        drop(wallets);

        let mut wallet = wallet_arc.write().unwrap();
        // Find the matching entry (platform_address_info is keyed by core Address)
        let mut found: Option<(dash_sdk::dpp::dashcore::Address, u64, u32)> = None;
        for (core_addr, info) in wallet.platform_address_info.iter_mut() {
            if let Ok(pa) =
                dash_sdk::dpp::address_funds::PlatformAddress::try_from(core_addr.clone())
                && &pa == from_address
            {
                info.nonce += 1;
                found = Some((core_addr.clone(), info.balance, info.nonce));
                break;
            }
        }
        drop(wallet);

        // Persist updated nonce to DB
        if let Some((core_addr, balance, new_nonce)) = found {
            let _ = self.db.set_platform_address_info(
                seed_hash,
                &core_addr,
                balance,
                new_nonce,
                &self.network,
            );
        }
    }

    /// Get the default shielded payment address for a wallet.
    pub fn shielded_default_address(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Option<dash_sdk::grovedb_commitment_tree::PaymentAddress> {
        let states = self.shielded_states.lock().unwrap();
        states.get(seed_hash).map(|s| s.keys.default_address)
    }

    /// Initialize shielded wallet state by deriving ZIP32 keys from the wallet seed.
    fn initialize_shielded_wallet(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Check if already initialized
        {
            let states = self.shielded_states.lock().unwrap();
            if states.contains_key(&seed_hash) {
                let balance = states
                    .get(&seed_hash)
                    .map(|s| s.shielded_balance)
                    .unwrap_or(0);
                return Ok(BackendTaskSuccessResult::ShieldedInitialized { seed_hash, balance });
            }
        }

        // Get the wallet seed
        let seed_bytes = {
            let wallets = self.wallets.read().unwrap();
            let wallet_arc = wallets.get(&seed_hash).ok_or(TaskError::WalletNotFound)?;
            let wallet = wallet_arc.read().unwrap();
            match &wallet.wallet_seed {
                crate::model::wallet::WalletSeed::Open(open) => open.seed,
                crate::model::wallet::WalletSeed::Closed(_) => {
                    return Err(TaskError::WalletLocked);
                }
            }
        };

        let keys =
            derive_orchard_keys(&seed_bytes, self.network, 0).map_err(shielded_build_error)?;

        let network_str = self.network.to_string();

        let commitment_tree = ClientPersistentCommitmentTree::open_on_shared_connection(
            self.db.shared_connection(),
            100,
        )
        .map_err(|e| TaskError::ShieldedTreeUpdateFailed {
            detail: e.to_string(),
        })?;

        let mut last_synced_index = 0u64;

        // Resume from persisted tree state if available
        if let Ok(Some(pos)) = commitment_tree.max_leaf_position() {
            last_synced_index = u64::from(pos) + 1;
        }

        let (last_nullifier_sync_height, last_nullifier_sync_timestamp) = self
            .db
            .get_nullifier_sync_info(&seed_hash, &network_str)
            .unwrap_or((0, 0));

        let mut state = ShieldedWalletState {
            keys,
            notes: Vec::new(),
            commitment_tree: std::sync::Mutex::new(commitment_tree),
            last_synced_index,
            last_nullifier_sync_height,
            last_nullifier_sync_timestamp,
            shielded_balance: 0,
            last_notes_synced_at: None,
            last_nullifiers_synced_at: None,
        };

        // Load persisted notes from DB and reconstruct Note objects
        let note_rows = self
            .db
            .get_unspent_shielded_notes(&seed_hash, &network_str)?;
        for row in note_rows {
            if let Some(note) = crate::model::wallet::shielded::deserialize_note(&row.note_data)
                && let Some(nullifier) = Nullifier::from_bytes(&row.nullifier).into_option()
            {
                state.notes.push(ShieldedNote {
                    note,
                    position: Position::from(row.position),
                    cmx: row.cmx,
                    nullifier,
                    block_height: row.block_height,
                    is_spent: false,
                    value: row.value,
                });
            }
        }
        state.recalculate_balance();

        // Safety net: if the tree has been synced but no notes exist at all
        // (spent or unspent), force a full resync from index 0. This handles
        // the case where change notes from prior operations were only in memory
        // and the app restarted before the next sync persisted them.
        // We check ALL notes (including spent) to avoid a false positive when
        // the user legitimately spent everything.
        if state.last_synced_index > 0 && state.notes.is_empty() {
            let all_notes = self.db.get_all_shielded_notes(&seed_hash, &network_str)?;
            if all_notes.is_empty() {
                tracing::info!(
                    "Shielded init: tree synced to index {} but no notes in DB — forcing full resync",
                    state.last_synced_index,
                );
                self.db.clear_commitment_tree_tables()?;
                let fresh_tree = ClientPersistentCommitmentTree::open_on_shared_connection(
                    self.db.shared_connection(),
                    100,
                )
                .map_err(|e| TaskError::ShieldedTreeUpdateFailed {
                    detail: e.to_string(),
                })?;
                state.commitment_tree = std::sync::Mutex::new(fresh_tree);
                state.last_synced_index = 0;
            }
        }

        let balance = state.shielded_balance;

        let mut states = self.shielded_states.lock().unwrap();
        states.insert(seed_hash, state);

        Ok(BackendTaskSuccessResult::ShieldedInitialized { seed_hash, balance })
    }

    /// Sync shielded notes from platform.
    async fn sync_shielded_notes(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Take the state temporarily for the async operation
        let mut state = {
            let mut states = self.shielded_states.lock().unwrap();
            states.remove(&seed_hash).ok_or(TaskError::WalletNotFound)?
        };

        let result = crate::backend_task::shielded::sync::sync_notes(
            self,
            &seed_hash,
            &mut state,
            self.network,
        )
        .await;

        if result.is_ok() {
            state.last_notes_synced_at = Some(std::time::Instant::now());
        }

        // Put state back
        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(seed_hash, state);
        }

        let (new_notes, balance) = result?;
        Ok(BackendTaskSuccessResult::ShieldedNotesSynced {
            seed_hash,
            new_notes,
            balance,
        })
    }

    /// Shield credits from a platform address into the shielded pool.
    ///
    /// Unlike other shielded operations, shield_credits only needs the
    /// payment address from the shielded state (no tree or notes access).
    /// We read it without removing the state so parallel operations can share it.
    async fn shield_credits_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        from_address: dash_sdk::dpp::address_funds::PlatformAddress,
        nonce_override: Option<u32>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let default_address = {
            let states = self.shielded_states.lock().unwrap();
            let state = states.get(&seed_hash).ok_or(TaskError::WalletNotFound)?;
            state.keys.default_address
        };

        crate::backend_task::shielded::bundle::shield_credits(
            self,
            &seed_hash,
            &default_address,
            amount,
            from_address,
            nonce_override,
            None,
        )
        .await?;

        self.bump_platform_address_nonce(&seed_hash, &from_address);

        Ok(BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount })
    }

    /// Transfer credits within the shielded pool.
    async fn shielded_transfer_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        recipient_address_bytes: Vec<u8>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.with_anchor_retry(
            &seed_hash,
            "transfer",
            async |state: &ShieldedWalletState| {
                crate::backend_task::shielded::bundle::shielded_transfer(
                    self,
                    &seed_hash,
                    state,
                    amount,
                    &recipient_address_bytes,
                )
                .await
            },
        )
        .await?;
        Ok(BackendTaskSuccessResult::ShieldedTransferComplete { seed_hash, amount })
    }

    /// Unshield credits from the shielded pool to a platform address.
    async fn unshield_credits_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        to_platform_address: dash_sdk::dpp::address_funds::PlatformAddress,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.with_anchor_retry(
            &seed_hash,
            "unshield",
            async |state: &ShieldedWalletState| {
                crate::backend_task::shielded::bundle::unshield_credits(
                    self,
                    &seed_hash,
                    state,
                    amount,
                    to_platform_address,
                )
                .await
            },
        )
        .await?;
        Ok(BackendTaskSuccessResult::ShieldedCreditsUnshielded { seed_hash, amount })
    }

    /// Withdraw credits from the shielded pool to a core L1 address.
    async fn shielded_withdrawal_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        to_core_address: dash_sdk::dpp::dashcore::Address,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let to_core_address_clone = to_core_address.clone();
        self.with_anchor_retry(
            &seed_hash,
            "withdrawal",
            async |state: &ShieldedWalletState| {
                crate::backend_task::shielded::bundle::shielded_withdrawal(
                    self,
                    &seed_hash,
                    state,
                    amount,
                    to_core_address_clone.clone(),
                )
                .await
            },
        )
        .await?;
        Ok(BackendTaskSuccessResult::ShieldedWithdrawalComplete { seed_hash, amount })
    }

    /// Run a shielded operation with automatic retry on anchor mismatch.
    ///
    /// Takes the shielded state from the map, calls `operation`, and if it
    /// fails with `ShieldedAnchorMismatch`, syncs notes once and retries.
    /// On success, marks spent notes and puts the state back.
    async fn with_anchor_retry(
        self: &Arc<Self>,
        seed_hash: &WalletSeedHash,
        operation_name: &str,
        operation: impl AsyncFn(&ShieldedWalletState) -> Result<Vec<Nullifier>, TaskError>,
    ) -> Result<Vec<Nullifier>, TaskError> {
        let mut state = {
            let mut states = self.shielded_states.lock().unwrap();
            states.remove(seed_hash).ok_or(TaskError::WalletNotFound)?
        };

        let result = operation(&state).await;

        let result = if matches!(result, Err(TaskError::ShieldedAnchorMismatch { .. })) {
            tracing::info!(
                "Shielded anchor mismatch during {operation_name} — syncing notes and retrying"
            );
            crate::backend_task::shielded::sync::sync_notes(
                self,
                seed_hash,
                &mut state,
                self.network,
            )
            .await
            .map_err(|e| {
                tracing::warn!("Note sync after anchor mismatch failed: {e}");
                e
            })?;
            state.last_notes_synced_at = Some(std::time::Instant::now());
            operation(&state).await
        } else {
            result
        };

        if let Ok(ref spent_nullifiers) = result {
            let notes_before = state.unspent_notes().len();
            self.mark_notes_spent(seed_hash, &mut state, spent_nullifiers);
            let notes_after = state.unspent_notes().len();
            tracing::info!(
                "Shielded {operation_name}: marked {} note(s) spent (unspent notes: {} -> {}), new balance: {} credits",
                spent_nullifiers.len(),
                notes_before,
                notes_after,
                state.shielded_balance,
            );
        }

        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(*seed_hash, state);
        }

        result
    }

    /// Shield core DASH directly into the shielded pool via asset lock.
    async fn shield_from_asset_lock_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount_duffs: u64,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let state_ref = {
            let mut states = self.shielded_states.lock().unwrap();
            states.remove(&seed_hash).ok_or(TaskError::WalletNotFound)?
        };

        let result = crate::backend_task::shielded::bundle::shield_from_asset_lock(
            self,
            &seed_hash,
            &state_ref,
            amount_duffs,
        )
        .await;

        // Put state back
        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(seed_hash, state_ref);
        }

        let credits = result?;
        Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash,
            amount: credits,
        })
    }

    /// Check nullifiers to detect spent notes.
    async fn check_nullifiers_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut state = {
            let mut states = self.shielded_states.lock().unwrap();
            states.remove(&seed_hash).ok_or(TaskError::WalletNotFound)?
        };

        let result = crate::backend_task::shielded::nullifiers::check_nullifiers(
            self,
            &seed_hash,
            &mut state,
            self.network,
        )
        .await;

        if result.is_ok() {
            state.last_nullifiers_synced_at = Some(std::time::Instant::now());
        }

        // Put state back
        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(seed_hash, state);
        }

        let spent_count = result?;
        Ok(BackendTaskSuccessResult::ShieldedNullifiersChecked {
            seed_hash,
            spent_count,
        })
    }

    /// Mark notes as spent in both memory and DB after a successful broadcast.
    fn mark_notes_spent(
        &self,
        seed_hash: &WalletSeedHash,
        state: &mut ShieldedWalletState,
        spent_nullifiers: &[Nullifier],
    ) {
        let network_str = self.network.to_string();
        for nf in spent_nullifiers {
            let nf_bytes = nf.to_bytes();
            for note in &mut state.notes {
                if !note.is_spent && note.nullifier.to_bytes() == nf_bytes {
                    note.is_spent = true;
                    let _ = self
                        .db
                        .mark_shielded_note_spent(seed_hash, &nf_bytes, &network_str);
                }
            }
        }
        state.recalculate_balance();
    }
}
