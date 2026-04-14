use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use crate::platform_wallet_bridge::IdentityFunding;
use dash_sdk::Error;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::dashcore::hashes::Hash;
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

        let (out_point, identity_index, top_up_index) = match identity_funding_method {
            TopUpIdentityFundingMethod::UseAssetLock(out_point) => {
                let _platform_wallet = {
                    let guard = wallet.read().map_err(TaskError::from)?;
                    guard
                        .platform_wallet
                        .clone()
                        .ok_or(TaskError::WalletNotFound)?
                };

                (out_point, 0u32, None)
            }
            TopUpIdentityFundingMethod::FundWithWallet(amount, identity_index, top_up_index) => {
                let platform_wallet = {
                    let guard = wallet.read().map_err(TaskError::from)?;
                    guard
                        .platform_wallet
                        .clone()
                        .ok_or(TaskError::WalletNotFound)?
                };

                // Single call: builds asset lock TX, broadcasts, waits for
                // finality proof (IS or CL), and returns the proof + key.
                // The lock is tracked by AssetLockManager for later resumption.
                let (_asset_lock_proof, _asset_lock_proof_private_key, out_point) = platform_wallet
                    .asset_locks()
                    .create_funded_asset_lock_proof(
                        amount,
                        0,
                        platform_wallet::AssetLockFundingType::IdentityTopUp,
                        identity_index,
                    )
                    .await
                    .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                        detail: e.to_string(),
                    })?;

                (out_point, identity_index, Some((amount, top_up_index)))
            }
        };

        let tx_id = out_point.txid;

        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                qualified_identity.identity.id().as_bytes(),
            )?;

        // Track balance before top-up for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_topup();

        // Use the one-call API which handles IS→CL fallback internally.
        // The asset lock is already tracked by the manager from the funding
        // phase above, so FromExistingAssetLock resumes it efficiently.
        let maybe_platform_wallet = self.platform_wallet_for_identity(&qualified_identity).ok();

        let platform_wallet = maybe_platform_wallet
            .as_ref()
            .ok_or(TaskError::WalletNotFound)?;

        let funding = IdentityFunding::FromExistingAssetLock { out_point };

        let top_up_result = platform_wallet
            .identity()
            .funded_top_up_identity(
                &qualified_identity.identity,
                funding.clone(),
                identity_index,
                None,
            )
            .await;

        let updated_identity_balance = match top_up_result {
            Ok(new_balance) => new_balance,
            Err(platform_wallet::PlatformWalletError::Sdk(ref e))
                if matches!(e, Error::Protocol(ProtocolError::UnknownVersionError(_))) =>
            {
                // Retry once on version mismatch.
                platform_wallet
                    .identity()
                    .funded_top_up_identity(
                        &qualified_identity.identity,
                        funding,
                        identity_index,
                        None,
                    )
                    .await
                    .map_err(|retry_err| match retry_err {
                        platform_wallet::PlatformWalletError::Sdk(sdk_err) => self
                            .log_drive_proof_error(sdk_err, RequestType::BroadcastStateTransition),
                        other => TaskError::PlatformWallet {
                            source: Box::new(other),
                        },
                    })?
            }
            Err(platform_wallet::PlatformWalletError::Sdk(e)) => {
                return Err(self.log_drive_proof_error(e, RequestType::BroadcastStateTransition));
            }
            Err(other) => {
                return Err(TaskError::PlatformWallet {
                    source: Box::new(other),
                });
            }
        };

        qualified_identity
            .identity
            .set_balance(updated_identity_balance);

        // Calculate and log actual fee paid
        // For top-ups, the "fee" is the difference between expected new balance and actual
        let expected_credits_from_topup = if let Some((amount, _)) = top_up_index {
            // amount is in duffs, 1 duff = 1000 credits
            amount * 1000
        } else {
            // For asset lock method, calculate from the asset lock amount
            0 // Can't easily determine without more info
        };

        if expected_credits_from_topup > 0 {
            let balance_increase = updated_identity_balance.saturating_sub(balance_before);
            let actual_fee = expected_credits_from_topup.saturating_sub(balance_increase);
            tracing::info!(
                "Identity top-up complete: topped up {} credits (from {} duffs), estimated fee {} credits, actual fee {} credits, balance increased by {} credits",
                expected_credits_from_topup,
                expected_credits_from_topup / 1000,
                estimated_fee,
                actual_fee,
                balance_increase
            );
            if actual_fee != estimated_fee {
                tracing::warn!(
                    "Top-up fee mismatch: estimated {} vs actual {} (diff: {})",
                    estimated_fee,
                    actual_fee,
                    actual_fee as i128 - estimated_fee as i128
                );
            }
        } else {
            tracing::info!(
                "Identity top-up complete: balance before {} credits, balance after {} credits",
                balance_before,
                updated_identity_balance
            );
        }

        self.update_local_qualified_identity(&qualified_identity)?;

        // Identity persistence is owned by `update_local_qualified_identity`
        // above. The persister doesn't write the `identity` table
        // (scope was reduced) — see `src/changeset/sqlite.rs`
        // for the rationale and the plan to unify identity
        // persistence under the persister once the platform-wallet is
        // QualifiedIdentity-aware.

        self.db.set_asset_lock_identity_id(
            tx_id.as_byte_array(),
            qualified_identity.identity.id().as_bytes(),
        )?;

        if let Some((amount, top_up_index)) = top_up_index {
            self.db.insert_top_up(
                qualified_identity.identity.id().as_bytes(),
                top_up_index,
                amount,
            )?;
        }

        // Calculate actual fee for the FeeResult
        let actual_fee = if expected_credits_from_topup > 0 {
            let balance_increase = updated_identity_balance.saturating_sub(balance_before);
            expected_credits_from_topup.saturating_sub(balance_increase)
        } else {
            estimated_fee // Fall back to estimated when we can't calculate actual
        };
        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            qualified_identity,
            fee_result,
        ))
    }
}
