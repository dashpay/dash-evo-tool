use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::PlatformPathIndex;
use dash_sdk::dpp::address_funds::{OrchardAddress, PlatformAddress};
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::Address;
use std::sync::Arc;

/// The shielded-pool operations DET dispatches into the upstream
/// `platform-wallet` coordinator.
///
/// Sync, nullifier scanning, key derivation and the commitment tree are all
/// owned by the upstream coordinator, so only the five fund-moving operations
/// remain here. Balance/notes/activity are read through the push snapshot and
/// the coordinator store, not through a task.
#[derive(Debug, Clone, PartialEq)]
pub enum ShieldedTask {
    /// Shield core DASH directly into the shielded pool via asset lock (Type 18).
    ShieldFromAssetLock {
        seed_hash: WalletSeedHash,
        amount_duffs: u64,
    },

    /// Shield credits from the wallet's platform balance into the shielded
    /// pool (Type 15). Upstream selects the input addresses; DET does not pick
    /// a `from_address` or manage nonces.
    ShieldFromBalance {
        seed_hash: WalletSeedHash,
        amount: u64,
    },

    /// Private transfer within the shielded pool (Type 16).
    ShieldedTransfer {
        seed_hash: WalletSeedHash,
        amount: u64,
        /// Serialized Orchard payment address (43 raw bytes).
        recipient_address_bytes: Vec<u8>,
    },

    /// Unshield credits to a platform address (Type 17).
    UnshieldCredits {
        seed_hash: WalletSeedHash,
        amount: u64,
        to_platform_address: PlatformAddress,
    },

    /// Withdraw from the shielded pool directly to a core L1 address (Type 19).
    ShieldedWithdrawal {
        seed_hash: WalletSeedHash,
        amount: u64,
        to_core_address: Address,
    },
}

impl AppContext {
    /// Run a shielded-pool task by forwarding to the upstream coordinator
    /// through the [`WalletBackend`](crate::wallet_backend::WalletBackend)
    /// shielded ops. Each op already maps upstream errors via
    /// `map_shielded_op_error`, so this layer is a thin adapter: it shapes the
    /// task payload into the backend call and refreshes the frame-safe balance
    /// snapshot once the op confirms.
    ///
    /// Binding is handled at unlock time (`bootstrap_wallet_addresses_jit` →
    /// `ensure_shielded_bound`); an operation on a still-unbound wallet surfaces
    /// [`TaskError::ShieldedNotBound`].
    pub async fn run_shielded_task(
        self: &Arc<Self>,
        task: ShieldedTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !FeatureGate::ShieldedOperations.is_available(self) {
            tracing::warn!(
                "Refused a shielded fund movement because shielded operations are unavailable"
            );
            return Err(TaskError::ShieldedOperationsUnavailable);
        }

        let backend = self.wallet_backend()?;
        let result = self.run_shielded_task_inner(task, backend.as_ref()).await;
        super::contextualize_wallet_backend_dapi_result(result, backend.as_ref())
    }

    async fn run_shielded_task_inner(
        self: &Arc<Self>,
        task: ShieldedTask,
        backend: &crate::wallet_backend::WalletBackend,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            ShieldedTask::ShieldFromAssetLock {
                seed_hash,
                amount_duffs,
            } => {
                use platform_wallet::wallet::asset_lock::AssetLockFunding;

                // The asset lock must also cover the shielded fee; upstream
                // passes the whole lock value (minus that flat fee) to the
                // recipient, so we size the lock to `amount + fee`.
                let (platform_fee_duffs, _l1_fee_duffs) =
                    self.fee_estimator().estimate_shield_from_core_fees_duffs();
                let lock_amount_duffs = amount_duffs.saturating_add(platform_fee_duffs);

                // Deposit into this wallet's own default Orchard address. The
                // keys are bound at unlock; an unbound wallet has no address.
                let raw = backend
                    .shielded_default_address(&seed_hash, 0)
                    .await?
                    .ok_or(TaskError::ShieldedNotBound)?;
                let recipient = OrchardAddress::from_raw_bytes(&raw)
                    .map_err(|_| TaskError::ShieldedInvalidRecipientAddress)?;

                let funding = AssetLockFunding::FromWalletBalance {
                    amount_duffs: lock_amount_duffs,
                    account_index: 0,
                };

                backend
                    .shield_from_asset_lock(&seed_hash, funding, recipient, 0, None)
                    .await?;

                self.refresh_shielded_balance_snapshot(&seed_hash).await;

                let credits = amount_duffs.saturating_mul(CREDITS_PER_DUFF);
                Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
                    seed_hash,
                    amount: credits,
                })
            }

            ShieldedTask::ShieldFromBalance { seed_hash, amount } => {
                // The upstream op selects the funding platform addresses; DET
                // only supplies the wallet's address→path index for signing.
                let path_index = self.platform_path_index(&seed_hash)?;
                backend
                    .shield_from_balance(&seed_hash, &path_index, 0, 0, amount)
                    .await?;

                self.refresh_shielded_balance_snapshot(&seed_hash).await;

                Ok(BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount })
            }

            ShieldedTask::ShieldedTransfer {
                seed_hash,
                amount,
                recipient_address_bytes,
            } => {
                let recipient_raw: [u8; 43] = recipient_address_bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| TaskError::ShieldedInvalidRecipientAddress)?;

                backend
                    .shielded_transfer(&seed_hash, 0, &recipient_raw, amount, [0u8; 36])
                    .await?;

                self.refresh_shielded_balance_snapshot(&seed_hash).await;

                Ok(BackendTaskSuccessResult::ShieldedTransferComplete { seed_hash, amount })
            }

            ShieldedTask::UnshieldCredits {
                seed_hash,
                amount,
                to_platform_address,
            } => {
                let to_bech32m = to_platform_address.to_bech32m_string(self.network);
                backend
                    .shielded_unshield(&seed_hash, 0, &to_bech32m, amount)
                    .await?;

                self.refresh_shielded_balance_snapshot(&seed_hash).await;

                Ok(BackendTaskSuccessResult::ShieldedCreditsUnshielded { seed_hash, amount })
            }

            ShieldedTask::ShieldedWithdrawal {
                seed_hash,
                amount,
                to_core_address,
            } => {
                let to_core = to_core_address.to_string();
                // `core_fee_per_byte = 1`: the historical DET default for
                // shielded withdrawals; upstream prices the withdrawal document.
                backend
                    .shielded_withdraw(&seed_hash, 0, &to_core, amount, 1)
                    .await?;

                self.refresh_shielded_balance_snapshot(&seed_hash).await;

                Ok(BackendTaskSuccessResult::ShieldedWithdrawalComplete { seed_hash, amount })
            }
        }
    }

    /// Build the address→path index for `seed_hash`'s in-memory wallet, used to
    /// sign platform-address spends in `shield_from_balance`. The read guard is
    /// dropped before returning so no wallet lock is held across an await.
    fn platform_path_index(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<PlatformPathIndex, TaskError> {
        let wallet_arc = self.wallet_arc(seed_hash)?;
        let wallet = wallet_arc.read()?;
        Ok(PlatformPathIndex::from_wallet(&wallet, self.network))
    }

    /// Refresh the frame-safe shielded balance snapshot for `seed_hash` from the
    /// coordinator store after a confirmed operation.
    ///
    /// Keeps the UI balance current immediately after a spend, without waiting
    /// for the next `on_shielded_sync_completed` push from the 60-second sync
    /// loop. Best-effort — a failed read leaves the previous snapshot in place.
    async fn refresh_shielded_balance_snapshot(self: &Arc<Self>, seed_hash: &WalletSeedHash) {
        let backend = match self.wallet_backend() {
            Ok(backend) => backend,
            Err(_) => return,
        };
        match backend.shielded_balances(seed_hash).await {
            Ok(balances) => {
                let total: u64 = balances.values().copied().sum();
                if let Ok(mut map) = self.shielded_balances.lock() {
                    map.insert(*seed_hash, total);
                }
            }
            Err(error) => {
                tracing::debug!(?error, "post-operation shielded balance refresh failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::backend_task::BackendTask;
    use crate::context::test_support::test_app_context;
    use crate::model::user_role::UserRole;
    use crate::utils::egui_mpsc::SenderAsync;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mcp_style_direct_dispatch_rejects_unavailable_shielded_write() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        ctx.set_user_role(UserRole::Developer);
        assert!(FeatureGate::Shielded.is_available(&ctx));
        assert!(!FeatureGate::ShieldedOperations.is_available(&ctx));

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, egui::Context::default());
        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
            seed_hash: WalletSeedHash::default(),
            amount_duffs: 1,
        });

        let result = ctx.run_backend_task(task, sender).await;

        assert!(
            matches!(&result, Err(TaskError::ShieldedOperationsUnavailable)),
            "a direct backend dispatch must reject unsupported shielded writes before moving funds: {result:?}"
        );
    }

    /// The shielded pre-check in `run_backend_task` refuses an unavailable
    /// shielded write *before* `ensure_wallet_backend` wires the backend — which
    /// would otherwise materialize seeds, register upstream, and bind Orchard for
    /// every loaded wallet just to run an op the app refuses. A wired backend
    /// after a rejected dispatch proves the pre-check ran too late.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shielded_pre_check_refuses_without_wiring_the_wallet_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        ctx.set_user_role(UserRole::Developer);
        assert!(!FeatureGate::ShieldedOperations.is_available(&ctx));
        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: the wallet backend is not wired before dispatch"
        );

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, egui::Context::default());
        let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
            seed_hash: WalletSeedHash::default(),
            amount_duffs: 1,
        });

        let result = ctx.run_backend_task(task, sender).await;

        assert!(
            matches!(&result, Err(TaskError::ShieldedOperationsUnavailable)),
            "the pre-check must reject the shielded write: {result:?}"
        );
        assert!(
            ctx.wallet_backend().is_err(),
            "the shielded pre-check must return before ensure_wallet_backend wires the backend"
        );
    }
}
