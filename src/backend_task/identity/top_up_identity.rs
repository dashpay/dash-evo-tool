use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::{AppContext, get_transaction_info};
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
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
    ) -> Result<BackendTaskSuccessResult, String> {
        let IdentityTopUpInfo {
            mut qualified_identity,
            wallet,
            identity_funding_method,
        } = input;

        let sdk = self.sdk.load().as_ref().clone();

        let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None)
            .await
            .map_err(|e| e.to_string())?;

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
                        let wallet = wallet.read().map_err(|e| e.to_string())?;
                        wallet
                            .private_key_for_address(&address, self.network)?
                            .ok_or("Asset Lock not valid for wallet")?
                    };
                    let asset_lock_proof = if let AssetLockProof::Instant(
                        instant_asset_lock_proof,
                    ) = asset_lock_proof.as_ref()
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
                                return Err(format!(
                                    "Cannot use this asset lock yet. The instant lock proof has expired (quorum rotated), \
                                        and Platform hasn't verified Core block {} yet (Platform has verified up to Core block {}). \
                                        Please wait for Platform to sync with Core chain.",
                                    tx_block_height, metadata.core_chain_locked_height
                                ));
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
                    let (
                        asset_lock_transaction,
                        asset_lock_proof_private_key,
                        _,
                        used_utxos,
                        wallet_seed_hash,
                    ) = {
                        let mut wallet = wallet.write().map_err(|e| e.to_string())?;
                        let seed_hash = wallet.seed_hash();
                        let tx_result = match wallet.top_up_asset_lock_transaction(
                            self,
                            sdk.network,
                            amount,
                            true,
                            identity_index,
                            top_up_index,
                        ) {
                            Ok(transaction) => transaction,
                            Err(e) => {
                                // Reload UTXOs (RPC: fetches from Core; SPV: no-op).
                                // Only retry if something actually changed.
                                if !wallet.reload_utxos(self)? {
                                    return Err(e);
                                }
                                wallet.top_up_asset_lock_transaction(
                                    self,
                                    sdk.network,
                                    amount,
                                    true,
                                    identity_index,
                                    top_up_index,
                                )?
                            }
                        };
                        (
                            tx_result.0,
                            tx_result.1,
                            tx_result.2,
                            tx_result.3,
                            seed_hash,
                        )
                    };

                    let tx_id = self
                        .broadcast_and_commit_asset_lock(
                            &asset_lock_transaction,
                            amount,
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
                        let mut wallet = wallet.write().map_err(|e| e.to_string())?;
                        let seed_hash = wallet.seed_hash();
                        let tx_result = wallet.top_up_asset_lock_transaction_for_utxo(
                            self,
                            sdk.network,
                            utxo,
                            tx_out.clone(),
                            input_address.clone(),
                            identity_index,
                            top_up_index,
                        )?;
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
            )
            .map_err(|e| e.to_string())?;

        // Track balance before top-up for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_topup();

        let updated_identity_balance = match qualified_identity
            .identity
            .top_up_identity(
                &sdk,
                asset_lock_proof.clone(),
                &asset_lock_proof_private_key,
                None,
                None,
            )
            .await
        {
            Ok(updated_identity) => updated_identity,
            Err(e) => {
                // Log proof errors first
                if let Error::DriveProofError(ref proof_error, ref proof_bytes, ref block_info) = e
                {
                    if let Err(e) = self.db.insert_proof_log_item(ProofLogItem {
                        request_type: RequestType::BroadcastStateTransition,
                        request_bytes: vec![],
                        verification_path_query_bytes: vec![],
                        height: block_info.height,
                        time_ms: block_info.time_ms,
                        proof_bytes: proof_bytes.clone(),
                        error: Some(proof_error.to_string()),
                    }) {
                        tracing::warn!("Failed to persist proof log: {}", e);
                    }
                    return Err(format!(
                        "Error topping up identity: {}, proof error logged",
                        proof_error
                    ));
                }

                let error_string = e.to_string();

                // Check if this is an instant lock proof expiration error
                if error_string.contains("Instant lock proof signature is invalid")
                    || error_string.contains("wasn't created recently")
                {
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

                            // Retry with chain asset lock proof
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
                                .map_err(|e| {
                                    // Log proof errors from retry
                                    if let Error::DriveProofError(
                                        ref proof_error,
                                        ref proof_bytes,
                                        ref block_info,
                                    ) = e
                                    {
                                        if let Err(e) =
                                            self.db.insert_proof_log_item(ProofLogItem {
                                                request_type: RequestType::BroadcastStateTransition,
                                                request_bytes: vec![],
                                                verification_path_query_bytes: vec![],
                                                height: block_info.height,
                                                time_ms: block_info.time_ms,
                                                proof_bytes: proof_bytes.clone(),
                                                error: Some(proof_error.to_string()),
                                            })
                                        {
                                            tracing::warn!("Failed to persist proof log: {}", e);
                                        }
                                        return format!(
                                            "Error topping up identity: {}, proof error logged",
                                            proof_error
                                        );
                                    }
                                    e.to_string()
                                })?
                        } else {
                            return Err(format!(
                                "Cannot use this asset lock yet. The instant lock proof has expired (quorum rotated), \
                                and Platform hasn't verified Core block {} yet (Platform has verified up to Core block {}). \
                                Please wait for Platform to sync with Core chain.",
                                tx_block_height, metadata.core_chain_locked_height
                            ));
                        }
                    } else {
                        return Err("Cannot use this asset lock. The instant lock proof has expired and the transaction \
                            is not yet chainlocked. Please wait for the transaction to be chainlocked.".to_string());
                    }
                } else if matches!(e, Error::Protocol(ProtocolError::UnknownVersionError(_))) {
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
                        .map_err(|e| {
                            // Log proof errors from retry
                            if let Error::DriveProofError(
                                ref proof_error,
                                ref proof_bytes,
                                ref block_info,
                            ) = e
                            {
                                if let Err(e) = self.db.insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes: proof_bytes.clone(),
                                    error: Some(proof_error.to_string()),
                                }) {
                                    tracing::warn!("Failed to persist proof log: {}", e);
                                }
                                return format!(
                                    "Error topping up identity: {}, proof error logged",
                                    proof_error
                                );
                            }

                            match IdentityTopUpTransition::try_from_identity(
                                &qualified_identity.identity,
                                asset_lock_proof,
                                asset_lock_proof_private_key.inner.as_ref(),
                                0,
                                self.platform_version(),
                                None,
                            ) {
                                Ok(transition) => format!(
                                    "error: {}, transaction is {:?}",
                                    e, transition
                                ),
                                Err(transition_err) => format!(
                                    "error: {}, also failed to recreate transition for debugging: {}",
                                    e, transition_err
                                ),
                            }
                        })?
                } else {
                    return Err(error_string);
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
                    actual_fee as i64 - estimated_fee as i64
                );
            }
        } else {
            tracing::info!(
                "Identity top-up complete: balance before {} credits, balance after {} credits",
                balance_before,
                updated_identity_balance
            );
        }

        self.update_local_qualified_identity(&qualified_identity)
            .map_err(|e| e.to_string())?;

        {
            let mut wallet = wallet.write().map_err(|e| e.to_string())?;
            wallet
                .unused_asset_locks
                .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
        }

        self.db
            .set_asset_lock_identity_id(
                tx_id.as_byte_array(),
                qualified_identity.identity.id().as_bytes(),
            )
            .map_err(|e| e.to_string())?;

        if let Some((amount, top_up_index)) = top_up_index {
            self.db
                .insert_top_up(
                    qualified_identity.identity.id().as_bytes(),
                    top_up_index,
                    amount,
                )
                .map_err(|e| e.to_string())?;
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
