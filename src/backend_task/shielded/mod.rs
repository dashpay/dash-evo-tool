use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::feature_gate::{Check, FeatureGate};
use crate::model::fee_estimation::ShieldFromCoreFunding;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::PlatformPathIndex;
use dash_sdk::Error as SdkError;
use dash_sdk::dpp::address_funds::{OrchardAddress, PlatformAddress};
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

pub(super) fn shielded_operations_unavailable_error(ctx: &AppContext) -> Option<TaskError> {
    match FeatureGate::ShieldedOperations.first_unmet_check(ctx) {
        None => None,
        Some(Check::Capability(_)) => Some(TaskError::ShieldedOperationsNetworkUnavailable),
        Some(Check::MinRole(_) | Check::Experimental(_)) => {
            Some(TaskError::ShieldedOperationsRoleUnavailable)
        }
    }
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
        if let Some(error) = shielded_operations_unavailable_error(self) {
            tracing::warn!(
                ?error,
                "Refused a shielded fund movement because shielded operations are unavailable"
            );
            return Err(error);
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
                // recipient, so we size the lock to `amount + fee`. A fee that
                // cannot be read stops the shield before any funds move.
                let result = self
                    .shield_from_core_with(seed_hash, amount_duffs, async |shield| {
                        // Deposit into this wallet's own default Orchard address. The
                        // keys are bound at unlock; an unbound wallet has no address.
                        let raw = backend
                            .shielded_default_address(&seed_hash, 0)
                            .await?
                            .ok_or(TaskError::ShieldedNotBound)?;
                        let recipient = OrchardAddress::from_raw_bytes(&raw)
                            .map_err(|_| TaskError::ShieldedInvalidRecipientAddress)?;

                        let funding = AssetLockFunding::FromWalletBalance {
                            amount_duffs: shield.lock_duffs,
                            account_index: 0,
                        };

                        backend
                            .shield_from_asset_lock(&seed_hash, funding, recipient, 0, None)
                            .await
                    })
                    .await?;

                self.refresh_shielded_balance_snapshot(&seed_hash).await;

                Ok(result)
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

impl AppContext {
    /// Shield `amount_duffs` from the Core wallet through `transfer`, the call
    /// that locks the sized funding and shields it. The protocol version is
    /// read before the funding is sized and again once `transfer` has
    /// finished; the result names an amount only when the two agree.
    async fn shield_from_core_with<F>(
        &self,
        seed_hash: WalletSeedHash,
        amount_duffs: u64,
        transfer: impl FnOnce(ShieldFromCoreFunding) -> F,
    ) -> Result<BackendTaskSuccessResult, TaskError>
    where
        F: Future<Output = Result<(), TaskError>>,
    {
        let (shield, version_at_sizing) = self.plan_shield_from_core(amount_duffs)?;
        transfer(shield).await?;
        let amount = shielded_amount_to_report(
            shield.shielded_credits,
            version_at_sizing,
            self.shield_from_core_protocol_version().0,
        );
        Ok(BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash, amount })
    }

    /// Size a shield of `amount_duffs` from the Core wallet, and note the
    /// number of the protocol version the fee was read from.
    pub(crate) fn plan_shield_from_core(
        &self,
        amount_duffs: u64,
    ) -> Result<(ShieldFromCoreFunding, u32), TaskError> {
        let (version_at_sizing, platform_version) = self.shield_from_core_protocol_version();
        let shield = self
            .fee_estimator()
            .shield_from_core_funding(amount_duffs, platform_version)
            .map_err(|e| TaskError::AssetLockNetworkFeeUnavailable {
                source_error: Box::new(SdkError::Protocol(*e)),
            })?;
        Ok((shield, version_at_sizing))
    }
}

/// The shielded amount to report once a shield from the Core wallet has gone
/// through. The transfer works its fee out again while it runs and does not
/// return what it shielded. The version only ever moves forward, so the same
/// version before and after means the transfer used the fee the prediction
/// did; otherwise there is no figure the app can vouch for.
fn shielded_amount_to_report(
    predicted_credits: u64,
    version_at_sizing: u32,
    version_after: u32,
) -> Option<u64> {
    (version_after == version_at_sizing).then_some(predicted_credits)
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
            matches!(
                &result,
                Err(TaskError::ShieldedOperationsNetworkUnavailable)
            ),
            "a direct backend dispatch must reject unsupported shielded writes before moving funds: {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shielded_handler_reports_role_gate_before_touching_the_backend() {
        use dash_sdk::dpp::version::feature_initial_protocol_versions::SHIELDED_POOL_INITIAL_PROTOCOL_VERSION;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        ctx.set_platform_protocol_version(SHIELDED_POOL_INITIAL_PROTOCOL_VERSION);
        assert!(!FeatureGate::ShieldedOperations.is_available(&ctx));
        assert!(ctx.wallet_backend().is_err(), "precondition");

        let result = ctx
            .run_shielded_task(ShieldedTask::ShieldFromAssetLock {
                seed_hash: WalletSeedHash::default(),
                amount_duffs: 1,
            })
            .await;

        assert!(
            matches!(&result, Err(TaskError::ShieldedOperationsRoleUnavailable)),
            "the handler must explain that the user's role blocks shielded operations: {result:?}"
        );
        assert!(
            ctx.wallet_backend().is_err(),
            "the role gate must return before the handler touches the wallet backend"
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
            matches!(
                &result,
                Err(TaskError::ShieldedOperationsNetworkUnavailable)
            ),
            "the pre-check must reject the shielded write: {result:?}"
        );
        assert!(
            ctx.wallet_backend().is_err(),
            "the shielded pre-check must return before ensure_wallet_backend wires the backend"
        );
    }

    /// The predicted amount is what upstream shields only while the protocol
    /// version stays the one the fee was read from.
    #[test]
    fn shield_from_core_reports_the_amount_when_the_version_did_not_move() {
        assert_eq!(
            shielded_amount_to_report(100_000_800, 13, 13),
            Some(100_000_800)
        );
    }

    /// The transfer works the fee out again while it runs. When the version
    /// moved in between, the app cannot tell which fee was used, so it must
    /// not name a figure.
    #[test]
    fn shield_from_core_reports_no_amount_when_the_version_moved() {
        assert_eq!(shielded_amount_to_report(100_000_800, 13, 14), None);
    }

    /// The transfer runs on the wallet backend's connection, which keeps its
    /// own protocol version once the app's connection has been rebuilt. The
    /// funding must be sized with the backend's version: 212 852 duffs of fee
    /// under protocol 13, not the 164 140 of protocol 14.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shield_from_core_is_sized_with_the_wallet_backend_version() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        ctx.ensure_wallet_backend(SenderAsync::new(tx, egui::Context::default()))
            .await
            .expect("wallet backend");
        let backend = ctx.wallet_backend().expect("wallet backend");
        assert_eq!(backend.sdk().protocol_version_number(), 13);

        let pv14 = dash_sdk::dpp::version::PlatformVersion::get(14).expect("PV14");
        ctx.sdk.store(Arc::new(
            dash_sdk::SdkBuilder::new_mock()
                .with_version(pv14)
                .build()
                .expect("mock sdk"),
        ));

        let (shield, version_at_sizing) = ctx.plan_shield_from_core(100_000).expect("known fee");

        assert_eq!(version_at_sizing, 13);
        assert_eq!(shield.lock_duffs, 100_000 + 212_852);
        assert_eq!(shield.shielded_credits, 100_000_800);
    }

    /// A context whose wallet backend runs on protocol 13.
    async fn ctx_with_backend(
        dir: &std::path::Path,
    ) -> (Arc<AppContext>, Arc<crate::wallet_backend::WalletBackend>) {
        let ctx = test_app_context(dir);
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        ctx.ensure_wallet_backend(SenderAsync::new(tx, egui::Context::default()))
            .await
            .expect("wallet backend");
        let backend = ctx.wallet_backend().expect("wallet backend");
        assert_eq!(backend.sdk().protocol_version_number(), 13);
        (ctx, backend)
    }

    /// Tell `sdk` the network runs protocol `version`, the way a response
    /// from the network does.
    fn network_reports_protocol_version(sdk: &dash_sdk::Sdk, version: u32) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64;
        sdk.verify_response_metadata(
            "test",
            &dash_sdk::dapi_grpc::platform::v0::ResponseMetadata {
                height: 1,
                time_ms: now_ms,
                protocol_version: version,
                ..Default::default()
            },
        )
        .expect("fresh metadata");
        assert_eq!(sdk.protocol_version_number(), version);
    }

    /// The version moves while the transfer is under way, and the transfer
    /// then completes: the task must still succeed, and must not name an
    /// amount it sized under the earlier version.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shield_from_core_task_succeeds_without_an_amount_when_the_version_moves_mid_transfer()
    {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (ctx, backend) = ctx_with_backend(tmp.path()).await;
        let seed_hash = WalletSeedHash::default();

        let result = ctx
            .shield_from_core_with(seed_hash, 100_000, async |funding| {
                assert_eq!(funding.lock_duffs, 100_000 + 212_852);
                network_reports_protocol_version(backend.sdk(), 14);
                Ok(())
            })
            .await;

        assert!(
            matches!(
                &result,
                Ok(BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash: reported, amount: None })
                    if *reported == seed_hash
            ),
            "got {result:?}"
        );
    }

    /// With the version unchanged across the transfer the task reports what
    /// was shielded, including the 800 credits that rounding the fee up to
    /// whole duffs adds under protocol 13.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shield_from_core_task_reports_the_amount_when_the_version_holds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (ctx, _backend) = ctx_with_backend(tmp.path()).await;
        let seed_hash = WalletSeedHash::default();

        let result = ctx
            .shield_from_core_with(seed_hash, 100_000, async |_funding| Ok(()))
            .await;

        assert!(
            matches!(
                &result,
                Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
                    amount: Some(100_000_800),
                    ..
                })
            ),
            "got {result:?}"
        );
    }

    /// A transfer that fails stays a failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shield_from_core_task_keeps_a_failed_transfer_a_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (ctx, _backend) = ctx_with_backend(tmp.path()).await;

        let result = ctx
            .shield_from_core_with(WalletSeedHash::default(), 100_000, async |_funding| {
                Err(TaskError::ShieldedNotBound)
            })
            .await;

        assert!(
            matches!(&result, Err(TaskError::ShieldedNotBound)),
            "got {result:?}"
        );
    }
}
