use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
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
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_topup();

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
        let backend = self.wallet_backend()?;
        let new_balance = backend
            .top_up_identity(&seed_hash, &identity_id, funding, identity_index, None)
            .await?;
        qualified_identity.identity.set_balance(new_balance);

        let actual_fee = match amount_duffs_for_fee {
            Some(amount) => {
                let expected_credits = amount.saturating_mul(1000);
                let balance_increase = new_balance.saturating_sub(balance_before);
                expected_credits.saturating_sub(balance_increase)
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

        self.update_local_qualified_identity(&qualified_identity)?;
        if let Some(amount) = amount_duffs_for_fee {
            self.db.insert_top_up(
                qualified_identity.identity.id().as_bytes(),
                top_up_index,
                amount,
            )?;
        }

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            qualified_identity,
            fee_result,
        ))
    }
}
