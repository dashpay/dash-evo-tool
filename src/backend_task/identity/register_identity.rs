use crate::backend_task::identity::{IdentityRegistrationInfo, RegisterIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::{AppContext, get_transaction_info_via_dapi};
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_sdk::dash_spv::Network;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{OutPoint, PrivateKey};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::native_bls::NativeBlsModule;
use dash_sdk::dpp::prelude::{AddressNonce, AssetLockProof};
use dash_sdk::dpp::state_transition::identity_create_transition::IdentityCreateTransition;
use dash_sdk::dpp::state_transition::identity_create_transition::methods::IdentityCreateTransitionMethodsV0;
use dash_sdk::platform::transition::put_identity::PutIdentity;
use dash_sdk::platform::{Fetch, FetchMany, Identity};
use dash_sdk::query_types::AddressInfo;
use dash_sdk::{Error, Sdk};
use std::collections::BTreeMap;
use std::time::Duration;

impl AppContext {
    pub(super) async fn register_identity(
        &self,
        input: IdentityRegistrationInfo,
    ) -> Result<BackendTaskSuccessResult, String> {
        let IdentityRegistrationInfo {
            alias_input,
            keys,
            wallet,
            wallet_identity_index,
            identity_funding_method,
        } = input;

        let sdk = {
            let guard = self.sdk.read().unwrap();
            guard.clone()
        };

        let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None)
            .await
            .map_err(|e| e.to_string())?;

        let wallet_id;

        let (asset_lock_proof, asset_lock_proof_private_key, tx_id) = match identity_funding_method
        {
            RegisterIdentityFundingMethod::UseAssetLock(address, asset_lock_proof, transaction) => {
                let tx_id = transaction.txid();

                // Scope the read guard so it's dropped before the async DAPI call below
                let private_key = {
                    let wallet = wallet.read().unwrap();
                    wallet_id = wallet.seed_hash();
                    wallet
                        .private_key_for_address(&address, self.network)?
                        .ok_or("Asset Lock not valid for wallet")?
                };
                let asset_lock_proof = if let AssetLockProof::Instant(instant_asset_lock_proof) =
                    asset_lock_proof.as_ref()
                {
                    // we need to make sure the instant send asset lock is recent
                    let tx_info = get_transaction_info_via_dapi(&sdk, &tx_id).await?;

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
                (asset_lock_proof, private_key, tx_id)
            }
            RegisterIdentityFundingMethod::FundWithWallet(amount, identity_index) => {
                // Scope the write lock to avoid holding it across an await.
                let (asset_lock_transaction, asset_lock_proof_private_key, _, used_utxos) = {
                    let mut wallet = wallet.write().unwrap();
                    wallet_id = wallet.seed_hash();
                    match wallet.registration_asset_lock_transaction(
                        sdk.network,
                        amount,
                        true,
                        identity_index,
                        Some(self),
                    ) {
                        Ok(transaction) => transaction,
                        Err(_) => {
                            wallet
                                .reload_utxos(
                                    &self
                                        .core_client
                                        .read()
                                        .expect("Core client lock was poisoned"),
                                    self.network,
                                    Some(self),
                                )
                                .map_err(|e| e.to_string())?;
                            wallet.registration_asset_lock_transaction(
                                sdk.network,
                                amount,
                                true,
                                identity_index,
                                Some(self),
                            )?
                        }
                    }
                };

                let tx_id = asset_lock_transaction.txid();

                {
                    let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
                    proofs.insert(tx_id, None);
                }

                self.core_client
                    .read()
                    .expect("Core client lock was poisoned")
                    .send_raw_transaction(&asset_lock_transaction)
                    .map_err(|e| e.to_string())?;

                // Store the asset lock transaction in the database immediately after sending.
                // This ensures it's tracked even if the proof times out or identity creation fails.
                // SPV will update the instant_lock_data when it detects the transaction.
                self.db
                    .store_asset_lock_transaction(
                        &asset_lock_transaction,
                        amount,
                        None, // No islock yet - SPV will update this
                        &wallet_id,
                        self.network,
                    )
                    .map_err(|e| format!("Failed to store asset lock transaction: {}", e))?;

                // TODO: UTXO removal timing issue - UTXOs are removed here BEFORE the asset
                // lock proof is confirmed below. If the transaction fails or times out after
                // this point, the UTXOs will be "lost" from wallet tracking even though they
                // weren't actually spent. This should be refactored to remove UTXOs only AFTER
                // successful proof confirmation. See Phase 2.2 in PR review plan.
                {
                    let mut wallet = wallet.write().unwrap();
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
                        let _ = wallet.update_address_balance(&address, new_balance, self);
                    }
                }

                // Wait for asset lock proof with timeout (2 minutes)
                const ASSET_LOCK_PROOF_TIMEOUT: Duration = Duration::from_secs(120);
                let asset_lock_proof = match tokio::time::timeout(ASSET_LOCK_PROOF_TIMEOUT, async {
                    loop {
                        {
                            let proofs = self.transactions_waiting_for_finality.lock().unwrap();
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
                        let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
                        proofs.remove(&tx_id);
                        return Err(format!(
                            "Timeout waiting for asset lock proof after {} seconds. \
                             The transaction may not have been confirmed by the network.",
                            ASSET_LOCK_PROOF_TIMEOUT.as_secs()
                        ));
                    }
                };

                (asset_lock_proof, asset_lock_proof_private_key, tx_id)
            }
            RegisterIdentityFundingMethod::FundWithPlatformAddresses {
                inputs,
                wallet_seed_hash,
            } => {
                // Fetch fresh nonces from platform to ensure we have current values
                let addresses_to_fetch: std::collections::BTreeSet<PlatformAddress> =
                    inputs.keys().cloned().collect();

                let fetched_address_infos =
                    AddressInfo::fetch_many(&sdk, addresses_to_fetch.clone())
                        .await
                        .map_err(|e| {
                            format!("Failed to fetch address info from platform: {}", e)
                        })?;

                // Build inputs with fresh nonces incremented by 1
                let inputs_with_nonces = inputs
                    .into_iter()
                    .map(|(addr, credits)| {
                        // Get the fetched info, falling back to cached info if not found on platform
                        let nonce = fetched_address_infos
                            .get(&addr)
                            .and_then(|opt| opt.as_ref())
                            .map(|info| info.nonce)
                            .or_else(|| {
                                self.get_platform_address_best_info(&addr, self.network)
                                    .map(|info| info.nonce)
                            })
                            .unwrap_or(0);
                        (addr, (nonce.saturating_add(1), credits))
                    })
                    .collect::<BTreeMap<PlatformAddress, (AddressNonce, Credits)>>();

                return self
                    .register_identity_from_platform_addresses(
                        alias_input,
                        keys,
                        wallet,
                        wallet_identity_index,
                        inputs_with_nonces,
                        wallet_seed_hash,
                    )
                    .await;
            }
            RegisterIdentityFundingMethod::FundWithUtxo(
                utxo,
                tx_out,
                input_address,
                identity_index,
            ) => {
                // Scope the write lock to avoid holding it across an await.
                let (asset_lock_transaction, asset_lock_proof_private_key) = {
                    let mut wallet = wallet.write().unwrap();
                    wallet_id = wallet.seed_hash();
                    wallet.registration_asset_lock_transaction_for_utxo(
                        sdk.network,
                        utxo,
                        tx_out.clone(),
                        input_address.clone(),
                        identity_index,
                        Some(self),
                    )?
                };

                let tx_id = asset_lock_transaction.txid();

                {
                    let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
                    proofs.insert(tx_id, None);
                }

                self.core_client
                    .read()
                    .expect("Core client lock was poisoned")
                    .send_raw_transaction(&asset_lock_transaction)
                    .map_err(|e| e.to_string())?;

                // Store the asset lock transaction in the database immediately after sending.
                // This ensures it's tracked even if the proof times out or identity creation fails.
                // SPV will update the instant_lock_data when it detects the transaction.
                self.db
                    .store_asset_lock_transaction(
                        &asset_lock_transaction,
                        tx_out.value,
                        None, // No islock yet - SPV will update this
                        &wallet_id,
                        self.network,
                    )
                    .map_err(|e| format!("Failed to store asset lock transaction: {}", e))?;

                // TODO: UTXO removal timing issue - see comment above for FundWithWallet case.
                {
                    let mut wallet = wallet.write().unwrap();
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
                    let _ = wallet.update_address_balance(&input_address, new_balance, self);
                }

                // Wait for asset lock proof with timeout (2 minutes)
                const ASSET_LOCK_PROOF_TIMEOUT: Duration = Duration::from_secs(120);
                let asset_lock_proof = match tokio::time::timeout(ASSET_LOCK_PROOF_TIMEOUT, async {
                    loop {
                        {
                            let proofs = self.transactions_waiting_for_finality.lock().unwrap();
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
                        let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
                        proofs.remove(&tx_id);
                        return Err(format!(
                            "Timeout waiting for asset lock proof after {} seconds. \
                             The transaction may not have been confirmed by the network.",
                            ASSET_LOCK_PROOF_TIMEOUT.as_secs()
                        ));
                    }
                };

                (asset_lock_proof, asset_lock_proof_private_key, tx_id)
            }
        };

        let identity_id = asset_lock_proof
            .create_identifier()
            .expect("expected to create an identifier");

        let public_keys = keys.to_public_keys_map()?;

        // Debug: Log the keys being registered to verify contract bounds are set
        for (key_id, key) in &public_keys {
            match key {
                dash_sdk::dpp::identity::IdentityPublicKey::V0(key_v0) => {
                    tracing::info!(
                        "Identity key {}: purpose={:?}, security_level={:?}, key_type={:?}, contract_bounds={:?}",
                        key_id,
                        key_v0.purpose,
                        key_v0.security_level,
                        key_v0.key_type,
                        key_v0.contract_bounds
                    );
                }
            }
        }

        // Calculate fee estimate for identity creation
        let key_count = public_keys.len();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_create(key_count);

        let existing_identity = match Identity::fetch_by_identifier(&sdk, identity_id).await {
            Ok(result) => result,
            Err(e) => return Err(format!("Error fetching identity: {}", e)),
        };

        let identity = existing_identity.clone().unwrap_or_else(|| {
            Identity::new_with_id_and_keys(identity_id, public_keys, sdk.version())
                .expect("expected to make identity")
        });

        let wallet_seed_hash = { wallet.read().unwrap().seed_hash() };
        let mut qualified_identity = QualifiedIdentity {
            identity: identity.clone(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: keys.to_key_storage(wallet_seed_hash),
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(
                wallet.read().unwrap().seed_hash(),
                wallet.clone(),
            )]),
            wallet_index: Some(wallet_identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::PendingCreation,
            network: self.network,
        };

        if !alias_input.is_empty() {
            qualified_identity.alias = Some(alias_input);
        }

        if let Some(existing_identity) = existing_identity {
            qualified_identity.identity = existing_identity;
            qualified_identity.status = IdentityStatus::Unknown;

            self.insert_local_qualified_identity(
                &qualified_identity,
                &Some((wallet_id, wallet_identity_index)),
            )
            .map_err(|e| e.to_string())?;

            {
                let mut wallet = wallet.write().unwrap();
                wallet
                    .unused_asset_locks
                    .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
                wallet
                    .identities
                    .insert(wallet_identity_index, qualified_identity.identity.clone());
            }

            self.db
                .set_asset_lock_identity_id(tx_id.as_byte_array(), identity_id.as_bytes())
                .map_err(|e| e.to_string())?;

            let fee_result = FeeResult::new(estimated_fee, estimated_fee);
            return Ok(BackendTaskSuccessResult::RegisteredIdentity(
                qualified_identity,
                fee_result,
            ));
        }

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_id, wallet_identity_index)),
        )
        .map_err(|e| e.to_string())?;
        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                identity_id.as_bytes(),
            )
            .map_err(|e| e.to_string())?;

        match self
            .put_new_identity_to_platform(
                &sdk,
                &identity,
                asset_lock_proof.clone(),
                &asset_lock_proof_private_key,
                qualified_identity.clone(),
            )
            .await
        {
            Ok(updated_identity) => {
                qualified_identity.identity = updated_identity;
                qualified_identity.status = IdentityStatus::Unknown; // force refresh of the status
            }
            Err(e) => {
                // Check if this is an instant lock proof expiration error
                if e.contains("Instant lock proof signature is invalid")
                    || e.contains("wasn't created recently")
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
                            match self
                                .put_new_identity_to_platform(
                                    &sdk,
                                    &identity,
                                    chain_asset_lock_proof,
                                    &asset_lock_proof_private_key,
                                    qualified_identity.clone(),
                                )
                                .await
                            {
                                Ok(updated_identity) => {
                                    qualified_identity.identity = updated_identity;
                                    qualified_identity.status = IdentityStatus::Unknown;
                                }
                                Err(retry_err) => {
                                    qualified_identity
                                        .status
                                        .update(IdentityStatus::FailedCreation);

                                    self.insert_local_qualified_identity(
                                        &qualified_identity,
                                        &Some((wallet_id, wallet_identity_index)),
                                    )
                                    .map_err(|e| e.to_string())?;

                                    return Err(retry_err);
                                }
                            }
                        } else {
                            qualified_identity
                                .status
                                .update(IdentityStatus::FailedCreation);

                            self.insert_local_qualified_identity(
                                &qualified_identity,
                                &Some((wallet_id, wallet_identity_index)),
                            )
                            .map_err(|e| e.to_string())?;

                            return Err(format!(
                                "Cannot use this asset lock yet. The instant lock proof has expired (quorum rotated), \
                                and Platform hasn't verified Core block {} yet (Platform has verified up to Core block {}). \
                                Please wait for Platform to sync with Core chain.",
                                tx_block_height, metadata.core_chain_locked_height
                            ));
                        }
                    } else {
                        qualified_identity
                            .status
                            .update(IdentityStatus::FailedCreation);

                        self.insert_local_qualified_identity(
                            &qualified_identity,
                            &Some((wallet_id, wallet_identity_index)),
                        )
                        .map_err(|e| e.to_string())?;

                        return Err("Cannot use this asset lock. The instant lock proof has expired and the transaction \
                            is not yet chainlocked. Please wait for the transaction to be chainlocked.".to_string());
                    }
                } else {
                    // we failed, set the status accordingly and terminate the process
                    qualified_identity
                        .status
                        .update(IdentityStatus::FailedCreation);

                    self.insert_local_qualified_identity(
                        &qualified_identity,
                        &Some((wallet_id, wallet_identity_index)),
                    )
                    .map_err(|e| e.to_string())?;

                    return Err(e);
                }
            }
        }

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_id, wallet_identity_index)),
        )
        .map_err(|e| e.to_string())?;
        {
            let mut wallet = wallet.write().unwrap();
            wallet
                .unused_asset_locks
                .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
            wallet.identities.insert(wallet_identity_index, identity);
        }

        self.db
            .set_asset_lock_identity_id(tx_id.as_byte_array(), identity_id.as_bytes())
            .map_err(|e| e.to_string())?;

        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::RegisteredIdentity(
            qualified_identity,
            fee_result,
        ))
    }

    async fn put_new_identity_to_platform(
        &self,
        sdk: &Sdk,
        identity: &Identity,
        asset_lock_proof: AssetLockProof,
        asset_lock_proof_private_key: &PrivateKey,
        qualified_identity: QualifiedIdentity,
    ) -> Result<Identity, String> {
        match identity
            .put_to_platform_and_wait_for_response(
                sdk,
                asset_lock_proof.clone(),
                asset_lock_proof_private_key,
                &qualified_identity,
                None,
            )
            .await
        {
            Ok(updated_identity) => Ok(updated_identity),
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
                        "Error registering identity: {}, proof error logged",
                        proof_error
                    ));
                }

                if matches!(e, Error::Protocol(ProtocolError::UnknownVersionError(_))) {
                    identity
                        .put_to_platform_and_wait_for_response(
                            sdk,
                            asset_lock_proof.clone(),
                            asset_lock_proof_private_key,
                            &qualified_identity,
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
                                    "Error registering identity: {}, proof error logged",
                                    proof_error
                                );
                            }

                            let identity_create_transition =
                                IdentityCreateTransition::try_from_identity_with_signer(
                                    identity,
                                    asset_lock_proof,
                                    asset_lock_proof_private_key.inner.as_ref(),
                                    &qualified_identity,
                                    &NativeBlsModule,
                                    0,
                                    self.platform_version(),
                                )
                                .expect("expected to make transition");
                            format!(
                                "error: {}, transaction is {:?}",
                                e, identity_create_transition
                            )
                        })
                } else {
                    Err(e.to_string())
                }
            }
        }
    }

    /// Register a new identity funded by Platform addresses.
    ///
    /// `inputs` is a map of Platform addresses to (nonce, credits) tuples. Nonces must be incremented by 1
    /// from the current nonce of the address.
    async fn register_identity_from_platform_addresses(
        &self,
        alias_input: String,
        keys: super::IdentityKeys,
        wallet: std::sync::Arc<std::sync::RwLock<super::Wallet>>,
        wallet_identity_index: u32,
        inputs: BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            (AddressNonce, dash_sdk::dpp::fee::Credits),
        >,
        wallet_seed_hash: super::WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::platform::transition::put_identity::PutIdentity;

        let sdk = {
            let guard = self.sdk.read().unwrap();
            guard.clone()
        };

        let public_keys = keys.to_public_keys_map()?;

        // Calculate fee estimate for identity creation from platform addresses
        let key_count = public_keys.len();
        let input_count = inputs.len();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_create_from_addresses(
            input_count,
            false,
            key_count,
        );

        // Clone the wallet for use as the address signer (needed across async boundary)
        let wallet_clone = { wallet.read().map_err(|e| e.to_string())?.clone() };

        let identity = Identity::new_with_input_addresses_and_keys(
            &inputs,
            public_keys.clone(),
            sdk.version(),
        )
        .map_err(|e| format!("Failed to create identity: {}", e))?;

        let wallet_seed_hash_actual = { wallet.read().unwrap().seed_hash() };
        let mut qualified_identity = QualifiedIdentity {
            identity: identity.clone(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: keys.to_key_storage(wallet_seed_hash_actual),
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(wallet_seed_hash_actual, wallet.clone())]),
            wallet_index: Some(wallet_identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::PendingCreation,
            network: self.network,
        };

        if !alias_input.is_empty() {
            qualified_identity.alias = Some(alias_input);
        }

        // Send to Platform using address funding and wait for response
        match identity
            .put_with_address_funding(&sdk, inputs, None, &qualified_identity, &wallet_clone, None)
            .await
        {
            Ok((updated_identity, address_infos)) => {
                qualified_identity.identity = updated_identity;
                qualified_identity.status = IdentityStatus::Unknown; // Force refresh

                // Update source address balances using proof-verified data from SDK response
                if let Err(e) = self
                    .update_wallet_platform_address_info_from_sdk(wallet_seed_hash, &address_infos)
                {
                    tracing::warn!("Failed to update wallet platform address info: {}", e);
                }

                self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                )
                .map_err(|e| e.to_string())?;

                {
                    let mut wallet_guard = wallet.write().unwrap();
                    wallet_guard
                        .identities
                        .insert(wallet_identity_index, qualified_identity.identity.clone());
                }

                let fee_result = FeeResult::new(estimated_fee, estimated_fee);
                Ok(BackendTaskSuccessResult::RegisteredIdentity(
                    qualified_identity,
                    fee_result,
                ))
            }
            Err(e) => {
                // Log proof errors
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

                    qualified_identity
                        .status
                        .update(IdentityStatus::FailedCreation);

                    self.insert_local_qualified_identity(
                        &qualified_identity,
                        &Some((wallet_seed_hash, wallet_identity_index)),
                    )
                    .map_err(|e| e.to_string())?;

                    return Err(format!(
                        "Failed to create identity from Platform addresses: {}, proof error logged",
                        proof_error
                    ));
                }

                qualified_identity
                    .status
                    .update(IdentityStatus::FailedCreation);

                self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                )
                .map_err(|e| e.to_string())?;

                Err(format!(
                    "Failed to create identity from Platform addresses: {}",
                    e
                ))
            }
        }
    }

    /// Get the best (most recent nonce) AddressInfo from all wallets for the given [PlatformAddress] in current [Self::network].
    ///
    /// Returns `None`` if no info is found.
    fn get_platform_address_best_info(
        &self,
        platform_address: &PlatformAddress,
        network: Network,
    ) -> Option<AddressInfo> {
        let generic_address = platform_address.to_address_with_network(network);
        let wallets = self
            .wallets
            .read()
            .inspect_err(|e| tracing::error!(err=%e, "wallet lock poisoned"))
            .ok()?;

        let mut recent_info: Option<AddressInfo> = None;
        for wallet in wallets.values() {
            let wallet_guard = wallet.read().ok()?;

            if let Some(new_info) = wallet_guard.get_platform_address_info(&generic_address)
                && recent_info
                    .as_ref()
                    .is_none_or(|recent| new_info.nonce > recent.nonce)
            {
                recent_info = Some(AddressInfo {
                    address: *platform_address,
                    balance: new_info.balance,
                    nonce: new_info.nonce,
                });
            }
        }

        recent_info
    }
}
