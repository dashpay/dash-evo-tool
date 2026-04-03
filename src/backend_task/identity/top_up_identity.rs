use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::{AppContext, get_transaction_info};
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use dash_sdk::Error;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::identity_topup_transition::IdentityTopUpTransition;
use dash_sdk::dpp::state_transition::identity_topup_transition::methods::IdentityTopUpTransitionMethodsV0;
use dash_sdk::platform::Fetch;
use dash_sdk::platform::transition::top_up_identity::TopUpIdentity;
use std::collections::BTreeMap;

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

        let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None).await?;

        let (asset_lock_proof, asset_lock_proof_private_key, tx_id, top_up_index) =
            match identity_funding_method {
                TopUpIdentityFundingMethod::UseAssetLock(
                    address,
                    asset_lock_proof,
                    transaction,
                ) => {
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
                        if let AssetLockProof::Instant(instant_asset_lock_proof) =
                            asset_lock_proof.as_ref()
                        {
                            // we need to make sure the instant send asset lock is recent
                            let tx_info = get_transaction_info(&sdk, &tx_id).await?;

                            if tx_info.is_chain_locked
                                && tx_info.height > 0
                                && tx_info.confirmations > 8
                            {
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
                    (asset_lock_proof, private_key, tx_id, None)
                }
                TopUpIdentityFundingMethod::FundWithWallet(
                    amount,
                    identity_index,
                    top_up_index,
                ) => {
                    // Scope the write lock to avoid holding it across an await.
                    // UTXOs are selected but NOT removed yet — removal happens after broadcast.
                    let (platform_wallet, wallet_seed_hash) = {
                        let guard = wallet.read().map_err(TaskError::from)?;
                        (
                            guard.platform_wallet.clone().ok_or(TaskError::WalletNotFound)?,
                            guard.seed_hash(),
                        )
                    };

                    let (asset_lock_transaction, asset_lock_proof_private_key) = platform_wallet
                        .core()
                        .build_asset_lock_transaction(
                            amount,
                            platform_wallet::AssetLockFundingType::IdentityTopUp,
                            identity_index,
                        )
                        .await
                        .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                            detail: e.to_string(),
                        })?;

                    let tx_id = self
                        .broadcast_and_commit_asset_lock(
                            &asset_lock_transaction,
                            amount,
                            &wallet_seed_hash,
                            &wallet,
                            &BTreeMap::new(),
                        )
                        .await?;

                    let asset_lock_proof = self.wait_for_asset_lock_proof(tx_id).await?;

                    (
                        asset_lock_proof,
                        asset_lock_proof_private_key,
                        tx_id,
                        Some((amount, top_up_index)),
                    )
                }
                TopUpIdentityFundingMethod::FundWithUtxo(
                    utxo,
                    tx_out,
                    input_address,
                    identity_index,
                    top_up_index,
                ) => {
                    // Scope the write lock to avoid holding it across an await.
                    let (asset_lock_transaction, asset_lock_proof_private_key, wallet_seed_hash) = {
                        let mut wallet = wallet.write().map_err(TaskError::from)?;
                        let seed_hash = wallet.seed_hash();
                        let tx_result = wallet
                            .top_up_asset_lock_transaction_for_utxo(
                                self,
                                sdk.network,
                                utxo,
                                tx_out.clone(),
                                input_address.clone(),
                                identity_index,
                                top_up_index,
                            )
                            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                                detail: e,
                            })?;
                        (tx_result.0, tx_result.1, seed_hash)
                    };

                    let used_utxos =
                        BTreeMap::from([(utxo, (tx_out.clone(), input_address.clone()))]);

                    let tx_id = self
                        .broadcast_and_commit_asset_lock(
                            &asset_lock_transaction,
                            tx_out.value,
                            &wallet_seed_hash,
                            &wallet,
                            &used_utxos,
                        )
                        .await?;

                    let asset_lock_proof = self.wait_for_asset_lock_proof(tx_id).await?;

                    (
                        asset_lock_proof,
                        asset_lock_proof_private_key,
                        tx_id,
                        Some((tx_out.value, top_up_index)),
                    )
                }
            };

        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                qualified_identity.identity.id().as_bytes(),
            )?;

        // Track balance before top-up for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_topup();

        // Delegate the SDK call to platform-wallet when available,
        // falling back to direct SDK call otherwise.
        let maybe_platform_wallet = self.platform_wallet_for_identity(&qualified_identity).ok();

        let top_up_result = if let Some(ref pw) = maybe_platform_wallet {
            pw.identity()
                .top_up_identity_with_signer(
                    &qualified_identity.identity,
                    asset_lock_proof.clone(),
                    &asset_lock_proof_private_key,
                )
                .await
        } else {
            qualified_identity
                .identity
                .top_up_identity(
                    &sdk,
                    asset_lock_proof.clone(),
                    &asset_lock_proof_private_key,
                    None,
                    None,
                )
                .await
        };

        let updated_identity_balance = match top_up_result {
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

                            // Retry with chain asset lock proof via platform-wallet or fallback
                            let cl_result = if let Some(ref pw) = maybe_platform_wallet {
                                pw.identity()
                                    .top_up_identity_with_signer(
                                        &qualified_identity.identity,
                                        chain_asset_lock_proof,
                                        &asset_lock_proof_private_key,
                                    )
                                    .await
                            } else {
                                qualified_identity
                                    .identity
                                    .top_up_identity(
                                        &sdk,
                                        chain_asset_lock_proof,
                                        &asset_lock_proof_private_key,
                                        None,
                                        None,
                                    )
                                    .await
                            };

                            cl_result.map_err(|e| {
                                self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
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
                } else if matches!(e, Error::Protocol(ProtocolError::UnknownVersionError(_))) {
                    let retry_result = if let Some(ref pw) = maybe_platform_wallet {
                        pw.identity()
                            .top_up_identity_with_signer(
                                &qualified_identity.identity,
                                asset_lock_proof.clone(),
                                &asset_lock_proof_private_key,
                            )
                            .await
                    } else {
                        qualified_identity
                            .identity
                            .top_up_identity(
                                &sdk,
                                asset_lock_proof.clone(),
                                &asset_lock_proof_private_key,
                                None,
                                None,
                            )
                            .await
                    };

                    retry_result.map_err(|retry_err| {
                        let logged = self.log_drive_proof_error(
                            retry_err,
                            RequestType::BroadcastStateTransition,
                        );
                        if matches!(logged, TaskError::ProofError { .. }) {
                            return logged;
                        }
                        // Log the reconstructed transition for debugging before returning the error.
                        if let Ok(transition) = IdentityTopUpTransition::try_from_identity(
                            &qualified_identity.identity,
                            asset_lock_proof,
                            asset_lock_proof_private_key.inner.as_ref(),
                            0,
                            self.platform_version(),
                            None,
                        ) {
                            tracing::debug!(
                                "Top-up retry failed; reconstructed transition: {:?}",
                                transition
                            );
                        }
                        logged
                    })?
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
