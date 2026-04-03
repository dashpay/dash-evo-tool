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

/// Minimum fee for an asset lock transaction (single input, no change).
const MIN_ASSET_LOCK_FEE: u64 = 100;

/// Result of asset lock fee calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssetLockFeeResult {
    /// The actual fee to use (may differ from requested amount for small UTXOs).
    pub fee: u64,
    /// The actual credit amount after fee deduction.
    pub actual_amount: u64,
}

/// Calculate the fee for an asset lock transaction.
pub(crate) fn calculate_asset_lock_fee(
    total_input_value: u64,
    requested_amount: u64,
    num_inputs: usize,
    allow_take_fee_from_amount: bool,
) -> Result<AssetLockFeeResult, String> {
    // Fee formula: base fee + per-input cost
    let fee_no_change = MIN_ASSET_LOCK_FEE + (num_inputs as u64) * 148;

    if total_input_value >= requested_amount + fee_no_change {
        return Ok(AssetLockFeeResult {
            fee: fee_no_change,
            actual_amount: requested_amount,
        });
    }

    if allow_take_fee_from_amount {
        let adjusted = total_input_value.saturating_sub(fee_no_change);
        if adjusted > 0 {
            return Ok(AssetLockFeeResult {
                fee: fee_no_change,
                actual_amount: adjusted,
            });
        }
        return Err("Insufficient funds for transaction fee".to_string());
    }

    Err(format!(
        "Insufficient funds: need {} + {} fee, have {}",
        requested_amount, fee_no_change, total_input_value
    ))
}

// Asset lock transaction building from wallet UTXO selection has been removed.
// Use platform_wallet.core().build_asset_lock_transaction() instead.
//
// The _for_utxo methods below remain for the QR-funded-UTXO flow where
// the user provides a specific UTXO from a funding address.

impl Wallet {
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

        let fee_result = calculate_asset_lock_fee(
            previous_tx_output.value,
            previous_tx_output.value.saturating_sub(MIN_ASSET_LOCK_FEE),
            1,
            true,
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

        #[allow(clippy::drop_non_drop)]
        drop(cache);

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                let message = Message::from_digest(sighash.into());

                let private_key = self
                    .private_key_for_address(&input_address, network)?
                    .ok_or(format!(
                        "Expected address {} to be in wallet for input",
                        input_address
                    ))?;

                let sig = secp.sign_ecdsa(&message, &private_key.inner);

                let mut serialized_sig = sig.serialize_der().to_vec();

                let mut sig_script = vec![serialized_sig.len() as u8 + 1];

                sig_script.append(&mut serialized_sig);

                sig_script.push(1);

                let mut serialized_pub_key = private_key.public_key(&secp).serialize();

                sig_script.push(serialized_pub_key.len() as u8);
                sig_script.append(&mut serialized_pub_key);
                input.script_sig = ScriptBuf::from_bytes(sig_script);
                Ok::<(), String>(())
            })?;

        Ok((tx, private_key))
    }
}
