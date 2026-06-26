use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};

impl AppContext {
    pub(super) async fn top_up_identity(
        &self,
        input: IdentityTopUpInfo,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let IdentityTopUpInfo {
            mut qualified_identity,
            wallet,
            identity_funding_method,
        } = input;

        let balance_before = qualified_identity.identity.balance();
        // This estimate is shown to the user and feeds the actual-fee
        // plausibility band, so it must track the active network fee multiplier —
        // use the context estimator rather than the hardcoded default.
        let fee_estimator = self.fee_estimator();
        let estimated_fee = fee_estimator.estimate_identity_topup();

        // Both wallet-funded top-up paths (fresh asset lock or resume from a
        // tracked asset lock) run end-to-end through the upstream
        // `IdentityWallet::top_up_identity_with_funding`. Upstream owns the
        // asset-lock build/broadcast, the IS→CL fallback, the
        // `TopUpIdentity` submission, and the asset-lock cleanup. DET only
        // mirrors the new balance into its local stores.
        let (funding, identity_index, top_up_index, amount_duffs_for_fee) =
            match identity_funding_method {
                TopUpIdentityFundingMethod::FundWithWallet(
                    amount,
                    identity_index,
                    top_up_index,
                ) => {
                    let funding =
                        platform_wallet::wallet::asset_lock::AssetLockFunding::FromWalletBalance {
                            amount_duffs: amount,
                            account_index: 0,
                        };
                    (funding, identity_index, top_up_index, Some(amount))
                }
                TopUpIdentityFundingMethod::UseAssetLock {
                    out_point,
                    identity_index,
                    top_up_index,
                } => {
                    let funding = platform_wallet::wallet::asset_lock::AssetLockFunding::FromExistingAssetLock {
                        out_point,
                    };
                    (funding, identity_index, top_up_index, None)
                }
            };

        let seed_hash = wallet.read().map_err(TaskError::from)?.seed_hash();
        let identity_id = qualified_identity.identity.id();
        // Fail-closed: the op's HD index must match the identity's recorded
        // wallet index, or the funding account mis-derives. Reject before any
        // funds move rather than sign against the wrong slot.
        if let Some(wallet_index) = qualified_identity.wallet_index
            && identity_index != wallet_index
        {
            tracing::warn!(
                identity = %identity_id,
                op_index = identity_index,
                wallet_index,
                "Top-up rejected: requested index does not match the identity's wallet index"
            );
            return Err(TaskError::IdentityIndexMismatch { identity_id });
        }
        let backend = self.wallet_backend()?;
        let new_balance = backend
            .top_up_identity(
                &seed_hash,
                &qualified_identity.identity,
                funding,
                identity_index,
                None,
            )
            .await?;
        qualified_identity.identity.set_balance(new_balance);

        let actual_fee = match amount_duffs_for_fee {
            Some(amount) => {
                fee_estimator.resolve_identity_topup_actual_fee(amount, balance_before, new_balance)
            }
            None => estimated_fee,
        };
        tracing::info!(
            "Identity top-up complete: balance before {} credits, balance after {} credits, estimated fee {} credits, actual fee {} credits",
            balance_before,
            new_balance,
            estimated_fee,
            actual_fee,
        );

        if let Some(amount) = amount_duffs_for_fee {
            qualified_identity.top_ups.insert(top_up_index, amount);
            self.save_top_ups(
                &qualified_identity.identity.id(),
                &qualified_identity.top_ups,
            )?;
        }
        self.update_local_qualified_identity(&qualified_identity)?;

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            qualified_identity,
            fee_result,
        ))
    }
}
