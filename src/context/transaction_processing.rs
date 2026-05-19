use super::AppContext;
use crate::backend_task::error::TaskError;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, InstantLock, OutPoint, Transaction, TxOut, Txid};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};

impl AppContext {
    /// Handle a finalized (InstantLock/ChainLock) transaction.
    ///
    /// Wallet-UTXO/balance bookkeeping is owned by the upstream wallet and the
    /// display-only `WalletSnapshot`. This path only registers asset-lock
    /// finality (`store_asset_lock_transaction` + `unused_asset_locks`) for the
    /// ZMQ-fed asset-lock consumer. The `Vec` return type is retained for the
    /// ZMQ call sites; it is always empty.
    pub(crate) fn received_transaction_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<Vec<(OutPoint, TxOut, Address)>, TaskError> {
        if matches!(
            tx.special_transaction_payload,
            Some(AssetLockPayloadType(_))
        ) {
            self.received_asset_lock_finality(tx, islock, chain_locked_height)?;
        }
        Ok(Vec::new())
    }

    /// Store the asset lock transaction in the database and update the wallet.
    pub(crate) fn received_asset_lock_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<(), TaskError> {
        // Extract the asset lock payload from the transaction
        let Some(AssetLockPayloadType(payload)) = tx.special_transaction_payload.as_ref() else {
            return Ok(());
        };

        let proof = if let Some(islock) = islock.as_ref() {
            // Deserialize the InstantLock
            Some(AssetLockProof::Instant(InstantAssetLockProof::new(
                islock.clone(),
                tx.clone(),
                0,
            )))
        } else {
            chain_locked_height.map(|chain_locked_height| {
                AssetLockProof::Chain(ChainAssetLockProof {
                    core_chain_locked_height: chain_locked_height,
                    out_point: OutPoint::new(tx.txid(), 0),
                })
            })
        };

        {
            let mut transactions = self.transactions_waiting_for_finality.lock()?;

            if let Some(asset_lock_proof) = transactions.get_mut(&tx.txid()) {
                *asset_lock_proof = proof.clone();
            }
        }

        // Identify the wallet associated with the transaction
        let wallets = self.wallets.read()?;
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write()?;

            // Check if any of the addresses in the transaction outputs match the wallet's known addresses
            let matches_wallet = payload.credit_outputs.iter().any(|tx_out| {
                if let Ok(output_addr) = Address::from_script(&tx_out.script_pubkey, self.network) {
                    wallet.known_addresses.contains_key(&output_addr)
                } else {
                    false
                }
            });

            if matches_wallet {
                // Calculate the total amount from the credit outputs
                let amount: u64 = payload
                    .credit_outputs
                    .iter()
                    .map(|tx_out| tx_out.value)
                    .sum();

                // Store the asset lock transaction in the database
                self.db.store_asset_lock_transaction(
                    tx,
                    amount,
                    islock.as_ref(),
                    &wallet.seed_hash(),
                    self.network,
                )?;

                let first = payload
                    .credit_outputs
                    .first()
                    .ok_or(TaskError::AssetLockNoCreditOutputs)?;

                let address = Address::from_script(&first.script_pubkey, self.network)
                    .map_err(|e| TaskError::AssetLockAddressDerivationFailed { source: e })?;

                // Add the asset lock to the wallet's unused_asset_locks
                wallet
                    .unused_asset_locks
                    .push((tx.clone(), address, amount, islock, proof));

                break; // Exit the loop after updating the relevant wallet
            }
        }

        Ok(())
    }
}

pub(crate) struct DapiTransactionInfo {
    pub is_chain_locked: bool,
    pub height: u32,
    pub confirmations: u32,
}

/// Query transaction info from DAPI. Works in both SPV and RPC modes
/// since DAPI (platform gRPC) is always available via the SDK.
pub(crate) async fn get_transaction_info(
    sdk: &Sdk,
    tx_id: &Txid,
) -> Result<DapiTransactionInfo, TaskError> {
    use dash_sdk::dapi_client::{DapiRequestExecutor, IntoInner, RequestSettings};
    use dash_sdk::dapi_grpc::core::v0::GetTransactionRequest;

    let response = sdk
        .execute(
            GetTransactionRequest {
                id: tx_id.to_string(),
            },
            RequestSettings::default(),
        )
        .await
        .into_inner()
        .map_err(|e| TaskError::PlatformFetchError {
            source: Box::new(dash_sdk::Error::DapiClientError(e)),
        })?;

    Ok(DapiTransactionInfo {
        is_chain_locked: response.is_chain_locked,
        height: response.height,
        confirmations: response.confirmations,
    })
}
