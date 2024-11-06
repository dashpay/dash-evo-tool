use crate::database::Database;
use dash_sdk::dashcore_rpc::dashcore::{OutPoint, ScriptBuf, TxOut, Txid};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, Network};
use rusqlite::params;

impl Database {
    /// Deletes a UTXO from the database given its OutPoint and network.
    pub fn drop_utxo(&self, outpoint: &OutPoint, network: &str) -> rusqlite::Result<()> {
        let txid_bytes = outpoint.txid.as_byte_array(); // &[u8; 32]
        let vout = outpoint.vout as i64; // i64

        self.execute(
            "DELETE FROM utxos WHERE txid = ? AND vout = ? AND network = ?",
            params![txid_bytes, vout, network],
        )?;

        Ok(())
    }

    pub(crate) fn insert_utxo(
        &self,
        txid: &[u8],
        vout: u32,
        address: &Address,
        value: u64,
        script_pubkey: &[u8],
        network: Network,
    ) -> rusqlite::Result<()> {
        self.execute(
            "INSERT OR IGNORE INTO utxos (txid, vout, address, value, script_pubkey, network)
         VALUES (?, ?, ?, ?, ?, ?)",
            params![
                txid,
                vout,
                address.to_string(),
                value,
                script_pubkey,
                network.to_string()
            ],
        )?;
        Ok(())
    }

    fn get_utxos_by_address(
        &self,
        address: &str,
        network: &str,
    ) -> Result<Vec<(OutPoint, TxOut)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT txid, vout, value, script_pubkey FROM utxos
         WHERE address = ? AND network = ?",
            )
            .map_err(|e| e.to_string())?;

        let tx_out_iter = stmt
            .query_map(params![address, network], |row| {
                let txid_bytes: Vec<u8> = row.get(0)?;
                let vout: u32 = row.get(1)?;
                let value: u64 = row.get(2)?;
                let script_pubkey_bytes: Vec<u8> = row.get(3)?;

                let txid = Txid::from_slice(&txid_bytes)
                    .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
                let outpoint = OutPoint { txid, vout };

                let script_pubkey = ScriptBuf::from_bytes(script_pubkey_bytes);

                let tx_out = TxOut {
                    value,
                    script_pubkey,
                };

                Ok((outpoint, tx_out))
            })
            .map_err(|e| e.to_string())?;

        let mut utxos = Vec::new();
        for utxo in tx_out_iter {
            utxos.push(utxo.map_err(|e| e.to_string())?);
        }

        Ok(utxos)
    }
}
