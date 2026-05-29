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
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// File name (under `<spv_dir>/<network>/`) for the shielded commitment tree.
///
/// A dedicated SQLite file, sibling to upstream's `platform-wallet.sqlite`.
/// The four `commitment_tree_*` tables grovedb creates live in this file
/// and never in DET's shared `data.db`.
pub(crate) const SHIELDED_COMMITMENT_TREE_FILE: &str = "shielded-commitment-tree.sqlite";

/// Resolve the per-network shielded commitment tree path inside `spv_dir`.
///
/// `spv_dir` is expected to be the already-network-scoped directory returned
/// by `WalletBackend::spv_storage_dir()` (i.e. it already includes the
/// `mainnet` / `testnet` / `devnet` / `regtest` segment).
pub(crate) fn shielded_commitment_tree_path(spv_dir: &Path) -> PathBuf {
    spv_dir.join(SHIELDED_COMMITMENT_TREE_FILE)
}

/// The four commitment-tree table names grovedb materialises on first use.
const COMMITMENT_TREE_TABLES: [&str; 4] = [
    "commitment_tree_shards",
    "commitment_tree_cap",
    "commitment_tree_checkpoints",
    "commitment_tree_checkpoint_marks_removed",
];

/// One-shot, silent migrator: copy legacy `commitment_tree_*` rows from the
/// shared `data.db` into a newly-created per-network shielded SQLite file.
///
/// Runs at most once per install (the gate is "the new file did not exist
/// before this `open_path` call"). Returns `Ok(())` when nothing needed to
/// move; returns `Err(...)` and deletes the partially-written destination
/// file on any failure, so the next launch can retry from scratch.
///
/// Legacy `data.db` rows are intentionally left in place; a future cleanup
/// pass will drop them once every install has migrated.
pub(crate) fn migrate_commitment_tree_if_needed(
    data_db_path: &Path,
    new_path: &Path,
) -> rusqlite::Result<()> {
    if !data_db_path.exists() {
        return Ok(());
    }

    let src = rusqlite::Connection::open(data_db_path)?;
    if !any_commitment_tree_rows(&src)? {
        return Ok(());
    }

    let new_path_str = new_path
        .to_str()
        .ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(format!(
                "shielded commitment tree path is not valid UTF-8: {}",
                new_path.display()
            ))
        })?
        .to_string();

    let result: rusqlite::Result<()> = (|| {
        src.execute_batch(&format!("ATTACH DATABASE '{new_path_str}' AS dest"))?;
        let copy = |conn: &rusqlite::Connection| -> rusqlite::Result<()> {
            for table in COMMITMENT_TREE_TABLES {
                conn.execute(
                    &format!("INSERT INTO dest.{table} SELECT * FROM main.{table}"),
                    [],
                )?;
            }
            Ok(())
        };
        let copy_result = copy(&src);
        // DETACH unconditionally, even on copy failure, so the connection is
        // left clean. Swallow detach errors — the copy error (if any) is the
        // one we want to surface.
        let _ = src.execute_batch("DETACH DATABASE dest");
        copy_result
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(new_path);
    }
    result
}

/// True when at least one of the four commitment-tree tables exists in the
/// given connection and contains at least one row.
fn any_commitment_tree_rows(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    for table in COMMITMENT_TREE_TABLES {
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get::<_, i32>(0).map(|c| c > 0),
        )?;
        if !exists {
            continue;
        }
        let count: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
        if count > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Open the per-network shielded commitment tree at `path`, running the
/// one-shot legacy migration from `data.db` on the first cold start where
/// the destination file does not yet exist.
fn open_commitment_tree_with_migration(
    path: &Path,
    data_db_path: Option<&Path>,
) -> Result<ClientPersistentCommitmentTree, TaskError> {
    let needs_migration = !path.exists() && data_db_path.is_some();

    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|source| TaskError::FileSystem { source })?;
    }

    let tree = ClientPersistentCommitmentTree::open_path(path, 100).map_err(|e| {
        TaskError::ShieldedTreeUpdateFailed {
            detail: e.to_string(),
        }
    })?;

    if needs_migration && let Some(data_db) = data_db_path {
        // Drop the freshly-opened tree handle so the destination file is
        // not held open by another connection while the migrator runs its
        // ATTACH/INSERT pass.
        drop(tree);
        migrate_commitment_tree_if_needed(data_db, path).map_err(|e| {
            TaskError::ShieldedTreeUpdateFailed {
                detail: format!("commitment-tree migration failed: {e}"),
            }
        })?;
        return ClientPersistentCommitmentTree::open_path(path, 100).map_err(|e| {
            TaskError::ShieldedTreeUpdateFailed {
                detail: e.to_string(),
            }
        });
    }

    Ok(tree)
}

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
    /// Resolve the on-disk path of this network's shielded commitment-tree
    /// SQLite file, materialising the parent directory if absent.
    ///
    /// The file is a sibling of the upstream platform-wallet persister at
    /// `<data_dir>/spv/<network>/shielded-commitment-tree.sqlite`. Requires
    /// the wallet backend to be initialised — callers reach this method on
    /// the shielded path, which only runs after backend init.
    pub(crate) fn shielded_commitment_tree_path(&self) -> Result<PathBuf, TaskError> {
        let backend = self.wallet_backend()?;
        let dir = backend.spv_storage_dir().to_path_buf();
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|source| TaskError::FileSystem { source })?;
        }
        Ok(shielded_commitment_tree_path(&dir))
    }

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
                source_address: _,
            } => {
                // `source_address` is ignored: coin selection is delegated to
                // the upstream wallet's authoritative live UTXO set at asset-
                // lock construction time. DET never selects spendable inputs.
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
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let wallet_arc = match wallets.get(seed_hash) {
            Some(w) => w.clone(),
            None => return,
        };
        drop(wallets);

        let mut wallet = wallet_arc.write().unwrap_or_else(|e| e.into_inner());
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

        // Persist updated nonce to k/v
        if let Some((core_addr, balance, new_nonce)) = found
            && let Err(e) =
                self.set_platform_address_info(seed_hash, &core_addr, balance, new_nonce)
        {
            tracing::warn!(error = ?e, "failed to persist platform address nonce bump");
        }
    }

    /// Set the cached nonce for a platform address to a specific value.
    /// Used during nonce mismatch retry to align the cache with Platform's
    /// expected nonce.
    fn set_platform_address_nonce(
        &self,
        seed_hash: &WalletSeedHash,
        from_address: &dash_sdk::dpp::address_funds::PlatformAddress,
        nonce: u32,
    ) {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let wallet_arc = match wallets.get(seed_hash) {
            Some(w) => w.clone(),
            None => return,
        };
        drop(wallets);

        let mut wallet = wallet_arc.write().unwrap_or_else(|e| e.into_inner());
        for (core_addr, info) in wallet.platform_address_info.iter_mut() {
            if let Ok(pa) =
                dash_sdk::dpp::address_funds::PlatformAddress::try_from(core_addr.clone())
                && &pa == from_address
            {
                info.nonce = nonce;
                if let Err(e) =
                    self.set_platform_address_info(seed_hash, core_addr, info.balance, nonce)
                {
                    tracing::warn!(error = ?e, "failed to persist platform address nonce override");
                }
                break;
            }
        }
    }

    /// Extract the expected nonce from an `AddressInvalidNonceError` inside
    /// a boxed SDK error. Returns `None` if the error chain doesn't contain
    /// the expected nonce.
    fn extract_expected_nonce(sdk_error: &dash_sdk::Error) -> Option<u32> {
        use dash_sdk::dpp::consensus::ConsensusError;
        use dash_sdk::dpp::consensus::state::state_error::StateError;
        let consensus = match sdk_error {
            dash_sdk::Error::StateTransitionBroadcastError(e) => e.cause.as_ref()?,
            dash_sdk::Error::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(ce)) => {
                ce.as_ref()
            }
            _ => return None,
        };
        if let ConsensusError::StateError(StateError::AddressInvalidNonceError(e)) = consensus {
            Some(e.expected_nonce())
        } else {
            None
        }
    }

    /// Get the default shielded payment address for a wallet.
    pub fn shielded_default_address(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Option<dash_sdk::grovedb_commitment_tree::PaymentAddress> {
        let states = self
            .shielded_states
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        states.get(seed_hash).map(|s| s.keys.default_address)
    }

    /// Initialize shielded wallet state by deriving ZIP32 keys from the wallet seed.
    pub(crate) fn initialize_shielded_wallet(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Check if already initialized
        {
            let states = self.shielded_states.lock()?;
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
            let wallets = self.wallets.read()?;
            let wallet_arc = wallets.get(&seed_hash).ok_or(TaskError::WalletNotFound)?;
            let wallet = wallet_arc.read()?;
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

        let tree_path = self.shielded_commitment_tree_path()?;
        let data_db_path = self.db.db_file_path();
        let commitment_tree =
            open_commitment_tree_with_migration(&tree_path, data_db_path.as_deref())?;

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
                tracing::warn!(
                    "Shielded init: wallet {} tree synced to index {} but no notes in DB — forcing full resync",
                    hex::encode(seed_hash.as_slice()),
                    state.last_synced_index,
                );
                // Unlink the per-network shielded SQLite file so the next
                // `open_path` materialises a pristine commitment tree. This
                // replaces the legacy in-place table truncation on `data.db`.
                if let Err(e) = std::fs::remove_file(&tree_path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(TaskError::FileSystem { source: e });
                }
                // No legacy `data.db` migration here: this branch is reached
                // only after a prior successful sync, so the upstream rows
                // (if any) are stale relative to the wallet.
                let fresh_tree = ClientPersistentCommitmentTree::open_path(&tree_path, 100)
                    .map_err(|e| TaskError::ShieldedTreeUpdateFailed {
                        detail: e.to_string(),
                    })?;
                state.commitment_tree = std::sync::Mutex::new(fresh_tree);
                state.last_synced_index = 0;
            }
        }

        let balance = state.shielded_balance;

        let mut states = self.shielded_states.lock()?;
        states.insert(seed_hash, state);

        Ok(BackendTaskSuccessResult::ShieldedInitialized { seed_hash, balance })
    }

    /// Sync shielded notes from platform.
    pub(crate) async fn sync_shielded_notes(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Take the state temporarily for the async operation
        let mut state = {
            let mut states = self.shielded_states.lock()?;
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
            let mut states = self.shielded_states.lock()?;
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
            let states = self.shielded_states.lock()?;
            let state = states.get(&seed_hash).ok_or(TaskError::WalletNotFound)?;
            state.keys.default_address
        };

        let result = crate::backend_task::shielded::bundle::shield_credits(
            self,
            &seed_hash,
            &default_address,
            amount,
            from_address,
            nonce_override,
            None,
        )
        .await;

        match result {
            Ok(()) => {
                self.bump_platform_address_nonce(&seed_hash, &from_address);
                Ok(BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount })
            }
            Err(TaskError::ShieldedNonceMismatch { ref source_error }) => {
                // Extract the expected nonce from the error and retry once.
                // The proof is already built — only the nonce in the state
                // transition needs updating. Unfortunately the current API
                // rebuilds the entire transition, but this avoids the user
                // having to manually retry.
                let expected_nonce = Self::extract_expected_nonce(source_error);
                if let Some(nonce) = expected_nonce {
                    tracing::info!(
                        "Shield credits: nonce mismatch, retrying with expected nonce {nonce}"
                    );
                    // Update cached nonce to expected - 1 so the retry reads expected
                    self.set_platform_address_nonce(
                        &seed_hash,
                        &from_address,
                        nonce.saturating_sub(1),
                    );

                    crate::backend_task::shielded::bundle::shield_credits(
                        self,
                        &seed_hash,
                        &default_address,
                        amount,
                        from_address,
                        Some(nonce),
                        None,
                    )
                    .await?;

                    self.bump_platform_address_nonce(&seed_hash, &from_address);
                    Ok(BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount })
                } else {
                    // Could not extract nonce — surface the original error
                    Err(result.unwrap_err())
                }
            }
            Err(e) => Err(e),
        }
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
            let mut states = self.shielded_states.lock()?;
            states.remove(seed_hash).ok_or(TaskError::WalletNotFound)?
        };

        let result = operation(&state).await;

        // INTENTIONAL(SEC-001): Shielded state removed from map during async operation.
        // Panic during shielded operation loses state for the session — restart recovers.

        let result = if matches!(result, Err(TaskError::ShieldedAnchorMismatch { .. })) {
            tracing::debug!(
                "Shielded anchor mismatch during {operation_name} — syncing notes and retrying"
            );
            let sync_result = crate::backend_task::shielded::sync::sync_notes(
                self,
                seed_hash,
                &mut state,
                self.network,
            )
            .await;
            match sync_result {
                Ok(_) => {
                    state.last_notes_synced_at = Some(std::time::Instant::now());
                    // Fix SEC-002: verify sufficient balance after sync before retrying
                    state.recalculate_balance();
                    if state.shielded_balance == 0 {
                        tracing::warn!(
                            "Shielded {operation_name}: no unspent balance after anchor retry sync"
                        );
                        Err(TaskError::ShieldedInsufficientBalance {
                            available: 0,
                            required: 1,
                        })
                    } else {
                        operation(&state).await
                    }
                }
                Err(e) => {
                    tracing::warn!("Note sync after anchor mismatch failed: {e}");
                    Err(e)
                }
            }
        } else {
            result
        };

        if let Ok(ref spent_nullifiers) = result {
            let notes_before = state.unspent_notes().len();
            self.mark_notes_spent(seed_hash, &mut state, spent_nullifiers);
            let notes_after = state.unspent_notes().len();
            tracing::debug!(
                "Shielded {operation_name}: marked {} note(s) spent (unspent notes: {} -> {}), new balance: {} credits",
                spent_nullifiers.len(),
                notes_before,
                notes_after,
                state.shielded_balance,
            );
        }

        {
            let mut states = self.shielded_states.lock()?;
            states.insert(*seed_hash, state);
        }

        result
    }

    /// Shield core DASH directly into the shielded pool via asset lock.
    ///
    /// Unlike operations that spend shielded notes (transfer, unshield, withdrawal),
    /// shield-from-asset-lock only reads the payment address — it doesn't use the
    /// commitment tree for witnesses, so anchor retry is not applicable.
    async fn shield_from_asset_lock_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount_duffs: u64,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let state_ref = {
            let mut states = self.shielded_states.lock()?;
            states.remove(&seed_hash).ok_or(TaskError::WalletNotFound)?
        };

        let result = crate::backend_task::shielded::bundle::shield_from_asset_lock(
            self,
            &seed_hash,
            &state_ref,
            amount_duffs,
        )
        .await;

        // Always put state back
        {
            let mut states = self.shielded_states.lock()?;
            states.insert(seed_hash, state_ref);
        }

        let credits = result?;
        Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash,
            amount: credits,
        })
    }

    /// Check nullifiers to detect spent notes.
    pub(crate) async fn check_nullifiers_task(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut state = {
            let mut states = self.shielded_states.lock()?;
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
            let mut states = self.shielded_states.lock()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Seed a `data.db`-style file with the four legacy `commitment_tree_*`
    /// tables, each holding a single sentinel row. Returns the file path.
    fn seed_legacy_data_db(dir: &Path) -> PathBuf {
        let path = dir.join("data.db");
        let conn = Connection::open(&path).expect("open seed data.db");
        // Use the exact upstream schema for `commitment_tree_*` (see
        // `grovedb-commitment-tree/.../sqlite_store/sql_helpers.rs`) so the
        // INSERT...SELECT migration into the production schema succeeds.
        conn.execute_batch(
            "CREATE TABLE commitment_tree_shards (
                shard_index INTEGER PRIMARY KEY,
                shard_data  BLOB NOT NULL
            );
            CREATE TABLE commitment_tree_cap (
                id       INTEGER PRIMARY KEY CHECK (id = 0),
                cap_data BLOB NOT NULL
            );
            CREATE TABLE commitment_tree_checkpoints (
                checkpoint_id INTEGER PRIMARY KEY,
                position      INTEGER
            );
            CREATE TABLE commitment_tree_checkpoint_marks_removed (
                checkpoint_id INTEGER NOT NULL,
                position      INTEGER NOT NULL,
                PRIMARY KEY (checkpoint_id, position),
                FOREIGN KEY (checkpoint_id) REFERENCES commitment_tree_checkpoints(checkpoint_id)
            );
            INSERT INTO commitment_tree_shards (shard_index, shard_data)
                VALUES (0, x'00');
            INSERT INTO commitment_tree_cap (id, cap_data) VALUES (0, x'11');
            INSERT INTO commitment_tree_checkpoints (checkpoint_id, position)
                VALUES (1, 42);
            INSERT INTO commitment_tree_checkpoint_marks_removed (checkpoint_id, position)
                VALUES (1, 7);
            ",
        )
        .expect("seed schema + rows");
        drop(conn);
        path
    }

    fn row_count(path: &Path, table: &str) -> i64 {
        let conn = Connection::open(path).expect("open for count");
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count row")
    }

    #[test]
    fn migrator_copies_all_four_tables_into_fresh_destination() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_db = seed_legacy_data_db(tmp.path());
        let new_path = tmp
            .path()
            .join("spv/testnet/shielded-commitment-tree.sqlite");

        // Materialise the destination schema the way production does — via
        // `ClientPersistentCommitmentTree::open_path`. The migrator then
        // copies the legacy rows into the new file.
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let _ = ClientPersistentCommitmentTree::open_path(&new_path, 100)
            .expect("create fresh shielded sqlite");

        migrate_commitment_tree_if_needed(&data_db, &new_path).expect("migrator runs cleanly");

        for table in COMMITMENT_TREE_TABLES {
            assert_eq!(
                row_count(&new_path, table),
                1,
                "table {table} should hold the migrated row"
            );
        }
    }

    #[test]
    fn migrator_is_a_noop_when_legacy_tables_are_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_db = tmp.path().join("data.db");
        // Empty file — no schema, no rows.
        Connection::open(&data_db).expect("touch empty data.db");

        let new_path = tmp.path().join("shielded-commitment-tree.sqlite");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let _ = ClientPersistentCommitmentTree::open_path(&new_path, 100)
            .expect("create fresh shielded sqlite");

        migrate_commitment_tree_if_needed(&data_db, &new_path)
            .expect("no-op migration must succeed");

        for table in COMMITMENT_TREE_TABLES {
            assert_eq!(row_count(&new_path, table), 0, "{table} should stay empty");
        }
    }

    #[test]
    fn migrator_is_a_noop_when_data_db_is_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let new_path = tmp.path().join("shielded-commitment-tree.sqlite");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let _ = ClientPersistentCommitmentTree::open_path(&new_path, 100)
            .expect("create fresh shielded sqlite");

        migrate_commitment_tree_if_needed(tmp.path().join("missing.db").as_path(), &new_path)
            .expect("absent data.db => silent no-op");
    }

    #[test]
    fn shielded_commitment_tree_path_is_sibling_of_platform_wallet_sqlite() {
        let dir = std::path::Path::new("/tmp/spv/testnet");
        assert_eq!(
            shielded_commitment_tree_path(dir),
            dir.join(SHIELDED_COMMITMENT_TREE_FILE),
        );
    }
}
