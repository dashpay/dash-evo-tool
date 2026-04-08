use crate::database::Database;
use chrono::Utc;
use rusqlite::{Connection, params};
use std::fs;
use std::path::Path;

pub const DEFAULT_DB_VERSION: u16 = 33;

pub const DEFAULT_NETWORK: &str = "mainnet";

impl Database {
    pub fn initialize(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // First, ensure all required columns exist in tables that may have been
        // created with an older schema. This must happen before any queries that
        // depend on these columns (like db_schema_version which needs database_version).
        {
            let conn = self.conn.lock().unwrap();
            // Check if settings table exists before trying to ensure columns
            let settings_exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='settings'",
                [],
                |row| row.get::<_, i32>(0).map(|count| count > 0),
            )?;
            if settings_exists {
                self.ensure_settings_columns_exist(&conn)?;
            }
            self.ensure_wallet_columns_exist(&conn)?;
        }

        // Check if this is the first time setup by looking for entries in the settings table.
        if self.is_first_time_setup()? {
            self.create_tables()?;
            self.set_default_version()?;
        } else {
            self.run_consistency_checks();

            let current_version = self.db_schema_version()?;
            if current_version != DEFAULT_DB_VERSION {
                self.backup_db(db_file_path)?;
                if let Err(e) = self.try_perform_migration(current_version, DEFAULT_DB_VERSION) {
                    let version_after_migration = self.db_schema_version().unwrap_or(0);
                    return Err(rusqlite::Error::InvalidParameterName(format!(
                        "Database migration from version {} to {} failed (database is at version {}): {}",
                        current_version, DEFAULT_DB_VERSION, version_after_migration, e
                    )));
                }
            }
        }

        Ok(())
    }

    fn apply_version_changes(&self, version: u16, tx: &Connection) -> rusqlite::Result<()> {
        match version {
            // Versions 28-32 were consolidated into v33 to resolve migration
            // numbering conflicts between the zk and v1.0-dev branches.
            // If migrating from < 28, these are no-ops that just bump the version.
            28..=32 => {}
            33 => {
                // Consolidated migration: all changes from v28-v32 in one step.
                // Every sub-migration is idempotent (IF NOT EXISTS / column checks),
                // so this is safe to run on any DB that already applied some or all
                // of the individual steps.
                self.clean_orphaned_fk_rows(tx)?;
                self.add_core_wallet_name_column(tx)?;
                self.init_contacts_tables(tx)?;
                self.create_shielded_tables(tx)?;
                self.create_shielded_wallet_meta_table(tx)?;
                self.add_nullifier_sync_timestamp_column(tx)?;
                self.rename_network_dash_to_mainnet(tx)?;
                self.add_wallet_transaction_status_column(tx)?;
            }
            27 => {
                self.add_network_indexes(tx)?;
            }
            26 => {
                self.add_last_full_sync_balance_column(tx)?;
            }
            25 => {
                self.add_avatar_bytes_column(tx)?;
            }
            24 => {
                self.add_selected_wallet_columns(tx)?;
            }
            23 => {
                self.add_last_terminal_block_column(tx)?;
            }
            22 => {
                self.add_network_column_to_dashpay_contact_requests(tx)?;
                self.add_network_column_to_dashpay_contacts(tx)?;
            }
            21 => {
                self.add_network_column_to_dashpay_profiles(tx)?;
            }
            20 => {
                self.add_platform_sync_columns(tx)?;
            }
            19 => {
                self.initialize_platform_address_balances_table(tx)?;
            }
            18 => {
                self.initialize_single_key_wallet_table(tx)?;
            }
            17 => {
                self.add_address_total_received_column(tx)?;
            }
            16 => {
                self.add_wallet_balance_columns(tx)?;
            }
            15 => {
                self.add_core_backend_mode_column(tx)?;
            }
            14 => {
                self.initialize_wallet_transactions_table(tx)?;
            }
            13 => {
                // Add DashPay tables in version 12
                self.init_dashpay_tables_in_tx(tx)?;
            }
            12 => self.add_disable_zmq_column(tx)?,
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
                tx.execute("DROP TABLE IF EXISTS identity_order", [])?;
                self.initialize_identity_order_table(tx)?;
                tx.execute("DROP TABLE IF EXISTS token_order", [])?;
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
            tracing::info!("Old database backed up to {:?}", backup_path);
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
            disable_zmq INTEGER DEFAULT 0,
            theme_preference TEXT DEFAULT 'System',
            core_backend_mode INTEGER DEFAULT 1,
            database_version INTEGER NOT NULL,
            onboarding_completed INTEGER DEFAULT 0,
            show_evonode_tools INTEGER DEFAULT 0,
            user_mode TEXT DEFAULT 'Advanced',
            use_local_spv_node INTEGER DEFAULT 0,
            auto_start_spv INTEGER DEFAULT 0,
            close_dash_qt_on_exit INTEGER DEFAULT 1,
            selected_wallet_hash BLOB,
            selected_single_key_hash BLOB
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
                network TEXT NOT NULL,
                confirmed_balance INTEGER DEFAULT 0,
                unconfirmed_balance INTEGER DEFAULT 0,
                total_balance INTEGER DEFAULT 0,
                last_platform_full_sync INTEGER DEFAULT 0,
                last_platform_sync_checkpoint INTEGER DEFAULT 0,
                last_terminal_block INTEGER DEFAULT 0,
                core_wallet_name TEXT DEFAULT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_wallet_network ON wallet (network)",
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
                total_received INTEGER DEFAULT 0,
                PRIMARY KEY (seed_hash, address),
                FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        // Create indexes for wallet addresses table
        conn.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference ON wallet_addresses (path_reference)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type ON wallet_addresses (path_type)", [])?;

        // Create Platform address balances table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS platform_address_balances (
                seed_hash BLOB NOT NULL,
                address TEXT NOT NULL,
                balance INTEGER NOT NULL DEFAULT 0,
                nonce INTEGER NOT NULL DEFAULT 0,
                network TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT 0,
                last_full_sync_balance INTEGER DEFAULT NULL,
                PRIMARY KEY (seed_hash, address, network),
                FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

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

        // Create wallet transactions table for SPV history
        self.initialize_wallet_transactions_table(&conn)?;

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

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_asset_lock_transaction_network ON asset_lock_transaction (network)",
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

        // Initialize contacts and DashPay tables while holding the same connection lock
        self.init_contacts_tables(&conn)?;
        self.init_dashpay_tables_in_tx(&conn)?;

        // Initialize single key wallet table
        self.initialize_single_key_wallet_table(&conn)?;

        // Initialize shielded pool tables
        self.create_shielded_tables(&conn)?;
        self.create_shielded_wallet_meta_table(&conn)?;

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
        // Default start_root_screen to 20 (RootScreenDashPayProfile)
        self.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, ?, 20, ?)
             ON CONFLICT(id) DO UPDATE SET database_version = excluded.database_version",
            params![DEFAULT_NETWORK, version],
        )?;
        Ok(())
    }

    /// Migration: Create platform_address_balances table (version 19).
    fn initialize_platform_address_balances_table(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS platform_address_balances (
                seed_hash BLOB NOT NULL,
                address TEXT NOT NULL,
                balance INTEGER NOT NULL DEFAULT 0,
                nonce INTEGER NOT NULL DEFAULT 0,
                network TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (seed_hash, address, network),
                FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    /// Migration: Add platform sync columns to wallet table (version 20).
    /// - last_platform_full_sync: Unix timestamp of last full platform address sync
    /// - last_platform_sync_checkpoint: Block height checkpoint from last full sync
    fn add_platform_sync_columns(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "ALTER TABLE wallet ADD COLUMN last_platform_full_sync INTEGER DEFAULT 0",
            [],
        )?;
        conn.execute(
            "ALTER TABLE wallet ADD COLUMN last_platform_sync_checkpoint INTEGER DEFAULT 0",
            [],
        )?;
        Ok(())
    }

    /// Migration: Add last_terminal_block column to wallet table (version 23).
    /// Tracks the highest block height processed by terminal balance updates to avoid
    /// re-applying the same balance changes on subsequent terminal-only syncs.
    fn add_last_terminal_block_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "ALTER TABLE wallet ADD COLUMN last_terminal_block INTEGER DEFAULT 0",
            [],
        )?;
        Ok(())
    }

    /// Migration: Add selected wallet hash columns to settings table (version 24).
    /// Persists the user's selected wallet across app restarts.
    fn add_selected_wallet_columns(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if selected_wallet_hash column exists
        let wallet_hash_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='selected_wallet_hash'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !wallet_hash_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN selected_wallet_hash BLOB DEFAULT NULL",
                [],
            )?;
        }

        // Check if selected_single_key_hash column exists
        let single_key_hash_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='selected_single_key_hash'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !single_key_hash_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN selected_single_key_hash BLOB DEFAULT NULL",
                [],
            )?;
        }

        Ok(())
    }

    fn add_network_column_to_dashpay_profiles(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if dashpay_profiles table exists
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dashpay_profiles'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if table_exists {
            // Check if network column already exists
            let has_network_column: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('dashpay_profiles') WHERE name='network'",
                    [],
                    |row| row.get::<_, i32>(0).map(|count| count > 0),
                )
                .unwrap_or(false);

            if !has_network_column {
                // Add network column with default value
                conn.execute(
                    "ALTER TABLE dashpay_profiles ADD COLUMN network TEXT NOT NULL DEFAULT 'dash'",
                    [],
                )?;

                // Drop the old primary key and recreate with composite key
                // SQLite doesn't support dropping primary key, so we need to recreate the table
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS dashpay_profiles_new (
                        identity_id BLOB NOT NULL,
                        network TEXT NOT NULL,
                        display_name TEXT,
                        bio TEXT,
                        avatar_url TEXT,
                        avatar_hash BLOB,
                        avatar_fingerprint BLOB,
                        public_message TEXT,
                        created_at INTEGER DEFAULT (unixepoch()),
                        updated_at INTEGER DEFAULT (unixepoch()),
                        PRIMARY KEY (identity_id, network)
                    )",
                    [],
                )?;

                // Copy data from old table
                conn.execute(
                    "INSERT OR REPLACE INTO dashpay_profiles_new
                     SELECT identity_id, network, display_name, bio, avatar_url,
                            avatar_hash, avatar_fingerprint, public_message, created_at, updated_at
                     FROM dashpay_profiles",
                    [],
                )?;

                // Drop old table and rename new one
                conn.execute("DROP TABLE dashpay_profiles", [])?;
                conn.execute(
                    "ALTER TABLE dashpay_profiles_new RENAME TO dashpay_profiles",
                    [],
                )?;
            }
        }

        Ok(())
    }

    fn add_network_column_to_dashpay_contact_requests(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        // Check if dashpay_contact_requests table exists
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dashpay_contact_requests'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if table_exists {
            // Check if network column already exists
            let has_network_column: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('dashpay_contact_requests') WHERE name='network'",
                    [],
                    |row| row.get::<_, i32>(0).map(|count| count > 0),
                )
                .unwrap_or(false);

            if !has_network_column {
                // Add network column with default value
                conn.execute(
                    "ALTER TABLE dashpay_contact_requests ADD COLUMN network TEXT NOT NULL DEFAULT 'dash'",
                    [],
                )?;
            }
        }

        Ok(())
    }

    fn add_network_column_to_dashpay_contacts(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if dashpay_contacts table exists
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dashpay_contacts'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if table_exists {
            // Check if network column already exists
            let has_network_column: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('dashpay_contacts') WHERE name='network'",
                    [],
                    |row| row.get::<_, i32>(0).map(|count| count > 0),
                )
                .unwrap_or(false);

            if !has_network_column {
                // Add network column with default value
                conn.execute(
                    "ALTER TABLE dashpay_contacts ADD COLUMN network TEXT NOT NULL DEFAULT 'dash'",
                    [],
                )?;

                // Recreate the table with composite primary key
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS dashpay_contacts_new (
                        owner_identity_id BLOB NOT NULL,
                        contact_identity_id BLOB NOT NULL,
                        network TEXT NOT NULL,
                        username TEXT,
                        display_name TEXT,
                        avatar_url TEXT,
                        public_message TEXT,
                        contact_status TEXT DEFAULT 'pending',
                        created_at INTEGER DEFAULT (unixepoch()),
                        updated_at INTEGER DEFAULT (unixepoch()),
                        last_seen INTEGER,
                        PRIMARY KEY (owner_identity_id, contact_identity_id, network)
                    )",
                    [],
                )?;

                // Copy data from old table
                conn.execute(
                    "INSERT OR REPLACE INTO dashpay_contacts_new
                     SELECT owner_identity_id, contact_identity_id, network, username, display_name,
                            avatar_url, public_message, contact_status, created_at, updated_at, last_seen
                     FROM dashpay_contacts",
                    [],
                )?;

                // Drop old table and rename new one
                conn.execute("DROP TABLE dashpay_contacts", [])?;
                conn.execute(
                    "ALTER TABLE dashpay_contacts_new RENAME TO dashpay_contacts",
                    [],
                )?;
            }
        }

        Ok(())
    }

    /// Migration: Add avatar_bytes column to dashpay_profiles table (version 25).
    /// Stores the actual avatar image bytes to avoid re-fetching from network on every app start.
    fn add_avatar_bytes_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if dashpay_profiles table exists
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dashpay_profiles'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if table_exists {
            // Check if avatar_bytes column already exists
            let has_avatar_bytes_column: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('dashpay_profiles') WHERE name='avatar_bytes'",
                    [],
                    |row| row.get::<_, i32>(0).map(|count| count > 0),
                )
                .unwrap_or(false);

            if !has_avatar_bytes_column {
                conn.execute(
                    "ALTER TABLE dashpay_profiles ADD COLUMN avatar_bytes BLOB DEFAULT NULL",
                    [],
                )?;
            }
        }

        Ok(())
    }

    /// Migration: Add last_full_sync_balance column to platform_address_balances table (version 26).
    /// Stores the balance from the last FULL sync (checkpoint), separate from the current balance
    /// which includes terminal sync updates. This prevents double-counting AddToCredits during
    /// terminal-only syncs after app restart.
    fn add_last_full_sync_balance_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if platform_address_balances table exists
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='platform_address_balances'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if table_exists {
            // Check if last_full_sync_balance column already exists
            let has_column: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('platform_address_balances') WHERE name='last_full_sync_balance'",
                    [],
                    |row| row.get::<_, i32>(0).map(|count| count > 0),
                )
                .unwrap_or(false);

            if !has_column {
                // Add column with NULL default - existing rows will need a full sync to populate
                conn.execute(
                    "ALTER TABLE platform_address_balances ADD COLUMN last_full_sync_balance INTEGER DEFAULT NULL",
                    [],
                )?;
            }
        }

        Ok(())
    }

    /// Migration: Add core_wallet_name column to wallet and single_key_wallet tables (version 28).
    fn add_core_wallet_name_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        let wallet_has_column: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet') WHERE name='core_wallet_name'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;
        if !wallet_has_column {
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN core_wallet_name TEXT DEFAULT NULL",
                [],
            )?;
        }

        let single_key_wallet_has_column: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('single_key_wallet') WHERE name='core_wallet_name'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;
        if !single_key_wallet_has_column {
            conn.execute(
                "ALTER TABLE single_key_wallet ADD COLUMN core_wallet_name TEXT DEFAULT NULL",
                [],
            )?;
        }
        Ok(())
    }

    /// Migration: Add network indexes to high-traffic tables (version 27).
    /// These tables are frequently queried with WHERE network = ? but lacked indexes.
    fn add_network_indexes(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_wallet_network ON wallet (network)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_token_network ON token (network)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_identity_token_balances_network ON identity_token_balances (network)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_scheduled_votes_network ON scheduled_votes (network)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_asset_lock_transaction_network ON asset_lock_transaction (network)",
            [],
        )?;
        Ok(())
    }

    /// Migration 30: add `status` column to `wallet_transactions`.
    fn add_wallet_transaction_status_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        let has_status: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet_transactions') WHERE name='status'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;
        if !has_status {
            // DEFAULT 2 (Confirmed) for migration: existing transactions predate status
            // tracking and are assumed confirmed. Fresh installs use DEFAULT 0 (Unconfirmed)
            // in the CREATE TABLE (wallet.rs).
            conn.execute(
                // DEFAULT 2 (Confirmed) for migration: existing transactions predate status
                // tracking and are assumed confirmed. Fresh installs use DEFAULT 0 (Unconfirmed)
                // in the CREATE TABLE (wallet.rs).
                "ALTER TABLE wallet_transactions ADD COLUMN status INTEGER NOT NULL DEFAULT 2",
                [],
            )?;
        }
        Ok(())
    }

    // Shielded table helpers (create_shielded_tables, create_shielded_wallet_meta_table,
    // add_nullifier_sync_timestamp_column) are implemented in database/shielded.rs.

    /// Remove orphaned child rows whose parent wallet was deleted while FK
    /// enforcement was off (system SQLite before bundled build). The UPDATE in
    /// `rename_network_dash_to_mainnet` re-validates FKs and fails on these
    /// orphans. All affected data is fully recoverable from the network via
    /// resync, so deletion is safe.
    fn clean_orphaned_fk_rows(&self, conn: &Connection) -> rusqlite::Result<()> {
        // --- CASCADE children of wallet(seed_hash) ---
        // Delete orphaned rows where parent wallet no longer exists.
        let wallet_fk_delete: &[(&str, &str)] = &[
            ("wallet_addresses", "seed_hash"),
            ("wallet_transactions", "seed_hash"),
            ("platform_address_balances", "seed_hash"),
            ("shielded_notes", "wallet_seed_hash"),
            ("shielded_wallet_meta", "wallet_seed_hash"),
            ("asset_lock_transaction", "wallet"),
        ];
        for (table, fk_col) in wallet_fk_delete {
            if self.table_exists(conn, table)? {
                let deleted = conn.execute(
                    &format!(
                        "DELETE FROM {table} WHERE {fk_col} NOT IN (SELECT seed_hash FROM wallet)"
                    ),
                    [],
                )?;
                if deleted > 0 {
                    tracing::info!(
                        "Cleaned {deleted} orphaned row(s) from {table} (missing wallet)"
                    );
                }
            }
        }

        // identity.wallet is nullable with ON DELETE CASCADE — delete orphaned
        // identities whose wallet no longer exists (but skip NULL wallet).
        if self.table_exists(conn, "identity")? {
            let deleted = conn.execute(
                "DELETE FROM identity WHERE wallet IS NOT NULL
                 AND wallet NOT IN (SELECT seed_hash FROM wallet)",
                [],
            )?;
            if deleted > 0 {
                tracing::info!("Cleaned {deleted} orphaned identity row(s) (missing wallet)");
            }
        }

        // --- CASCADE children of identity(id) ---
        let identity_fk_delete: &[(&str, &str)] = &[
            ("top_up", "identity_id"),
            ("scheduled_votes", "identity_id"),
            ("identity_order", "identity_id"),
            ("identity_token_balances", "identity_id"),
            ("token_order", "identity_id"),
        ];
        for (table, fk_col) in identity_fk_delete {
            if self.table_exists(conn, table)? {
                let deleted = conn.execute(
                    &format!("DELETE FROM {table} WHERE {fk_col} NOT IN (SELECT id FROM identity)"),
                    [],
                )?;
                if deleted > 0 {
                    tracing::info!(
                        "Cleaned {deleted} orphaned row(s) from {table} (missing identity)"
                    );
                }
            }
        }

        // --- SET NULL children of identity(id) ---
        // asset_lock_transaction has ON DELETE SET NULL for identity_id columns.
        if self.table_exists(conn, "asset_lock_transaction")? {
            conn.execute(
                "UPDATE asset_lock_transaction SET identity_id = NULL
                 WHERE identity_id IS NOT NULL
                   AND identity_id NOT IN (SELECT id FROM identity)",
                [],
            )?;
            conn.execute(
                "UPDATE asset_lock_transaction SET identity_id_potentially_in_creation = NULL
                 WHERE identity_id_potentially_in_creation IS NOT NULL
                   AND identity_id_potentially_in_creation NOT IN (SELECT id FROM identity)",
                [],
            )?;
        }

        // --- CASCADE children of token(id) ---
        if self.table_exists(conn, "identity_token_balances")?
            && self.table_exists(conn, "token")?
        {
            conn.execute(
                "DELETE FROM identity_token_balances
                 WHERE token_id NOT IN (SELECT id FROM token)",
                [],
            )?;
        }
        if self.table_exists(conn, "token_order")? && self.table_exists(conn, "token")? {
            conn.execute(
                "DELETE FROM token_order WHERE token_id NOT IN (SELECT id FROM token)",
                [],
            )?;
        }

        // --- CASCADE children of contract ---
        if self.table_exists(conn, "token")? && self.table_exists(conn, "contract")? {
            conn.execute(
                "DELETE FROM token WHERE (data_contract_id, network)
                 NOT IN (SELECT contract_id, network FROM contract)",
                [],
            )?;
        }

        // --- CASCADE children of contested_name ---
        if self.table_exists(conn, "contestant")? && self.table_exists(conn, "contested_name")? {
            conn.execute(
                "DELETE FROM contestant
                 WHERE (normalized_contested_name, network)
                 NOT IN (SELECT normalized_contested_name, network FROM contested_name)",
                [],
            )?;
        }

        Ok(())
    }

    /// Check if a table exists in the database.
    fn table_exists(&self, conn: &Connection, table: &str) -> rusqlite::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )
    }

    /// Migration 29: rename network value `"dash"` to `"mainnet"` in all tables.
    ///
    /// Upstream `dashcore` renamed `Network::Dash` to `Network::Mainnet`,
    /// changing the `Display`/`FromStr` representation. This migration updates
    /// every table that stores the network as a string column.
    fn rename_network_dash_to_mainnet(&self, conn: &Connection) -> rusqlite::Result<()> {
        let tables = [
            "settings",
            "wallet",
            "identity_token_balances",
            "platform_address_balances",
            "utxos",
            "asset_lock_transaction",
            "identity",
            "contested_name",
            "contestant",
            "contract",
            "scheduled_votes",
            "dashpay_profiles",
            "dashpay_contact_requests",
            "dashpay_contacts",
            "wallet_transactions",
            "single_key_wallet",
            "token",
            "shielded_notes",
            "shielded_wallet_meta",
        ];
        for table in tables {
            conn.execute(
                &format!("UPDATE {table} SET network = 'mainnet' WHERE network = 'dash'"),
                [],
            )?;
        }
        Ok(())
    }

    /// Run database consistency checks on startup.
    /// Non-fatal: logs warnings for any issues found but does not fail.
    fn run_consistency_checks(&self) {
        let conn = self.conn.lock().unwrap();

        // PRAGMA quick_check is a faster subset of integrity_check.
        // It verifies b-tree structure without cross-checking indexes.
        match conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)) {
            Ok(ref result) if result == "ok" => {
                tracing::debug!("Database quick_check passed");
            }
            Ok(result) => {
                tracing::warn!("Database quick_check found issues: {result}");
            }
            Err(e) => {
                tracing::warn!("Database quick_check failed to execute: {e}");
            }
        }

        // PRAGMA foreign_key_check returns rows for each FK violation.
        match conn.prepare("PRAGMA foreign_key_check") {
            Ok(mut stmt) => {
                match stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                }) {
                    Ok(rows) => {
                        let violations: Vec<_> = rows.filter_map(|r| r.ok()).collect();
                        if violations.is_empty() {
                            tracing::debug!("Database foreign_key_check passed — no violations");
                        } else {
                            tracing::warn!(
                                "Database foreign_key_check found {} violation(s):",
                                violations.len()
                            );
                            for (table, rowid, parent, fk_idx) in &violations {
                                tracing::warn!(
                                    "  FK violation: {table} rowid={rowid} -> {parent} (fk_index={fk_idx})"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Database foreign_key_check query failed: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Database foreign_key_check failed to prepare: {e}");
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::database::initialization::DEFAULT_DB_VERSION;
    use rusqlite::{Connection, params};

    /// Helper: assert that a table exists in the database.
    fn assert_table_exists(conn: &Connection, table: &str) {
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap();
        assert!(exists, "table `{table}` should exist");
    }

    /// Helper: assert that a column exists in a table.
    fn assert_column_exists(conn: &Connection, table: &str, column: &str) {
        let exists: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{column}'"),
                [],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap();
        assert!(exists, "column `{column}` should exist in table `{table}`");
    }

    /// Verify the full v33 schema: all tables and columns introduced in v28-v33.
    fn assert_v33_schema(conn: &Connection) {
        // wallet.core_wallet_name (v28)
        assert_column_exists(conn, "wallet", "core_wallet_name");

        // shielded_notes table (v29)
        assert_table_exists(conn, "shielded_notes");
        for col in [
            "wallet_seed_hash",
            "note_data",
            "position",
            "cmx",
            "nullifier",
            "block_height",
            "is_spent",
            "value",
            "network",
        ] {
            assert_column_exists(conn, "shielded_notes", col);
        }

        // shielded_wallet_meta table with last_nullifier_sync_timestamp (v30)
        assert_table_exists(conn, "shielded_wallet_meta");
        assert_column_exists(
            conn,
            "shielded_wallet_meta",
            "last_nullifier_sync_timestamp",
        );

        // wallet_transactions.status (v30)
        assert_column_exists(conn, "wallet_transactions", "status");

        // contact_private_info table (v29)
        assert_table_exists(conn, "contact_private_info");

        // dashpay_contact_requests table (pre-existing, but checked for completeness)
        assert_table_exists(conn, "dashpay_contact_requests");
    }

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

    #[test]
    fn test_v33_migration_fresh_install() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("fresh.db");
        let db = super::Database::new(&db_file_path).unwrap();
        db.initialize(&db_file_path).unwrap();

        let conn = db.conn.lock().unwrap();

        let version: u16 = conn
            .query_row(
                "SELECT database_version FROM settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, DEFAULT_DB_VERSION);
        assert_eq!(version, 33);

        assert_v33_schema(&conn);
    }

    #[test]
    fn test_v33_migration_from_v27() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("v27.db");
        let db = super::Database::new(&db_file_path).unwrap();

        // Build a full database then strip v28+ additions to simulate v27.
        db.create_tables().unwrap();
        db.set_default_version().unwrap();

        {
            let conn = db.conn.lock().unwrap();

            // Remove v28+ tables entirely
            conn.execute("DROP TABLE IF EXISTS shielded_notes", [])
                .unwrap();
            conn.execute("DROP TABLE IF EXISTS shielded_wallet_meta", [])
                .unwrap();
            conn.execute("DROP TABLE IF EXISTS contact_private_info", [])
                .unwrap();

            // Recreate `wallet` without `core_wallet_name` (SQLite has no DROP COLUMN)
            conn.execute_batch(
                "CREATE TABLE wallet_old AS SELECT
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, password_hint, network,
                    confirmed_balance, unconfirmed_balance, total_balance,
                    last_platform_full_sync, last_platform_sync_checkpoint,
                    last_terminal_block
                 FROM wallet;
                 DROP TABLE wallet;
                 CREATE TABLE wallet (
                    seed_hash BLOB NOT NULL PRIMARY KEY,
                    encrypted_seed BLOB NOT NULL,
                    salt BLOB NOT NULL,
                    nonce BLOB NOT NULL,
                    master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
                    alias TEXT,
                    is_main INTEGER,
                    uses_password INTEGER NOT NULL,
                    password_hint TEXT,
                    network TEXT NOT NULL,
                    confirmed_balance INTEGER DEFAULT 0,
                    unconfirmed_balance INTEGER DEFAULT 0,
                    total_balance INTEGER DEFAULT 0,
                    last_platform_full_sync INTEGER DEFAULT 0,
                    last_platform_sync_checkpoint INTEGER DEFAULT 0,
                    last_terminal_block INTEGER DEFAULT 0
                 );
                 INSERT INTO wallet SELECT * FROM wallet_old;
                 DROP TABLE wallet_old;",
            )
            .unwrap();

            // Recreate `wallet_transactions` without `status`
            conn.execute_batch(
                "DROP TABLE IF EXISTS wallet_transactions;
                 CREATE TABLE wallet_transactions (
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
                    PRIMARY KEY (seed_hash, txid, network)
                 );",
            )
            .unwrap();

            // Recreate `single_key_wallet` without `core_wallet_name`
            conn.execute_batch(
                "DROP TABLE IF EXISTS single_key_wallet;
                 CREATE TABLE single_key_wallet (
                    key_hash BLOB NOT NULL PRIMARY KEY,
                    encrypted_private_key BLOB NOT NULL,
                    salt BLOB NOT NULL,
                    nonce BLOB NOT NULL,
                    public_key BLOB NOT NULL,
                    address TEXT NOT NULL,
                    alias TEXT,
                    uses_password INTEGER NOT NULL,
                    network TEXT NOT NULL,
                    confirmed_balance INTEGER DEFAULT 0,
                    unconfirmed_balance INTEGER DEFAULT 0,
                    total_balance INTEGER DEFAULT 0
                 );",
            )
            .unwrap();

            // Set version to 27
            conn.execute("UPDATE settings SET database_version = 27 WHERE id = 1", [])
                .unwrap();
        }

        // Verify version is 27 before migration
        assert_eq!(db.db_schema_version().unwrap(), 27);

        // Run migration from v27 to current
        let result = db.try_perform_migration(27, DEFAULT_DB_VERSION);
        assert!(
            result.is_ok(),
            "migration from v27 to v{DEFAULT_DB_VERSION} failed: {:?}",
            result.err()
        );

        // Verify final version
        assert_eq!(db.db_schema_version().unwrap(), 33);

        // Verify full v33 schema
        let conn = db.conn.lock().unwrap();
        assert_v33_schema(&conn);
    }

    #[test]
    fn test_v33_migration_with_orphaned_fk_rows() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("orphans.db");
        let db = super::Database::new(&db_file_path).unwrap();

        // Build full schema at current version
        db.create_tables().unwrap();
        db.set_default_version().unwrap();

        let valid_seed_hash = vec![0xAAu8; 32];
        let orphan_seed_hash = vec![0xBBu8; 32];

        {
            let conn = db.conn.lock().unwrap();

            // Insert a real wallet with the old network name
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, uses_password, network
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    valid_seed_hash,
                    vec![1u8; 16],
                    vec![2u8; 16],
                    vec![3u8; 12],
                    vec![4u8; 33],
                    0,
                    "dash"
                ],
            )
            .unwrap();

            // Disable FK enforcement to simulate legacy system SQLite
            conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

            // Insert orphaned wallet_transactions row (seed_hash not in wallet table).
            // Shielded table orphans are not needed: those tables get dropped to
            // simulate v27, then recreated empty by the migration.
            conn.execute(
                "INSERT INTO wallet_transactions (
                    seed_hash, txid, network, timestamp, net_amount,
                    is_ours, raw_transaction, status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    orphan_seed_hash,
                    vec![0xCCu8; 32],
                    "dash",
                    1000,
                    -50000,
                    1,
                    vec![0u8; 100],
                    0
                ],
            )
            .unwrap();

            // Insert valid wallet_transactions row for the real wallet
            conn.execute(
                "INSERT INTO wallet_transactions (
                    seed_hash, txid, network, timestamp, net_amount,
                    is_ours, raw_transaction, status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    valid_seed_hash,
                    vec![0xDDu8; 32],
                    "dash",
                    2000,
                    100000,
                    1,
                    vec![1u8; 100],
                    0
                ],
            )
            .unwrap();

            // Insert orphaned wallet_addresses row
            conn.execute(
                "INSERT INTO wallet_addresses (
                    seed_hash, address, derivation_path, balance,
                    path_reference, path_type
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![orphan_seed_hash, "yOrphanAddr1", "m/44'/1'/0'/0/0", 0, 0, 0],
            )
            .unwrap();

            // Insert valid wallet_addresses row
            conn.execute(
                "INSERT INTO wallet_addresses (
                    seed_hash, address, derivation_path, balance,
                    path_reference, path_type
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    valid_seed_hash,
                    "yValidAddr1",
                    "m/44'/1'/0'/0/0",
                    1000,
                    0,
                    0
                ],
            )
            .unwrap();

            // Insert a real identity for the valid wallet
            let valid_identity_id = vec![0xEEu8; 32];
            let orphan_identity_id = vec![0xFFu8; 32];
            conn.execute(
                "INSERT INTO identity (id, is_local, identity_type, alias, network)
                 VALUES (?1, 1, 'user', 'test', 'dash')",
                params![valid_identity_id],
            )
            .unwrap();

            // Insert asset_lock_transaction referencing a deleted identity
            conn.execute(
                "INSERT INTO asset_lock_transaction (
                    tx_id, transaction_data, amount, identity_id, wallet, network
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    vec![0xA1u8; 32],
                    vec![0u8; 50],
                    100_000,
                    orphan_identity_id,
                    valid_seed_hash,
                    "dash"
                ],
            )
            .unwrap();

            // Insert asset_lock_transaction referencing a valid identity
            conn.execute(
                "INSERT INTO asset_lock_transaction (
                    tx_id, transaction_data, amount, identity_id, wallet, network
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    vec![0xA2u8; 32],
                    vec![1u8; 50],
                    200_000,
                    valid_identity_id,
                    valid_seed_hash,
                    "dash"
                ],
            )
            .unwrap();

            // Strip v28+ additions to simulate v27 state (same as test_v33_migration_from_v27)
            // Remove shielded tables — they'll be recreated by migration
            conn.execute("DROP TABLE IF EXISTS shielded_notes", [])
                .unwrap();
            conn.execute("DROP TABLE IF EXISTS shielded_wallet_meta", [])
                .unwrap();
            conn.execute("DROP TABLE IF EXISTS contact_private_info", [])
                .unwrap();

            // Recreate wallet without core_wallet_name
            conn.execute_batch(
                "CREATE TABLE wallet_old AS SELECT
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, password_hint, network,
                    confirmed_balance, unconfirmed_balance, total_balance,
                    last_platform_full_sync, last_platform_sync_checkpoint,
                    last_terminal_block
                 FROM wallet;
                 DROP TABLE wallet;
                 CREATE TABLE wallet (
                    seed_hash BLOB NOT NULL PRIMARY KEY,
                    encrypted_seed BLOB NOT NULL,
                    salt BLOB NOT NULL,
                    nonce BLOB NOT NULL,
                    master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
                    alias TEXT,
                    is_main INTEGER,
                    uses_password INTEGER NOT NULL,
                    password_hint TEXT,
                    network TEXT NOT NULL,
                    confirmed_balance INTEGER DEFAULT 0,
                    unconfirmed_balance INTEGER DEFAULT 0,
                    total_balance INTEGER DEFAULT 0,
                    last_platform_full_sync INTEGER DEFAULT 0,
                    last_platform_sync_checkpoint INTEGER DEFAULT 0,
                    last_terminal_block INTEGER DEFAULT 0
                 );
                 INSERT INTO wallet SELECT * FROM wallet_old;
                 DROP TABLE wallet_old;",
            )
            .unwrap();

            // Recreate wallet_transactions without status but WITH FK constraint,
            // preserving orphaned rows (FK enforcement is still OFF).
            conn.execute_batch(
                "CREATE TABLE wallet_transactions_old AS SELECT
                    seed_hash, txid, network, timestamp, height, block_hash,
                    net_amount, fee, label, is_ours, raw_transaction
                 FROM wallet_transactions;
                 DROP TABLE wallet_transactions;
                 CREATE TABLE wallet_transactions (
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
                 );
                 INSERT INTO wallet_transactions SELECT * FROM wallet_transactions_old;
                 DROP TABLE wallet_transactions_old;",
            )
            .unwrap();

            // Recreate single_key_wallet without core_wallet_name
            conn.execute_batch(
                "DROP TABLE IF EXISTS single_key_wallet;
                 CREATE TABLE single_key_wallet (
                    key_hash BLOB NOT NULL PRIMARY KEY,
                    encrypted_private_key BLOB NOT NULL,
                    salt BLOB NOT NULL,
                    nonce BLOB NOT NULL,
                    public_key BLOB NOT NULL,
                    address TEXT NOT NULL,
                    alias TEXT,
                    uses_password INTEGER NOT NULL,
                    network TEXT NOT NULL,
                    confirmed_balance INTEGER DEFAULT 0,
                    unconfirmed_balance INTEGER DEFAULT 0,
                    total_balance INTEGER DEFAULT 0
                 );",
            )
            .unwrap();

            // Re-enable FK enforcement
            conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

            // Set version to 27
            conn.execute("UPDATE settings SET database_version = 27 WHERE id = 1", [])
                .unwrap();
        }

        assert_eq!(db.db_schema_version().unwrap(), 27);

        // Run migration — this would fail without clean_orphaned_fk_rows
        let result = db.try_perform_migration(27, DEFAULT_DB_VERSION);
        assert!(
            result.is_ok(),
            "migration with orphaned FK rows failed: {:?}",
            result.err()
        );

        assert_eq!(db.db_schema_version().unwrap(), DEFAULT_DB_VERSION);

        let conn = db.conn.lock().unwrap();
        assert_v33_schema(&conn);

        // Orphaned wallet_transactions should be gone
        let orphan_txs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_transactions WHERE seed_hash = ?1",
                params![orphan_seed_hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_txs, 0,
            "orphaned wallet_transactions should be deleted"
        );

        // Shielded tables should exist but be empty (recreated fresh by migration;
        // the cleanup handles them gracefully even when just-created)
        assert_table_exists(&conn, "shielded_notes");
        assert_table_exists(&conn, "shielded_wallet_meta");

        // Valid wallet_transactions should survive with network renamed to mainnet
        let valid_txs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_transactions WHERE seed_hash = ?1 AND network = 'mainnet'",
                params![valid_seed_hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            valid_txs, 1,
            "valid wallet_transactions should survive with network=mainnet"
        );

        // Wallet itself should have mainnet
        let wallet_network: String = conn
            .query_row(
                "SELECT network FROM wallet WHERE seed_hash = ?1",
                params![valid_seed_hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wallet_network, "mainnet");

        // Orphaned wallet_addresses should be gone, valid ones survive
        let orphan_addrs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_addresses WHERE seed_hash = ?1",
                params![orphan_seed_hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_addrs, 0,
            "orphaned wallet_addresses should be deleted"
        );

        let valid_addrs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_addresses WHERE seed_hash = ?1",
                params![valid_seed_hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            valid_addrs, 1,
            "valid wallet_addresses should survive migration"
        );

        // asset_lock_transaction with orphaned identity_id should be SET NULL
        let valid_identity_id = vec![0xEEu8; 32];

        let orphan_lock_identity: Option<Vec<u8>> = conn
            .query_row(
                "SELECT identity_id FROM asset_lock_transaction WHERE tx_id = ?1",
                params![vec![0xA1u8; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            orphan_lock_identity.is_none(),
            "orphaned asset_lock identity_id should be NULL, got {:?}",
            orphan_lock_identity
        );

        // asset_lock_transaction with valid identity_id should keep it
        let valid_lock_identity: Option<Vec<u8>> = conn
            .query_row(
                "SELECT identity_id FROM asset_lock_transaction WHERE tx_id = ?1",
                params![vec![0xA2u8; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            valid_lock_identity,
            Some(valid_identity_id),
            "valid asset_lock identity_id should be preserved"
        );
    }
}
