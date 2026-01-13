use crate::database::Database;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{
    AddressInfo, ClosedKeyItem, DerivationPathReference, DerivationPathType, OpenWalletSeed,
    Wallet, WalletSeed, WalletTransaction,
};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::address::{NetworkChecked, NetworkUnchecked};
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    self, BlockHash, InstantLock, Network, OutPoint, ScriptBuf, Transaction, TxOut, Txid,
};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, ExtendedPubKey};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

impl Database {
    /// Insert a new wallet into the wallet table
    pub fn store_wallet(&self, wallet: &Wallet, network: &Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();

        // Serialize the extended public keys
        let master_ecdsa_bip44_account_0_epk_bytes =
            wallet.master_bip44_ecdsa_extended_public_key.encode();

        self.execute(
            "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint, network, confirmed_balance, unconfirmed_balance, total_balance)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                wallet.seed_hash(),
                wallet.encrypted_seed_slice(),
                wallet.salt(),
                wallet.nonce(),
                master_ecdsa_bip44_account_0_epk_bytes,
                wallet.alias.clone(),
                wallet.is_main as i32,
                wallet.uses_password,
                wallet.password_hint().clone(),
                network_str,
                wallet.confirmed_balance as i64,
                wallet.unconfirmed_balance as i64,
                wallet.total_balance as i64
            ],
        )?;
        Ok(())
    }

    /// Update the alias of a wallet based on the seed.
    /// If the alias is `None`, it sets the alias to NULL in the database.
    pub fn set_wallet_alias(
        &self,
        seed_hash: &[u8; 32],
        new_alias: Option<String>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE wallet SET alias = ? WHERE seed_hash = ?",
            params![new_alias, seed_hash],
        )?;

        Ok(())
    }

    /// Remove a wallet and all associated records from the database.
    ///
    /// This clears dependent records (addresses, utxos, asset locks, identity links)
    /// to keep the database consistent before deleting the wallet itself.
    pub fn remove_wallet(&self, seed_hash: &[u8; 32], network: &Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut address_stmt =
            tx.prepare("SELECT address FROM wallet_addresses WHERE seed_hash = ?")?;
        let address_rows =
            address_stmt.query_map(params![seed_hash], |row| row.get::<_, String>(0))?;
        let mut addresses = Vec::new();
        for address in address_rows {
            addresses.push(address?);
        }
        drop(address_stmt);

        for address in addresses {
            tx.execute(
                "DELETE FROM utxos WHERE address = ? AND network = ?",
                params![address, &network_str],
            )?;
        }

        tx.execute(
            "UPDATE identity SET wallet = NULL, wallet_index = NULL WHERE wallet = ? AND network = ?",
            params![seed_hash, &network_str],
        )?;

        tx.execute(
            "DELETE FROM wallet WHERE seed_hash = ? AND network = ?",
            params![seed_hash, &network_str],
        )?;

        tx.commit()
    }

    /// Update only the alias and is_main fields of a wallet
    #[allow(dead_code)] // May be used for batch wallet metadata updates
    pub fn update_wallet_alias_and_main(
        &self,
        seed_hash: &[u8; 32],
        new_alias: Option<String>,
        is_main: bool,
    ) -> rusqlite::Result<()> {
        self.execute(
            "UPDATE wallet SET alias = ?, is_main = ? WHERE seed_hash = ?",
            params![new_alias, is_main as i32, seed_hash],
        )?;
        Ok(())
    }

    /// Add a new address to a wallet with optional balance.
    /// If the address already exists, it does nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn add_address_if_not_exists(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        network: &Network,
        derivation_path: &DerivationPath,
        path_reference: DerivationPathReference,
        path_type: DerivationPathType,
        balance: Option<u64>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        let address = check_address_for_network(address.as_unchecked().clone(), network)
            .expect("Expected address to be valid for network");

        // Step 1: Check if the address already exists for the given seed.
        let mut stmt = conn.prepare(
            "SELECT COUNT(1) FROM wallet_addresses
         WHERE seed_hash = ? AND address = ?",
        )?;
        let count: u32 =
            stmt.query_row(params![seed_hash, address.to_string()], |row| row.get(0))?;

        // Step 2: If the address doesn't exist, insert it.
        if count == 0 {
            conn.execute(
                "INSERT INTO wallet_addresses
             (seed_hash, address, derivation_path, path_reference, path_type, balance)
             VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    seed_hash,
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
        seed_hash: &[u8; 32],
        address: &Address,
        new_balance: u64,
    ) -> rusqlite::Result<()> {
        let rows_affected = self.execute(
            "UPDATE wallet_addresses
         SET balance = ?
         WHERE seed_hash = ? AND address = ?",
            params![new_balance, seed_hash, address.to_string()],
        )?;

        if rows_affected == 0 {
            Err(rusqlite::Error::QueryReturnedNoRows)
        } else {
            Ok(())
        }
    }

    /// Add a balance to an existing address.
    pub fn add_to_address_balance(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        additional_balance: u64,
    ) -> rusqlite::Result<()> {
        let rows_affected = self.execute(
            "UPDATE wallet_addresses
         SET balance = balance + ?
         WHERE seed_hash = ? AND address = ?",
            params![additional_balance, seed_hash, address.to_string()],
        )?;

        if rows_affected == 0 {
            Err(rusqlite::Error::QueryReturnedNoRows)
        } else {
            Ok(())
        }
    }

    /// Migration: Add balance columns to wallet table (version 16).
    pub fn add_wallet_balance_columns(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if confirmed_balance column exists
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet') WHERE name='confirmed_balance'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN confirmed_balance INTEGER DEFAULT 0;",
                (),
            )?;
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN unconfirmed_balance INTEGER DEFAULT 0;",
                (),
            )?;
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN total_balance INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Update the wallet's balance fields in the database.
    pub fn update_wallet_balances(
        &self,
        seed_hash: &[u8; 32],
        confirmed_balance: u64,
        unconfirmed_balance: u64,
        total_balance: u64,
    ) -> rusqlite::Result<()> {
        self.execute(
            "UPDATE wallet SET confirmed_balance = ?, unconfirmed_balance = ?, total_balance = ? WHERE seed_hash = ?",
            params![confirmed_balance as i64, unconfirmed_balance as i64, total_balance as i64, seed_hash],
        )?;
        Ok(())
    }

    /// Migration: Add total_received column to wallet_addresses table.
    pub fn add_address_total_received_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if total_received column exists
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet_addresses') WHERE name='total_received'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE wallet_addresses ADD COLUMN total_received INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Ensures all required columns exist in wallet-related tables.
    /// This handles the case where old tables exist with missing columns.
    pub fn ensure_wallet_columns_exist(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if wallet_addresses table exists before trying to add columns
        let wallet_addresses_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wallet_addresses'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if wallet_addresses_exists {
            self.add_address_total_received_column(conn)?;
        }

        // Check if wallet table exists and add balance columns if needed
        let wallet_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wallet'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if wallet_exists {
            self.add_wallet_balance_columns(conn)?;
        }

        Ok(())
    }

    /// Update the total_received for an address.
    pub fn update_address_total_received(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        total_received: u64,
    ) -> rusqlite::Result<()> {
        self.execute(
            "UPDATE wallet_addresses SET total_received = ? WHERE seed_hash = ? AND address = ?",
            params![total_received as i64, seed_hash, address.to_string()],
        )?;
        Ok(())
    }

    pub fn initialize_wallet_transactions_table(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS wallet_transactions (
                seed_hash BLOB NOT NULL,
                txid BLOB NOT NULL,
                network TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                height INTEGER,
                block_hash BLOB,
                net_amount INTEGER NOT NULL,
                fee INTEGER,
                label TEXT,
                is_ours INTEGER NOT NULL,
                raw_transaction BLOB NOT NULL,
                PRIMARY KEY (seed_hash, txid, network),
                FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_wallet_transactions_network_ts
             ON wallet_transactions (network, timestamp DESC)",
            [],
        )?;

        Ok(())
    }

    /// Replace all persisted transactions for a wallet+network with the provided set.
    pub fn replace_wallet_transactions(
        &self,
        seed_hash: &[u8; 32],
        network: &Network,
        transactions: &[WalletTransaction],
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let network_str = network.to_string();

        tx.execute(
            "DELETE FROM wallet_transactions WHERE seed_hash = ?1 AND network = ?2",
            params![seed_hash, &network_str],
        )?;

        if transactions.is_empty() {
            tx.commit()?;
            return Ok(());
        }

        {
            let mut insert_stmt = tx.prepare(
                "INSERT INTO wallet_transactions (
                    seed_hash,
                    txid,
                    network,
                    timestamp,
                    height,
                    block_hash,
                    net_amount,
                    fee,
                    label,
                    is_ours,
                    raw_transaction
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;

            for transaction in transactions {
                let tx_bytes = serialize(&transaction.transaction);
                let block_hash_bytes: Option<Vec<u8>> = transaction
                    .block_hash
                    .as_ref()
                    .map(|hash| hash.as_raw_hash().as_byte_array().to_vec());
                let fee = transaction.fee.map(|f| f as i64);
                insert_stmt.execute(params![
                    seed_hash,
                    <dash_sdk::dpp::dashcore::Txid as AsRef<[u8]>>::as_ref(&transaction.txid),
                    &network_str,
                    transaction.timestamp as i64,
                    transaction.height.map(|h| h as i64),
                    block_hash_bytes.as_deref(),
                    transaction.net_amount,
                    fee,
                    transaction.label.as_deref(),
                    transaction.is_ours,
                    tx_bytes,
                ])?;
            }
        }

        tx.commit()
    }

    /// Retrieve all wallets for a specific network, including their addresses, balances, and known addresses.
    pub fn get_wallets(&self, network: &Network) -> rusqlite::Result<Vec<Wallet>> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();

        tracing::trace!("step 1: retrieve all wallets for the given network");
        let mut stmt = conn.prepare(
            "SELECT seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint, confirmed_balance, unconfirmed_balance, total_balance FROM wallet WHERE network = ?",
        )?;

        let mut wallets_map: BTreeMap<[u8; 32], Wallet> = BTreeMap::new();

        let wallet_rows = stmt.query_map([network_str.clone()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let encrypted_seed: Vec<u8> = row.get(1)?;
            let salt: Vec<u8> = row.get(2)?;
            let nonce: Vec<u8> = row.get(3)?;
            let master_ecdsa_bip44_account_0_epk_bytes: Vec<u8> = row.get(4)?;
            let alias: Option<String> = row.get(5)?;
            let is_main: bool = row.get(6)?;
            let uses_password: bool = row.get(7)?;
            let password_hint: Option<String> = row.get(8)?;
            let confirmed_balance: i64 = row.get::<_, Option<i64>>(9)?.unwrap_or(0);
            let unconfirmed_balance: i64 = row.get::<_, Option<i64>>(10)?.unwrap_or(0);
            let total_balance: i64 = row.get::<_, Option<i64>>(11)?.unwrap_or(0);

            // Reconstruct the extended public keys
            let master_ecdsa_extended_public_key =
                ExtendedPubKey::decode(&master_ecdsa_bip44_account_0_epk_bytes)
                    .expect("Failed to decode ExtendedPubKey");

            let seed_hash_array: [u8; 32] =
                seed_hash.try_into().expect("Seed hash should be 32 bytes");
            let closed_wallet_seed = ClosedKeyItem {
                seed_hash: seed_hash_array,
                encrypted_seed: encrypted_seed.clone(),
                salt,
                nonce,
                password_hint,
            };
            let wallet_seed = if uses_password {
                WalletSeed::Closed(closed_wallet_seed)
            } else {
                WalletSeed::Open(OpenWalletSeed {
                    seed: encrypted_seed
                        .try_into()
                        .expect("expected to decrypt seed with no password"),
                    wallet_info: closed_wallet_seed,
                })
            };

            tracing::trace!(
                alias = ?alias,
                wallet_seed = ?seed_hash_array,
                network = network_str,
                "new wallet loaded from database"
            );

            // Insert a new Wallet into the map
            wallets_map.insert(
                seed_hash_array,
                Wallet {
                    wallet_seed,
                    uses_password,
                    master_bip44_ecdsa_extended_public_key: master_ecdsa_extended_public_key,
                    address_balances: BTreeMap::new(),
                    address_total_received: BTreeMap::new(),
                    known_addresses: BTreeMap::new(),
                    watched_addresses: BTreeMap::new(),
                    unused_asset_locks: vec![],
                    alias,
                    identities: HashMap::new(),
                    utxos: HashMap::new(),
                    transactions: Vec::new(),
                    is_main,
                    confirmed_balance: confirmed_balance as u64,
                    unconfirmed_balance: unconfirmed_balance as u64,
                    total_balance: total_balance as u64,
                    platform_address_info: BTreeMap::new(),
                },
            );

            Ok(())
        })?;

        // Collect any errors during wallet row processing
        for wallet in wallet_rows {
            wallet?;
        }

        tracing::trace!(
            "step 2: retrieve all addresses, balances, and derivation paths associated with the wallets"
        );
        let mut address_stmt = conn.prepare(
            "SELECT seed_hash, address, derivation_path, balance, path_reference, path_type, total_received FROM wallet_addresses WHERE seed_hash IN (SELECT seed_hash FROM wallet WHERE network = ?)",
        )?;

        let address_rows = address_stmt.query_map([network_str.clone()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let address_str: String = row.get(1)?;
            let derivation_path: String = row.get(2)?;
            let balance: Option<u64> = row.get(3)?;
            let path_reference: u32 = row.get(4)?;
            let path_type: u32 = row.get(5)?;
            let total_received: Option<u64> = row.get(6)?;

            let seed_hash_array: [u8; 32] =
                seed_hash.try_into().expect("Seed hash should be 32 bytes");

            // Convert u32 to DerivationPathReference safely
            let path_reference =
                DerivationPathReference::try_from(path_reference).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(std::fmt::Error),
                    )
                })?;

            // Parse address - Platform addresses (DIP-17/18) use Bech32m encoding with dashevo/tdashevo prefix
            // and need special handling when stored (we store as Core address format internally)
            let address = if path_reference == DerivationPathReference::PlatformPayment {
                // Platform addresses are stored as Core P2PKH format for efficient internal lookup.
                // We use assume_checked() here because:
                // 1. Network validation was already performed at insertion time
                // 2. Platform addresses (bech32m) map to Core P2PKH addresses internally
                // 3. The stored address format doesn't have the same network version byte rules
                Address::from_str(&address_str)
                    .map(|a| a.assume_checked())
                    .map_err(|e| {
                        tracing::error!(address = %address_str, error = ?e, "Failed to parse Platform address");
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(std::fmt::Error),
                        )
                    })?
            } else {
                // Standard Core addresses - validate network
                let address_unchecked =
                    Address::from_str(&address_str).expect("Invalid address format");
                check_address_for_network(address_unchecked, network)?
            };

            let derivation_path = DerivationPath::from_str(&derivation_path)
                .expect("Expected to convert to derivation path");

            let path_type = DerivationPathType::from_bits_truncate(path_type);

            Ok((
                seed_hash_array,
                address,
                derivation_path,
                balance,
                path_reference,
                path_type,
                total_received,
            ))
        })?;

        tracing::trace!("step 3: add addresses, balances, and known addresses to wallets");
        for row in address_rows {
            if row.is_err() {
                continue;
            }
            let (
                seed_array,
                address,
                derivation_path,
                balance,
                path_reference,
                path_type,
                total_received,
            ) = row?;
            if let Some(wallet) = wallets_map.get_mut(&seed_array) {
                // Update the address balance if available.
                if let Some(balance) = balance {
                    wallet.address_balances.insert(address.clone(), balance);
                }
                // Update total received if available.
                if let Some(total_received) = total_received {
                    wallet
                        .address_total_received
                        .insert(address.clone(), total_received);
                }

                // Add the address to the `known_addresses` map.
                wallet
                    .known_addresses
                    .insert(address.clone(), derivation_path.clone());
                tracing::trace!(
                    address = ?address,
                    network = address.network().to_string(),
                    expected_network = network.to_string(),
                    "loaded address from database");

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

        tracing::trace!("step 4: retrieve UTXOs for each wallet and add them to the wallets");
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

        tracing::trace!("step 5: add the UTXOs to the corresponding wallets.");
        for row in utxo_rows {
            let (address, outpoint, tx_out) = row?;

            for wallet in wallets_map.values_mut() {
                if wallet.known_addresses.contains_key(&address) {
                    wallet
                        .utxos
                        .entry(address.clone())
                        .or_insert_with(HashMap::new)
                        .insert(outpoint, tx_out.clone());
                }
            }
        }
        tracing::trace!("step 6: load asset lock transactions for each wallet");
        let mut asset_lock_stmt = conn.prepare(
            "SELECT wallet, amount, transaction_data, instant_lock_data, chain_locked_height FROM asset_lock_transaction where identity_id IS NULL AND network = ?",
        )?;

        let asset_lock_rows = asset_lock_stmt.query_map([network.to_string()], |row| {
            let wallet_seed: Vec<u8> = row.get(0)?;
            let amount: Duffs = row.get(1)?;
            let tx_data: Vec<u8> = row.get(2)?;
            let islock_data: Option<Vec<u8>> = row.get(3)?;
            let chain_locked_height: Option<CoreBlockHeight> = row.get(4)?;

            let wallet_seed_hash_array: [u8; 32] =
                wallet_seed.try_into().expect("Seed should be 64 bytes");
            let tx: Transaction = deserialize(&tx_data).expect("Failed to deserialize transaction");

            // Ensure the transaction payload is AssetLockPayloadType
            let Some(TransactionPayload::AssetLockPayloadType(payload)) =
                &tx.special_transaction_payload
            else {
                panic!("Expected AssetLockPayloadType in special_transaction_payload");
            };

            // Get the first credit output
            let first = payload
                .credit_outputs
                .first()
                .expect("Expected at least one credit output");

            let address =
                Address::from_script(&first.script_pubkey, *network).expect("expected an address");

            let (islock, proof) = if let Some(islock_bytes) = islock_data {
                // Deserialize the InstantLock
                let is_lock: InstantLock =
                    deserialize(&islock_bytes).expect("Failed to deserialize InstantLock");
                (
                    Some(is_lock.clone()),
                    Some(AssetLockProof::Instant(InstantAssetLockProof::new(
                        is_lock,
                        tx.clone(),
                        0,
                    ))),
                )
            } else if let Some(chain_locked_height) = chain_locked_height {
                (
                    None,
                    Some(AssetLockProof::Chain(ChainAssetLockProof {
                        core_chain_locked_height: chain_locked_height,
                        out_point: OutPoint::new(tx.txid(), 0),
                    })),
                )
            } else {
                (None, None)
            };

            Ok((wallet_seed_hash_array, tx, address, amount, islock, proof))
        })?;

        tracing::trace!("step 7: add the asset lock transactions to the wallet");
        for row in asset_lock_rows {
            let (wallet_seed, tx, address, amount, islock, proof) = row?;

            if let Some(wallet) = wallets_map.get_mut(&wallet_seed) {
                wallet
                    .unused_asset_locks
                    .push((tx, address, amount, islock, proof));
            }
        }

        tracing::trace!("step 7: load wallet transactions for each wallet");
        let mut tx_stmt = conn.prepare(
            "SELECT seed_hash, txid, timestamp, height, block_hash, net_amount, fee, label, is_ours, raw_transaction
             FROM wallet_transactions WHERE network = ? ORDER BY timestamp DESC",
        )?;

        let tx_rows = tx_stmt.query_map([network_str.clone()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let txid_bytes: Vec<u8> = row.get(1)?;
            let timestamp: i64 = row.get(2)?;
            let height: Option<i64> = row.get(3)?;
            let block_hash_bytes: Option<Vec<u8>> = row.get(4)?;
            let net_amount: i64 = row.get(5)?;
            let fee: Option<i64> = row.get(6)?;
            let label: Option<String> = row.get(7)?;
            let is_ours: bool = row.get(8)?;
            let raw_transaction: Vec<u8> = row.get(9)?;

            let seed_hash_array: [u8; 32] =
                seed_hash.try_into().expect("Seed hash should be 32 bytes");
            let txid = Txid::from_slice(&txid_bytes).expect("Invalid txid bytes");
            let transaction: Transaction =
                deserialize(&raw_transaction).expect("Failed to deserialize transaction");
            let block_hash = block_hash_bytes
                .as_ref()
                .map(|bytes| BlockHash::from_slice(bytes).expect("Invalid block hash"));
            let fee = fee.map(|f| f as u64);
            let height = height.map(|h| h as u32);

            Ok((
                seed_hash_array,
                WalletTransaction {
                    txid,
                    transaction,
                    timestamp: timestamp as u64,
                    height,
                    block_hash,
                    net_amount,
                    fee,
                    label,
                    is_ours,
                },
            ))
        })?;

        for row in tx_rows {
            let (seed_hash, transaction) = row?;
            if let Some(wallet) = wallets_map.get_mut(&seed_hash) {
                wallet.transactions.push(transaction);
            }
        }

        tracing::trace!(
            network = network_str,
            "step 8: retrieve identities for wallets"
        );
        let mut identity_stmt = conn.prepare(
            "SELECT data, wallet, wallet_index FROM identity WHERE network = ? AND wallet IS NOT NULL AND wallet_index IS NOT NULL",
        )?;

        let identity_rows = identity_stmt.query_map([network_str.clone()], |row| {
            let data: Vec<u8> = row.get(0)?;
            let wallet_seed_hash: Vec<u8> = row.get(1)?;
            let wallet_index: u32 = row.get(2)?;

            let wallet_seed_hash_array: [u8; 32] = wallet_seed_hash
                .try_into()
                .expect("Seed hash should be 32 bytes");

            Ok((data, wallet_seed_hash_array, wallet_index))
        })?;
        // Process the identities and add them to the corresponding wallets.
        for row in identity_rows {
            let (identity_data, wallet_seed_hash_array, wallet_index) = row?;

            if let Some(wallet) = wallets_map.get_mut(&wallet_seed_hash_array) {
                let mut identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&identity_data);
                identity.wallet_index = Some(wallet_index);
                identity.network = *network;

                tracing::trace!(
                    wallet_seed = hex::encode(wallet_seed_hash_array),
                    wallet_alias = ?wallet.alias,
                    identity = ?identity.identity.id().to_string(Encoding::Base58),
                    identity_alias = ?identity.alias,
                    wallet_index = wallet_index,
                    "adding identity to wallet"
                );
                // Insert the identity into the wallet's identities HashMap with wallet_index as the key
                wallet.identities.insert(wallet_index, identity.identity);
            }
        }

        tracing::trace!(
            network = network_str,
            "step 9: retrieve platform address info for wallets"
        );
        // Load platform address info for each wallet (using existing connection to avoid deadlock)
        let mut platform_stmt = conn.prepare(
            "SELECT seed_hash, address, balance, nonce FROM platform_address_balances WHERE network = ?",
        )?;
        let platform_rows = platform_stmt.query_map([network_str.clone()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let address_str: String = row.get(1)?;
            let balance: i64 = row.get(2)?;
            let nonce: i64 = row.get(3)?;
            let seed_hash_array: [u8; 32] =
                seed_hash.try_into().expect("Seed hash should be 32 bytes");
            Ok((seed_hash_array, address_str, balance as u64, nonce as u32))
        })?;

        for row in platform_rows {
            if let Ok((seed_hash, address_str, balance, nonce)) = row
                && let Some(wallet) = wallets_map.get_mut(&seed_hash)
                && let Ok(address) = Address::<NetworkUnchecked>::from_str(&address_str)
            {
                let address = address.assume_checked();
                wallet.platform_address_info.insert(
                    address,
                    crate::model::wallet::PlatformAddressInfo { balance, nonce },
                );
            }
        }

        // Convert the BTreeMap into a Vec of Wallets.
        Ok(wallets_map.into_values().collect())
    }

    /// Store or update Platform address balance and nonce
    pub fn set_platform_address_info(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        balance: u64,
        nonce: u32,
        network: &Network,
    ) -> rusqlite::Result<()> {
        let network_str = network.to_string();
        let address_str = address.to_string();
        let updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.execute(
            "INSERT OR REPLACE INTO platform_address_balances
             (seed_hash, address, balance, nonce, network, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                seed_hash,
                address_str,
                balance as i64,
                nonce as i64,
                network_str,
                updated_at
            ],
        )?;
        Ok(())
    }

    /// Get Platform address balance and nonce for a specific address
    pub fn get_platform_address_info(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        network: &Network,
    ) -> rusqlite::Result<Option<(u64, u32)>> {
        let conn = self.conn.lock().unwrap();
        let network_str = network.to_string();
        let address_str = address.to_string();

        let mut stmt = conn.prepare(
            "SELECT balance, nonce FROM platform_address_balances
             WHERE seed_hash = ? AND address = ? AND network = ?",
        )?;

        let result = stmt.query_row(params![seed_hash, address_str, network_str], |row| {
            let balance: i64 = row.get(0)?;
            let nonce: i64 = row.get(1)?;
            Ok((balance as u64, nonce as u32))
        });

        match result {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Get all Platform address balances for a wallet
    pub fn get_all_platform_address_info(
        &self,
        seed_hash: &[u8; 32],
        network: &Network,
    ) -> rusqlite::Result<Vec<(Address, u64, u32)>> {
        let conn = self.conn.lock().unwrap();
        let network_str = network.to_string();

        let mut stmt = conn.prepare(
            "SELECT address, balance, nonce FROM platform_address_balances
             WHERE seed_hash = ? AND network = ?",
        )?;

        let rows = stmt.query_map(params![seed_hash, network_str], |row| {
            let address_str: String = row.get(0)?;
            let balance: i64 = row.get(1)?;
            let nonce: i64 = row.get(2)?;
            Ok((address_str, balance as u64, nonce as u32))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (address_str, balance, nonce) = row?;
            if let Ok(address) = Address::<NetworkUnchecked>::from_str(&address_str) {
                let address = address.assume_checked();
                results.push((address, balance, nonce));
            }
        }

        Ok(results)
    }

    /// Delete Platform address balances for a wallet (used when removing wallet)
    pub fn delete_platform_address_info(
        &self,
        seed_hash: &[u8; 32],
        network: &Network,
    ) -> rusqlite::Result<()> {
        let network_str = network.to_string();
        self.execute(
            "DELETE FROM platform_address_balances WHERE seed_hash = ? AND network = ?",
            params![seed_hash, network_str],
        )?;
        Ok(())
    }

    /// Clear ALL Platform address balances for a network (developer tool)
    pub fn clear_all_platform_address_info(&self, network: &Network) -> rusqlite::Result<usize> {
        let network_str = network.to_string();
        self.execute(
            "DELETE FROM platform_address_balances WHERE network = ?",
            params![network_str],
        )
    }

    /// Clear ALL Platform addresses entirely for a network (developer tool)
    /// This removes both the addresses from wallet_addresses and their balances from platform_address_balances
    pub fn clear_all_platform_addresses(&self, network: &Network) -> rusqlite::Result<usize> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();

        // Delete from platform_address_balances
        conn.execute(
            "DELETE FROM platform_address_balances WHERE network = ?",
            params![network_str],
        )?;

        // Delete platform addresses from wallet_addresses (path_reference = 16 is PlatformPayment)
        // We need to join with wallet table to filter by network
        let deleted = conn.execute(
            "DELETE FROM wallet_addresses
             WHERE path_reference = 16
             AND seed_hash IN (SELECT seed_hash FROM wallet WHERE network = ?)",
            params![network_str],
        )?;

        Ok(deleted)
    }
}

/// Ensure the address is valid for the given network and
/// update its network if necessary.
///
/// Consumes the address and returns a new Address with the correct network.
fn check_address_for_network(
    address_unchecked: Address<NetworkUnchecked>,
    network: &Network,
) -> Result<Address<NetworkChecked>, WalletError> {
    let address_checked = address_unchecked
        .require_network(*network)
        .inspect_err(|e| {
            tracing::error!("address is not valid for the network: {}", e);
        })?;

    // For devnet/regtest addresses, require_network() accepts testnet addresses; we need to overwrite it here in case there is
    // a mismatch to match the network we are using.
    //
    // See also logic in [`Address::is_valid_for_network()`].
    match address_checked.network() {
        // When the address is correct, do nothing
        address_network if network == address_network => Ok(address_checked),
        // For devnet/regtest addresses, address type can default to testnet, require_network() accepts this;
        //  we need to overwrite it with correct network.
        Network::Testnet if network == &Network::Devnet || network == &Network::Regtest => {
            Ok(Address::new(*network, address_checked.payload().clone()))
        }
        // other cases, like mainnet or testnet, return an error on mismatch
        address_network => {
            tracing::error!(address = ?address_checked,
            network = address_network.to_string(),
            required_network = network.to_string(),
            "address has invalid network set");

            Err(WalletError::AddressError(
                dashcore::address::Error::NetworkValidation {
                    required: *network,
                    found: *address_checked.network(),
                    address: address_checked.as_unchecked().clone(),
                },
            ))
        }
    }
}

#[derive(thiserror::Error, Debug)]
/// Error type for wallet operations.
pub enum WalletError {
    #[error("Error in address: {0}")]
    AddressError(#[from] dashcore::address::Error),
}

impl From<WalletError> for rusqlite::Error {
    fn from(err: WalletError) -> Self {
        rusqlite::Error::UserFunctionError(Box::new(err))
    }
}
