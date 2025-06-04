use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::{Client, RpcApi};
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut};
use std::collections::{BTreeMap, HashMap, HashSet};

impl Wallet {
    pub fn take_unspent_utxos_for(
        &mut self,
        amount: u64,
        fee: u64,
        allow_take_fee_from_amount: bool,
    ) -> Option<(BTreeMap<OutPoint, (TxOut, Address)>, Option<u64>)> {
        // Ensure UTXOs exist
        let utxos = &mut self.utxos;

        let mut required: i64 = (amount + fee) as i64;
        let mut taken_utxos = BTreeMap::new();
        let mut utxos_to_remove = Vec::new();

        // Iterate over the UTXOs to collect enough to cover the required amount
        for (address, outpoints) in utxos.iter_mut() {
            for (outpoint, tx_out) in outpoints.iter() {
                if required <= 0 {
                    break;
                }

                // Add the UTXO to the result
                taken_utxos.insert(*outpoint, (tx_out.clone(), address.clone()));

                required -= tx_out.value as i64;
                utxos_to_remove.push((address.clone(), *outpoint));
            }
        }

        // If not enough UTXOs were found, try to adjust if allowed
        if required > 0 {
            if allow_take_fee_from_amount {
                let total_collected = (amount + fee) as i64 - required;
                if total_collected >= amount as i64 {
                    // We have enough to cover the amount, but not the fee
                    // So we can reduce the amount by the missing fee
                    let missing_fee = required; // required > 0
                    let adjusted_amount = amount as i64 - missing_fee;
                    if adjusted_amount <= 0 {
                        // Cannot adjust amount to cover missing fee
                        return None;
                    }
                    // Remove UTXOs from wallet
                    for (address, outpoint) in utxos_to_remove {
                        if let Some(outpoints) = utxos.get_mut(&address) {
                            outpoints.remove(&outpoint);
                            if outpoints.is_empty() {
                                utxos.remove(&address);
                            }
                        }
                    }
                    // Return collected UTXOs and None for change
                    Some((taken_utxos, None))
                } else {
                    // Not enough to cover amount even after adjusting
                    None
                }
            } else {
                // Not enough UTXOs and not allowed to take fee from amount
                None
            }
        } else {
            // Remove the collected UTXOs from the wallet's UTXO map
            for (address, outpoint) in utxos_to_remove {
                if let Some(outpoints) = utxos.get_mut(&address) {
                    outpoints.remove(&outpoint);
                    if outpoints.is_empty() {
                        utxos.remove(&address);
                    }
                }
            }
            // Calculate change amount
            let total_input = (amount + fee) as i64 - required; // total input collected
            let change = total_input as u64 - amount - fee;

            // If change is zero, return None
            let change_option = if change > 0 { Some(change) } else { None };

            // Return the collected UTXOs and the change amount
            Some((taken_utxos, change_option))
        }
    }

    pub fn reload_utxos(
        &mut self,
        core_client: &Client,
        network: Network,
        save: Option<&AppContext>,
    ) -> Result<HashMap<OutPoint, TxOut>, String> {
        // Collect the addresses for which we want to load UTXOs.
        let addresses: Vec<_> = self.known_addresses.keys().collect();
        if tracing::enabled!(tracing::Level::TRACE) {
            for addr in addresses.iter() {
                let (net, payload) = (*addr).clone().into_parts();
                tracing::trace!(net=net.to_string(),payload=?payload , "Address to load UTXOs for");
            }
        }

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
                    new_utxo_map.insert(outpoint, tx_out);
                    new_outpoints.insert(outpoint);
                }

                // Collect current UTXOs into a set for comparison
                let mut old_outpoints = HashSet::new();
                for (_address, utxos) in self.utxos.iter() {
                    for (outpoint, _tx_out) in utxos.iter() {
                        old_outpoints.insert(*outpoint);
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
                        .or_default()
                        .insert(*outpoint, tx_out.clone());
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
