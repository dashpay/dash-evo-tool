//! Refresh Single Key Wallet Info - Reload UTXOs and balances for a single key wallet

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::{OutPoint, TxOut};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Refresh a single key wallet by reloading UTXOs from Core RPC
    pub fn refresh_single_key_wallet_info(
        &self,
        wallet: Arc<RwLock<SingleKeyWallet>>,
    ) -> Result<(), TaskError> {
        let (address, key_hash, core_wallet_name) = {
            let wallet_guard = wallet.read()?;
            (
                wallet_guard.address.clone(),
                wallet_guard.key_hash,
                wallet_guard.core_wallet_name.clone(),
            )
        };

        let client = self.core_client_for_wallet(core_wallet_name.as_deref())?;

        if let Err(e) = client.import_address(&address, None, Some(false)) {
            tracing::debug!(?e, address = %address, "import_address failed during single key refresh");
        }

        let utxo_map = {
            let utxos = client.list_unspent(Some(0), None, Some(&[&address]), None, None)?;

            let mut map: HashMap<OutPoint, TxOut> = HashMap::new();
            for utxo in utxos {
                let outpoint = OutPoint::new(utxo.txid, utxo.vout);
                let tx_out = TxOut {
                    value: utxo.amount.to_sat(),
                    script_pubkey: utxo.script_pub_key,
                };
                map.insert(outpoint, tx_out);
            }
            map
        };

        let total_balance: u64 = utxo_map.values().map(|tx_out| tx_out.value).sum();

        {
            let mut wallet_guard = wallet.write()?;
            wallet_guard.utxos = utxo_map.clone();
            wallet_guard.update_balances(total_balance, 0, total_balance);
        }

        if let Err(e) =
            self.db
                .update_single_key_wallet_balances(&key_hash, total_balance, 0, total_balance)
        {
            tracing::warn!(error = %e, "Failed to persist single key wallet balances");
        }

        for (outpoint, tx_out) in &utxo_map {
            self.db.insert_utxo(
                outpoint.txid.as_ref(),
                outpoint.vout,
                &address,
                tx_out.value,
                &tx_out.script_pubkey.to_bytes(),
                self.network,
            )?;
        }

        Ok(())
    }
}
