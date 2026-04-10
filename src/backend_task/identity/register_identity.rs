use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityRegistrationInfo, RegisterIdentityFundingMethod};
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use crate::platform_wallet_bridge::IdentityFunding;
use dash_sdk::Error;
use dash_sdk::dash_spv::Network;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::prelude::AddressNonce;
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

        let wallet_id;

        let (asset_lock_proof, out_point) = match identity_funding_method {
            RegisterIdentityFundingMethod::UseAssetLock(out_point) => {
                let platform_wallet = {
                    let guard = wallet.read().map_err(TaskError::from)?;
                    wallet_id = guard.seed_hash();
                    guard
                        .platform_wallet
                        .clone()
                        .ok_or(TaskError::WalletNotFound)?
                };

                // platform-wallet handles IS→CL fallback and key re-derivation internally
                let (asset_lock_proof, _private_key) = platform_wallet
                    .asset_locks()
                    .resume_asset_lock(&out_point, std::time::Duration::from_secs(300))
                    .await
                    .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                        detail: e.to_string(),
                    })?;

                (asset_lock_proof, out_point)
            }
            RegisterIdentityFundingMethod::FundWithWallet(amount, identity_index) => {
                let platform_wallet = {
                    let guard = wallet.read().map_err(TaskError::from)?;
                    wallet_id = guard.seed_hash();
                    guard
                        .platform_wallet
                        .clone()
                        .ok_or(TaskError::WalletNotFound)?
                };

                // Single call: builds asset lock TX, broadcasts, waits for
                // finality proof (IS or CL), and returns the proof + key.
                let (asset_lock_proof, _asset_lock_proof_private_key, out_point) = platform_wallet
                    .asset_locks()
                    .create_funded_asset_lock_proof(
                        amount,
                        0,
                        platform_wallet::AssetLockFundingType::IdentityRegistration,
                        identity_index,
                    )
                    .await
                    .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                        detail: e.to_string(),
                    })?;

                (asset_lock_proof, out_point)
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

        let public_keys = keys
            .to_public_keys_map()
            .map_err(|e| TaskError::PublicKeyMapBuildFailed { detail: e })?;

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
            Err(e) => return Err(TaskError::from(e)),
        };

        let identity = match existing_identity.clone() {
            Some(id) => id,
            None => Identity::new_with_id_and_keys(identity_id, public_keys, sdk.version())
                .map_err(|e| TaskError::IdentityCreationError {
                    source: Box::new(e),
                })?,
        };

        let wallet_seed_hash = { wallet.read().map_err(TaskError::from)?.seed_hash() };
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
                wallet.read().map_err(TaskError::from)?.seed_hash(),
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

        let tx_id = out_point.txid;

        if let Some(existing_identity) = existing_identity {
            qualified_identity.identity = existing_identity;
            qualified_identity.status = IdentityStatus::Unknown;

            self.insert_local_qualified_identity(
                &qualified_identity,
                &Some((wallet_id, wallet_identity_index)),
            )?;

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
            &Some((wallet_id, wallet_identity_index)),
        )?;
        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                identity_id.as_bytes(),
            )?;

        // Use the one-call API which handles IS→CL fallback internally.
        // The asset lock is already tracked by the manager from the funding
        // phase above, so FromExistingAssetLock resumes it efficiently.
        match self
            .put_new_identity_to_platform(
                &identity,
                IdentityFunding::FromExistingAssetLock { out_point },
                wallet_identity_index,
                qualified_identity.clone(),
                &wallet_id,
            )
            .await
        {
            Ok(updated_identity) => {
                qualified_identity.identity = updated_identity;
                qualified_identity.status = IdentityStatus::Unknown;
            }
            Err(e) => {
                qualified_identity
                    .status
                    .update(IdentityStatus::FailedCreation);

                self.insert_local_qualified_identity(
                    &qualified_identity,
                    &Some((wallet_id, wallet_identity_index)),
                )?;

                return Err(e);
            }
        }

        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_id, wallet_identity_index)),
        )?;

        self.db
            .set_asset_lock_identity_id(tx_id.as_byte_array(), identity_id.as_bytes())?;

        // Stage an IdentityChangeSet capturing the confirmed identity and its
        // balance so the changeset reflects the Platform confirmation.
        //
        // TODO(Phase 9a-5d): this is a duplicate write — the identity is
        // already inserted via `insert_local_qualified_identity` above
        // and via the platform-wallet's own identity manager. The plan
        // calls for backend tasks to mutate the platform wallet
        // exclusively and let the persister catch the emitted changeset
        // automatically. Until that wiring lands, the explicit queue
        // here is the source of truth for the persister round-trip.
        if let Some(pw) = self.get_platform_wallet(&wallet_id) {
            use platform_wallet::changeset::changeset::{
                IdentityChangeSet, IdentityEntry, PlatformWalletChangeSet,
            };
            let changeset = PlatformWalletChangeSet {
                identities: Some(IdentityChangeSet {
                    identities: BTreeMap::from([(
                        identity_id,
                        IdentityEntry {
                            identity: qualified_identity.identity.clone(),
                            identity_index: wallet_identity_index,
                            label: qualified_identity.alias.clone(),
                            last_updated_balance_block_time: None,
                            last_synced_keys_block_time: None,
                            dpns_names: vec![],
                            top_ups: Default::default(),
                            status: Default::default(),
                            key_storage: Default::default(),
                            wallet_seed_hash: Some(wallet_id),
                        },
                    )]),
                    removed: Default::default(),
                    primary_identity: None,
                    last_scanned_index: None,
                }),
                ..Default::default()
            };
            pw.queue_persist(changeset);
        }

        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::RegisteredIdentity(
            qualified_identity,
            fee_result,
        ))
    }

    async fn put_new_identity_to_platform(
        &self,
        identity: &Identity,
        funding: IdentityFunding,
        identity_index: u32,
        qualified_identity: QualifiedIdentity,
        wallet_seed_hash: &[u8; 32],
    ) -> Result<Identity, TaskError> {
        // Use the one-call API which handles IS→CL fallback internally.
        let platform_wallet = self
            .get_platform_wallet(wallet_seed_hash)
            .ok_or(TaskError::WalletNotFound)?;

        let result = platform_wallet
            .identity()
            .funded_register_identity(
                identity,
                funding.clone(),
                identity_index,
                &qualified_identity,
                None,
            )
            .await;

        match result {
            Ok(updated_identity) => Ok(updated_identity),
            Err(platform_wallet::PlatformWalletError::Sdk(ref e))
                if matches!(e, Error::Protocol(ProtocolError::UnknownVersionError(_))) =>
            {
                // Retry once on version mismatch.
                let retry_result = platform_wallet
                    .identity()
                    .funded_register_identity(
                        identity,
                        funding,
                        identity_index,
                        &qualified_identity,
                        None,
                    )
                    .await;

                retry_result.map_err(|retry_err| match retry_err {
                    platform_wallet::PlatformWalletError::Sdk(sdk_err) => {
                        self.log_drive_proof_error(sdk_err, RequestType::BroadcastStateTransition)
                    }
                    other => TaskError::PlatformWallet {
                        source: Box::new(other),
                    },
                })
            }
            Err(platform_wallet::PlatformWalletError::Sdk(e)) => {
                Err(self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))
            }
            Err(other) => Err(TaskError::PlatformWallet {
                source: Box::new(other),
            }),
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

        // Get the platform wallet's address signer (PlatformAddressWallet implements Signer<PlatformAddress>)
        let platform_wallet = {
            let wallet_guard = wallet.read().map_err(TaskError::from)?;
            wallet_guard
                .platform_wallet
                .as_ref()
                .cloned()
                .ok_or(TaskError::WalletLocked)?
        };

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
            .put_with_address_funding(
                &sdk,
                inputs,
                None,
                &qualified_identity,
                platform_wallet.platform(),
                None,
            )
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
    /// Returns `None` if no info is found.
    fn get_platform_address_best_info(
        &self,
        platform_address: &PlatformAddress,
        network: Network,
    ) -> Option<AddressInfo> {
        let core_addr = platform_address.to_address_with_network(network);
        let wallets = self
            .wallets
            .read()
            .inspect_err(|e| tracing::error!(err=%e, "wallet lock poisoned"))
            .ok()?;

        let mut recent_info: Option<AddressInfo> = None;
        for wallet in wallets.values() {
            let wallet_guard = wallet.read().ok()?;
            if let Ok(Some((balance, nonce))) =
                self.db
                    .get_platform_address_info(&wallet_guard.seed_hash(), &core_addr, &network)
            {
                if recent_info
                    .as_ref()
                    .is_none_or(|recent| nonce > recent.nonce)
                {
                    recent_info = Some(AddressInfo {
                        address: *platform_address,
                        balance,
                        nonce,
                    });
                }
            }
        }

        recent_info
    }
}
