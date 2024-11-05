use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::{Client, RpcApi};
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut};
use std::collections::{BTreeMap, HashMap, HashSet};

impl Wallet {
    pub fn take_unspent_utxos_for(
        &mut self,
        amount: u64,
    ) -> Option<(BTreeMap<OutPoint, (TxOut, Address)>, u64)> {
        // Ensure UTXOs exist
        let utxos = &mut self.utxos;

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
        network: Network,
        save: Option<&AppContext>,
    ) -> Result<HashMap<OutPoint, TxOut>, String> {
        // Collect the addresses for which we want to load UTXOs.
        let addresses: Vec<_> = self.known_addresses.keys().collect();

        // Use the RPC client to list unspent outputs.
        match core_client.list_unspent(None, None, Some(&addresses), Some(false), None) {
            Ok(utxos) => {
                // Initialize the HashMap to store the new UTXOs.
                let mut new_utxo_map = HashMap::new();
                // Build a set of new OutPoints for easy comparison.
                let mut new_outpoints = HashSet::new();

                // Iterate over the retrieved UTXOs and populate the HashMaps.
                for utxo in utxos {
                    let outpoint = OutPoint::new(utxo.txid, utxo.vout);
                    let tx_out = TxOut {
                        value: utxo.amount.to_sat(),
                        script_pubkey: utxo.script_pub_key.clone(),
                    };
                    new_utxo_map.insert(outpoint.clone(), tx_out);
                    new_outpoints.insert(outpoint);
                }

                // Collect current UTXOs into a set for comparison
                let mut old_outpoints = HashSet::new();
                for (_address, utxos) in self.utxos.iter() {
                    for (outpoint, _tx_out) in utxos.iter() {
                        old_outpoints.insert(outpoint.clone());
                    }
                }

                // Determine UTXOs to be removed and added
                let removed_outpoints: HashSet<_> =
                    old_outpoints.difference(&new_outpoints).cloned().collect();
                let added_outpoints: HashSet<_> =
                    new_outpoints.difference(&old_outpoints).cloned().collect();

                // Now update self.utxos by removing UTXOs not present in new_outpoints
                let current_utxos = &mut self.utxos;
                // Remove UTXOs that are no longer unspent
                for utxos in current_utxos.values_mut() {
                    utxos.retain(|outpoint, _| new_outpoints.contains(outpoint));
                }
                // Remove addresses with no UTXOs
                current_utxos.retain(|_, utxos| !utxos.is_empty());

                // Add new UTXOs to self.utxos
                let current_utxos = &mut self.utxos;
                for (outpoint, tx_out) in &new_utxo_map {
                    // Get the address from the script_pubkey
                    let address = Address::from_script(&tx_out.script_pubkey, network)
                        .map_err(|e| e.to_string())?;
                    // Add or update the UTXO in the wallet
                    current_utxos
                        .entry(address.clone())
                        .or_insert_with(HashMap::new)
                        .insert(outpoint.clone(), tx_out.clone());
                }

                // If save is Some, update the database
                if let Some(app_context) = save {
                    let db = &app_context.db;

                    // Remove UTXOs that are no longer unspent
                    for outpoint in removed_outpoints {
                        db.drop_utxo(&outpoint, &network.to_string())
                            .map_err(|e| e.to_string())?;
                    }

                    // Add new UTXOs
                    for outpoint in added_outpoints {
                        let tx_out = &new_utxo_map[&outpoint];
                        let address = Address::from_script(&tx_out.script_pubkey, network)
                            .map_err(|e| e.to_string())?;

                        db.insert_utxo(
                            outpoint.txid.as_ref(),
                            outpoint.vout,
                            &address,
                            tx_out.value,
                            tx_out.script_pubkey.as_bytes(),
                            network,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }

                // Return the new UTXO map
                Ok(new_utxo_map)
            }
            Err(first_error) => Err(first_error.to_string()),
        }
    }
}
