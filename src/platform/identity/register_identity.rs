use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::platform::identity::IdentityRegistrationInfo;
use dash_sdk::dapi_client::DapiRequestExecutor;
use dash_sdk::dapi_grpc::core::v0::{
    BroadcastTransactionRequest, GetBlockchainStatusRequest, GetTransactionRequest,
    GetTransactionResponse,
};
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::psbt::serialize::Serialize;
use dash_sdk::dpp::dashcore::{Address, Transaction};
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::transition::put_identity::PutIdentity;
use dash_sdk::platform::Identity;
use dash_sdk::{RequestSettings, Sdk};
use rand::prelude::StdRng;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::MutexGuard;

impl AppContext {
    pub(crate) async fn broadcast_and_retrieve_asset_lock(
        &self,
        asset_lock_transaction: &Transaction,
        address: &Address,
    ) -> Result<AssetLockProof, dash_sdk::Error> {
        // Use the span only for synchronous logging before the first await.
        // tracing::debug_span!(
        //     "broadcast_and_retrieve_asset_lock",
        //     transaction_id = asset_lock_transaction.txid().to_string(),
        // )
        // .in_scope(|| {
        //     tracing::debug!("Starting asset lock broadcast.");
        // });

        let sdk = &self.sdk;

        let block_hash = sdk
            .execute(GetBlockchainStatusRequest {}, RequestSettings::default())
            .await?
            .chain
            .map(|chain| chain.best_block_hash)
            .ok_or_else(|| dash_sdk::Error::DapiClientError("Missing `chain` field".to_owned()))?;

        // tracing::debug!(
        //     "Starting the stream from the tip block hash {}",
        //     hex::encode(&block_hash)
        // );

        let mut asset_lock_stream = sdk
            .start_instant_send_lock_stream(block_hash, address)
            .await?;

        // tracing::debug!("Stream is started.");

        let request = BroadcastTransactionRequest {
            transaction: asset_lock_transaction.serialize(),
            allow_high_fees: false,
            bypass_limits: false,
        };

        // tracing::debug!("Broadcasting the transaction.");

        match sdk.execute(request, RequestSettings::default()).await {
            Ok(_) => {}
            Err(error) if error.to_string().contains("AlreadyExists") => {
                // tracing::warn!("Transaction already broadcasted.");

                let GetTransactionResponse { block_hash, .. } = sdk
                    .execute(
                        GetTransactionRequest {
                            id: asset_lock_transaction.txid().to_string(),
                        },
                        RequestSettings::default(),
                    )
                    .await?;

                // tracing::debug!(
                //     "Restarting the stream from the transaction mined block hash {}",
                //     hex::encode(&block_hash)
                // );

                asset_lock_stream = sdk
                    .start_instant_send_lock_stream(block_hash, address)
                    .await?;

                // tracing::debug!("Stream restarted.");
            }
            Err(error) => {
                // tracing::error!("Transaction broadcast failed: {error}");
                return Err(error.into());
            }
        }

        // tracing::debug!("Waiting for asset lock proof.");

        sdk.wait_for_asset_lock_proof_for_transaction(
            asset_lock_stream,
            asset_lock_transaction,
            Some(Duration::from_secs(4 * 60)),
        )
        .await
    }

    pub(super) async fn register_identity(
        &self,
        input: IdentityRegistrationInfo,
    ) -> Result<(), String> {
        let IdentityRegistrationInfo {
            alias_input,
            amount,
            keys,
            identity_index,
            wallet,
        } = input;

        let sdk = self.sdk.clone();

        // Scope the write lock to avoid holding it across an await.
        let (asset_lock_transaction, asset_lock_proof_private_key, change_address) = {
            let mut wallet = wallet.write().unwrap();
            match wallet.asset_lock_transaction(sdk.network, amount, identity_index, Some(self)) {
                Ok(transaction) => transaction,
                Err(_) => {
                    wallet
                        .reload_utxos(&self.core_client)
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

        let asset_lock_proof = self
            .broadcast_and_retrieve_asset_lock(&asset_lock_transaction, &change_address)
            .await
            .map_err(|e| e.to_string())?;

        let identity_id = asset_lock_proof
            .create_identifier()
            .expect("expected to create an identifier");

        let public_keys = keys.to_public_keys_map();
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
        };

        let updated_identity = identity
            .put_to_platform_and_wait_for_response(
                &sdk,
                asset_lock_proof.clone(),
                &asset_lock_proof_private_key,
                &qualified_identity,
            )
            .await
            .map_err(|e| e.to_string())?;

        qualified_identity.identity = updated_identity;

        self.insert_local_qualified_identity(&qualified_identity)
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}
