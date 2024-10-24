use crate::database::Database;
use crate::model::wallet::{AddressInfo, DerivationPathReference, DerivationPathType, Wallet};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Network, OutPoint, ScriptBuf, TxOut, Txid};
use rusqlite::params;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

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

    /// Update the alias of a wallet based on the seed.
    /// If the alias is `None`, it sets the alias to NULL in the database.
    pub fn set_wallet_alias(
        &self,
        seed: &[u8; 64],
        new_alias: Option<String>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE wallet SET alias = ? WHERE seed = ?",
            params![new_alias, seed],
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

    /// Add a new address to a wallet with optional balance.
    /// If the address already exists, it does nothing.
    pub fn add_address(
        &self,
        seed: &[u8; 64],
        address: &Address,
        derivation_path: &DerivationPath,
        path_reference: DerivationPathReference,
        path_type: DerivationPathType,
        balance: Option<u64>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        // Step 1: Check if the address already exists for the given seed.
        let mut stmt = conn.prepare(
            "SELECT COUNT(1) FROM wallet_addresses
         WHERE seed = ? AND address = ?",
        )?;
        let count: u32 = stmt.query_row(params![seed, address.to_string()], |row| row.get(0))?;

        // Step 2: If the address doesn't exist, insert it.
        if count == 0 {
            conn.execute(
                "INSERT INTO wallet_addresses
             (seed, address, derivation_path, path_reference, path_type, balance)
             VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    seed,
                    address.to_string(),
                    derivation_path.to_string(),
                    path_reference as u32,
                    path_type.bits(),
                    balance,
                ],
            )?;
        }
        Ok(())
    }

    /// Update the balance of an existing address.
    pub fn update_address_balance(
        &self,
        seed: &[u8; 64],
        address: &Address,
        new_balance: u64,
    ) -> rusqlite::Result<()> {
        let rows_affected = self.execute(
            "UPDATE wallet_addresses
         SET balance = ?
         WHERE seed = ? AND address = ?",
            params![new_balance, seed, address.to_string()],
        )?;

        if rows_affected == 0 {
            Err(rusqlite::Error::QueryReturnedNoRows)
        } else {
            Ok(())
        }
    }

    /// Retrieve all wallets for a specific network, including their addresses, balances, and known addresses.
    pub fn get_wallets(&self, network: &Network) -> rusqlite::Result<Vec<Wallet>> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();

        // Step 1: Retrieve all wallets for the given network.
        let mut stmt = conn
            .prepare("SELECT seed, alias, is_main, password_hint FROM wallet WHERE network = ?")?;

        let mut wallets_map: BTreeMap<[u8; 64], Wallet> = BTreeMap::new();

        let wallet_rows = stmt.query_map([network_str.clone()], |row| {
            let seed: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?;
            let is_main: bool = row.get(2)?;
            let password_hint: Option<String> = row.get(3)?;

            let seed_array: [u8; 64] = seed.try_into().expect("Seed should be 64 bytes");

            // Insert a new Wallet into the map
            wallets_map.insert(
                seed_array,
                Wallet {
                    seed: seed_array,
                    address_balances: BTreeMap::new(),
                    known_addresses: BTreeMap::new(),
                    watched_addresses: BTreeMap::new(),
                    alias,
                    utxos: None,
                    is_main,
                    password_hint,
                },
            );
            Ok(())
        })?;

        // Collect any errors during wallet row processing
        for wallet in wallet_rows {
            wallet?;
        }

        // Step 2: Retrieve all addresses, balances, and derivation paths associated with the wallets.
        let mut address_stmt = conn.prepare(
            "SELECT seed, address, derivation_path, balance, path_reference, path_type FROM wallet_addresses",
        )?;

        let address_rows = address_stmt.query_map([], |row| {
            let seed: Vec<u8> = row.get(0)?;
            let address: String = row.get(1)?;
            let derivation_path: String = row.get(2)?;
            let balance: Option<u64> = row.get(3)?;
            let path_reference: u32 = row.get(4)?;
            let path_type: u32 = row.get(5)?;

            let seed_array: [u8; 64] = seed.try_into().expect("Seed should be 64 bytes");
            let address = Address::from_str(&address)
                .expect("Invalid address format")
                .assume_checked();
            let derivation_path = DerivationPath::from_str(&derivation_path)
                .expect("Expected to convert to derivation path");

            // Convert u32 to DerivationPathReference safely
            let path_reference =
                DerivationPathReference::try_from(path_reference).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(std::fmt::Error),
                    )
                })?;

            let path_type = DerivationPathType::from_bits_truncate(path_type as u32);

            Ok((
                seed_array,
                address,
                derivation_path,
                balance,
                path_reference,
                path_type,
            ))
        })?;

        // Step 3: Add addresses, balances, and known addresses to the corresponding wallets.
        for row in address_rows {
            let (seed_array, address, derivation_path, balance, path_reference, path_type) = row?;

            if let Some(wallet) = wallets_map.get_mut(&seed_array) {
                // Update the address balance if available.
                if let Some(balance) = balance {
                    wallet.address_balances.insert(address.clone(), balance);
                }

                // Add the address to the `known_addresses` map.
                wallet
                    .known_addresses
                    .insert(address.clone(), derivation_path.clone());

                // Add the address to the `watched_addresses` map with AddressInfo.
                let address_info = AddressInfo {
                    address: address.clone(),
                    path_reference,
                    path_type,
                };
                wallet
                    .watched_addresses
                    .insert(derivation_path, address_info);
            }
        }

        // Step 4: Retrieve UTXOs for each wallet and add them to the wallets.
        let mut utxo_stmt = conn.prepare(
            "SELECT txid, vout, address, value, script_pubkey FROM utxos WHERE network = ?",
        )?;

        let utxo_rows = utxo_stmt.query_map([network_str.clone()], |row| {
            let txid: Vec<u8> = row.get(0)?;
            let vout: i64 = row.get(1)?;
            let address: String = row.get(2)?;
            let value: i64 = row.get(3)?;
            let script_pubkey: Vec<u8> = row.get(4)?;

            let address = Address::from_str(&address)
                .expect("Invalid address format")
                .assume_checked();

            let outpoint = OutPoint {
                txid: Txid::from_slice(&txid).expect("Invalid txid"),
                vout: vout as u32,
            };
            let tx_out = TxOut {
                value: value as u64,
                script_pubkey: ScriptBuf::from_bytes(script_pubkey),
            };
            Ok((address, outpoint, tx_out))
        })?;

        // Step 5: Add the UTXOs to the corresponding wallets.
        for row in utxo_rows {
            let (address, outpoint, tx_out) = row?;

            for wallet in wallets_map.values_mut() {
                if wallet.known_addresses.contains_key(&address) {
                    wallet
                        .utxos
                        .get_or_insert_with(HashMap::new)
                        .entry(address.clone())
                        .or_insert_with(HashMap::new)
                        .insert(outpoint, tx_out.clone());
                }
            }
        }

        // Convert the BTreeMap into a Vec of Wallets.
        Ok(wallets_map.into_values().collect())
    }
}
