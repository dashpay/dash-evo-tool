use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::psbt::serialize::Serialize;
use dash_sdk::dpp::dashcore::secp256k1::Message;
use dash_sdk::dpp::dashcore::sighash::SighashCache;
use dash_sdk::dpp::dashcore::transaction::special_transaction::asset_lock::AssetLockPayload;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, ScriptBuf, Transaction, TxIn, TxOut};

impl Wallet {
    pub fn asset_lock_transaction(
        &mut self,
        network: Network,
        amount: u64,
        identity_index: u32,
        register_addresses: Option<&AppContext>,
    ) -> Result<(Transaction, PrivateKey, Address), String> {
        let secp = Secp256k1::new();
        let private_key = self.identity_registration_ecdsa_private_key(network, identity_index);
        let asset_lock_public_key = private_key.public_key(&secp);

        let one_time_key_hash = asset_lock_public_key.pubkey_hash();
        let fee = 3_000;
        let (mut utxos, change) = self
            .take_unspent_utxos_for(amount + fee)
            .ok_or("take_unspent_utxos_for() returned None".to_string())?;

        let change_address = self.change_address(network, register_addresses)?;

        let payload_output = TxOut {
            value: amount,
            script_pubkey: ScriptBuf::new_p2pkh(&one_time_key_hash),
        };
        let burn_output = TxOut {
            value: amount,
            script_pubkey: ScriptBuf::new_op_return(&[]),
        };
        if change < fee {
            return Err("Change < Fee in asset_lock_transaction()".to_string());
        }
        let change_output = TxOut {
            value: change - fee,
            script_pubkey: change_address.script_pubkey(),
        };
        let payload = AssetLockPayload {
            version: 1,
            credit_outputs: vec![payload_output],
        };

        // we need to get all inputs from utxos to add them to the transaction

        let inputs = utxos
            .iter()
            .map(|(utxo, _)| {
                let mut tx_in = TxIn::default();
                tx_in.previous_output = utxo.clone();
                tx_in
            })
            .collect();

        let sighash_u32 = 1u32;

        let mut tx: Transaction = Transaction {
            version: 3,
            lock_time: 0,
            input: inputs,
            output: vec![burn_output, change_output],
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(payload)),
        };

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

        tx.input
            .iter_mut()
            .zip(sighashes.into_iter())
            .for_each(|(input, sighash)| {
                // You need to provide the actual script_pubkey of the UTXO being spent
                let (_, public_key, input_address) = utxos
                    .remove(&input.previous_output)
                    .expect("expected a txout");
                let message = Message::from_digest(sighash.into()).expect("Error creating message");

                let private_key = self.private_key_for_address(&input_address);

                // Sign the message with the private key
                let sig = secp.sign_ecdsa(&message, &private_key.inner);

                // Serialize the DER-encoded signature and append the sighash type
                let mut serialized_sig = sig.serialize_der().to_vec();

                let mut sig_script = vec![serialized_sig.len() as u8 + 1];

                sig_script.append(&mut serialized_sig);

                sig_script.push(1);

                let mut serialized_pub_key = public_key.serialize();

                sig_script.push(serialized_pub_key.len() as u8);
                sig_script.append(&mut serialized_pub_key);
                // Create script_sig
                input.script_sig = ScriptBuf::from_bytes(sig_script);
            });

        Ok((tx, private_key, change_address))
    }
}
