use std::{fs, path::Path};

use rusqlite::{params, Connection};

use crate::database::Database;

pub const MIN_SUPPORTED_DB_VERSION: u16 = 1;
pub const DB_BACKUP_SUFFIX: &str = ".backup";

impl Database {
    pub fn initialize(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // Check if the current database version meets the minimum requirements
        if self.is_outdated()? {
            self.backup_and_recreate_db(db_file_path)?;
        }

        // Create the settings table
        self.execute(
            "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            database_version INTEGER NOT NULL
        )",
            [],
        )?;

        // Create the wallet table
        self.execute(
            "CREATE TABLE IF NOT EXISTS wallet (
        seed_hash BLOB NOT NULL PRIMARY KEY,
        encrypted_seed BLOB NOT NULL,
        salt BLOB NOT NULL,
        nonce BLOB NOT NULL,
        master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
        alias TEXT,
        is_main INTEGER,
        uses_password INTEGER NOT NULL,
        password_hint TEXT,
        network TEXT NOT NULL
    )",
            [],
        )?;

        // Create wallet addresses
        self.execute(
            "CREATE TABLE IF NOT EXISTS wallet_addresses (
        seed_hash BLOB NOT NULL,
        address TEXT NOT NULL,
        derivation_path TEXT NOT NULL,
        balance INTEGER,
        path_reference INTEGER NOT NULL,
        path_type INTEGER NOT NULL,
        PRIMARY KEY (seed_hash, address),
        FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
    )",
            [],
        )?;

        // Create the utxos table
        self.execute(
            "CREATE TABLE IF NOT EXISTS utxos (
                txid BLOB NOT NULL,
                vout INTEGER NOT NULL,
                address TEXT NOT NULL,
                value INTEGER NOT NULL,
                script_pubkey BLOB NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY (txid, vout, network)
            );",
            [],
        )?;

        self.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference ON wallet_addresses (path_reference)", [])?;
        self.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type ON wallet_addresses (path_type)", [])?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_address ON utxos (address)",
            [],
        )?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_network ON utxos (network)",
            [],
        )?;

        self.execute(
            "CREATE TABLE IF NOT EXISTS asset_lock_transaction (
                tx_id TEXT PRIMARY KEY,
                transaction_data BLOB NOT NULL,
                amount INTEGER,
                instant_lock_data BLOB,
                chain_locked_height INTEGER,
                identity_id BLOB,
                identity_id_potentially_in_creation BLOB,
                wallet BLOB NOT NULL,
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE,
                FOREIGN KEY (identity_id_potentially_in_creation) REFERENCES identity(id),
                FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        // Create the identities table
        self.execute(
            "CREATE TABLE IF NOT EXISTS identity (
                id BLOB PRIMARY KEY,
                data BLOB,
                is_in_creation INTEGER NOT NULL DEFAULT 0,
                is_local INTEGER NOT NULL,
                alias TEXT,
                info TEXT,
                wallet BLOB,
                wallet_index INTEGER,
                identity_type TEXT,
                network TEXT NOT NULL,
                CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL) OR (wallet IS NULL AND wallet_index IS NULL)),
                FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        // Create the composite index for faster querying
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_identity_local_network_type
     ON identity (is_local, network, identity_type)",
            [],
        )?;

        // Create the contested names table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contested_name (
                normalized_contested_name TEXT NOT NULL,
                locked_votes INTEGER,
                abstain_votes INTEGER,
                awarded_to BLOB,
                end_time INTEGER,
                locked INTEGER NOT NULL DEFAULT 0,
                last_updated INTEGER,
                network TEXT NOT NULL,
                PRIMARY KEY (normalized_contested_name, network)
            )",
            [],
        )?;

        // Create the contestants table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contestant (
                normalized_contested_name TEXT NOT NULL,
                identity_id BLOB NOT NULL,
                name TEXT,
                votes INTEGER,
                created_at INTEGER,
                created_at_block_height INTEGER,
                created_at_core_block_height INTEGER,
                document_id BLOB,
                network TEXT NOT NULL,
                PRIMARY KEY (normalized_contested_name, identity_id, network),
                FOREIGN KEY (normalized_contested_name, network) REFERENCES contested_name(normalized_contested_name, network) ON DELETE CASCADE
            )",
            [],
        )?;

        // Create the contracts table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contract (
                contract_id BLOB,
                contract BLOB,
                name TEXT,
                network TEXT NOT NULL,
                PRIMARY KEY (contract_id, network)
            )",
            [],
        )?;

        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_name_network ON contract (name, network)",
            [],
        )?;

        // Ensure the database version is set
        self.set_default_version()?;

        Ok(())
    }

    fn is_outdated(&self) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let version: u16 = conn
            .query_row(
                "SELECT database_version FROM settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0); // Default to version 0 if there's no version set
        Ok(version < MIN_SUPPORTED_DB_VERSION)
    }

    fn backup_and_recreate_db(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // Close any active connection to allow file operations
        let _ = self; // Ensure the current connection is closed

        if db_file_path.exists() {
            // Create a backup file path by appending the backup suffix
            let backup_path = db_file_path.with_extension(DB_BACKUP_SUFFIX);
            fs::rename(db_file_path, &backup_path)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            println!("Old database backed up to {:?}", backup_path);
        }

        // Recreate a new database file and initialize the schema
        let new_conn = Connection::open(db_file_path)?;
        new_conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                network TEXT NOT NULL,
                start_root_screen INTEGER NOT NULL,
                database_version INTEGER NOT NULL
            )",
            [],
        )?;

        // Set the version for the new database
        new_conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, 'default_network', 0, ?)",
            params![MIN_SUPPORTED_DB_VERSION],
        )?;
        Ok(())
    }

    fn set_default_version(&self) -> rusqlite::Result<()> {
        self.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, 'default_network', 0, ?)
             ON CONFLICT(id) DO UPDATE SET database_version = excluded.database_version",
            params![MIN_SUPPORTED_DB_VERSION],
        )?;
        Ok(())
    }
}
