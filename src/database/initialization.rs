use crate::database::Database;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;

pub const DEFAULT_DB_VERSION: u16 = 4;

pub const DEFAULT_NETWORK: &str = "dash";

impl Database {
    pub fn initialize(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // Check if this is the first time setup by looking for entries in the settings table.
        if self.is_first_time_setup()? {
            self.create_tables()?;
            self.set_default_version()?;
        } else {
            // If outdated, back up and either migrate or recreate the database.
            if let Some(current_version) = self.is_outdated()? {
                self.backup_db(db_file_path)?;
                if let Err(e) = self.try_perform_migration(current_version, DEFAULT_DB_VERSION) {
                    // The migration failed
                    println!("Migration failed: {:?}", e);
                    self.recreate_db(db_file_path)?;
                    self.create_tables()?;
                    self.set_default_version()?;
                    println!("Database reinitialized with default settings.");
                }
            }
        }

        Ok(())
    }

    fn apply_version_changes(&self, version: u16) -> rusqlite::Result<()> {
        match version {
            4 => {
                self.initialize_top_up_table()?;
            }
            3 => {
                self.add_custom_dash_qt_columns()?;
            }
            2 => {
                self.initialize_proof_log_table()?;
            }
            _ => {}
        }

        Ok(())
    }
    fn try_perform_migration(
        &self,
        original_version: u16,
        to_version: u16,
    ) -> rusqlite::Result<()> {
        for version in (original_version + 1)..=to_version {
            self.apply_version_changes(version)?;
            self.update_database_version(version)?;
        }
        Ok(())
    }

    /// Checks if the `settings` table is empty or missing, indicating a first-time setup.
    fn is_first_time_setup(&self) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();

        // Check if the `settings` table exists by querying `sqlite_master`
        let table_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')",
            [],
            |row| row.get(0),
        )?;

        if !table_exists {
            // If the `settings` table does not exist, this is a first-time setup
            Ok(true)
        } else {
            // If the table exists, check if it has any entries
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))?;
            Ok(count == 0)
        }
    }

    /// Checks if the version in the current database settings is below DEFAULT_DB_VERSION.
    /// If outdated, returns the version in the current database settings
    fn is_outdated(&self) -> rusqlite::Result<Option<u16>> {
        let conn = self.conn.lock().unwrap();
        let version: u16 = conn
            .query_row(
                "SELECT database_version FROM settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0); // Default to version 0 if there's no version set
        if version < DEFAULT_DB_VERSION {
            Ok(Some(version))
        } else {
            Ok(None)
        }
    }

    /// Backs up the existing database with a unique timestamped filename in backups directory.
    fn backup_db(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        if db_file_path.exists() {
            // Create a "backups" folder in the same directory as `data.db` if not exists
            let backups_dir = db_file_path
                .parent()
                .expect("Expected parent directory in creating db backup folder")
                .join("backups");
            fs::create_dir_all(&backups_dir).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(
                    format!("Failed to create db backups directory: {}", e).into(),
                )
            })?;

            // Generate a unique filename with a timestamp for the backup
            let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
            let backup_filename = format!("data_backup_{}.db", timestamp);
            let backup_path = backups_dir.join(backup_filename);

            // Copy `data.db` to the unique backup file
            fs::copy(db_file_path, &backup_path)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            println!("Old database backed up to {:?}", backup_path);
        }

        Ok(())
    }

    /// Recreates `data.db`, and refreshes the connection.
    fn recreate_db(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // Remove the existing database file if it exists
        if db_file_path.exists() {
            fs::remove_file(db_file_path).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        }
        // Create a new empty `data.db` file and set up the initial schema
        let new_conn = Connection::open(db_file_path)?;

        // Initialize the `settings` table in the new database
        new_conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            password_check BLOB,
            main_password_salt BLOB,
            main_password_nonce BLOB,
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            database_version INTEGER NOT NULL
        )",
            [],
        )?;

        // Insert default settings for the new database
        new_conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
         VALUES (1, ?, 0, ?)",
            params![DEFAULT_NETWORK, DEFAULT_DB_VERSION],
        )?;

        // Update the connection in `self.conn` to use the new `data.db` file
        let mut conn_lock = self.conn.lock().unwrap();
        *conn_lock = new_conn;

        Ok(())
    }

    /// Creates all required tables with indexes if they don't already exist.
    fn create_tables(&self) -> rusqlite::Result<()> {
        // Create the settings table
        self.execute(
            "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            password_check BLOB,
            main_password_salt BLOB,
            main_password_nonce BLOB,
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            custom_dash_qt_path TEXT,
            overwrite_dash_conf INTEGER,
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

        // Create indexes for wallet addresses table
        self.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference ON wallet_addresses (path_reference)", [])?;
        self.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type ON wallet_addresses (path_type)", [])?;

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

        // Create indexes for utxos table
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_address ON utxos (address)",
            [],
        )?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_network ON utxos (network)",
            [],
        )?;

        // Create asset lock transaction table
        self.execute(
            "CREATE TABLE IF NOT EXISTS asset_lock_transaction (
                        tx_id BLOB PRIMARY KEY,
                        transaction_data BLOB NOT NULL,
                        amount INTEGER,
                        instant_lock_data BLOB,
                        chain_locked_height INTEGER,
                        identity_id BLOB,
                        identity_id_potentially_in_creation BLOB,
                        wallet BLOB NOT NULL,
                        network TEXT NOT NULL,
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

        // Create indexes for the contracts table
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_name_network ON contract (name, network)",
            [],
        )?;

        self.initialize_proof_log_table()?;

        self.initialize_top_up_table()?;

        Ok(())
    }

    /// Ensures that the default database version is set in the settings table.
    fn set_default_version(&self) -> rusqlite::Result<()> {
        self.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, ?, 0, ?)
             ON CONFLICT(id) DO UPDATE SET database_version = excluded.database_version",
            params![DEFAULT_NETWORK, DEFAULT_DB_VERSION],
        )?;
        Ok(())
    }
}
