use crate::context::AppContext;
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use crate::spv::CoreBackendMode;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::json::ListUnspentResultEntry;
use dash_sdk::dpp::dashcore::{Address, OutPoint, TxOut};
use std::collections::{BTreeMap, HashMap, HashSet};

impl Wallet {
    #[allow(clippy::type_complexity)]
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

    /// Reload UTXOs from Core RPC, updating both in-memory state and database.
    ///
    /// Returns `true` if the UTXO set changed (something was added or
    /// removed), `false` if nothing changed. In SPV mode this is a no-op
    /// (`Ok(false)`) because the wallet state is authoritative — UTXOs are
    /// synced continuously via compact block filters.
    pub fn reload_utxos(&mut self, app_context: &AppContext) -> Result<bool, String> {
        let network = app_context.network;

        // SPV wallet state is authoritative — reload is a no-op.
        if app_context.core_backend_mode() == CoreBackendMode::Spv {
            return Ok(false);
        }

        let core_client = app_context
            .core_client
            .read()
            .map_err(|e| format!("Core client lock was poisoned: {}", e))?;

        // Collect Core chain addresses for which we want to load UTXOs.
        // Platform addresses are NOT valid on Core chain and must be excluded.
        let addresses: Vec<_> = self
            .known_addresses
            .iter()
            .filter(|(_, path)| !path.is_platform_payment(network))
            .map(|(addr, _)| addr)
            .collect();
        if tracing::enabled!(tracing::Level::TRACE) {
            for addr in addresses.iter() {
                let (net, payload) = (*addr).clone().into_parts();
                tracing::trace!(net=net.to_string(),payload=?payload , "Address to load UTXOs for");
            }
        }

        // Calling list_unspent with an empty addresses vector will return all UTXOs,
        // which is not what we want here. Instead, we handle the empty case explicitly.
        let utxos: Vec<ListUnspentResultEntry> = if addresses.is_empty() {
            Vec::new()
        } else {
            core_client
                .list_unspent(None, None, Some(&addresses), Some(false), None)
                .map_err(|e| e.to_string())?
        };

        // Drop the RPC client guard before the rest of the bookkeeping
        drop(core_client);

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

        let changed = !removed_outpoints.is_empty() || !added_outpoints.is_empty();

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
            let address =
                Address::from_script(&tx_out.script_pubkey, network).map_err(|e| e.to_string())?;
            // Add or update the UTXO in the wallet
            current_utxos
                .entry(address.clone())
                .or_default()
                .insert(*outpoint, tx_out.clone());
        }

        // Always persist changes to the database
        let db = &app_context.db;

        // Remove UTXOs that are no longer unspent
        for outpoint in removed_outpoints {
            db.drop_utxo(&outpoint, &network.to_string())
                .map_err(|e| e.to_string())?;
        }

        // Add new UTXOs
        for outpoint in added_outpoints {
            let tx_out = &new_utxo_map[&outpoint];
            let address =
                Address::from_script(&tx_out.script_pubkey, network).map_err(|e| e.to_string())?;

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

        Ok(changed)
    }

    /// Get all addresses with their total UTXO balances
    pub fn utxos_by_address(&self) -> Vec<(Address, u64)> {
        self.utxos
            .iter()
            .map(|(address, utxos)| {
                let total_balance: u64 = utxos.values().map(|tx_out| tx_out.value).sum();
                (address.clone(), total_balance)
            })
            .filter(|(_, balance)| *balance > 0)
            .collect()
    }
}
