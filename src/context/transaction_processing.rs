use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::spv::CoreBackendMode;
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, InstantLock, OutPoint, Transaction, TxOut, Txid};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Broadcast a raw transaction via Core RPC or SPV depending on backend mode.
    pub(crate) async fn broadcast_raw_transaction(
        &self,
        tx: &Transaction,
    ) -> Result<Txid, TaskError> {
        match self.core_backend_mode() {
            CoreBackendMode::Rpc => self
                .core_client
                .read()?
                .send_raw_transaction(tx)
                .map_err(TaskError::from),
            CoreBackendMode::Spv => {
                self.spv_manager
                    .broadcast_transaction(tx)
                    .await
                    .map_err(|e| TaskError::SpvBroadcastFailed { detail: e })?;
                Ok(tx.txid())
            }
        }
    }

    /// Wait for an asset lock proof (InstantLock or ChainLock) for the given transaction.
    ///
    /// Polls `transactions_waiting_for_finality` until a proof appears, with a
    /// backend-mode-dependent timeout (SPV: 5 min, RPC: 2 min). Cleans up the
    /// tracking entry on both success and timeout.
    pub(crate) async fn wait_for_asset_lock_proof(
        &self,
        tx_id: Txid,
    ) -> Result<AssetLockProof, TaskError> {
        use tokio::time::Duration;

        let timeout_duration = match self.core_backend_mode() {
            CoreBackendMode::Spv => Duration::from_secs(300),
            CoreBackendMode::Rpc => Duration::from_secs(120),
        };

        match tokio::time::timeout(timeout_duration, async {
            loop {
                {
                    let proofs = self.transactions_waiting_for_finality.lock()?;
                    if let Some(Some(proof)) = proofs.get(&tx_id) {
                        return Ok::<_, TaskError>(proof.clone());
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        {
            Ok(Ok(proof)) => {
                if let Ok(mut proofs) = self.transactions_waiting_for_finality.lock() {
                    proofs.remove(&tx_id);
                }
                Ok(proof)
            }
            Ok(Err(e)) => {
                // Lock poisoned — return immediately instead of spinning
                Err(e)
            }
            Err(_) => {
                if let Ok(mut proofs) = self.transactions_waiting_for_finality.lock() {
                    proofs.remove(&tx_id);
                }
                Err(TaskError::ConfirmationTimeout)
            }
        }
    }

    /// Broadcast an asset lock transaction with the store-before-broadcast
    /// safety pattern, removing spent UTXOs only after a successful broadcast.
    ///
    /// Performs the following steps in order:
    /// 1. Register the transaction for finality tracking.
    /// 2. Store the asset lock in the DB **before** broadcast — prevents an SPV
    ///    InstantSend race where the finality proof arrives before the DB row.
    /// 3. Broadcast the transaction. On failure: clean up the finality tracker
    ///    and pre-stored DB row; UTXOs are **not** removed.
    /// 4. On success: remove spent UTXOs from the wallet and DB.
    ///
    /// # UTXO selection race window
    ///
    /// Callers select UTXOs while holding the wallet write lock, then drop it
    /// before calling this method (which re-acquires the lock in step 4). During
    /// the gap — covering steps 1–3 and the network broadcast — another task
    /// could theoretically select the same UTXOs via `select_unspent_utxos_for`,
    /// leading to a double-spend attempt on-chain (which Core would reject).
    ///
    /// We cannot hold the `std::sync::RwLock` write guard across the async
    /// broadcast because the guard is `!Send` and tokio tasks require `Send`
    /// futures. Fixing this properly requires either migrating to
    /// `tokio::sync::RwLock` (large refactor) or adding a UTXO reservation
    /// mechanism. In practice the risk is negligible: users would have to
    /// trigger two fund operations on the same wallet near-simultaneously.
    ///
    /// Returns the [`Txid`] of the broadcast transaction.
    pub(crate) async fn broadcast_and_commit_asset_lock(
        &self,
        asset_lock_transaction: &Transaction,
        amount: u64,
        wallet_seed_hash: &WalletSeedHash,
        wallet: &Arc<RwLock<Wallet>>,
        used_utxos: Option<&BTreeMap<OutPoint, (TxOut, Address)>>,
    ) -> Result<Txid, TaskError> {
        let tx_id = asset_lock_transaction.txid();

        // Step 1: Register for finality tracking.
        {
            let mut proofs = self.transactions_waiting_for_finality.lock()?;
            proofs.insert(tx_id, None);
        }

        // Step 2: Store the asset lock transaction in the database *before* broadcast.
        self.db.store_asset_lock_transaction(
            asset_lock_transaction,
            amount,
            None,
            wallet_seed_hash,
            self.network,
        )?;

        // Step 3: Broadcast. On failure, clean up DB row and finality tracker.
        if let Err(e) = self.broadcast_raw_transaction(asset_lock_transaction).await {
            if let Ok(mut proofs) = self.transactions_waiting_for_finality.try_lock() {
                proofs.remove(&tx_id);
            }
            let _ = self.db.delete_asset_lock_transaction(tx_id.as_byte_array());
            return Err(e);
        }

        // Step 4: Remove consumed UTXOs from the old Wallet model (only needed
        // for the QR-funded-UTXO flow; PlatformWallet paths handle UTXOs internally).
        if let Some(utxos) = used_utxos {
            if !utxos.is_empty() {
                let mut wallet_guard = wallet.write()?;
                wallet_guard
                    .remove_selected_utxos(utxos, &self.db, self.network)
                    .map_err(|e| TaskError::UtxoUpdateFailed { detail: e })?;
            }
        }

        Ok(tx_id)
    }

    pub(crate) fn received_transaction_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<Vec<(OutPoint, TxOut, Address)>, TaskError> {
        // Initialize a vector to collect wallet outpoints
        let mut wallet_outpoints = Vec::new();

        // Identify the wallets associated with the transaction
        let wallets = self.wallets.read()?;
        for wallet_arc in wallets.values() {
            let wallet = wallet_arc.write()?;
            for (vout, tx_out) in tx.output.iter().enumerate() {
                let address = if let Ok(output_addr) =
                    Address::from_script(&tx_out.script_pubkey, self.network)
                {
                    if wallet.has_address(&output_addr) {
                        output_addr
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                // UTXOs and address balances are persisted via the changeset
                // path (SPV adapter stages them, auto-flushed via
                // FlushStrategy::Immediate). Only in-memory wallet state and
                // app-level metadata (DashPay contacts) are updated here.

                // Collect the outpoint (UTXOs tracked by ManagedWalletInfo)
                let out_point = OutPoint::new(tx.txid(), vout as u32);
                wallet_outpoints.push((out_point, tx_out.clone(), address.clone()));

                // Check if this is a DashPay contact payment
                if let Ok(Some((owner_id, contact_id, address_index))) =
                    self.db.get_dashpay_address_mapping(&address)
                {
                    // Update the highest receive index if needed
                    if let Ok(indices) = self.db.get_contact_address_indices(&owner_id, &contact_id)
                        && address_index >= indices.highest_receive_index
                    {
                        let _ = self.db.update_highest_receive_index(
                            &owner_id,
                            &contact_id,
                            address_index + 1,
                        );
                    }

                    // Save the payment record
                    let _ = self.db.save_payment(
                        &tx.txid().to_string(),
                        &contact_id, // from contact
                        &owner_id,   // to us
                        tx_out.value as i64,
                        None, // memo not available for incoming
                        "received",
                    );

                    tracing::info!(
                        "DashPay payment received: {} duffs from contact {} to address {} (index {})",
                        tx_out.value,
                        contact_id.to_string(
                            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58
                        ),
                        address,
                        address_index
                    );
                }
            }
        }
        if matches!(
            tx.special_transaction_payload,
            Some(AssetLockPayloadType(_))
        ) {
            self.received_asset_lock_finality(tx, islock, chain_locked_height)?;
        }
        Ok(wallet_outpoints)
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
                    wallet.has_address(&output_addr)
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

                // Register with PlatformWallet's AssetLockManager
                if let Some(pw) = wallet.platform_wallet.as_ref() {
                    pw.asset_locks().recover_asset_lock_blocking(
                        tx.clone(),
                        amount,
                        0,
                        platform_wallet::AssetLockFundingType::IdentityRegistration,
                        0,
                        0, // output_index: default to first credit output
                        proof.clone(),
                    );
                }

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
