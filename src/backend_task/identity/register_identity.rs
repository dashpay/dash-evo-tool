use crate::app::TaskResult;
use crate::backend_task::identity::{IdentityRegistrationInfo, IdentityRegistrationMethod};
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_sdk::dapi_client::DapiRequestExecutor;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::psbt::serialize::Serialize;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::native_bls::NativeBlsModule;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::identity_create_transition::methods::IdentityCreateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::identity_create_transition::IdentityCreateTransition;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::proto::{get_epochs_info_request, GetEpochsInfoRequest};
use dash_sdk::platform::transition::put_identity::PutIdentity;
use dash_sdk::platform::types::evonode::EvoNode;
use dash_sdk::platform::{Fetch, FetchUnproved, Identity};
use dash_sdk::query_types::EvoNodeStatus;
use std::time::Duration;
use tokio::sync::mpsc;

impl AppContext {
    // pub(crate) async fn broadcast_and_retrieve_asset_lock(
    //     &self,
    //     asset_lock_transaction: &Transaction,
    //     address: &Address,
    // ) -> Result<AssetLockProof, dash_sdk::Error> {
    //     // Use the span only for synchronous logging before the first await.
    //     // tracing::debug_span!(
    //     //     "broadcast_and_retrieve_asset_lock",
    //     //     transaction_id = asset_lock_transaction.txid().to_string(),
    //     // )
    //     // .in_scope(|| {
    //     //     tracing::debug!("Starting asset lock broadcast.");
    //     // });
    //
    //     let sdk = &self.sdk;
    //
    //     let block_hash = sdk
    //         .execute(GetBlockchainStatusRequest {}, RequestSettings::default())
    //         .await?
    //         .chain
    //         .map(|chain| chain.best_block_hash)
    //         .ok_or_else(|| dash_sdk::Error::DapiClientError("Missing `chain` field".to_owned()))?;
    //
    //     // tracing::debug!(
    //     //     "Starting the stream from the tip block hash {}",
    //     //     hex::encode(&block_hash)
    //     // );
    //
    //     let mut asset_lock_stream = sdk
    //         .start_instant_send_lock_stream(block_hash, address)
    //         .await?;
    //
    //     // tracing::debug!("Stream is started.");
    //
    //     let request = BroadcastTransactionRequest {
    //         transaction: asset_lock_transaction.serialize(),
    //         allow_high_fees: false,
    //         bypass_limits: false,
    //     };
    //
    //     // tracing::debug!("Broadcasting the transaction.");
    //
    //     match sdk.execute(request, RequestSettings::default()).await {
    //         Ok(_) => {}
    //         Err(error) if error.to_string().contains("AlreadyExists") => {
    //             // tracing::warn!("Transaction already broadcasted.");
    //
    //             let GetTransactionResponse { block_hash, .. } = sdk
    //                 .execute(
    //                     GetTransactionRequest {
    //                         id: asset_lock_transaction.txid().to_string(),
    //                     },
    //                     RequestSettings::default(),
    //                 )
    //                 .await?;
    //
    //             // tracing::debug!(
    //             //     "Restarting the stream from the transaction mined block hash {}",
    //             //     hex::encode(&block_hash)
    //             // );
    //
    //             asset_lock_stream = sdk
    //                 .start_instant_send_lock_stream(block_hash, address)
    //                 .await?;
    //
    //             // tracing::debug!("Stream restarted.");
    //         }
    //         Err(error) => {
    //             // tracing::error!("Transaction broadcast failed: {error}");
    //             return Err(error.into());
    //         }
    //     }
    //
    //     // tracing::debug!("Waiting for asset lock proof.");
    //
    //     sdk.wait_for_asset_lock_proof_for_transaction(
    //         asset_lock_stream,
    //         asset_lock_transaction,
    //         Some(Duration::from_secs(4 * 60)),
    //     )
    //     .await
    // }

    pub(super) async fn register_identity(
        &self,
        input: IdentityRegistrationInfo,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        let IdentityRegistrationInfo {
            alias_input,
            keys,
            wallet,
            identity_registration_method,
        } = input;

        let sdk = self.sdk.clone();

        let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None)
            .await
            .map_err(|e| e.to_string())?;

        let (asset_lock_proof, asset_lock_proof_private_key, tx_id) =
            match identity_registration_method {
                IdentityRegistrationMethod::UseAssetLock(
                    address,
                    asset_lock_proof,
                    transaction,
                ) => {
                    let tx_id = transaction.txid();
                    let wallet = wallet.read().unwrap();
                    let private_key = wallet
                        .private_key_for_address(&address, self.network)?
                        .ok_or("Asset Lock not valid for wallet")?;
                    let asset_lock_proof =
                        if let AssetLockProof::Instant(instant_asset_lock_proof) =
                            asset_lock_proof.as_ref()
                        {
                            // we need to make sure the instant send asset lock is recent
                            let raw_transaction_info = self
                                .core_client
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
                            asset_lock_proof
                        };
                    (asset_lock_proof, private_key, tx_id)
                }
                IdentityRegistrationMethod::FundWithWallet(amount, identity_index) => {
                    // Scope the write lock to avoid holding it across an await.
                    let (asset_lock_transaction, asset_lock_proof_private_key, change_address) = {
                        let mut wallet = wallet.write().unwrap();
                        match wallet.asset_lock_transaction(
                            sdk.network,
                            amount,
                            identity_index,
                            Some(self),
                        ) {
                            Ok(transaction) => transaction,
                            Err(_) => {
                                wallet
                                    .reload_utxos(&self.core_client, self.network, Some(self))
                                    .map_err(|e| e.to_string())?;
                                wallet.asset_lock_transaction(
                                    sdk.network,
                                    amount,
                                    identity_index,
                                    Some(self),
                                )?
                            }
                        }
                    };

                    let tx_id = asset_lock_transaction.txid();
                    // todo: maybe one day we will want to use platform again, but for right now we use
                    //  the local core as it is more stable
                    // let asset_lock_proof = self
                    //     .broadcast_and_retrieve_asset_lock(&asset_lock_transaction, &change_address)
                    //     .await
                    //     .map_err(|e| e.to_string())?;

                    {
                        let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
                        proofs.insert(tx_id, None);
                    }

                    self.core_client
                        .send_raw_transaction(&asset_lock_transaction)
                        .map_err(|e| e.to_string())?;

                    let mut asset_lock_proof;

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

        match Identity::fetch_by_identifier(&sdk, identity_id).await {
            Ok(Some(_)) => return Err("Identity already exists".to_string()),
            Ok(None) => {}
            Err(e) => return Err(format!("Error fetching identity: {}", e)),
        };

        let identity = Identity::new_with_id_and_keys(identity_id, public_keys, sdk.version())
            .expect("expected to make identity");

        let mut qualified_identity = QualifiedIdentity {
            identity: identity.clone(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            encrypted_private_keys: keys.to_encrypted_private_keys(),
            dpns_names: vec![],
        };

        if !alias_input.is_empty() {
            qualified_identity.alias = Some(alias_input);
        }

        self.insert_local_qualified_identity_in_creation(&qualified_identity)
            .map_err(|e| e.to_string())?;
        self.db
            .set_asset_lock_identity_id_before_confirmation_by_network(
                tx_id.as_byte_array(),
                Some(identity_id.as_slice()),
            )
            .map_err(|e| e.to_string())?;

        let updated_identity = identity
            .put_to_platform_and_wait_for_response(
                &sdk,
                asset_lock_proof.clone(),
                &asset_lock_proof_private_key,
                &qualified_identity,
            )
            .await
            .map_err(|e| {
                let identity_create_transition =
                    IdentityCreateTransition::try_from_identity_with_signer(
                        &identity,
                        asset_lock_proof,
                        asset_lock_proof_private_key.inner.as_ref(),
                        &qualified_identity,
                        &NativeBlsModule,
                        0,
                        PlatformVersion::latest(),
                    )
                    .expect("expected to make transition");
                format!(
                    "error: {}, transaction is {:?}",
                    e.to_string(),
                    identity_create_transition
                )
            })?;

        qualified_identity.identity = updated_identity;

        self.insert_local_qualified_identity(&qualified_identity)
            .map_err(|e| e.to_string())?;
        self.db
            .set_asset_lock_identity_id(tx_id.as_byte_array(), Some(identity_id.as_slice()))
            .map_err(|e| e.to_string())?;

        sender
            .send(TaskResult::Success(BackendTaskSuccessResult::None))
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}
