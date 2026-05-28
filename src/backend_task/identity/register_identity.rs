use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityRegistrationInfo, RegisterIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::{AppContext, get_transaction_info};
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_sdk::dash_spv::Network;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::dashcore::PrivateKey;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AddressNonce, AssetLockProof};
use dash_sdk::platform::transition::put_identity::PutIdentity;
use dash_sdk::platform::{Fetch, FetchMany, Identity};
use dash_sdk::query_types::AddressInfo;
use std::collections::BTreeMap;

impl AppContext {
    pub(super) async fn register_identity(
        &self,
        input: IdentityRegistrationInfo,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let IdentityRegistrationInfo {
            alias_input,
            keys,
            wallet,
            wallet_identity_index,
            identity_funding_method,
        } = input;

        let sdk = self.sdk.load().as_ref().clone();

        let public_keys = keys
            .to_public_keys_map()
            .map_err(|e| TaskError::PublicKeyMapBuildFailed { detail: e })?;
        let key_count = public_keys.len();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_create(key_count);

        let wallet_seed_hash = { wallet.read().map_err(TaskError::from)?.seed_hash() };

        // Fast-path: a brand-new identity funded directly from this wallet's
        // UTXOs is handled end-to-end by the upstream `IdentityWallet`. It
        // builds the asset lock, broadcasts, waits for IS/CL, submits the
        // identity-create state transition with the upstream IS→CL fallback,
        // and cleans up the tracked asset lock on success. The caller-side
        // retry/fallback chain DET used to maintain is no longer needed.
        if let RegisterIdentityFundingMethod::FundWithWallet(amount_duffs, identity_index) =
            &identity_funding_method
        {
            return self
                .register_identity_via_wallet_backend(
                    *amount_duffs,
                    *identity_index,
                    wallet_identity_index,
                    public_keys,
                    keys,
                    wallet,
                    wallet_seed_hash,
                    alias_input,
                    estimated_fee,
                )
                .await;
        }

        let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None).await?;

        let (asset_lock_proof, asset_lock_proof_private_key, tx_id) = match identity_funding_method
        {
            RegisterIdentityFundingMethod::UseAssetLock(address, asset_lock_proof, transaction) => {
                let tx_id = transaction.txid();

                // Scope the read guard so it's dropped before the async DAPI call below
                let private_key = {
                    let wallet = wallet.read().map_err(TaskError::from)?;
                    wallet
                        .private_key_for_address(&address, self.network)
                        .map_err(|e| TaskError::WalletKeyLookupFailed { detail: e })?
                        .ok_or(TaskError::AssetLockNotValidForWallet)?
                };
                let asset_lock_proof = if let AssetLockProof::Instant(instant_asset_lock_proof) =
                    asset_lock_proof.as_ref()
                {
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
                (asset_lock_proof, private_key, tx_id)
            }
            RegisterIdentityFundingMethod::FundWithWallet(_, _) => {
                unreachable!("FundWithWallet handled by fast-path above")
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
                        .map_err(|e| TaskError::PlatformFetchError {
                            source: Box::new(e),
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
        };

        let identity_id = asset_lock_proof
            .create_identifier()
            .map_err(|e| TaskError::from(dash_sdk::Error::Protocol(e)))?;

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

        let existing_identity = match Identity::fetch_by_identifier(&sdk, identity_id).await {
            Ok(result) => result,
            Err(e) => return Err(TaskError::from(e)),
        };

        let identity = match existing_identity.clone() {
            Some(id) => id,
            None => Identity::new_with_id_and_keys(identity_id, public_keys, sdk.version())
                .map_err(|e| TaskError::IdentityCreationError {
                    source: Box::new(e),
                })?,
        };

        let mut qualified_identity = QualifiedIdentity {
            identity: identity.clone(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: keys.to_key_storage(wallet_seed_hash),
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(wallet_seed_hash, wallet.clone())]),
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
                &Some((wallet_seed_hash, wallet_identity_index)),
            )?;

            {
                let mut wallet = wallet.write().map_err(TaskError::from)?;
                wallet
                    .unused_asset_locks
                    .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
                wallet
                    .identities
                    .insert(wallet_identity_index, qualified_identity.identity.clone());
            }

            self.db
                .set_asset_lock_identity_id(tx_id.as_byte_array(), identity_id.as_bytes())?;

            let fee_result = FeeResult::new(estimated_fee, estimated_fee);
            return Ok(BackendTaskSuccessResult::RegisteredIdentity(
                qualified_identity,
                fee_result,
            ));
        }

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_seed_hash, wallet_identity_index)),
        )?;
        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                identity_id.as_bytes(),
            )?;

        match self
            .put_identity_with_staged_asset_lock(
                &identity,
                asset_lock_proof.clone(),
                &asset_lock_proof_private_key,
                &qualified_identity,
            )
            .await
        {
            Ok(updated_identity) => {
                qualified_identity.identity = updated_identity;
                qualified_identity.status = IdentityStatus::Unknown; // force refresh of the status
            }
            Err(e) => {
                if matches!(e, TaskError::AssetLockInstantLockProofInvalid { .. }) {
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
                            match self
                                .put_identity_with_staged_asset_lock(
                                    &identity,
                                    chain_asset_lock_proof,
                                    &asset_lock_proof_private_key,
                                    &qualified_identity,
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
                                        &Some((wallet_seed_hash, wallet_identity_index)),
                                    )?;

                                    return Err(retry_err);
                                }
                            }
                        } else {
                            qualified_identity
                                .status
                                .update(IdentityStatus::FailedCreation);

                            self.insert_local_qualified_identity(
                                &qualified_identity,
                                &Some((wallet_seed_hash, wallet_identity_index)),
                            )?;

                            return Err(TaskError::AssetLockExpired {
                                tx_block_height,
                                platform_height: metadata.core_chain_locked_height,
                            });
                        }
                    } else {
                        qualified_identity
                            .status
                            .update(IdentityStatus::FailedCreation);

                        self.insert_local_qualified_identity(
                            &qualified_identity,
                            &Some((wallet_seed_hash, wallet_identity_index)),
                        )?;

                        return Err(TaskError::AssetLockInstantLockExpiredNotChainlocked);
                    }
                } else {
                    // we failed, set the status accordingly and terminate the process
                    qualified_identity
                        .status
                        .update(IdentityStatus::FailedCreation);

                    self.insert_local_qualified_identity(
                        &qualified_identity,
                        &Some((wallet_seed_hash, wallet_identity_index)),
                    )?;

                    return Err(e);
                }
            }
        }

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_seed_hash, wallet_identity_index)),
        )?;
        {
            let mut wallet = wallet.write().map_err(TaskError::from)?;
            wallet
                .unused_asset_locks
                .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
            wallet.identities.insert(wallet_identity_index, identity);
        }

        self.db
            .set_asset_lock_identity_id(tx_id.as_byte_array(), identity_id.as_bytes())?;

        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::RegisteredIdentity(
            qualified_identity,
            fee_result,
        ))
    }

    /// Drive identity registration through the upstream signer-driven
    /// orchestrator. Upstream owns asset-lock build/broadcast, IS→CL
    /// fallback, the actual submit, and the tracked-asset-lock cleanup —
    /// DET stays out of that loop and only updates its own local mirror.
    #[allow(clippy::too_many_arguments)]
    async fn register_identity_via_wallet_backend(
        &self,
        amount_duffs: u64,
        identity_index: u32,
        wallet_identity_index: u32,
        public_keys: BTreeMap<
            dash_sdk::dpp::identity::KeyID,
            dash_sdk::dpp::identity::IdentityPublicKey,
        >,
        keys: super::IdentityKeys,
        wallet: std::sync::Arc<std::sync::RwLock<super::Wallet>>,
        wallet_seed_hash: super::WalletSeedHash,
        alias_input: String,
        estimated_fee: u64,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let backend = self.wallet_backend()?;

        // Build a placeholder identity to seed the local QualifiedIdentity
        // bookkeeping; the upstream call returns the authoritative Identity
        // and we replace it on success.
        let placeholder_id = dash_sdk::platform::Identifier::default();
        let placeholder_identity = Identity::new_with_id_and_keys(
            placeholder_id,
            public_keys.clone(),
            self.platform_version(),
        )
        .map_err(|e| TaskError::IdentityCreationError {
            source: Box::new(e),
        })?;

        let mut qualified_identity = QualifiedIdentity {
            identity: placeholder_identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: if alias_input.is_empty() {
                None
            } else {
                Some(alias_input)
            },
            private_keys: keys.to_key_storage(wallet_seed_hash),
            dpns_names: vec![],
            associated_wallets: BTreeMap::from([(wallet_seed_hash, wallet.clone())]),
            wallet_index: Some(wallet_identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::PendingCreation,
            network: self.network,
        };

        let registered_identity = backend
            .register_identity(
                &wallet_seed_hash,
                identity_index,
                amount_duffs,
                public_keys,
                &qualified_identity,
                None,
            )
            .await
            .inspect_err(|_| {
                qualified_identity
                    .status
                    .update(IdentityStatus::FailedCreation);
                let _ = self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                );
            })?;

        qualified_identity.identity = registered_identity.clone();
        qualified_identity.status = IdentityStatus::Unknown; // force refresh

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_seed_hash, wallet_identity_index)),
        )?;
        {
            let mut wallet_w = wallet.write().map_err(TaskError::from)?;
            wallet_w
                .identities
                .insert(wallet_identity_index, registered_identity);
        }
        // The upstream identity discovery loop owns the asset-lock → identity
        // mapping on the new path; the DET-side `asset_lock_to_identity_id`
        // table is only consulted on the legacy staged-asset-lock recovery
        // path, so no mirror write is needed here.

        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::RegisteredIdentity(
            qualified_identity,
            fee_result,
        ))
    }

    /// Submit a `PutIdentity` for an asset lock the user staged separately
    /// (no upstream wallet-backend orchestration).
    async fn put_identity_with_staged_asset_lock(
        &self,
        identity: &Identity,
        asset_lock_proof: AssetLockProof,
        asset_lock_proof_private_key: &PrivateKey,
        qualified_identity: &QualifiedIdentity,
    ) -> Result<Identity, TaskError> {
        let sdk = self.sdk.load().as_ref().clone();
        identity
            .put_to_platform_and_wait_for_response_with_private_key(
                &sdk,
                asset_lock_proof,
                asset_lock_proof_private_key,
                qualified_identity,
                None,
            )
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use dash_sdk::platform::transition::put_identity::PutIdentity;

        let sdk = self.sdk.load().as_ref().clone();

        let public_keys = keys
            .to_public_keys_map()
            .map_err(|e| TaskError::PublicKeyMapBuildFailed { detail: e })?;

        // Calculate fee estimate for identity creation from platform addresses
        let key_count = public_keys.len();
        let input_count = inputs.len();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_create_from_addresses(
            input_count,
            false,
            key_count,
        );

        // Clone the wallet for use as the address signer (needed across async boundary)
        let wallet_clone = { wallet.read().map_err(TaskError::from)?.clone() };

        let identity = Identity::new_with_input_addresses_and_keys(
            &inputs,
            public_keys.clone(),
            sdk.version(),
        )
        .map_err(|e| TaskError::IdentityCreationError {
            source: Box::new(e),
        })?;

        let wallet_seed_hash_actual = { wallet.read().map_err(TaskError::from)?.seed_hash() };
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
                )?;

                {
                    let mut wallet_guard = wallet.write().map_err(TaskError::from)?;
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
                // Log proof errors and convert via log_drive_proof_error for consistent handling
                let task_error =
                    self.log_drive_proof_error(e, RequestType::BroadcastStateTransition);

                qualified_identity
                    .status
                    .update(IdentityStatus::FailedCreation);

                self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_seed_hash, wallet_identity_index)),
                )?;

                Err(task_error)
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
