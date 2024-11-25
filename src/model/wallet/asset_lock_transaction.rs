use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::psbt::serialize::Serialize;
use dash_sdk::dpp::dashcore::secp256k1::Message;
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::transaction::special_transaction::asset_lock::AssetLockPayload;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::{
    Address, Network, OutPoint, PrivateKey, ScriptBuf, Transaction, TxIn, TxOut,
};
use std::collections::BTreeMap;

impl Wallet {
    pub fn asset_lock_transaction(
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
        let secp = Secp256k1::new();
        let private_key = self.identity_registration_ecdsa_private_key(
            network,
            identity_index,
            register_addresses,
        )?;
        let asset_lock_public_key = private_key.public_key(&secp);

        let one_time_key_hash = asset_lock_public_key.pubkey_hash();
        let fee = 3_000;

        let (utxos, change_option) = self
            .take_unspent_utxos_for(amount, fee, allow_take_fee_from_amount)
            .ok_or("take_unspent_utxos_for() returned None".to_string())?;

        let actual_amount = if change_option.is_none() && allow_take_fee_from_amount {
            // The amount has been adjusted by taking the fee from the amount
            // Calculate the adjusted amount based on the total value of the UTXOs minus the fee
            let total_input_value: u64 = utxos.iter().map(|(_, (tx_out, _))| tx_out.value).sum();
            total_input_value - fee
        } else {
            amount
        };

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
            .iter()
            .map(|(utxo, _)| TxIn {
                previous_output: utxo.clone(),
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
                    .ok_or("Expected address to be in wallet")?;

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

        Ok((tx, private_key, change_address, utxos))
    }

    pub fn asset_lock_transaction_for_utxo(
        &mut self,
        network: Network,
        utxo: OutPoint,
        previous_tx_output: TxOut,
        input_address: Address,
        identity_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<(Transaction, PrivateKey), String> {
        let secp = Secp256k1::new();
        let private_key = self.identity_registration_ecdsa_private_key(
            network,
            identity_index,
            register_addresses,
        )?;
        let asset_lock_public_key = private_key.public_key(&secp);

        let one_time_key_hash = asset_lock_public_key.pubkey_hash();
        let fee = 3_000;
        let output_amount = previous_tx_output.value - fee;

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
        tx_in.previous_output = utxo.clone();

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
        drop(cache);

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .try_for_each(|(input, sighash)| {
                // You need to provide the actual script_pubkey of the UTXO being spent
                let message = Message::from_digest(sighash.into());

                let private_key = self
                    .private_key_for_address(&input_address, network)?
                    .ok_or("Expected address to be in wallet")?;

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
