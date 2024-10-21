use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::{Client, RpcApi};
use dash_sdk::dpp::dashcore::{Address, OutPoint, PublicKey, TxOut};
use std::collections::{BTreeMap, HashMap};

impl Wallet {
    pub fn take_unspent_utxos_for(
        &mut self,
        amount: u64,
    ) -> Option<(BTreeMap<OutPoint, (TxOut, PublicKey, Address)>, u64)> {
        let Some(utxos) = self.utxos.as_ref() else {
            return None;
        };
        let mut required: i64 = amount as i64;
        let mut taken_utxos = BTreeMap::new();

        for (outpoint, utxo) in utxos.iter() {
            if required <= 0 {
                break;
            }
            required -= utxo.value as i64;
            taken_utxos.insert(
                outpoint.clone(),
                (utxo.clone(), self.public_key, self.address.clone()),
            );
        }

        // If we didn't gather enough UTXOs to cover the required amount
        if required > 0 {
            return None;
        }

        // Remove taken UTXOs from the original list
        for (outpoint, _) in &taken_utxos {
            self.utxos.remove(outpoint);
        }

        Some((taken_utxos, required.abs() as u64))
    }

    pub async fn reload_utxos(
        &mut self,
        core_client: &Client,
    ) -> Result<HashMap<OutPoint, TxOut>, String> {
        let addresses = self.address_balances.keys().collect::<Vec<_>>();
        // First, let's try to get UTXOs from the RPC client using `list_unspent`.
        match core_client.list_unspent(Some(1), None, Some(addresses.as_slice()), None, None) {
            Ok(utxos) => {
                // Test log statement
                tracing::info!("{:?} utxos", utxos.len());

                // Convert RPC UTXOs to the desired HashMap format
                let mut utxo_map = HashMap::new();
                for utxo in utxos {
                    let outpoint = OutPoint::new(utxo.txid, utxo.vout);
                    let tx_out = TxOut {
                        value: utxo.amount.to_sat(),
                        script_pubkey: utxo.script_pub_key,
                    };
                    utxo_map.insert(outpoint, tx_out);
                }
                self.utxos = Some(utxo_map.clone());
                Ok(utxo_map)
            }
            Err(first_error) => Err(first_error.to_string()),
        }
    }
}
