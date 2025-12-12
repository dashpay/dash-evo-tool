use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::identity::{IdentityRegistrationInfo, RegisterIdentityFundingMethod};
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{OutPoint, PrivateKey};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::native_bls::NativeBlsModule;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::identity_create_transition::IdentityCreateTransition;
use dash_sdk::dpp::state_transition::identity_create_transition::methods::IdentityCreateTransitionMethodsV0;
use dash_sdk::platform::transition::put_identity::PutIdentity;
use dash_sdk::platform::{Fetch, Identity};
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

                // eprintln!("UseAssetLock: transaction id for {:#?} is {}", transaction, tx_id);
                let wallet = wallet.read().unwrap();
                wallet_id = wallet.seed_hash();
                let private_key = wallet
                    .private_key_for_address(&address, self.network)?
                    .ok_or("Asset Lock not valid for wallet")?;
                let asset_lock_proof = if let AssetLockProof::Instant(instant_asset_lock_proof) =
                    asset_lock_proof.as_ref()
                {
                    // we need to make sure the instant send asset lock is recent
                    let raw_transaction_info = self
                        .core_client
                        .read()
                        .expect("Core client lock was poisoned")
                        .get_raw_transaction_info(&tx_id, None)
                        .map_err(|e| e.to_string())?;

                    if raw_transaction_info.chainlock
                        && raw_transaction_info.height.is_some()
                        && raw_transaction_info.confirmations.is_some()
                        && raw_transaction_info.confirmations.unwrap() > 8
                    {
                        // we should use a chain lock instead
                        AssetLockProof::Chain(ChainAssetLockProof {
                            core_chain_locked_height: metadata.core_chain_locked_height,
                            out_point: OutPoint::new(tx_id, 0),
                        })
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
                }

                let asset_lock_proof;

                loop {
                    {
                        let proofs = self.transactions_waiting_for_finality.lock().unwrap();
                        if let Some(Some(proof)) = proofs.get(&tx_id) {
                            asset_lock_proof = proof.clone();
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                (asset_lock_proof, asset_lock_proof_private_key, tx_id)
            }
            RegisterIdentityFundingMethod::FundWithPlatformAddresses {
                inputs,
                wallet_seed_hash,
            } => {
                // This is a separate flow - we call a dedicated function for Platform address funding
                return self
                    .register_identity_from_platform_addresses(
                        alias_input,
                        keys,
                        wallet,
                        wallet_identity_index,
                        inputs,
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

                {
                    let mut wallet = wallet.write().unwrap();
                    wallet.utxos.retain(|_, utxo_map| {
                        utxo_map.retain(|outpoint, _| outpoint != &utxo);
                        !utxo_map.is_empty()
                    });
                    self.db
                        .drop_utxo(&utxo, &self.network.to_string())
                        .map_err(|e| e.to_string())?;
                }

                let asset_lock_proof;

                loop {
                    {
                        let proofs = self.transactions_waiting_for_finality.lock().unwrap();
                        if let Some(Some(proof)) = proofs.get(&tx_id) {
                            asset_lock_proof = proof.clone();
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                (asset_lock_proof, asset_lock_proof_private_key, tx_id)
            }
        };

        let identity_id = asset_lock_proof
            .create_identifier()
            .expect("expected to create an identifier");

        let public_keys = keys.to_public_keys_map();

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

            return Ok(BackendTaskSuccessResult::RegisteredIdentity(
                qualified_identity,
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
                asset_lock_proof,
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

        Ok(BackendTaskSuccessResult::RegisteredIdentity(
            qualified_identity,
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

    /// Register a new identity funded by Platform addresses (DIP-17)
    async fn register_identity_from_platform_addresses(
        &self,
        alias_input: String,
        keys: super::IdentityKeys,
        wallet: std::sync::Arc<std::sync::RwLock<super::Wallet>>,
        wallet_identity_index: u32,
        inputs: BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            dash_sdk::dpp::fee::Credits,
        >,
        wallet_seed_hash: super::WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::platform::transition::put_identity::PutIdentity;

        let sdk = {
            let guard = self.sdk.read().unwrap();
            guard.clone()
        };

        let public_keys = keys.to_public_keys_map();

        // Clone the wallet for use as the address signer (needed across async boundary)
        let wallet_clone = { wallet.read().map_err(|e| e.to_string())?.clone() };

        // For Platform address funding, we need to compute the identity ID from the inputs
        // The SDK will handle this internally when creating the identity
        // We create a temporary identity with a placeholder ID, which will be computed correctly
        // during the state transition creation

        // Create a temporary identity ID - will be replaced by the actual one from Platform
        let temp_identity_id = dash_sdk::platform::Identifier::random();

        let identity =
            Identity::new_with_id_and_keys(temp_identity_id, public_keys.clone(), sdk.version())
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
            .put_with_address_funding(&sdk, inputs, &qualified_identity, &wallet_clone, None)
            .await
        {
            Ok((updated_identity, address_infos)) => {
                qualified_identity.identity = updated_identity;
                qualified_identity.status = IdentityStatus::Unknown; // Force refresh

                // Update source address balances using proof-verified data from SDK response
                if let Err(e) =
                    self.update_wallet_platform_address_info_from_sdk(wallet_seed_hash, &address_infos)
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

                Ok(BackendTaskSuccessResult::RegisteredIdentity(
                    qualified_identity,
                ))
            }
            Err(e) => {
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
}
