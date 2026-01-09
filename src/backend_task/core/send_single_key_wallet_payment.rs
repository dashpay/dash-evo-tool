//! Send Single Key Wallet Payment - Send funds from a single key wallet

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::WalletPaymentRequest;
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::{Address, OutPoint, ScriptBuf, Transaction, TxIn, TxOut};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::{EcdsaSighashType, secp256k1::Secp256k1};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeLevel;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Send a payment from a single key wallet
    pub async fn send_single_key_wallet_payment(
        &self,
        wallet: Arc<RwLock<SingleKeyWallet>>,
        request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Only RPC mode is supported for now
        self.send_single_key_wallet_payment_via_rpc(wallet, request)
            .await
    }

    async fn send_single_key_wallet_payment_via_rpc(
        &self,
        wallet: Arc<RwLock<SingleKeyWallet>>,
        request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Parse recipients first to know total output amount
        let mut outputs: Vec<TxOut> = Vec::new();
        let mut total_output: u64 = 0;

        for recipient in &request.recipients {
            let address = Address::from_str(&recipient.address)
                .map_err(|e| format!("Invalid address {}: {}", recipient.address, e))?
                .require_network(self.network)
                .map_err(|e| format!("Address network mismatch: {}", e))?;

            outputs.push(TxOut {
                value: recipient.amount_duffs,
                script_pubkey: address.script_pubkey(),
            });
            total_output += recipient.amount_duffs;
        }

        // Get wallet data and select UTXOs
        let (private_key, selected_utxos, change_address) = {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

            let private_key = wallet_guard
                .private_key(self.network)
                .ok_or_else(|| "Wallet must be unlocked to send".to_string())?;

            if wallet_guard.utxos.is_empty() {
                return Err("No UTXOs available to spend".to_string());
            }

            // Select UTXOs to cover the amount + estimated fee
            // Start with an estimate assuming ~10 inputs, then refine
            let num_outputs = outputs.len() + 1; // +1 for change
            let initial_fee_estimate = Self::estimate_p2pkh_tx_size(10, num_outputs);
            let initial_fee = request
                .override_fee
                .unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(initial_fee_estimate));

            let target_amount = total_output + initial_fee;

            // Sort UTXOs by value descending for efficient selection (use larger UTXOs first)
            let mut all_utxos: Vec<(OutPoint, TxOut)> = wallet_guard
                .utxos
                .iter()
                .map(|(op, tx_out)| (*op, tx_out.clone()))
                .collect();
            all_utxos.sort_by(|a, b| b.1.value.cmp(&a.1.value));

            // Select UTXOs until we have enough
            let mut selected: Vec<(OutPoint, TxOut)> = Vec::new();
            let mut selected_total: u64 = 0;

            for (outpoint, tx_out) in all_utxos {
                selected.push((outpoint, tx_out.clone()));
                selected_total += tx_out.value;

                // Recalculate fee with current input count
                let current_size = Self::estimate_p2pkh_tx_size(selected.len(), num_outputs);
                let current_fee = request
                    .override_fee
                    .unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(current_size));

                if selected_total >= total_output + current_fee {
                    break;
                }
            }

            // Final check if we have enough
            let final_size = Self::estimate_p2pkh_tx_size(selected.len(), num_outputs);
            let final_fee = request
                .override_fee
                .unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(final_size));

            if selected_total < total_output + final_fee {
                return Err(format!(
                    "Insufficient funds: have {} duffs, need {} duffs (including {} fee)",
                    wallet_guard.total_balance,
                    total_output + final_fee,
                    final_fee
                ));
            }

            let change_address = wallet_guard.address.clone();

            (private_key, selected, change_address)
        };

        // Calculate final fee with selected UTXOs
        let num_outputs_with_change = outputs.len() + 1;
        let estimated_size = Self::estimate_p2pkh_tx_size(selected_utxos.len(), num_outputs_with_change);
        let fee = request
            .override_fee
            .unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(estimated_size));

        let total_input: u64 = selected_utxos.iter().map(|(_, tx_out)| tx_out.value).sum();

        // Calculate change
        let change_amount = if request.subtract_fee_from_amount {
            // Subtract fee from the first output
            if outputs[0].value <= fee {
                return Err(format!(
                    "Output amount too small to subtract fee of {} duffs",
                    fee
                ));
            }
            outputs[0].value -= fee;
            total_input - total_output
        } else {
            total_input - total_output - fee
        };

        // Add change output if significant (above dust threshold)
        if change_amount > 546 {
            outputs.push(TxOut {
                value: change_amount,
                script_pubkey: change_address.script_pubkey(),
            });
        }

        // Build inputs
        let inputs: Vec<TxIn> = selected_utxos
            .iter()
            .map(|(outpoint, _)| TxIn {
                previous_output: *outpoint,
                ..Default::default()
            })
            .collect();

        // Create unsigned transaction
        let mut tx = Transaction {
            version: 2,
            lock_time: 0,
            input: inputs,
            output: outputs,
            special_transaction_payload: None,
        };

        // Sign all inputs
        let secp = Secp256k1::new();

        for (i, (_, tx_out)) in selected_utxos.iter().enumerate() {
            let sighash = SighashCache::new(&tx)
                .legacy_signature_hash(i, &tx_out.script_pubkey, EcdsaSighashType::All as u32)
                .map_err(|e| format!("Failed to compute sighash: {:?}", e))?;

            let message =
                dash_sdk::dpp::dashcore::secp256k1::Message::from_digest(sighash.to_byte_array());
            let sig = secp.sign_ecdsa(&message, &private_key.inner);

            // Build script_sig: <sig_len> <signature> <sighash_type> <pubkey_len> <pubkey>
            let mut serialized_sig = sig.serialize_der().to_vec();
            let mut script_sig = vec![serialized_sig.len() as u8 + 1];
            script_sig.append(&mut serialized_sig);
            script_sig.push(EcdsaSighashType::All as u8);

            let mut serialized_pub_key = private_key.public_key(&secp).to_bytes();
            script_sig.push(serialized_pub_key.len() as u8);
            script_sig.append(&mut serialized_pub_key);

            tx.input[i].script_sig = ScriptBuf::from_bytes(script_sig);
        }

        // Broadcast transaction
        let txid = self
            .core_client
            .read()
            .expect("Core client lock was poisoned")
            .send_raw_transaction(&tx)
            .map_err(|e| format!("Failed to broadcast transaction: {}", e))?;

        // Update wallet UTXOs - remove spent, add change
        {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;

            // Remove spent UTXOs
            for (outpoint, _) in &selected_utxos {
                wallet_guard.utxos.remove(outpoint);
            }

            // Add change UTXO if we created one
            let change_output_index = tx.output.len() - 1;
            if tx.output[change_output_index].script_pubkey == change_address.script_pubkey() {
                let change_outpoint = OutPoint::new(txid, change_output_index as u32);
                wallet_guard
                    .utxos
                    .insert(change_outpoint, tx.output[change_output_index].clone());
            }

            // Update balance
            let new_balance: u64 = wallet_guard.utxos.values().map(|tx| tx.value).sum();
            wallet_guard.update_balances(new_balance, 0, new_balance);
        }

        // Update database
        let key_hash = wallet.read().map_err(|e| e.to_string())?.key_hash;

        // Remove spent UTXOs from database
        for (outpoint, _) in &selected_utxos {
            let _ = self.db.drop_utxo(outpoint, &self.network.to_string());
        }

        // Persist new balance
        let balance = wallet.read().map_err(|e| e.to_string())?.total_balance;
        let _ = self
            .db
            .update_single_key_wallet_balances(&key_hash, balance, 0, balance);

        let total_sent: u64 = request.recipients.iter().map(|r| r.amount_duffs).sum();
        let recipients_result: Vec<(String, u64)> = request
            .recipients
            .iter()
            .map(|r| (r.address.clone(), r.amount_duffs))
            .collect();

        Ok(BackendTaskSuccessResult::WalletPayment {
            txid: txid.to_string(),
            total_amount: total_sent,
            recipients: recipients_result,
        })
    }
}
