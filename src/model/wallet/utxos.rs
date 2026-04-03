use crate::database::Database;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut};
use std::collections::BTreeMap;

impl Wallet {
    /// Persist UTXO removals to the database and recalculate affected
    /// address balances. In-memory UTXOs are tracked by ManagedWalletInfo.
    pub fn remove_selected_utxos(
        &self,
        selected: &BTreeMap<OutPoint, (TxOut, Address)>,
        db: &Database,
        network: Network,
    ) -> Result<(), String> {
        let network_str = network.to_string();
        for outpoint in selected.keys() {
            db.drop_utxo(outpoint, &network_str)
                .map_err(|e| e.to_string())?;
        }

        self.recalculate_affected_address_balances_with_db(selected, db)?;

        Ok(())
    }
}
