use crate::context::AppContext;
use crate::model::fee_estimation::{
    AssetLockFeeResult, MIN_ASSET_LOCK_FEE, calculate_asset_lock_fee, calculate_relay_fee,
    estimate_asset_lock_tx_size,
};
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

impl Wallet {
    /// Select UTXOs and compute fee, retrying with the real fee if the initial
    /// estimate was too low and additional UTXOs are available.
    #[allow(clippy::type_complexity)]
    fn select_utxos_with_fee_retry(
        &self,
        amount: u64,
        allow_take_fee_from_amount: bool,
    ) -> Result<(BTreeMap<OutPoint, (TxOut, Address)>, AssetLockFeeResult), String> {
        let mut fee_estimate = MIN_ASSET_LOCK_FEE;

        for _ in 0..2 {
            let (utxos, _) = self
                .select_unspent_utxos_for(amount, fee_estimate, allow_take_fee_from_amount)
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
                    fee_estimate = std::cmp::max(
                        MIN_ASSET_LOCK_FEE,
                        calculate_relay_fee(estimate_asset_lock_tx_size(num_inputs, 2)),
                    );
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
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<
        (
            Transaction,
            PrivateKey,
            Option<Address>,
            BTreeMap<OutPoint, (TxOut, Address)>,
        ),
        String,
    > {
        let private_key = self.identity_registration_ecdsa_private_key(
            network,
            identity_index,
            register_addresses,
        )?;
        self.asset_lock_transaction_from_private_key(
            network,
            amount,
            allow_take_fee_from_amount,
            private_key,
            register_addresses,
        )
    }

    #[allow(clippy::type_complexity)]
    pub fn top_up_asset_lock_transaction(
        &mut self,
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
        top_up_index: u32,
        register_addresses: Option<&AppContext>,
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
            network,
            identity_index,
            top_up_index,
            register_addresses,
        )?;
        self.asset_lock_transaction_from_private_key(
            network,
            amount,
            allow_take_fee_from_amount,
            private_key,
            register_addresses,
        )
    }

    /// Create an asset lock transaction with a randomly generated one-time key.
    /// This is used for generic platform address funding (not identity-specific).
    #[allow(clippy::type_complexity)]
    pub fn generic_asset_lock_transaction(
        &mut self,
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        register_addresses: Option<&AppContext>,
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
                network,
                amount,
                allow_take_fee_from_amount,
                private_key,
                register_addresses,
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
        network: Network,
        amount: u64,
        allow_take_fee_from_amount: bool,
        private_key: PrivateKey,
        register_addresses: Option<&AppContext>,
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
            self.select_utxos_with_fee_retry(amount, allow_take_fee_from_amount)?;

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
            let change_address = self.change_address(network, register_addresses)?;
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
        let sighashes: Vec<_> = tx
            .input
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let script_pubkey = utxos
                    .get(&input.previous_output)
                    .expect("expected a txout")
                    .0
                    .script_pubkey
                    .clone();
                cache
                    .legacy_signature_hash(i, &script_pubkey, sighash_u32)
                    .expect("expected sighash")
            })
            .collect();

        // Now we can drop the cache to end the immutable borrow
        #[allow(clippy::drop_non_drop)]
        drop(cache);

        let mut check_utxos = utxos.clone();

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                // You need to provide the actual script_pubkey of the UTXO being spent
                let (_, input_address) = check_utxos
                    .remove(&input.previous_output)
                    .expect("expected a txout");
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

        // Transaction is fully built and signed; commit the UTXO removals now.
        if let Some(context) = register_addresses {
            self.remove_selected_utxos(&utxos, &context.db, network)?;
        }

        Ok((tx, private_key, change_address, utxos))
    }

    pub fn registration_asset_lock_transaction_for_utxo(
        &mut self,
        network: Network,
        utxo: OutPoint,
        previous_tx_output: TxOut,
        input_address: Address,
        identity_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<(Transaction, PrivateKey), String> {
        let private_key = self.identity_registration_ecdsa_private_key(
            network,
            identity_index,
            register_addresses,
        )?;
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
        network: Network,
        utxo: OutPoint,
        previous_tx_output: TxOut,
        input_address: Address,
        identity_index: u32,
        top_up_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<(Transaction, PrivateKey), String> {
        let private_key = self.identity_top_up_ecdsa_private_key(
            network,
            identity_index,
            top_up_index,
            register_addresses,
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

        // Single-UTXO path: use calculate_asset_lock_fee (from fee_estimation)
        // for consistency with the multi-input path, ensuring dust check and
        // overflow safety.
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
        let sighashes: Vec<_> = tx
            .input
            .iter()
            .enumerate()
            .map(|(i, _)| {
                cache
                    .legacy_signature_hash(i, &previous_tx_output.script_pubkey, sighash_u32)
                    .expect("expected sighash")
            })
            .collect();

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
// Tests for calculate_asset_lock_fee and related functions have been
// consolidated into src/model/fee_estimation.rs.
