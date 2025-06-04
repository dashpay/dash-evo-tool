use crate::app::TaskResult;
use crate::backend_task::identity::{IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::identity_topup_transition::methods::IdentityTopUpTransitionMethodsV0;
use dash_sdk::dpp::state_transition::identity_topup_transition::IdentityTopUpTransition;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::platform::transition::top_up_identity::TopUpIdentity;
use dash_sdk::platform::Fetch;
use dash_sdk::Error;
use std::time::Duration;
use tokio::sync::mpsc;

impl AppContext {
    pub(super) async fn top_up_identity(
        &self,
        input: IdentityTopUpInfo,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let IdentityTopUpInfo {
            mut qualified_identity,
            wallet,
            identity_funding_method,
        } = input;

        let sdk = {
            let guard = self.sdk.read().unwrap();
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

                    // eprintln!("UseAssetLock: transaction id for {:#?} is {}", transaction, tx_id);
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
                    (asset_lock_proof, private_key, tx_id, None)
                }
                TopUpIdentityFundingMethod::FundWithWallet(
                    amount,
                    identity_index,
                    top_up_index,
                ) => {
                    // Scope the write lock to avoid holding it across an await.
                    let (asset_lock_transaction, asset_lock_proof_private_key, _, used_utxos) = {
                        let mut wallet = wallet.write().unwrap();
                        match wallet.top_up_asset_lock_transaction(
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
                                        &self
                                            .core_client
                                            .read()
                                            .expect("Core client lock was poisoned"),
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
                    let (asset_lock_transaction, asset_lock_proof_private_key) = {
                        let mut wallet = wallet.write().unwrap();
                        wallet.top_up_asset_lock_transaction_for_utxo(
                            sdk.network,
                            utxo,
                            tx_out.clone(),
                            input_address.clone(),
                            identity_index,
                            top_up_index,
                            Some(self),
                        )?
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
                if matches!(e, Error::Protocol(ProtocolError::UnknownVersionError(_))) {
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
                            let identity_create_transition =
                                IdentityTopUpTransition::try_from_identity(
                                    &qualified_identity.identity,
                                    asset_lock_proof,
                                    asset_lock_proof_private_key.inner.as_ref(),
                                    0,
                                    self.platform_version(),
                                    None,
                                )
                                .expect("expected to make transition");
                            format!(
                                "error: {}, transaction is {:?}",
                                e, identity_create_transition
                            )
                        })?
                } else {
                    return Err(e.to_string());
                }
            }
        };

        qualified_identity
            .identity
            .set_balance(updated_identity_balance);

        self.update_local_qualified_identity(&qualified_identity)
            .map_err(|e| e.to_string())?;

        {
            let mut wallet = wallet.write().unwrap();
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

        sender
            .send(TaskResult::Success(BackendTaskSuccessResult::None))
            .await
            .map_err(|e| e.to_string())?;

        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            qualified_identity,
        ))
    }
}
