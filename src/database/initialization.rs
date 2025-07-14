use crate::database::Database;
use chrono::Utc;
use rusqlite::{Connection, params};
use std::fs;
use std::path::Path;

pub const DEFAULT_DB_VERSION: u16 = 11;

pub const DEFAULT_NETWORK: &str = "dash";

impl Database {
    pub fn initialize(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // Check if this is the first time setup by looking for entries in the settings table.
        if self.is_first_time_setup()? {
            self.create_tables()?;
            self.set_default_version()?;
        } else {
            // If outdated, back up and either migrate or recreate the database.
            let current_version = self.db_schema_version()?;
            if current_version != DEFAULT_DB_VERSION {
                self.backup_db(db_file_path)?;
                if let Err(e) = self.try_perform_migration(current_version, DEFAULT_DB_VERSION) {
                    let version_after_migration = self.db_schema_version()?;
                    panic!(
                        "Database migration from version {} to {} failed, database is at version {}. Error: {:?}",
                        current_version, DEFAULT_DB_VERSION, version_after_migration, e
                    );
                }
            }
        }

        Ok(())
    }

    fn apply_version_changes(&self, version: u16, tx: &Connection) -> rusqlite::Result<()> {
        match version {
            11 => self.rename_identity_column_is_in_creation_to_status(tx)?,
            10 => {
                self.add_theme_preference_column(tx)?;
            }
            9 => {
                self.delete_all_identities_in_all_devnets_and_regtest(tx)?;
                self.delete_all_local_tokens_in_all_devnets_and_regtest(tx)?;
                self.remove_all_asset_locks_identity_id_for_all_devnets_and_regtest(tx)?;
                self.remove_all_contracts_in_all_devnets_and_regtest(tx)?;
                self.fix_identity_devnet_network_name(tx)?;
            }
            8 => {
                self.change_contract_name_to_alias(tx)?;
            }
            7 => {
                self.migrate_asset_lock_fk_to_set_null(tx)?;
            }
            6 => {
                self.update_scheduled_votes_table(tx)?;
                self.initialize_token_table(tx)?;
                self.drop_identity_token_balances_table(tx)?;
                self.initialize_identity_token_balances_table(tx)?;
                self.initialize_identity_order_table(tx)?;
                self.initialize_token_order_table(tx)?;
            }
            5 => {
                self.initialize_scheduled_votes_table(tx)?;
            }
            4 => {
                self.initialize_top_up_table(tx)?;
            }
            3 => {
                self.add_custom_dash_qt_columns(tx)?;
            }
            2 => {
                self.initialize_proof_log_table(tx)?;
            }
            _ => {
                tracing::warn!("No database changes for version {}", version);
            }
        }

        Ok(())
    }
    /// Migrates the database from the original version to the target version.
    ///
    /// This function performs the necessary migrations by applying changes for each version
    /// from the original version up to the target version.
    ///
    /// It uses a transaction to ensure that system integrity is maintained during the migration process.
    /// If any migration step fails, the transaction will be rolled back, and the user can safely
    /// downgrade his app to the previous version.
    ///
    /// ## Returns
    ///
    /// `rusqlite::Result<()>` - Returns `Ok(true)` if the migration was successful, Ok(false) if no migration needed,
    /// or an error if it failed.
    fn try_perform_migration(
        &self,
        original_version: u16,
        to_version: u16,
    ) -> Result<bool, String> {
        match original_version.cmp(&to_version) {
            std::cmp::Ordering::Equal => {
                tracing::trace!(
                    "No database migration needed, already at version {}",
                    to_version
                );
                Ok(false)
            }
            std::cmp::Ordering::Greater => Err(format!(
                "Database schema version {} is too new, max supported version: {}. Please update dash-evo-tool.",
                original_version, to_version
            )),
            std::cmp::Ordering::Less => {
                let mut conn = self
                    .conn
                    .lock()
                    .expect("Failed to lock database connection");

                for version in (original_version + 1)..=to_version {
                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    self.apply_version_changes(version, &tx)
                        .map_err(|e| e.to_string())?;
                    self.update_database_version(version, &tx)
                        .map_err(|e| e.to_string())?;
                    tx.commit().map_err(|e| e.to_string())?;
                }
                Ok(true)
            }
        }
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

    /// Checks version of the database.
    ///
    /// Returns the current version as `Ok(Some(version))`.
    ///
    /// Note it returns Ok(Some(version)) even is the current database is above the default version.
    /// This is to allow the app to detect when database version is too high and to prevent
    /// the app from running with an unsupported database version.
    fn db_schema_version(&self) -> rusqlite::Result<u16> {
        let conn = self.conn.lock().unwrap();
        let result: rusqlite::Result<u16> = conn.query_row(
            "SELECT database_version FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        );

        match result {
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                tracing::debug!("No database version found, returning default version 0");
                Ok(0)
            }
            x => x,
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

    /// Creates all required tables with indexes if they don't already exist.
    fn create_tables(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        // Create the settings table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            password_check BLOB,
            main_password_salt BLOB,
            main_password_nonce BLOB,
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            custom_dash_qt_path TEXT,
            overwrite_dash_conf INTEGER,
            theme_preference TEXT DEFAULT 'System',
            database_version INTEGER NOT NULL
        )",
            [],
        )?;

        // Create the wallet table
        conn.execute(
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
        conn.execute(
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
        conn.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference ON wallet_addresses (path_reference)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type ON wallet_addresses (path_type)", [])?;

        // Create the utxos table
        conn.execute(
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
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_address ON utxos (address)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_network ON utxos (network)",
            [],
        )?;

        // Create asset lock transaction table
        conn.execute(
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
                        FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE SET NULL,
                        FOREIGN KEY (identity_id_potentially_in_creation) REFERENCES identity(id) ON DELETE SET NULL,
                        FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                    )",
            [],
        )?;

        // Create the identities table
        conn.execute(
                    "CREATE TABLE IF NOT EXISTS identity (
                        id BLOB PRIMARY KEY,
                        data BLOB,
                        status INTEGER NOT NULL DEFAULT 0,
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
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_identity_local_network_type
             ON identity (is_local, network, identity_type)",
            [],
        )?;

        // Create the contested names table
        conn.execute(
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
        conn.execute(
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
        conn.execute(
            "CREATE TABLE IF NOT EXISTS contract (
                        contract_id BLOB,
                        contract BLOB,
                        alias TEXT,
                        network TEXT NOT NULL,
                        PRIMARY KEY (contract_id, network)
                    )",
            [],
        )?;

        // Create indexes for the contracts table
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_alias_network ON contract (alias, network)",
            [],
        )?;

        self.initialize_proof_log_table(&conn)?;
        self.initialize_top_up_table(&conn)?;
        self.initialize_scheduled_votes_table(&conn)?;
        self.initialize_token_table(&conn)?;
        self.initialize_identity_order_table(&conn)?;
        self.initialize_token_order_table(&conn)?;
        self.initialize_identity_token_balances_table(&conn)?;

        Ok(())
    }

    /// Ensures that the default database version is set in the settings table.
    fn set_default_version(&self) -> rusqlite::Result<()> {
        // TODO: Discuss migration approach with the team.
        // Suggested approach:
        // we don't change `create_tables`, we just add migrations
        // and rely on it to bring the database to the latest version.
        // It means that we put `1` in the `settings` table as the initial version
        self.set_db_version(DEFAULT_DB_VERSION)
    }
    fn set_db_version(&self, version: u16) -> rusqlite::Result<()> {
        self.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, ?, 0, ?)
             ON CONFLICT(id) DO UPDATE SET database_version = excluded.database_version",
            params![DEFAULT_NETWORK, version],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::database::initialization::DEFAULT_DB_VERSION;
    use rusqlite::params;

    #[test]
    /// Given a new database file,
    /// when `initialize` is called,
    /// then it should create the settings table with the default version.
    fn test_initialize_database() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("test_data.db");
        let db = super::Database::new(&db_file_path).unwrap();
        db.initialize(&db_file_path).unwrap();

        // Check if the settings table is created and has the default version
        let conn = db.conn.lock().unwrap();
        let version: u16 = conn
            .query_row(
                "SELECT database_version FROM settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, super::DEFAULT_DB_VERSION);
    }

    // Given a database with a missing `asset_lock_transaction` table,
    // when I run the migration number 9,
    // then it fails and reverts the database schema to the previous version,
    #[test]
    fn test_migration_failure_rolls_back() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("test_data.db");
        let db = super::Database::new(&db_file_path).unwrap();

        // Identities from regtest are deleted during migration 9
        const NETWORK: &str = "regtest";

        db.create_tables().unwrap();
        db.set_default_version().unwrap();

        // drop the `asset_lock_transaction` table to simulate a migration failure
        let conn = db.conn.lock().unwrap();
        conn.execute("DROP TABLE asset_lock_transaction", [])
            .expect("Failed to drop asset_lock_transaction table");
        // check that we don't have any identities yet
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM identity", [], |row| row.get(0))
            .expect("Failed to count identities");
        assert_eq!(count, 0);

        // add some identity to ensure the database is not empty
        conn.execute(
            "INSERT INTO identity (id, is_local, alias, network) VALUES (?, ?, ?, ?)",
            rusqlite::params![vec![1u8; 32], 1, "test_identity", NETWORK],
        )
        .expect("Failed to insert test identity");
        drop(conn);

        // change version to 8 to force migration number 9
        const START_VERSION: u16 = 8;
        db.set_db_version(START_VERSION).unwrap();

        // Simulate a migration failure by trying to apply an invalid change
        let result = db.try_perform_migration(START_VERSION, DEFAULT_DB_VERSION);
        assert!(result.is_err());
        println!("Migration failed as expected: {}", result.unwrap_err());

        // Check that the database version has not changed
        let version: u16 = db.db_schema_version().unwrap();
        assert_eq!(version, START_VERSION);

        // check that the identity was not deleted
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM identity WHERE network = ?",
                params![NETWORK],
                |row| row.get(0),
            )
            .expect("Failed to count identities");
        assert_eq!(
            count, 1,
            "Identity should not be deleted during migration failure"
        );
    }
}
