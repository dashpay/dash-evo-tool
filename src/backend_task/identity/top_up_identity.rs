use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::{AppContext, get_transaction_info};
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::Fetch;
use dash_sdk::platform::transition::top_up_identity::TopUpIdentity;

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

        let sdk = self.sdk.load().as_ref().clone();

        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_topup();

        // Fast-path: a wallet-funded top-up runs entirely through the upstream
        // `IdentityWallet::top_up_identity_with_funding`. Upstream owns the
        // asset-lock build/broadcast, the IS→CL fallback, the
        // `TopUpIdentity` submission, and the asset-lock cleanup. DET only
        // mirrors the new balance into its local stores.
        if let TopUpIdentityFundingMethod::FundWithWallet(amount, identity_index, top_up_index) =
            &identity_funding_method
        {
            let amount = *amount;
            let identity_index = *identity_index;
            let top_up_index = *top_up_index;
            let seed_hash = wallet.read().map_err(TaskError::from)?.seed_hash();
            let identity_id = qualified_identity.identity.id();
            let backend = self.wallet_backend()?;
            let new_balance = backend
                .top_up_identity(&seed_hash, &identity_id, amount, identity_index, None)
                .await?;
            qualified_identity.identity.set_balance(new_balance);

            let actual_fee = {
                let expected_credits = amount.saturating_mul(1000);
                let balance_increase = new_balance.saturating_sub(balance_before);
                expected_credits.saturating_sub(balance_increase)
            };
            tracing::info!(
                "Identity top-up complete: balance before {} credits, balance after {} credits, estimated fee {} credits, actual fee {} credits",
                balance_before,
                new_balance,
                estimated_fee,
                actual_fee,
            );

            self.update_local_qualified_identity(&qualified_identity)?;
            self.db.insert_top_up(
                qualified_identity.identity.id().as_bytes(),
                top_up_index,
                amount,
            )?;

            let fee_result = FeeResult::new(estimated_fee, actual_fee);
            return Ok(BackendTaskSuccessResult::ToppedUpIdentity(
                qualified_identity,
                fee_result,
            ));
        }

        let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None).await?;

        // Staged-asset-lock path: caller supplies a pre-existing asset lock
        // (and the wallet-derived spending key). DET still drives the
        // Platform-side `TopUpIdentity` and the IS→CL fallback by hand
        // because the upstream wallet backend never tracked this asset lock.
        let TopUpIdentityFundingMethod::UseAssetLock(address, asset_lock_proof, transaction) =
            identity_funding_method
        else {
            unreachable!("FundWithWallet handled by fast-path above")
        };

        let tx_id = transaction.txid();

        // Scope the read guard so it's dropped before the async DAPI call below
        let private_key = {
            let wallet = wallet.read().map_err(TaskError::from)?;
            wallet
                .private_key_for_address(&address, self.network)
                .map_err(|e| TaskError::WalletKeyLookupFailed { detail: e })?
                .ok_or(TaskError::AssetLockNotValidForWallet)?
        };
        let asset_lock_proof =
            if let AssetLockProof::Instant(instant_asset_lock_proof) = asset_lock_proof.as_ref() {
                // we need to make sure the instant send asset lock is recent
                let tx_info = get_transaction_info(&sdk, &tx_id).await?;

                if tx_info.is_chain_locked && tx_info.height > 0 && tx_info.confirmations > 8 {
                    // Transaction is old enough that instant lock may have expired
                    let tx_block_height = tx_info.height;

                    if tx_block_height <= metadata.core_chain_locked_height {
                        // Platform has verified this Core block, use chain lock proof
                        AssetLockProof::Chain(ChainAssetLockProof {
                            core_chain_locked_height: tx_block_height,
                            out_point: OutPoint::new(tx_id, 0),
                        })
                    } else {
                        // Platform hasn't verified this Core block yet
                        return Err(TaskError::AssetLockExpired {
                            tx_block_height,
                            platform_height: metadata.core_chain_locked_height,
                        });
                    }
                } else {
                    AssetLockProof::Instant(instant_asset_lock_proof.clone())
                }
            } else {
                asset_lock_proof.as_ref().clone()
            };

        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                qualified_identity.identity.id().as_bytes(),
            )?;

        let updated_identity_balance = match qualified_identity
            .identity
            .top_up_identity_with_private_key(&sdk, asset_lock_proof.clone(), &private_key, None)
            .await
        {
            Ok(updated_identity) => updated_identity,
            Err(e) => {
                if crate::backend_task::error::is_instant_lock_proof_invalid(&e) {
                    // Try to use chain asset lock proof instead
                    let tx_info = get_transaction_info(&sdk, &tx_id).await?;

                    if tx_info.is_chain_locked && tx_info.height > 0 {
                        let tx_block_height = tx_info.height;

                        if tx_block_height <= metadata.core_chain_locked_height {
                            // Platform has verified this Core block, use chain lock proof
                            let chain_asset_lock_proof =
                                AssetLockProof::Chain(ChainAssetLockProof {
                                    core_chain_locked_height: tx_block_height,
                                    out_point: OutPoint::new(tx_id, 0),
                                });

                            qualified_identity
                                .identity
                                .top_up_identity_with_private_key(
                                    &sdk,
                                    chain_asset_lock_proof,
                                    &private_key,
                                    None,
                                )
                                .await
                                .map_err(|e| {
                                    self.log_drive_proof_error(
                                        e,
                                        RequestType::BroadcastStateTransition,
                                    )
                                })?
                        } else {
                            return Err(TaskError::AssetLockExpired {
                                tx_block_height,
                                platform_height: metadata.core_chain_locked_height,
                            });
                        }
                    } else {
                        return Err(TaskError::AssetLockInstantLockExpiredNotChainlocked);
                    }
                } else {
                    return Err(
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    );
                }
            }
        };

        qualified_identity
            .identity
            .set_balance(updated_identity_balance);

        tracing::info!(
            "Identity top-up complete: balance before {} credits, balance after {} credits",
            balance_before,
            updated_identity_balance
        );

        self.update_local_qualified_identity(&qualified_identity)?;

        {
            let mut wallet = wallet.write().map_err(TaskError::from)?;
            wallet
                .unused_asset_locks
                .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
        }

        self.db.set_asset_lock_identity_id(
            tx_id.as_byte_array(),
            qualified_identity.identity.id().as_bytes(),
        )?;

        let fee_result = FeeResult::new(estimated_fee, estimated_fee);

        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            qualified_identity,
            fee_result,
        ))
    }
}
