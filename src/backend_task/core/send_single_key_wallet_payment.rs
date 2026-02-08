//! Send Single Key Wallet Payment - Send funds from a single key wallet

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::WalletPaymentRequest;
use crate::backend_task::wallet::WalletResult;
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::fee_estimation::estimate_p2pkh_tx_size;
use crate::model::wallet::single_key::SingleKeyWallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::{Address, OutPoint, ScriptBuf, Transaction, TxIn, TxOut};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::{EcdsaSighashType, secp256k1::Secp256k1};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeLevel;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

/// Dust threshold in duffs — change outputs below this are dropped
const DUST_THRESHOLD: u64 = 546;

/// Select UTXOs for a payment using a greedy descending-value strategy.
///
/// UTXOs are sorted largest-first and accumulated until the total covers
/// `total_output` plus the dynamically-recalculated fee for the current
/// input count.  Returns the selected UTXOs and the final fee, or an error
/// if the wallet cannot cover the payment.
fn select_utxos_for_payment(
    utxos: &[(OutPoint, TxOut)],
    total_output: u64,
    num_recipient_outputs: usize,
    override_fee: Option<u64>,
) -> Result<(Vec<(OutPoint, TxOut)>, u64), String> {
    if utxos.is_empty() {
        return Err("No UTXOs available to spend".to_string());
    }

    let mut sorted: Vec<_> = utxos.to_vec();
    sorted.sort_by(|a, b| b.1.value.cmp(&a.1.value));

    let num_outputs = num_recipient_outputs + 1; // +1 for potential change
    let mut selected: Vec<(OutPoint, TxOut)> = Vec::new();
    let mut selected_total: u64 = 0;

    for (outpoint, tx_out) in sorted {
        selected.push((outpoint, tx_out.clone()));
        selected_total += tx_out.value;

        let current_size = estimate_p2pkh_tx_size(selected.len(), num_outputs);
        let current_fee =
            override_fee.unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(current_size));

        if selected_total >= total_output + current_fee {
            break;
        }
    }

    // Final fee for the selected set
    let final_size = estimate_p2pkh_tx_size(selected.len(), num_outputs);
    let final_fee =
        override_fee.unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(final_size));

    if selected_total < total_output + final_fee {
        return Err(format!(
            "Insufficient funds: have {} duffs, need {} duffs (including {} fee)",
            selected_total,
            total_output + final_fee,
            final_fee
        ));
    }

    Ok((selected, final_fee))
}

/// Calculate the change amount and optionally adjust outputs when the fee is
/// subtracted from the send amount.
///
/// Returns `(change_amount, adjusted_outputs)` or an error if the first
/// output is too small to absorb the fee.
fn calculate_change(
    outputs: &[TxOut],
    total_input: u64,
    total_output: u64,
    fee: u64,
    subtract_fee_from_amount: bool,
) -> Result<(u64, Vec<TxOut>), String> {
    let mut adjusted = outputs.to_vec();

    let change_amount = if subtract_fee_from_amount {
        if adjusted[0].value <= fee {
            return Err(format!(
                "Output amount too small to subtract fee of {} duffs",
                fee
            ));
        }
        adjusted[0].value -= fee;
        total_input - total_output
    } else {
        total_input - total_output - fee
    };

    Ok((change_amount, adjusted))
}

/// Returns `true` if the change amount is above the dust threshold and
/// should be included as a transaction output.
fn should_include_change(change_amount: u64) -> bool {
    change_amount > DUST_THRESHOLD
}

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

            let all_utxos: Vec<(OutPoint, TxOut)> = wallet_guard
                .utxos
                .iter()
                .map(|(op, tx_out)| (*op, tx_out.clone()))
                .collect();

            let (selected, _fee) = select_utxos_for_payment(
                &all_utxos,
                total_output,
                outputs.len(),
                request.override_fee,
            )?;

            let change_address = wallet_guard.address.clone();

            (private_key, selected, change_address)
        };

        // Calculate final fee with selected UTXOs
        let num_outputs_with_change = outputs.len() + 1;
        let estimated_size = estimate_p2pkh_tx_size(selected_utxos.len(), num_outputs_with_change);
        let fee = request
            .override_fee
            .unwrap_or_else(|| FeeLevel::Normal.fee_rate().calculate_fee(estimated_size));

        let total_input: u64 = selected_utxos.iter().map(|(_, tx_out)| tx_out.value).sum();

        // Calculate change and adjust outputs for fee subtraction
        let (change_amount, mut outputs) = calculate_change(
            &outputs,
            total_input,
            total_output,
            fee,
            request.subtract_fee_from_amount,
        )?;

        // Add change output if significant (above dust threshold)
        if should_include_change(change_amount) {
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
                .map_err(|e| format!("Failed to compute sighash: {}", e))?;

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
            .read_or_recover()
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
            if let Err(e) = self.db.drop_utxo(outpoint, &self.network.to_string()) {
                tracing::warn!(
                    "Failed to remove spent UTXO {:?} from database: {}",
                    outpoint,
                    e
                );
            }
        }

        // Persist new balance
        let balance = wallet.read().map_err(|e| e.to_string())?.total_balance;
        if let Err(e) = self
            .db
            .update_single_key_wallet_balances(&key_hash, balance, 0, balance)
        {
            tracing::warn!(
                "Failed to update single key wallet balances in database: {}",
                e
            );
        }

        let total_sent: u64 = request.recipients.iter().map(|r| r.amount_duffs).sum();
        let recipients_result: Vec<(String, u64)> = request
            .recipients
            .iter()
            .map(|r| (r.address.clone(), r.amount_duffs))
            .collect();

        Ok(BackendTaskSuccessResult::Wallet(WalletResult::Payment {
            txid: txid.to_string(),
            total_amount: total_sent,
            recipients: recipients_result,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{Network, PublicKey, Txid};

    /// Helper: create a test address on Testnet
    fn test_address(index: u8) -> Address {
        let mut key_bytes = [2u8; 33]; // compressed pubkey prefix
        key_bytes[32] = index;
        let pubkey = PublicKey::from_slice(&key_bytes).expect("valid pubkey");
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// Helper: create an OutPoint with a deterministic txid
    fn test_outpoint(tx_index: u8, vout: u32) -> OutPoint {
        let mut txid_bytes = [0u8; 32];
        txid_bytes[0] = tx_index;
        OutPoint::new(Txid::from_slice(&txid_bytes).unwrap(), vout)
    }

    /// Helper: create a list of UTXOs with the given values
    fn make_utxos(values: &[u64]) -> Vec<(OutPoint, TxOut)> {
        let addr = test_address(1);
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                let outpoint = test_outpoint(i as u8 + 1, 0);
                let tx_out = TxOut {
                    value,
                    script_pubkey: addr.script_pubkey(),
                };
                (outpoint, tx_out)
            })
            .collect()
    }

    /// Helper: create a TxOut for a given value
    fn make_output(value: u64) -> TxOut {
        let addr = test_address(10);
        TxOut {
            value,
            script_pubkey: addr.script_pubkey(),
        }
    }

    // ========================================================================
    // UTXO selection tests
    // ========================================================================

    #[test]
    fn test_select_utxos_single_utxo_sufficient() {
        let utxos = make_utxos(&[1_000_000]);
        // Use override_fee for deterministic testing
        let result = select_utxos_for_payment(&utxos, 500_000, 1, Some(10_000));
        assert!(result.is_ok());
        let (selected, fee) = result.unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(fee, 10_000);
    }

    #[test]
    fn test_select_utxos_multiple_needed() {
        let utxos = make_utxos(&[30_000, 40_000, 50_000]);
        let result = select_utxos_for_payment(&utxos, 100_000, 1, Some(10_000));
        assert!(result.is_ok());
        let (selected, _fee) = result.unwrap();
        // All 3 UTXOs needed (total 120k for 100k + 10k fee)
        assert_eq!(selected.len(), 3);
        let total: u64 = selected.iter().map(|(_, tx)| tx.value).sum();
        assert_eq!(total, 120_000);
    }

    #[test]
    fn test_select_utxos_prefers_largest_first() {
        // UTXOs given in ascending order, should be sorted descending internally
        let utxos = make_utxos(&[10_000, 50_000, 100_000]);
        let result = select_utxos_for_payment(&utxos, 40_000, 1, Some(5_000));
        assert!(result.is_ok());
        let (selected, _fee) = result.unwrap();
        // Should pick the 100k UTXO first (largest), which covers 40k + 5k
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].1.value, 100_000);
    }

    #[test]
    fn test_select_utxos_insufficient_funds() {
        let utxos = make_utxos(&[10_000, 20_000]);
        let result = select_utxos_for_payment(&utxos, 100_000, 1, Some(5_000));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient funds"));
    }

    #[test]
    fn test_select_utxos_empty_wallet() {
        let utxos: Vec<(OutPoint, TxOut)> = vec![];
        let result = select_utxos_for_payment(&utxos, 100_000, 1, Some(5_000));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No UTXOs"));
    }

    #[test]
    fn test_select_utxos_exact_amount() {
        let utxos = make_utxos(&[110_000]);
        let result = select_utxos_for_payment(&utxos, 100_000, 1, Some(10_000));
        assert!(result.is_ok());
        let (selected, fee) = result.unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(fee, 10_000);
        // Total covers amount + fee exactly
        assert_eq!(selected[0].1.value, 100_000 + fee);
    }

    #[test]
    fn test_select_utxos_with_dynamic_fee() {
        // Without override_fee, the fee is calculated dynamically based on
        // the number of selected inputs. This tests the dynamic path.
        let utxos = make_utxos(&[1_000_000]);
        let result = select_utxos_for_payment(&utxos, 500_000, 1, None);
        assert!(result.is_ok());
        let (selected, fee) = result.unwrap();
        assert_eq!(selected.len(), 1);
        // Fee should be based on estimate_p2pkh_tx_size(1, 2)
        let expected_size = estimate_p2pkh_tx_size(1, 2);
        let expected_fee = FeeLevel::Normal.fee_rate().calculate_fee(expected_size);
        assert_eq!(fee, expected_fee);
    }

    #[test]
    fn test_select_utxos_multiple_recipients() {
        let utxos = make_utxos(&[500_000]);
        // 3 recipient outputs + 1 change = 4 outputs
        let result = select_utxos_for_payment(&utxos, 300_000, 3, Some(15_000));
        assert!(result.is_ok());
        let (selected, fee) = result.unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(fee, 15_000);
    }

    // ========================================================================
    // Change calculation tests
    // ========================================================================

    #[test]
    fn test_change_normal() {
        let outputs = vec![make_output(100_000)];
        let result = calculate_change(&outputs, 200_000, 100_000, 10_000, false);
        assert!(result.is_ok());
        let (change, adjusted) = result.unwrap();
        // change = 200k - 100k - 10k = 90k
        assert_eq!(change, 90_000);
        // Output unchanged
        assert_eq!(adjusted[0].value, 100_000);
    }

    #[test]
    fn test_change_zero() {
        let outputs = vec![make_output(90_000)];
        let result = calculate_change(&outputs, 100_000, 90_000, 10_000, false);
        assert!(result.is_ok());
        let (change, _adjusted) = result.unwrap();
        assert_eq!(change, 0);
    }

    #[test]
    fn test_change_subtract_fee_from_amount() {
        let outputs = vec![make_output(100_000)];
        let result = calculate_change(&outputs, 200_000, 100_000, 10_000, true);
        assert!(result.is_ok());
        let (change, adjusted) = result.unwrap();
        // When subtracting fee from amount: output reduced by fee
        assert_eq!(adjusted[0].value, 90_000);
        // change = total_input - total_output = 200k - 100k = 100k
        assert_eq!(change, 100_000);
    }

    #[test]
    fn test_change_subtract_fee_output_too_small() {
        let outputs = vec![make_output(500)];
        let result = calculate_change(&outputs, 10_500, 500, 1_000, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small to subtract fee"));
    }

    #[test]
    fn test_change_subtract_fee_output_equals_fee() {
        let outputs = vec![make_output(10_000)];
        // output.value == fee should also fail (we need strictly greater)
        let result = calculate_change(&outputs, 20_000, 10_000, 10_000, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_change_multiple_outputs() {
        let outputs = vec![make_output(50_000), make_output(30_000)];
        let result = calculate_change(&outputs, 200_000, 80_000, 10_000, false);
        assert!(result.is_ok());
        let (change, adjusted) = result.unwrap();
        // change = 200k - 80k - 10k = 110k
        assert_eq!(change, 110_000);
        // Both outputs unchanged
        assert_eq!(adjusted[0].value, 50_000);
        assert_eq!(adjusted[1].value, 30_000);
    }

    #[test]
    fn test_change_subtract_fee_only_affects_first_output() {
        let outputs = vec![make_output(50_000), make_output(30_000)];
        let result = calculate_change(&outputs, 200_000, 80_000, 10_000, true);
        assert!(result.is_ok());
        let (_change, adjusted) = result.unwrap();
        // Only first output is reduced
        assert_eq!(adjusted[0].value, 40_000);
        assert_eq!(adjusted[1].value, 30_000);
    }

    // ========================================================================
    // Dust threshold tests
    // ========================================================================

    #[test]
    fn test_dust_threshold_above() {
        assert!(should_include_change(547));
        assert!(should_include_change(1_000));
        assert!(should_include_change(1_000_000));
    }

    #[test]
    fn test_dust_threshold_at_boundary() {
        assert!(!should_include_change(546));
    }

    #[test]
    fn test_dust_threshold_below() {
        assert!(!should_include_change(0));
        assert!(!should_include_change(100));
        assert!(!should_include_change(545));
    }

    // ========================================================================
    // Integration: UTXO selection + change calculation
    // ========================================================================

    #[test]
    fn test_payment_flow_normal() {
        let utxos = make_utxos(&[500_000]);
        let fee = 10_000_u64;

        let (selected, actual_fee) =
            select_utxos_for_payment(&utxos, 200_000, 1, Some(fee)).unwrap();

        let total_input: u64 = selected.iter().map(|(_, tx)| tx.value).sum();
        let outputs = vec![make_output(200_000)];
        let (change, adjusted) =
            calculate_change(&outputs, total_input, 200_000, actual_fee, false).unwrap();

        // 500k - 200k - 10k = 290k change
        assert_eq!(change, 290_000);
        assert!(should_include_change(change));
        assert_eq!(adjusted[0].value, 200_000);
    }

    #[test]
    fn test_payment_flow_subtract_fee() {
        // Wallet has 210k, sending 200k with fee subtracted from output
        let utxos = make_utxos(&[210_000]);
        let fee = 10_000_u64;

        let (selected, actual_fee) =
            select_utxos_for_payment(&utxos, 200_000, 1, Some(fee)).unwrap();

        let total_input: u64 = selected.iter().map(|(_, tx)| tx.value).sum();
        let outputs = vec![make_output(200_000)];
        let (change, adjusted) =
            calculate_change(&outputs, total_input, 200_000, actual_fee, true).unwrap();

        // Recipient gets 200k - 10k fee = 190k
        assert_eq!(adjusted[0].value, 190_000);
        // change = 210k - 200k = 10k (the fee was absorbed by output reduction)
        assert_eq!(change, 10_000);
        assert!(should_include_change(change));
    }

    #[test]
    fn test_payment_flow_dust_change_dropped() {
        // Set up so change is exactly at dust threshold
        let utxos = make_utxos(&[100_546]);
        let fee = 10_000_u64;

        let (selected, actual_fee) =
            select_utxos_for_payment(&utxos, 90_000, 1, Some(fee)).unwrap();

        let total_input: u64 = selected.iter().map(|(_, tx)| tx.value).sum();
        let outputs = vec![make_output(90_000)];
        let (change, _adjusted) =
            calculate_change(&outputs, total_input, 90_000, actual_fee, false).unwrap();

        // change = 100_546 - 90_000 - 10_000 = 546 (exactly at dust threshold)
        assert_eq!(change, 546);
        assert!(!should_include_change(change));
    }

    #[test]
    fn test_payment_flow_just_above_dust() {
        let utxos = make_utxos(&[100_547]);
        let fee = 10_000_u64;

        let (selected, actual_fee) =
            select_utxos_for_payment(&utxos, 90_000, 1, Some(fee)).unwrap();

        let total_input: u64 = selected.iter().map(|(_, tx)| tx.value).sum();
        let outputs = vec![make_output(90_000)];
        let (change, _adjusted) =
            calculate_change(&outputs, total_input, 90_000, actual_fee, false).unwrap();

        // change = 100_547 - 90_000 - 10_000 = 547 (just above dust)
        assert_eq!(change, 547);
        assert!(should_include_change(change));
    }

    // ========================================================================
    // Amount validation tests
    // ========================================================================

    #[test]
    fn test_amount_validation_zero_amount_with_fee() {
        let utxos = make_utxos(&[50_000]);
        let result = select_utxos_for_payment(&utxos, 0, 1, Some(5_000));
        assert!(result.is_ok());
        let (selected, _) = result.unwrap();
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn test_amount_validation_fee_exceeds_balance() {
        let utxos = make_utxos(&[10_000]);
        // Amount is 0 but fee is larger than balance
        let result = select_utxos_for_payment(&utxos, 0, 1, Some(20_000));
        assert!(result.is_err());
    }

    #[test]
    fn test_amount_validation_many_small_utxos() {
        // 20 small UTXOs that individually can't cover the amount
        let utxos = make_utxos(&[10_000; 20]);
        let result = select_utxos_for_payment(&utxos, 150_000, 1, Some(5_000));
        assert!(result.is_ok());
        let (selected, _) = result.unwrap();
        // Need at least 16 UTXOs (155k / 10k = 15.5, round up)
        assert!(selected.len() >= 16);
        let total: u64 = selected.iter().map(|(_, tx)| tx.value).sum();
        assert!(total >= 155_000);
    }
}
