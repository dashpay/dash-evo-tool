use crate::context::AppContext;
use crate::database::{Database, WalletError};
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use crate::spv::CoreBackendMode;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::json::ListUnspentResultEntry;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut};
use std::collections::{BTreeMap, HashMap, HashSet};

impl Wallet {
    /// Selects UTXOs sufficient to cover `amount + fee` without removing them from the wallet.
    ///
    /// Returns the selected UTXOs and an optional change amount, or `None` if there are
    /// insufficient funds.  The returned change value is computed from the caller-supplied
    /// `fee` parameter; callers that recalculate fees afterward (e.g., based on the actual
    /// number of inputs) should ignore the change value and recompute it.
    ///
    /// Use this when you need to inspect or validate the selection before
    /// committing; call [`Self::remove_selected_utxos`] only once the operation
    /// is guaranteed to succeed.
    ///
    /// **Important:** The caller must hold the wallet write lock (`&mut self` on `Wallet`)
    /// continuously from this call through the corresponding [`Self::remove_selected_utxos`]
    /// call.  Dropping the lock between selection and removal would allow a concurrent
    /// caller to select the same UTXOs, creating a double-spend.
    #[allow(clippy::type_complexity)]
    pub fn select_unspent_utxos_for(
        &self,
        amount: u64,
        fee: u64,
        allow_take_fee_from_amount: bool,
        source_address: Option<&Address>,
    ) -> Option<(BTreeMap<OutPoint, (TxOut, Address)>, Option<u64>)> {
        let target = amount.checked_add(fee)?;
        let mut required: i64 = i64::try_from(target).ok()?;
        let mut selected_utxos = BTreeMap::new();

        let iter: Box<dyn Iterator<Item = (&Address, &HashMap<OutPoint, TxOut>)>> =
            match source_address {
                Some(addr) => Box::new(self.utxos.get(addr).into_iter().map(move |m| (addr, m))),
                None => Box::new(self.utxos.iter()),
            };
        for (address, outpoints) in iter {
            for (outpoint, tx_out) in outpoints.iter() {
                if required <= 0 {
                    break;
                }
                selected_utxos.insert(*outpoint, (tx_out.clone(), address.clone()));
                required -= tx_out.value as i64;
            }
        }

        if required > 0 {
            if allow_take_fee_from_amount {
                let total_collected = target as i64 - required;
                if total_collected >= amount as i64 {
                    let missing_fee = required; // required > 0
                    let adjusted_amount = amount as i64 - missing_fee;
                    if adjusted_amount <= 0 {
                        return None;
                    }
                    Some((selected_utxos, None))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            let total_input = target as i64 - required;
            let change = total_input as u64 - amount - fee;
            let change_option = if change > 0 { Some(change) } else { None };
            Some((selected_utxos, change_option))
        }
    }

    /// Removes the given UTXOs from the wallet's in-memory UTXO set, persists
    /// the removal to the database, and recalculates affected address balances.
    ///
    /// Typically called with the result of [`Self::select_unspent_utxos_for`]
    /// after the transaction has been fully built and signed.
    pub fn remove_selected_utxos(
        &mut self,
        selected: &BTreeMap<OutPoint, (TxOut, Address)>,
        db: &Database,
        network: Network,
    ) -> Result<(), String> {
        // Update in-memory UTXO map
        for (outpoint, (_, address)) in selected {
            if let Some(outpoints) = self.utxos.get_mut(address) {
                outpoints.remove(outpoint);
                if outpoints.is_empty() {
                    self.utxos.remove(address);
                }
            } else {
                tracing::debug!(
                    ?outpoint,
                    %address,
                    "remove_selected_utxos: outpoint not found in wallet, skipping"
                );
            }
        }

        // Persist UTXO removals to the database
        let network_str = network.to_string();
        for outpoint in selected.keys() {
            db.drop_utxo(outpoint, &network_str)
                .map_err(|e| e.to_string())?;
        }

        // Recalculate and persist balances for affected addresses
        self.recalculate_affected_address_balances_with_db(selected, db)?;

        Ok(())
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
            .core_client_for_wallet(self.core_wallet_name.as_deref())
            .map_err(|e| e.to_string())?;

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
            let address = Address::from_script(&tx_out.script_pubkey, network)
                .map_err(|e| WalletError::AddressError(e).to_string())?;
            // Add or update the UTXO in the wallet
            current_utxos
                .entry(address.clone())
                .or_default()
                .insert(*outpoint, tx_out.clone());
        }

        // Persist changes to the database only when something actually changed
        if changed {
            let db = &app_context.db;

            for outpoint in removed_outpoints {
                db.drop_utxo(&outpoint, &network.to_string())
                    .map_err(|e| e.to_string())?;
            }

            for outpoint in added_outpoints {
                let tx_out = &new_utxo_map[&outpoint];
                let address = Address::from_script(&tx_out.script_pubkey, network)
                    .map_err(|e| WalletError::AddressError(e).to_string())?;

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
