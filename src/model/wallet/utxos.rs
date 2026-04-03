use crate::database::Database;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut};
use std::collections::{BTreeMap, HashMap};

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

}
