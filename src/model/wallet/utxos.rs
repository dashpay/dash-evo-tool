use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::{Client, RpcApi};
use dash_sdk::dpp::dashcore::{Address, OutPoint, PublicKey, TxOut};
use std::collections::{BTreeMap, HashMap};
use tracing::info;

impl Wallet {
    pub fn take_unspent_utxos_for(
        &mut self,
        amount: u64,
    ) -> Option<(BTreeMap<OutPoint, (TxOut, Address)>, u64)> {
        // Ensure UTXOs exist
        let Some(utxos) = self.utxos.as_mut() else {
            return None;
        };

        let mut required: i64 = amount as i64;
        let mut taken_utxos = BTreeMap::new();
        let mut utxos_to_remove = Vec::new();

        // Iterate over the UTXOs to collect enough to cover the required amount
        for (address, outpoints) in utxos.iter_mut() {
            for (outpoint, tx_out) in outpoints.iter() {
                if required <= 0 {
                    break;
                }

                // Add the UTXO to the result
                taken_utxos.insert(outpoint.clone(), (tx_out.clone(), address.clone()));

                required -= tx_out.value as i64;
                utxos_to_remove.push((address.clone(), outpoint.clone()));
            }
        }

        // If not enough UTXOs were found, return None
        if required > 0 {
            return None;
        }

        // Remove the collected UTXOs from the wallet's UTXO map
        for (address, outpoint) in utxos_to_remove {
            if let Some(outpoints) = utxos.get_mut(&address) {
                outpoints.remove(&outpoint);
                if outpoints.is_empty() {
                    utxos.remove(&address);
                }
            }
        }

        // Return the collected UTXOs and the remaining amount (which should be zero or positive)
        Some((taken_utxos, required.abs() as u64))
    }

    pub fn reload_utxos(
        &mut self,
        core_client: &Client,
    ) -> Result<HashMap<OutPoint, TxOut>, String> {
        // Collect the addresses for which we want to load UTXOs.
        let addresses: Vec<_> = self.address_balances.keys().collect();

        // Use the RPC client to list unspent outputs.
        match core_client.list_unspent(Some(1), None, Some(&addresses), None, None) {
            Ok(utxos) => {
                // Log the number of UTXOs retrieved for debugging purposes.
                // info!("Retrieved {} UTXOs", utxos.len());

                // Initialize the HashMap to store the UTXOs.
                let mut utxo_map = HashMap::new();

                // Iterate over the retrieved UTXOs and populate the HashMap.
                for utxo in utxos {
                    let outpoint = OutPoint::new(utxo.txid, utxo.vout);
                    let tx_out = TxOut {
                        value: utxo.amount.to_sat(),
                        script_pubkey: utxo.script_pub_key,
                    };
                    utxo_map.insert(outpoint, tx_out);
                }

                // Update the wallet's UTXOs with the retrieved data.
                self.utxos = Some(
                    addresses
                        .iter()
                        .map(|address| {
                            let address_utxos = utxo_map
                                .iter()
                                .filter(|(_, tx_out)| {
                                    tx_out.script_pubkey == address.script_pubkey()
                                })
                                .map(|(outpoint, tx_out)| (outpoint.clone(), tx_out.clone()))
                                .collect();
                            ((*address).clone(), address_utxos)
                        })
                        .collect(),
                );

                // Return the UTXOs.
                Ok(utxo_map)
            }
            Err(first_error) => Err(first_error.to_string()),
        }
    }
}
