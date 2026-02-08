use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::{AppContext, get_transaction_info_via_dapi};
use crate::lock_helper::{MutexExt, RwLockExt};
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use dash_sdk::Error;
use dash_sdk::dashcore_rpc::RpcApi;
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
use std::time::Duration;

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

        let sdk = {
            let guard = self.sdk.read_or_recover();
            guard.clone()
        };

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
                        let wallet = wallet.read_or_recover();
                        wallet
                            .private_key_for_address(&address, self.network)?
                            .ok_or("Asset Lock not valid for wallet")?
                    };
                    let asset_lock_proof = if let AssetLockProof::Instant(
                        instant_asset_lock_proof,
                    ) = asset_lock_proof.as_ref()
                    {
                        // we need to make sure the instant send asset lock is recent
                        let tx_info = get_transaction_info_via_dapi(&sdk, &tx_id).await?;

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
                    let (
                        asset_lock_transaction,
                        asset_lock_proof_private_key,
                        _,
                        used_utxos,
                        wallet_seed_hash,
                    ) = {
                        let mut wallet = wallet.write_or_recover();
                        let seed_hash = wallet.seed_hash();
                        let tx_result = match wallet.top_up_asset_lock_transaction(
                            sdk.network,
                            amount,
                            true,
                            identity_index,
                            top_up_index,
                            Some(self),
                        ) {
                            Ok(transaction) => transaction,
                            Err(_) => {
                                wallet
                                    .reload_utxos(
                                        &self.core_client.read_or_recover(),
                                        self.network,
                                        Some(self),
                                    )
                                    .map_err(|e| e.to_string())?;
                                wallet.top_up_asset_lock_transaction(
                                    sdk.network,
                                    amount,
                                    true,
                                    identity_index,
                                    top_up_index,
                                    Some(self),
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

                    let tx_id = asset_lock_transaction.txid();
                    // todo: maybe one day we will want to use platform again, but for right now we use
                    //  the local core as it is more stable
                    // let asset_lock_proof = self
                    //     .broadcast_and_retrieve_asset_lock(&asset_lock_transaction, &change_address)
                    //     .await
                    //     .map_err(|e| e.to_string())?;

                    {
                        let mut proofs = self.transactions_waiting_for_finality.lock_or_recover();
                        proofs.insert(tx_id, None);
                    }

                    self.core_client
                        .read_or_recover()
                        .send_raw_transaction(&asset_lock_transaction)
                        .map_err(|e| e.to_string())?;

                    // Store the asset lock transaction in the database immediately after sending.
                    // This ensures it's tracked even if the proof times out or top-up fails.
                    // SPV will update the instant_lock_data when it detects the transaction.
                    self.db
                        .store_asset_lock_transaction(
                            &asset_lock_transaction,
                            amount,
                            None, // No islock yet - SPV will update this
                            &wallet_seed_hash,
                            self.network,
                        )
                        .map_err(|e| format!("Failed to store asset lock transaction: {}", e))?;

                    {
                        let mut wallet = wallet.write_or_recover();
                        wallet.utxos.retain(|_, utxo_map| {
                            utxo_map.retain(|outpoint, _| !used_utxos.contains_key(outpoint));
                            !utxo_map.is_empty() // Keep addresses that still have UTXOs
                        });
                        for utxo in used_utxos.keys() {
                            self.db
                                .drop_utxo(utxo, &self.network.to_string())
                                .map_err(|e| e.to_string())?;
                        }

                        // Update address_balances for affected addresses
                        let affected_addresses: std::collections::BTreeSet<_> =
                            used_utxos.values().map(|(_, addr)| addr.clone()).collect();
                        for address in affected_addresses {
                            // Recalculate balance from remaining UTXOs for this address
                            let new_balance = wallet
                                .utxos
                                .get(&address)
                                .map(|utxo_map| utxo_map.values().map(|tx_out| tx_out.value).sum())
                                .unwrap_or(0);
                            if let Err(e) =
                                wallet.update_address_balance(&address, new_balance, self)
                            {
                                tracing::warn!(
                                    "Failed to update address balance in database after identity top-up: {}",
                                    e
                                );
                            }
                        }
                    }

                    // Wait for asset lock proof with timeout (2 minutes)
                    const ASSET_LOCK_PROOF_TIMEOUT: Duration = Duration::from_secs(120);
                    let asset_lock_proof =
                        match tokio::time::timeout(ASSET_LOCK_PROOF_TIMEOUT, async {
                            loop {
                                {
                                    let proofs =
                                        self.transactions_waiting_for_finality.lock_or_recover();
                                    if let Some(Some(proof)) = proofs.get(&tx_id) {
                                        return proof.clone();
                                    }
                                }
                                tokio::time::sleep(Duration::from_millis(200)).await;
                            }
                        })
                        .await
                        {
                            Ok(proof) => proof,
                            Err(_) => {
                                // Clean up on timeout
                                let mut proofs =
                                    self.transactions_waiting_for_finality.lock_or_recover();
                                proofs.remove(&tx_id);
                                return Err(format!(
                                    "Timeout waiting for asset lock proof after {} seconds. \
                                 The transaction may not have been confirmed by the network.",
                                    ASSET_LOCK_PROOF_TIMEOUT.as_secs()
                                ));
                            }
                        };

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
                        let mut wallet = wallet.write_or_recover();
                        let seed_hash = wallet.seed_hash();
                        let tx_result = wallet.top_up_asset_lock_transaction_for_utxo(
                            sdk.network,
                            utxo,
                            tx_out.clone(),
                            input_address.clone(),
                            identity_index,
                            top_up_index,
                            Some(self),
                        )?;
                        (tx_result.0, tx_result.1, seed_hash)
                    };

                    let tx_id = asset_lock_transaction.txid();
                    // todo: maybe one day we will want to use platform again, but for right now we use
                    //  the local core as it is more stable
                    // let asset_lock_proof = self
                    //     .broadcast_and_retrieve_asset_lock(&asset_lock_transaction, &change_address)
                    //     .await
                    //     .map_err(|e| e.to_string())?;

                    {
                        let mut proofs = self.transactions_waiting_for_finality.lock_or_recover();
                        proofs.insert(tx_id, None);
                    }

                    self.core_client
                        .read_or_recover()
                        .send_raw_transaction(&asset_lock_transaction)
                        .map_err(|e| e.to_string())?;

                    // Store the asset lock transaction in the database immediately after sending.
                    // This ensures it's tracked even if the proof times out or top-up fails.
                    // SPV will update the instant_lock_data when it detects the transaction.
                    self.db
                        .store_asset_lock_transaction(
                            &asset_lock_transaction,
                            tx_out.value,
                            None, // No islock yet - SPV will update this
                            &wallet_seed_hash,
                            self.network,
                        )
                        .map_err(|e| format!("Failed to store asset lock transaction: {}", e))?;

                    {
                        let mut wallet = wallet.write_or_recover();
                        wallet.utxos.retain(|_, utxo_map| {
                            utxo_map.retain(|outpoint, _| outpoint != &utxo);
                            !utxo_map.is_empty()
                        });
                        self.db
                            .drop_utxo(&utxo, &self.network.to_string())
                            .map_err(|e| e.to_string())?;

                        // Update address_balance for the affected address
                        let new_balance = wallet
                            .utxos
                            .get(&input_address)
                            .map(|utxo_map| utxo_map.values().map(|tx_out| tx_out.value).sum())
                            .unwrap_or(0);
                        if let Err(e) =
                            wallet.update_address_balance(&input_address, new_balance, self)
                        {
                            tracing::warn!(
                                "Failed to update address balance in database after identity top-up: {}",
                                e
                            );
                        }
                    }

                    // Wait for asset lock proof with timeout (2 minutes)
                    const ASSET_LOCK_PROOF_TIMEOUT: Duration = Duration::from_secs(120);
                    let asset_lock_proof =
                        match tokio::time::timeout(ASSET_LOCK_PROOF_TIMEOUT, async {
                            loop {
                                {
                                    let proofs =
                                        self.transactions_waiting_for_finality.lock_or_recover();
                                    if let Some(Some(proof)) = proofs.get(&tx_id) {
                                        return proof.clone();
                                    }
                                }
                                tokio::time::sleep(Duration::from_millis(200)).await;
                            }
                        })
                        .await
                        {
                            Ok(proof) => proof,
                            Err(_) => {
                                // Clean up on timeout
                                let mut proofs =
                                    self.transactions_waiting_for_finality.lock_or_recover();
                                proofs.remove(&tx_id);
                                return Err(format!(
                                    "Timeout waiting for asset lock proof after {} seconds. \
                                 The transaction may not have been confirmed by the network.",
                                    ASSET_LOCK_PROOF_TIMEOUT.as_secs()
                                ));
                            }
                        };

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
                    let tx_info = get_transaction_info_via_dapi(&sdk, &tx_id).await?;

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
            let mut wallet = wallet.write_or_recover();
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
