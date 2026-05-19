use crate::model::wallet::Wallet;
use dash_sdk::dpp::dashcore::Address;

impl Wallet {
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
