use std::sync::OnceLock;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::shielded::ShieldedTask;
use crate::context::AppContext;
use crate::model::wallet::shielded::{ShieldedNote, ShieldedWalletState, derive_orchard_keys};
use dash_sdk::grovedb_commitment_tree::{Nullifier, Position, ProvingKey};
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
    ) -> Result<BackendTaskSuccessResult, String> {
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
            } => {
                self.shield_credits_task(seed_hash, amount, from_address)
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
        }
    }

    /// Initialize shielded wallet state by deriving ZIP32 keys from the wallet seed.
    fn initialize_shielded_wallet(
        self: &Arc<Self>,
        seed_hash: crate::model::wallet::WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
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
            let wallet_arc = wallets.get(&seed_hash).ok_or("Wallet not found")?;
            let wallet = wallet_arc.read().unwrap();
            match &wallet.wallet_seed {
                crate::model::wallet::WalletSeed::Open(open) => open.seed,
                crate::model::wallet::WalletSeed::Closed(_) => {
                    return Err("Wallet must be unlocked to initialize shielded state".to_string());
                }
            }
        };

        // Derive Orchard keys via ZIP32
        let keys = derive_orchard_keys(&seed_bytes, self.network, 0)?;

        let network_str = self.network.to_string();
        let mut state = ShieldedWalletState::new(keys);

        // Tree is always rebuilt from scratch (ClientCommitmentTree has no serde).
        // Notes are loaded from DB for instant balance; tree needs full re-sync.
        state.last_synced_index = 0;

        // Load persisted notes from DB and reconstruct Note objects
        if let Ok(note_rows) = self.db.get_unspent_shielded_notes(&seed_hash, &network_str) {
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
        }

        let balance = state.shielded_balance;

        let mut states = self.shielded_states.lock().unwrap();
        states.insert(seed_hash, state);

        Ok(BackendTaskSuccessResult::ShieldedInitialized { seed_hash, balance })
    }

    /// Sync shielded notes from platform.
    async fn sync_shielded_notes(
        self: &Arc<Self>,
        seed_hash: crate::model::wallet::WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Take the state temporarily for the async operation
        let mut state = {
            let mut states = self.shielded_states.lock().unwrap();
            states
                .remove(&seed_hash)
                .ok_or("Shielded wallet not initialized")?
        };

        let result = crate::backend_task::shielded::sync::sync_notes(
            self,
            &seed_hash,
            &mut state,
            self.network,
        )
        .await;

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
    async fn shield_credits_task(
        self: &Arc<Self>,
        seed_hash: crate::model::wallet::WalletSeedHash,
        amount: u64,
        from_address: dash_sdk::dpp::address_funds::PlatformAddress,
    ) -> Result<BackendTaskSuccessResult, String> {
        let state_ref = {
            let mut states = self.shielded_states.lock().unwrap();
            states
                .remove(&seed_hash)
                .ok_or("Shielded wallet not initialized")?
        };

        let result = crate::backend_task::shielded::bundle::shield_credits(
            self,
            &seed_hash,
            &state_ref,
            amount,
            from_address,
        )
        .await;

        // Put state back
        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(seed_hash, state_ref);
        }

        result?;
        Ok(BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount })
    }

    /// Transfer credits within the shielded pool.
    async fn shielded_transfer_task(
        self: &Arc<Self>,
        seed_hash: crate::model::wallet::WalletSeedHash,
        amount: u64,
        recipient_address_bytes: Vec<u8>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let state = {
            let mut states = self.shielded_states.lock().unwrap();
            states
                .remove(&seed_hash)
                .ok_or("Shielded wallet not initialized")?
        };

        let result = crate::backend_task::shielded::bundle::shielded_transfer(
            self,
            &seed_hash,
            &state,
            amount,
            &recipient_address_bytes,
        )
        .await;

        // Put state back
        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(seed_hash, state);
        }

        result?;
        Ok(BackendTaskSuccessResult::ShieldedTransferComplete { seed_hash, amount })
    }

    /// Unshield credits from the shielded pool to a platform address.
    async fn unshield_credits_task(
        self: &Arc<Self>,
        seed_hash: crate::model::wallet::WalletSeedHash,
        amount: u64,
        to_platform_address: dash_sdk::dpp::address_funds::PlatformAddress,
    ) -> Result<BackendTaskSuccessResult, String> {
        let state = {
            let mut states = self.shielded_states.lock().unwrap();
            states
                .remove(&seed_hash)
                .ok_or("Shielded wallet not initialized")?
        };

        let result = crate::backend_task::shielded::bundle::unshield_credits(
            self,
            &seed_hash,
            &state,
            amount,
            to_platform_address,
        )
        .await;

        // Put state back
        {
            let mut states = self.shielded_states.lock().unwrap();
            states.insert(seed_hash, state);
        }

        result?;
        Ok(BackendTaskSuccessResult::ShieldedCreditsUnshielded { seed_hash, amount })
    }

    /// Check nullifiers to detect spent notes.
    async fn check_nullifiers_task(
        self: &Arc<Self>,
        seed_hash: crate::model::wallet::WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut state = {
            let mut states = self.shielded_states.lock().unwrap();
            states
                .remove(&seed_hash)
                .ok_or("Shielded wallet not initialized")?
        };

        let result = crate::backend_task::shielded::nullifiers::check_nullifiers(
            self,
            &seed_hash,
            &mut state,
            self.network,
        )
        .await;

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
}
