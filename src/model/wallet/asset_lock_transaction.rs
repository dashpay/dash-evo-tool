use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::secp256k1::Message;
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::transaction::special_transaction::asset_lock::AssetLockPayload;
use dash_sdk::dpp::dashcore::{
    Address, Network, OutPoint, PrivateKey, ScriptBuf, Transaction, TxIn, TxOut,
};
use dash_sdk::dpp::key_wallet::psbt::serialize::Serialize;
use std::collections::BTreeMap;

/// Result of asset lock fee calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssetLockFeeResult {
    /// Transaction fee in duffs.
    pub fee: u64,
    /// Actual amount for the asset lock output (may differ from requested if
    /// `allow_take_fee_from_amount` is true).
    pub actual_amount: u64,
    /// Change to return, or `None` if no change output is needed.
    pub change: Option<u64>,
}

/// Minimum fee for an asset lock transaction (duffs).
const MIN_ASSET_LOCK_FEE: u64 = 3_000;

/// Minimum value for a change output (duffs). Outputs below this
/// threshold are considered dust and will be rejected by the network.
/// For P2PKH outputs the Dash Core dust limit is 546 duffs.
const DUST_THRESHOLD: u64 = 546;

/// Estimate the transaction size in bytes.
///
/// Assumes P2PKH inputs (~148 B each), standard outputs (~34 B each),
/// a ~10 B header, and a ~60 B asset-lock payload.
fn estimate_tx_size(num_inputs: usize, num_outputs: usize) -> usize {
    10 + (num_inputs * 148) + (num_outputs * 34) + 60
}

/// Calculate fee, actual amount, and change for an asset lock transaction.
///
/// Uses an iterative approach: starts assuming a change output exists,
/// then recomputes if the change disappears under the real fee.
///
/// # Errors
///
/// Returns an error string if there are insufficient funds.
pub(crate) fn calculate_asset_lock_fee(
    total_input_value: u64,
    requested_amount: u64,
    num_inputs: usize,
    allow_take_fee_from_amount: bool,
) -> Result<AssetLockFeeResult, String> {
    // First pass: assume 2 outputs (1 burn + 1 change).
    let fee_with_change = std::cmp::max(MIN_ASSET_LOCK_FEE, estimate_tx_size(num_inputs, 2) as u64);

    let required_with_change = requested_amount
        .checked_add(fee_with_change)
        .ok_or("Overflow computing required amount + fee")?;
    let tentative_change = total_input_value.checked_sub(required_with_change);

    // If change exceeds dust threshold, include it as an output.
    if let Some(change) = tentative_change
        && change >= DUST_THRESHOLD
    {
        return Ok(AssetLockFeeResult {
            fee: fee_with_change,
            actual_amount: requested_amount,
            change: Some(change),
        });
    }

    // Change is zero or negative under the 2-output fee.
    // Recompute with 1 output (no change).
    let fee_no_change = std::cmp::max(MIN_ASSET_LOCK_FEE, estimate_tx_size(num_inputs, 1) as u64);

    let required_no_change = requested_amount
        .checked_add(fee_no_change)
        .ok_or("Overflow computing required amount + fee")?;

    if total_input_value >= required_no_change {
        // Enough funds without a change output. Any leftover becomes
        // additional fee (it is too small for a viable change output).
        return Ok(AssetLockFeeResult {
            fee: total_input_value - requested_amount,
            actual_amount: requested_amount,
            change: None,
        });
    }

    // Insufficient for the requested amount at the 1-output fee rate.
    if allow_take_fee_from_amount {
        let adjusted = total_input_value.saturating_sub(fee_no_change);
        if adjusted == 0 {
            return Err("Insufficient funds for transaction fee".to_string());
        }
        Ok(AssetLockFeeResult {
            fee: fee_no_change,
            actual_amount: adjusted,
            change: None,
        })
    } else {
        Err(format!(
            "Insufficient funds: need {} + {} fee, have {}",
            requested_amount, fee_no_change, total_input_value
        ))
    }
}

impl Wallet {
    /// Select UTXOs and compute fee, retrying with the real fee if the initial
    /// estimate was too low and additional UTXOs are available.
    #[allow(clippy::type_complexity)]
    fn select_utxos_with_fee_retry(
        &self,
        amount: u64,
        allow_take_fee_from_amount: bool,
        source_address: Option<&Address>,
    ) -> Result<(BTreeMap<OutPoint, (TxOut, Address)>, AssetLockFeeResult), String> {
        let mut fee_estimate = MIN_ASSET_LOCK_FEE;

        for _ in 0..2 {
            let (utxos, _) = self
                .select_unspent_utxos_for(
                    amount,
                    fee_estimate,
                    allow_take_fee_from_amount,
                    source_address,
                )
                .ok_or_else(|| {
                    format!(
                        "Not enough spendable funds to create asset lock transaction: \
                         requested amount {} plus estimated fee {}",
                        amount, fee_estimate
                    )
                })?;

            let total_input_value: u64 = utxos.iter().map(|(_, (tx_out, _))| tx_out.value).sum();
            let num_inputs = utxos.len();

            match calculate_asset_lock_fee(
                total_input_value,
                amount,
                num_inputs,
                allow_take_fee_from_amount,
            ) {
                Ok(fee_result) => return Ok((utxos, fee_result)),
                Err(_) if fee_estimate == MIN_ASSET_LOCK_FEE => {
                    // The real fee may exceed our initial estimate.  Recompute
                    // with a 2-output size estimate and retry UTXO selection so
                    // we can pick up any additional marginal UTXOs.
                    fee_estimate =
                        std::cmp::max(MIN_ASSET_LOCK_FEE, estimate_tx_size(num_inputs, 2) as u64);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(format!(
            "Not enough spendable funds to create asset lock transaction: \
             requested amount {} plus fee {}",
            amount, fee_estimate
        ))
    }

    #[allow(clippy::type_complexity)]
    pub fn registration_asset_lock_transaction(
        &mut self,
        app_context: &AppContext,
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
        source_address: Option<&Address>,
    ) -> Result<
        (
            Transaction,
            PrivateKey,
            Option<Address>,
            BTreeMap<OutPoint, (TxOut, Address)>,
        ),
        String,
    > {
        let private_key =
            self.identity_registration_ecdsa_private_key(app_context, network, identity_index)?;
        self.asset_lock_transaction_from_private_key(
            app_context,
            network,
            amount,
            allow_take_fee_from_amount,
            private_key,
            source_address,
        )
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub fn top_up_asset_lock_transaction(
        &mut self,
        app_context: &AppContext,
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
        top_up_index: u32,
        source_address: Option<&Address>,
    ) -> Result<
        (
            Transaction,
            PrivateKey,
            Option<Address>,
            BTreeMap<OutPoint, (TxOut, Address)>,
        ),
        String,
    > {
        let private_key = self.identity_top_up_ecdsa_private_key(
            app_context,
            network,
            identity_index,
            top_up_index,
        )?;
        self.asset_lock_transaction_from_private_key(
            app_context,
            network,
            amount,
            allow_take_fee_from_amount,
            private_key,
            source_address,
        )
    }

    /// Create an asset lock transaction with a randomly generated one-time key.
    /// This is used for generic platform address funding (not identity-specific).
    #[allow(clippy::type_complexity)]
    pub fn generic_asset_lock_transaction(
        &mut self,
        app_context: &AppContext,
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        source_address: Option<&Address>,
    ) -> Result<
        (
            Transaction,
            PrivateKey,
            Address,
            Option<Address>,
            BTreeMap<OutPoint, (TxOut, Address)>,
        ),
        String,
    > {
        use bip39::rand::rngs::OsRng;

        // Generate a random private key for the asset lock
        let secp = Secp256k1::new();
        let (secret_key, _) = secp.generate_keypair(&mut OsRng);
        let private_key = PrivateKey::new(secret_key, network);
        let public_key = private_key.public_key(&secp);

        // The asset lock address is where the proof will be tied to
        let asset_lock_address = Address::p2pkh(&public_key, network);

        let (tx, returned_private_key, change_address, used_utxos) = self
            .asset_lock_transaction_from_private_key(
                app_context,
                network,
                amount,
                allow_take_fee_from_amount,
                private_key,
                source_address,
            )?;

        Ok((
            tx,
            returned_private_key,
            asset_lock_address,
            change_address,
            used_utxos,
        ))
    }

    #[allow(clippy::type_complexity)]
    fn asset_lock_transaction_from_private_key(
        &mut self,
        app_context: &AppContext,
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        private_key: PrivateKey,
        source_address: Option<&Address>,
    ) -> Result<
        (
            Transaction,
            PrivateKey,
            Option<Address>,
            BTreeMap<OutPoint, (TxOut, Address)>,
        ),
        String,
    > {
        let secp = Secp256k1::new();
        let asset_lock_public_key = private_key.public_key(&secp);

        let one_time_key_hash = asset_lock_public_key.pubkey_hash();

        // Select UTXOs without committing the removal yet.  UTXOs are only removed
        // from the wallet after the transaction is fully built and signed, so that a
        // failure at any later step (fee shortfall, missing private key, …) cannot
        // permanently drop UTXOs from the wallet — especially important in SPV mode
        // where there is no Core RPC reload fallback.
        //
        // Note: `&mut self` ensures exclusive access during the entire select → build
        // → sign → remove sequence, preventing concurrent UTXO double-selection.
        //
        // We use an initial fee estimate for UTXO selection, then recalculate the
        // real fee based on input count.  If the real fee exceeds the estimate and
        // the selected UTXOs are insufficient, we retry once with the computed fee
        // so that marginal UTXOs are not missed.
        let (utxos, fee_result) =
            self.select_utxos_with_fee_retry(amount, allow_take_fee_from_amount, source_address)?;

        let actual_amount = fee_result.actual_amount;
        let change_option = fee_result.change;

        let payload_output = TxOut {
            value: actual_amount,
            script_pubkey: ScriptBuf::new_p2pkh(&one_time_key_hash),
        };
        let burn_output = TxOut {
            value: actual_amount,
            script_pubkey: ScriptBuf::new_op_return(&[]),
        };

        let (change_output, change_address) = if let Some(change) = change_option {
            let change_address = self.change_address(network, Some(app_context))?;
            (
                Some(TxOut {
                    value: change,
                    script_pubkey: change_address.script_pubkey(),
                }),
                Some(change_address),
            )
        } else {
            (None, None)
        };

        let payload = AssetLockPayload {
            version: 1,
            credit_outputs: vec![payload_output],
        };

        // Collect inputs from UTXOs
        let inputs = utxos
            .keys()
            .map(|utxo| TxIn {
                previous_output: *utxo,
                ..Default::default()
            })
            .collect();

        let mut tx = Transaction {
            version: 3,
            lock_time: 0,
            input: inputs,
            output: {
                let mut outputs = vec![burn_output];
                if let Some(change_output) = change_output {
                    outputs.push(change_output);
                }
                outputs
            },
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(payload)),
        };

        let sighash_u32 = 1u32;

        let cache = SighashCache::new(&tx);

        // Next, collect the sighashes for each input since that's what we need from the
        // cache
        let sighashes: Result<Vec<_>, String> = tx
            .input
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let script_pubkey = utxos
                    .get(&input.previous_output)
                    .ok_or_else(|| {
                        format!(
                            "UTXO not found in selected set for input {}",
                            input.previous_output
                        )
                    })?
                    .0
                    .script_pubkey
                    .clone();
                cache
                    .legacy_signature_hash(i, &script_pubkey, sighash_u32)
                    .map_err(|e| format!("Failed to compute sighash for input {}: {}", i, e))
            })
            .collect();
        let sighashes = sighashes?;

        // Now we can drop the cache to end the immutable borrow
        #[allow(clippy::drop_non_drop)]
        drop(cache);

        let mut check_utxos = utxos.clone();

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                // You need to provide the actual script_pubkey of the UTXO being spent
                let (_, input_address) =
                    check_utxos.remove(&input.previous_output).ok_or_else(|| {
                        format!(
                            "UTXO not found in selected set for input {}",
                            input.previous_output
                        )
                    })?;
                let message = Message::from_digest(sighash.into());

                let private_key = self
                    .private_key_for_address(&input_address, network)?
                    .ok_or(format!(
                        "Expected address {} to be in wallet",
                        input_address
                    ))?;

                // Sign the message with the private key
                let sig = secp.sign_ecdsa(&message, &private_key.inner);

                // Serialize the DER-encoded signature and append the sighash type
                let mut serialized_sig = sig.serialize_der().to_vec();

                let mut sig_script = vec![serialized_sig.len() as u8 + 1];

                sig_script.append(&mut serialized_sig);

                sig_script.push(1);

                let mut serialized_pub_key = private_key.public_key(&secp).serialize();

                sig_script.push(serialized_pub_key.len() as u8);
                sig_script.append(&mut serialized_pub_key);
                // Create script_sig
                input.script_sig = ScriptBuf::from_bytes(sig_script);
                Ok::<(), String>(())
            })?;

        // Transaction is fully built and signed. UTXOs are returned to the caller
        // so they can be removed explicitly after successful broadcast. This avoids
        // permanently losing UTXO tracking if broadcast fails.
        Ok((tx, private_key, change_address, utxos))
    }

    pub fn registration_asset_lock_transaction_for_utxo(
        &mut self,
        app_context: &AppContext,
        network: Network,
        utxo: OutPoint,
        previous_tx_output: TxOut,
        input_address: Address,
        identity_index: u32,
    ) -> Result<(Transaction, PrivateKey), String> {
        let private_key =
            self.identity_registration_ecdsa_private_key(app_context, network, identity_index)?;
        self.asset_lock_transaction_for_utxo_from_private_key(
            network,
            utxo,
            previous_tx_output,
            input_address,
            private_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn top_up_asset_lock_transaction_for_utxo(
        &mut self,
        app_context: &AppContext,
        network: Network,
        utxo: OutPoint,
        previous_tx_output: TxOut,
        input_address: Address,
        identity_index: u32,
        top_up_index: u32,
    ) -> Result<(Transaction, PrivateKey), String> {
        let private_key = self.identity_top_up_ecdsa_private_key(
            app_context,
            network,
            identity_index,
            top_up_index,
        )?;
        self.asset_lock_transaction_for_utxo_from_private_key(
            network,
            utxo,
            previous_tx_output,
            input_address,
            private_key,
        )
    }

    pub fn asset_lock_transaction_for_utxo_from_private_key(
        &mut self,
        network: Network,
        utxo: OutPoint,
        previous_tx_output: TxOut,
        input_address: Address,
        private_key: PrivateKey,
    ) -> Result<(Transaction, PrivateKey), String> {
        let secp = Secp256k1::new();
        let asset_lock_public_key = private_key.public_key(&secp);

        let one_time_key_hash = asset_lock_public_key.pubkey_hash();

        // Single-UTXO path: use calculate_asset_lock_fee for consistency with
        // the multi-input path, ensuring dust check and overflow safety.
        let fee_result = calculate_asset_lock_fee(
            previous_tx_output.value,
            previous_tx_output.value.saturating_sub(MIN_ASSET_LOCK_FEE),
            1,    // single input
            true, // take fee from amount since the UTXO *is* the total
        )?;
        let output_amount = fee_result.actual_amount;

        let payload_output = TxOut {
            value: output_amount,
            script_pubkey: ScriptBuf::new_p2pkh(&one_time_key_hash),
        };
        let burn_output = TxOut {
            value: output_amount,
            script_pubkey: ScriptBuf::new_op_return(&[]),
        };
        let payload = AssetLockPayload {
            version: 1,
            credit_outputs: vec![payload_output],
        };

        // we need to get all inputs from utxos to add them to the transaction

        let mut tx_in = TxIn::default();
        #[allow(clippy::field_reassign_with_default)]
        {
            tx_in.previous_output = utxo;
        }

        let sighash_u32 = 1u32;

        let mut tx: Transaction = Transaction {
            version: 3,
            lock_time: 0,
            input: vec![tx_in],
            output: vec![burn_output],
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(payload)),
        };

        let cache = SighashCache::new(&tx);

        // Next, collect the sighashes for each input since that's what we need from the
        // cache
        let sighashes: Result<Vec<_>, String> = tx
            .input
            .iter()
            .enumerate()
            .map(|(i, _)| {
                cache
                    .legacy_signature_hash(i, &previous_tx_output.script_pubkey, sighash_u32)
                    .map_err(|e| format!("Failed to compute sighash for input {}: {}", i, e))
            })
            .collect();
        let sighashes = sighashes?;

        // Now we can drop the cache to end the immutable borrow
        #[allow(clippy::drop_non_drop)]
        drop(cache);

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                // You need to provide the actual script_pubkey of the UTXO being spent
                let message = Message::from_digest(sighash.into());

                let private_key = self
                    .private_key_for_address(&input_address, network)?
                    .ok_or(format!(
                        "Expected address {} to be in wallet for input",
                        input_address
                    ))?;

                // Sign the message with the private key
                let sig = secp.sign_ecdsa(&message, &private_key.inner);

                // Serialize the DER-encoded signature and append the sighash type
                let mut serialized_sig = sig.serialize_der().to_vec();

                let mut sig_script = vec![serialized_sig.len() as u8 + 1];

                sig_script.append(&mut serialized_sig);

                sig_script.push(1);

                let mut serialized_pub_key = private_key.public_key(&secp).serialize();

                sig_script.push(serialized_pub_key.len() as u8);
                sig_script.append(&mut serialized_pub_key);
                // Create script_sig
                input.script_sig = ScriptBuf::from_bytes(sig_script);
                Ok::<(), String>(())
            })?;

        Ok((tx, private_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_single_input_minimum() {
        // 1 input, amount=10000, total=50000.
        // With 2 outputs: size = 10 + 148 + 2*34 + 60 = 286 -> fee = max(3000, 286) = 3000.
        // Change = 50000 - 10000 - 3000 = 37000.
        let result = calculate_asset_lock_fee(50_000, 10_000, 1, false).expect("should succeed");
        assert_eq!(result.fee, 3_000);
        assert_eq!(result.actual_amount, 10_000);
        assert_eq!(result.change, Some(37_000));
    }

    #[test]
    fn fee_scales_with_inputs() {
        // 21 inputs, amount=50000, total=200000.
        // With 2 outputs: size = 10 + 21*148 + 2*34 + 60 = 3246, fee = max(3000, 3246) = 3246.
        // Change = 200000 - 50000 - 3246 = 146754.
        let result = calculate_asset_lock_fee(200_000, 50_000, 21, false).expect("should succeed");
        assert!(
            result.fee > MIN_ASSET_LOCK_FEE,
            "fee should exceed minimum for many inputs"
        );
        assert_eq!(result.fee, 3_246);
        assert_eq!(result.actual_amount, 50_000);
        assert_eq!(result.change, Some(146_754));
    }

    #[test]
    fn fee_exact_no_change() {
        // 1 input, amount=47000, total=50000.
        // With 2 outputs: size = 286, fee = 3000. change = 50000 - 47000 - 3000 = 0 -> None.
        // Re-check with 1 output: size = 10 + 148 + 34 + 60 = 252, fee = 3000.
        // 50000 >= 47000 + 3000 -> actual fee = 50000 - 47000 = 3000.
        let result = calculate_asset_lock_fee(50_000, 47_000, 1, false).expect("should succeed");
        assert_eq!(result.actual_amount, 47_000);
        assert_eq!(result.change, None);
    }

    #[test]
    fn fee_take_from_amount() {
        // 1 input, amount=50000, total=50000, allow_take_fee=true.
        // With 2 outputs: fee = 3000. change = 50000 - 50000 - 3000 < 0.
        // With 1 output: fee = 3000. 50000 < 50000 + 3000.
        // Take from amount: actual = 50000 - 3000 = 47000.
        let result = calculate_asset_lock_fee(50_000, 50_000, 1, true).expect("should succeed");
        assert_eq!(result.actual_amount, 47_000);
        assert_eq!(result.change, None);
    }

    #[test]
    fn fee_insufficient_funds() {
        // 1 input, amount=50000, total=50000, allow_take_fee=false.
        let result = calculate_asset_lock_fee(50_000, 50_000, 1, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient funds"));
    }

    #[test]
    fn fee_insufficient_for_fee_alone() {
        // 1 input, amount=2000, total=2000, allow_take_fee=true.
        // Fee = 3000 > total -> adjusted = 2000 - 3000 = 0 (saturating) -> error.
        let result = calculate_asset_lock_fee(2_000, 2_000, 1, true);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("Insufficient funds"),
            "should error when total cannot even cover the fee"
        );
    }

    #[test]
    fn fee_change_eliminated_by_real_fee_should_succeed() {
        // BUG TEST: 21 inputs, amount=50000, total=53220.
        //
        // Initial UTXO selection uses fee estimate of 3000:
        //   change = 53220 - 50000 - 3000 = 220 -> has_initial_change = true
        //
        // Buggy code uses 2 outputs: fee = 10 + 21*148 + 2*34 + 60 = 3246
        //   53220 < 50000 + 3246 = 53246 -> ERROR (false insufficient funds)
        //
        // Correct code recalculates with 1 output: fee = 10 + 21*148 + 1*34 + 60 = 3212
        //   53220 >= 50000 + 3212 -> succeeds, no change, fee = 3220
        let result = calculate_asset_lock_fee(53_220, 50_000, 21, false);
        assert!(
            result.is_ok(),
            "should NOT fail with insufficient funds; got: {:?}",
            result
        );
        let result = result.unwrap();
        assert_eq!(result.actual_amount, 50_000);
        assert_eq!(result.change, None);
        // Fee absorbs the leftover: 53220 - 50000 = 3220
        assert_eq!(result.fee, 3_220);
    }

    #[test]
    fn fee_overestimate_when_change_disappears() {
        // BUG TEST: Verify that when initial change exists under 3000 estimate but
        // disappears under the real fee, we do not overcharge by counting the
        // phantom change output.
        //
        // 21 inputs, amount=50000, total=53240.
        // 2-output fee = 3246 -> 53240 < 53246, would fail with buggy code.
        // 1-output fee = 3212 -> 53240 >= 53212, succeeds. fee = 3240, no change.
        let result = calculate_asset_lock_fee(53_240, 50_000, 21, false);
        assert!(
            result.is_ok(),
            "should succeed when recalculated without change output; got: {:?}",
            result
        );
        let result = result.unwrap();
        assert_eq!(result.actual_amount, 50_000);
        assert_eq!(result.change, None);
        // The fee with 2 outputs (3246) would be wrong here; only 1 output is needed.
        // Actual fee = leftover = 53240 - 50000 = 3240, which is >= 1-output minimum (3212).
        assert!(
            result.fee < 3_246,
            "fee should be less than the 2-output estimate of 3246, got {}",
            result.fee
        );
        assert_eq!(result.fee, 3_240);
    }
    #[test]
    fn fee_dust_change_absorbed_into_fee() {
        // 1 input, amount=46500, total=50000.
        // With 2 outputs: fee = 3000. change = 50000 - 46500 - 3000 = 500.
        // 500 < DUST_THRESHOLD (546) → change is dust, absorbed into fee.
        // Falls through to 1-output path: fee = 3000.
        // 50000 >= 46500 + 3000 → fee = 50000 - 46500 = 3500 (absorbs dust).
        let result = calculate_asset_lock_fee(50_000, 46_500, 1, false).expect("should succeed");
        assert_eq!(result.actual_amount, 46_500);
        assert_eq!(
            result.change, None,
            "dust change should not create an output"
        );
        assert_eq!(result.fee, 3_500, "dust should be absorbed into fee");
    }
}
