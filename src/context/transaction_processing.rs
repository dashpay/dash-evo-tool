use super::AppContext;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, InstantLock, OutPoint, Transaction, TxOut, Txid};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use rusqlite::Result;
use std::collections::HashMap;

impl AppContext {
    pub(crate) fn received_transaction_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<Vec<(OutPoint, TxOut, Address)>> {
        // Initialize a vector to collect wallet outpoints
        let mut wallet_outpoints = Vec::new();

        // Identify the wallets associated with the transaction
        let wallets = self.wallets.read().unwrap();
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write().unwrap();
            for (vout, tx_out) in tx.output.iter().enumerate() {
                let address = if let Ok(output_addr) =
                    Address::from_script(&tx_out.script_pubkey, self.network)
                {
                    if wallet.known_addresses.contains_key(&output_addr) {
                        output_addr
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                self.db.insert_utxo(
                    tx.txid().as_byte_array(),
                    vout as u32,
                    &address,
                    tx_out.value,
                    &tx_out.script_pubkey.to_bytes(),
                    self.network,
                )?;
                self.db
                    .add_to_address_balance(&wallet.seed_hash(), &address, tx_out.value)?;

                // Create the OutPoint and insert it into the wallet.utxos entry
                let out_point = OutPoint::new(tx.txid(), vout as u32);
                wallet
                    .utxos
                    .entry(address.clone())
                    .or_insert_with(HashMap::new) // Initialize inner HashMap if needed
                    .insert(out_point, tx_out.clone()); // Insert the TxOut at the OutPoint

                // Collect the outpoint
                wallet_outpoints.push((out_point, tx_out.clone(), address.clone()));

                wallet
                    .address_balances
                    .entry(address.clone())
                    .and_modify(|balance| *balance += tx_out.value)
                    .or_insert(tx_out.value);

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
    ) -> Result<()> {
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
            let mut transactions = self.transactions_waiting_for_finality.lock().unwrap();

            if let Some(asset_lock_proof) = transactions.get_mut(&tx.txid()) {
                *asset_lock_proof = proof.clone();
            }
        }

        // Identify the wallet associated with the transaction
        let wallets = self.wallets.read().unwrap();
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write().unwrap();

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
                    .expect("Expected at least one credit output");

                let address = Address::from_script(&first.script_pubkey, self.network)
                    .expect("expected an address");

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
pub(crate) async fn get_transaction_info_via_dapi(
    sdk: &Sdk,
    tx_id: &Txid,
) -> Result<DapiTransactionInfo, String> {
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
        .map_err(|e| format!("DAPI GetTransaction failed: {}", e))?;

    Ok(DapiTransactionInfo {
        is_chain_locked: response.is_chain_locked,
        height: response.height,
        confirmations: response.confirmations,
    })
}
