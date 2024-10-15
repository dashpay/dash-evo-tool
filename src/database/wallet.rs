use crate::database::Database;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::dashcore::Network;
use rusqlite::params;

impl Database {
    /// Insert a new wallet into the wallet table
    pub fn insert_wallet(&self, wallet: &Wallet, network: &Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();
        self.execute(
            "INSERT INTO wallet (seed, alias, is_main, password_hint, network)
             VALUES (?, ?, ?, ?, ?)",
            params![
                wallet.seed,
                wallet.alias.clone(),
                wallet.is_main as i32,
                wallet.password_hint.clone(),
                network_str
            ],
        )?;
        Ok(())
    }

    /// Update only the alias and is_main fields of a wallet
    pub fn update_wallet_alias_and_main(
        &self,
        seed: &[u8; 64],
        new_alias: Option<String>,
        is_main: bool,
    ) -> rusqlite::Result<()> {
        self.execute(
            "UPDATE wallet SET alias = ?, is_main = ? WHERE seed = ?",
            params![new_alias, is_main as i32, seed],
        )?;
        Ok(())
    }

    /// Retrieve all wallets for a specific network
    pub fn get_wallets(&self, network: &Network) -> rusqlite::Result<Vec<Wallet>> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT seed, alias, is_main, password_hint FROM wallet WHERE network = ?")?;

        let wallet_iter = stmt.query_map(params![network_str], |row| {
            let seed: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?;
            let is_main: bool = row.get(2)?;
            let password_hint: Option<String> = row.get(3)?;

            Ok(Wallet {
                seed: seed.try_into().expect("Seed should be 64 bytes"),
                alias,
                is_main,
                password_hint,
            })
        })?;

        wallet_iter.collect()
    }
}
