use crate::database::Database;
use chrono::Utc;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};
use rusqlite::{Connection, params};
use std::fs;
use std::path::Path;

/// Error during database migration with structured context.
#[derive(Debug, thiserror::Error)]
#[error("migration failed on {}{}: {source}",
    table.as_deref().unwrap_or("(unknown table)"),
    if details.is_empty() { String::new() } else { format!(" ({})", details) }
)]
pub struct MigrationError {
    /// Table being operated on when the error occurred, if known.
    pub table: Option<String>,
    /// Human-readable description of the operation that failed.
    pub details: String,
    /// Underlying SQLite error.
    #[source]
    pub source: rusqlite::Error,
}

/// Extension trait for converting `rusqlite::Result` into `MigrationError` with table context.
trait MigrationResultExt<T> {
    fn migration_err(self, table: &str, details: &str) -> Result<T, MigrationError>;
}

impl<T> MigrationResultExt<T> for rusqlite::Result<T> {
    fn migration_err(self, table: &str, details: &str) -> Result<T, MigrationError> {
        self.map_err(|e| MigrationError {
            table: Some(table.into()),
            details: details.into(),
            source: e,
        })
    }
}

pub const DEFAULT_DB_VERSION: u16 = 34;

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

                // Phase 1 of the v34 migration (add wallet_id column + backfill
                // no-password wallets) is extracted here so it commits before the
                // main migration transaction starts. This ensures the column persists
                // even if the main migration rolls back (e.g., because a
                // password-protected wallet still needs unlocking via
                // WalletMigrationScreen). Idempotent — safe to call repeatedly.
                if current_version < 34 {
                    self.ensure_wallet_id_column_and_backfill()?;
                }

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

    /// Idempotent Phase 1 of the v34 migration.
    ///
    /// Adds the nullable `wallet_id BLOB` column to `wallet` (if missing) and
    /// backfills `wallet_id` for every wallet where `uses_password = 0` by
    /// decrypting the 64-byte raw seed and computing
    /// `SHA256(root_pub_key.serialize() || root_chain_code)`.
    ///
    /// This method is intentionally separate from `migrate_v33_to_v34_consolidated`
    /// so that the column and backfill commit immediately — even if the main
    /// migration rolls back because a password-protected wallet has not been
    /// unlocked yet. `WalletMigrationScreen` writes `wallet_id` for those wallets,
    /// then calls `initialize` again; by that time this method is a no-op because
    /// the column already exists and all no-password wallets are already backfilled.
    pub fn ensure_wallet_id_column_and_backfill(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        // Step 1: Add wallet_id column to wallet table (idempotent).
        let has_wallet_id: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet') WHERE name='wallet_id'",
            [],
            |row| row.get::<_, i32>(0).map(|c| c > 0),
        )?;
        if !has_wallet_id {
            // wallet table may not exist at all on very old DBs (pre-v1). Guard.
            let wallet_table_exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wallet'",
                [],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )?;
            if wallet_table_exists {
                conn.execute("ALTER TABLE wallet ADD COLUMN wallet_id BLOB", [])?;
            } else {
                // No wallet table — nothing to backfill.
                return Ok(());
            }
        }

        // Step 2: Backfill wallet_id for no-password wallets.
        //         encrypted_seed stores raw 64-byte seed when uses_password = 0.
        let rows: Vec<(Vec<u8>, Vec<u8>, String)> = {
            let mut stmt = conn.prepare(
                "SELECT seed_hash, encrypted_seed, network
                 FROM wallet
                 WHERE uses_password = 0 AND wallet_id IS NULL",
            )?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?
        };

        for (seed_hash_bytes, raw_seed, network_str) in &rows {
            if raw_seed.len() != 64 {
                // Logged at error: the wallet row is corrupt and
                // unusable. We do NOT abort startup — other wallets
                // may be fine, and the locked-wallet query filters
                // `uses_password = 1`, so this no-password row will
                // not surface in WalletMigrationScreen and trap the
                // user in an infinite password loop. The wallet stays
                // present in SQL with `wallet_id IS NULL`; user-
                // visible recovery is out of scope (re-import).
                tracing::error!(
                    seed = %hex::encode(seed_hash_bytes),
                    len = raw_seed.len(),
                    "ensure_wallet_id_column_and_backfill: wallet has corrupt seed length, leaving wallet_id NULL — wallet will be unusable"
                );
                continue;
            }
            let seed: [u8; 64] = raw_seed.as_slice().try_into().unwrap();
            // Normalize the legacy "dash" network alias to "mainnet"
            // — this backfill runs before the v33 migration that
            // performs the schema-level rename, and v0.9.0 databases
            // store "dash" verbatim. After v33, the column always
            // holds "mainnet", but supporting both keeps the upgrade
            // path from v0.9.0 → v34 working.
            let normalized_network = if network_str == "dash" {
                "mainnet"
            } else {
                network_str.as_str()
            };
            // Parse strictly. A bad value here would silently derive
            // a wrong-network wallet_id (mainnet wallet keyed under
            // testnet derivation), permanently locking the user out
            // of their funds. Skip the row so the locked-wallet guard
            // surfaces the corruption to the user instead.
            let network = match normalized_network.parse::<Network>() {
                Ok(net) => net,
                Err(e) => {
                    tracing::error!(
                        seed = %hex::encode(seed_hash_bytes),
                        network_str = %network_str,
                        error = %e,
                        "ensure_wallet_id_column_and_backfill: unparseable network — \
                         skipping (locked-wallet guard will surface this)"
                    );
                    continue;
                }
            };
            let Some(wallet_id) = crate::model::wallet::derive_wallet_id_from_seed(&seed, network)
            else {
                tracing::warn!(
                    seed = %hex::encode(seed_hash_bytes),
                    "ensure_wallet_id_column_and_backfill: failed to derive wallet_id — skipping"
                );
                continue;
            };
            conn.execute(
                "UPDATE wallet SET wallet_id = ?1 WHERE seed_hash = ?2",
                params![&wallet_id[..], seed_hash_bytes],
            )?;
        }

        Ok(())
    }

    fn apply_version_changes(&self, version: u16, tx: &Connection) -> Result<(), MigrationError> {
        match version {
            34 => {
                self.migrate_v33_to_v34_consolidated(tx)?;
            }
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
                self.add_core_wallet_name_column(tx)
                    .migration_err("wallet", "add core_wallet_name column")?;
                self.init_contacts_tables(tx)
                    .migration_err("contact_private_info", "create contacts tables")?;
                self.create_shielded_tables(tx)
                    .migration_err("shielded_notes", "create shielded tables")?;
                self.create_shielded_wallet_meta_table(tx)
                    .migration_err("shielded_wallet_meta", "create shielded_wallet_meta table")?;
                self.add_nullifier_sync_timestamp_column(tx).migration_err(
                    "shielded_wallet_meta",
                    "add last_nullifier_sync_timestamp column",
                )?;
                // Defer FK checks so parent->child rename order doesn't matter
                // (contestant and token have composite FKs that include network).
                tx.execute_batch("PRAGMA defer_foreign_keys = ON")
                    .map_err(|e| MigrationError {
                        table: None,
                        details: "defer FK checks for network rename".into(),
                        source: e,
                    })?;
                self.rename_network_dash_to_mainnet(tx)?;
                self.add_wallet_transaction_status_column(tx)
                    .migration_err("wallet_transactions", "add status column")?;
            }
            27 => {
                self.add_network_indexes(tx).map_err(|e| MigrationError {
                    table: None,
                    details: "add network indexes".into(),
                    source: e,
                })?;
            }
            26 => {
                self.add_last_full_sync_balance_column(tx).migration_err(
                    "platform_address_balances",
                    "add last_full_sync_balance column",
                )?;
            }
            25 => {
                self.add_avatar_bytes_column(tx)
                    .migration_err("dashpay_profiles", "add avatar_bytes column")?;
            }
            24 => {
                self.add_selected_wallet_columns(tx)
                    .migration_err("settings", "add selected_wallet columns")?;
            }
            23 => {
                self.add_last_terminal_block_column(tx)
                    .migration_err("wallet", "add last_terminal_block column")?;
            }
            22 => {
                self.add_network_column_to_dashpay_contact_requests(tx)
                    .migration_err("dashpay_contact_requests", "add network column")?;
                self.add_network_column_to_dashpay_contacts(tx)
                    .migration_err("dashpay_contacts", "add network column")?;
            }
            21 => {
                self.add_network_column_to_dashpay_profiles(tx)
                    .migration_err("dashpay_profiles", "add network column")?;
            }
            20 => {
                self.add_platform_sync_columns(tx)
                    .migration_err("wallet", "add platform sync columns")?;
            }
            19 => {
                self.initialize_platform_address_balances_table(tx)
                    .migration_err("platform_address_balances", "create table")?;
            }
            18 => {
                self.initialize_single_key_wallet_table(tx)
                    .migration_err("single_key_wallet", "create table")?;
            }
            17 => {
                self.add_address_total_received_column(tx)
                    .migration_err("wallet_addresses", "add total_received column")?;
            }
            16 => {
                self.add_wallet_balance_columns(tx)
                    .migration_err("wallet", "add balance columns")?;
            }
            15 => {
                self.add_core_backend_mode_column(tx)
                    .migration_err("settings", "add core_backend_mode column")?;
            }
            14 => {
                self.initialize_wallet_transactions_table(tx)
                    .migration_err("wallet_transactions", "create table")?;
            }
            13 => {
                self.init_dashpay_tables_in_tx(tx)
                    .migration_err("dashpay_profiles", "create DashPay tables")?;
            }
            12 => {
                self.add_disable_zmq_column(tx)
                    .migration_err("settings", "add disable_zmq column")?;
            }
            11 => {
                self.rename_identity_column_is_in_creation_to_status(tx)
                    .migration_err("identity", "rename is_in_creation to status")?;
            }
            10 => {
                self.add_theme_preference_column(tx)
                    .migration_err("settings", "add theme_preference column")?;
            }
            9 => {
                self.delete_all_identities_in_all_devnets_and_regtest(tx)
                    .migration_err("identity", "delete devnet/regtest identities")?;
                self.delete_all_local_tokens_in_all_devnets_and_regtest(tx)
                    .migration_err("token", "delete devnet/regtest tokens")?;
                self.remove_all_asset_locks_identity_id_for_all_devnets_and_regtest(tx)
                    .migration_err(
                        "asset_lock_transaction",
                        "clear devnet/regtest asset lock identity IDs",
                    )?;
                self.remove_all_contracts_in_all_devnets_and_regtest(tx)
                    .migration_err("contract", "delete devnet/regtest contracts")?;
                self.fix_identity_devnet_network_name(tx)
                    .migration_err("identity", "fix devnet network name")?;
            }
            8 => {
                self.change_contract_name_to_alias(tx)
                    .migration_err("contract", "rename name to alias")?;
            }
            7 => {
                self.migrate_asset_lock_fk_to_set_null(tx)
                    .migration_err("asset_lock_transaction", "migrate FK to SET NULL")?;
            }
            6 => {
                self.update_scheduled_votes_table(tx)
                    .migration_err("scheduled_votes", "update table schema")?;
                self.initialize_token_table(tx)
                    .migration_err("token", "create table")?;
                self.drop_identity_token_balances_table(tx)
                    .migration_err("identity_token_balances", "drop table")?;
                self.initialize_identity_token_balances_table(tx)
                    .migration_err("identity_token_balances", "create table")?;
                tx.execute("DROP TABLE IF EXISTS identity_order", [])
                    .migration_err("identity_order", "drop table")?;
                self.initialize_identity_order_table(tx)
                    .migration_err("identity_order", "create table")?;
                tx.execute("DROP TABLE IF EXISTS token_order", [])
                    .migration_err("token_order", "drop table")?;
                self.initialize_token_order_table(tx)
                    .migration_err("token_order", "create table")?;
            }
            5 => {
                self.initialize_scheduled_votes_table(tx)
                    .migration_err("scheduled_votes", "create table")?;
            }
            4 => {
                self.initialize_top_up_table(tx)
                    .migration_err("top_up", "create table")?;
            }
            3 => {
                self.add_custom_dash_qt_columns(tx)
                    .migration_err("settings", "add custom dash_qt columns")?;
            }
            2 => {
                self.initialize_proof_log_table(tx)
                    .migration_err("proof_log", "create table")?;
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
    ) -> Result<bool, MigrationError> {
        match original_version.cmp(&to_version) {
            std::cmp::Ordering::Equal => {
                tracing::trace!(
                    "No database migration needed, already at version {}",
                    to_version
                );
                Ok(false)
            }
            std::cmp::Ordering::Greater => Err(MigrationError {
                table: None,
                details: format!(
                    "database is at version {original_version} but this build \
                     only supports up to version {to_version} — please update dash-evo-tool"
                ),
                source: rusqlite::Error::InvalidQuery,
            }),
            std::cmp::Ordering::Less => {
                // Phase 1 of the v34 migration (add wallet_id column + backfill
                // no-password wallets) must commit before the main migration
                // transaction starts — so it runs here, outside the lock below,
                // whenever we're upgrading through v34. This mirrors the call in
                // `initialize()` and ensures tests that call `try_perform_migration`
                // directly also get the column added first.
                if original_version < 34 && to_version >= 34 {
                    self.ensure_wallet_id_column_and_backfill()
                        .map_err(|e| MigrationError {
                            table: Some("wallet".into()),
                            details: "ensure_wallet_id_column_and_backfill".into(),
                            source: e,
                        })?;
                }

                let mut conn = self
                    .conn
                    .lock()
                    .expect("Failed to lock database connection");

                for version in (original_version + 1)..=to_version {
                    tracing::debug!("Applying migration v{version}");
                    let tx = conn.transaction().map_err(|e| MigrationError {
                        table: None,
                        details: format!("v{version}: begin transaction"),
                        source: e,
                    })?;
                    let result = self
                        .apply_version_changes(version, &tx)
                        .and_then(|()| {
                            self.update_database_version(version, &tx).migration_err(
                                "settings",
                                &format!("v{version}: update_database_version"),
                            )
                        })
                        .and_then(|()| {
                            tx.commit().map_err(|e| MigrationError {
                                table: None,
                                details: format!("v{version}: commit"),
                                source: e,
                            })
                        });

                    if let Err(ref migration_err) = result {
                        if let rusqlite::Error::SqliteFailure(err, _) = &migration_err.source
                            && err.extended_code == 787
                        {
                            // SQLITE_CONSTRAINT_FOREIGNKEY
                            Self::log_fk_violations(&conn);
                        }
                        return result.map(|()| true);
                    }
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

        // Create the wallet table (legacy v33 shape: seed_hash as PK,
        // wallet_id as nullable column). The v34 rebuild runs at the
        // end of `create_tables` to promote wallet_id to PRIMARY KEY
        // and drop seed_hash — keeping fresh-install and migration
        // paths converging on the same rebuild routine.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS wallet (
                seed_hash BLOB NOT NULL PRIMARY KEY,
                wallet_id BLOB,
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

        // Create wallet addresses (legacy v33 shape; rebuilt to v34 at
        // the end of `create_tables`).
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

        // Create Platform address balances table (legacy v33 shape;
        // rebuilt to v34 at the end of `create_tables`).
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
                        is_instant_locked INTEGER NOT NULL DEFAULT 0,
                        PRIMARY KEY (txid, vout, network)
                    );",
            [],
        )?;

        // Per-account address pool state (Phase 10 uniform key-wallet
        // state persistence). See the v36 migration for the shape
        // rationale. v34 renames the key column to `wallet_id` and
        // adds the FK to wallet(wallet_id) (applied by the rebuild at
        // the end of `create_tables`).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS wallet_account_pool_state (
                wallet_id BLOB NOT NULL,
                account_type BLOB NOT NULL,
                pool_type INTEGER NOT NULL,
                highest_used INTEGER,
                highest_generated INTEGER,
                PRIMARY KEY (wallet_id, account_type, pool_type)
            )",
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

        // Create asset lock transaction table (legacy v33 shape;
        // rebuilt to v34 at the end of `create_tables` to flip the
        // FK from wallet(seed_hash) to wallet(wallet_id)).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS asset_lock_transaction (
                        tx_id BLOB NOT NULL,
                        output_index INTEGER NOT NULL DEFAULT 0,
                        transaction_data BLOB NOT NULL,
                        amount INTEGER,
                        instant_lock_data BLOB,
                        chain_locked_height INTEGER,
                        identity_id BLOB,
                        identity_id_potentially_in_creation BLOB,
                        wallet BLOB NOT NULL,
                        network TEXT NOT NULL,
                        account_index INTEGER NOT NULL DEFAULT 0,
                        funding_type INTEGER NOT NULL DEFAULT 0,
                        identity_index INTEGER NOT NULL DEFAULT 0,
                        proof_data BLOB,
                        PRIMARY KEY (tx_id, output_index),
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

        // Create the identities table (legacy v33 shape; rebuilt to
        // v34 at the end of `create_tables` to flip the wallet FK
        // target to wallet(wallet_id)).
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

        // Initialize shielded pool tables (legacy v33 shape; rebuilt
        // to v34 by the final step below).
        self.create_shielded_tables(&conn)?;
        self.create_shielded_wallet_meta_table(&conn)?;

        // Finalize: promote wallet_id to PRIMARY KEY and flip every
        // child FK to reference wallet(wallet_id). Fresh installs
        // have an empty wallet table (no locked-wallet guard fires),
        // so this converges on the same v34 schema that migration
        // produces. Any MigrationError from the guard is surfaced as
        // a rusqlite::Error since create_tables signature is
        // rusqlite-typed; on an empty DB this branch is unreachable.
        self.migrate_v33_to_v34_consolidated(&conn)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

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

    /// Remove orphaned child rows left behind when parent rows were deleted
    /// while FK enforcement was off (system SQLite before bundled build).
    /// Bundled SQLite enables FK checks by default, so any subsequent UPDATE
    /// on these rows triggers re-validation and fails. Covers all FK
    /// relationships in the schema: wallet→children, identity→children,
    /// token→children, contract→children, contested_name→children.
    fn clean_orphaned_fk_rows(&self, conn: &Connection) -> Result<(), MigrationError> {
        // --- CASCADE children of wallet(seed_hash) ---
        // wallet_transactions and wallet_account_pool_state are omitted
        // because v34 drops and recreates them with the `wallet_id`
        // column name. Any orphans are cleaned by the DROP itself.
        let wallet_fk_delete: &[(&str, &str)] = &[
            ("wallet_addresses", "seed_hash"),
            ("platform_address_balances", "seed_hash"),
            ("shielded_notes", "wallet_seed_hash"),
            ("shielded_wallet_meta", "wallet_seed_hash"),
            ("asset_lock_transaction", "wallet"),
        ];
        for (table, fk_col) in wallet_fk_delete {
            if self
                .table_exists(conn, table)
                .migration_err(table, "check table existence")?
            {
                let deleted = conn
                    .execute(
                        &format!(
                            "DELETE FROM {table} WHERE {fk_col} NOT IN (SELECT seed_hash FROM wallet)"
                        ),
                        [],
                    )
                    .migration_err(table, "delete orphaned wallet FK rows")?;
                if deleted > 0 {
                    tracing::info!(
                        "Cleaned {deleted} orphaned row(s) from {table} (missing wallet)"
                    );
                }
            }
        }

        // identity.wallet is nullable with ON DELETE CASCADE — delete orphaned
        // identities whose wallet no longer exists (but skip NULL wallet).
        if self
            .table_exists(conn, "identity")
            .migration_err("identity", "check table existence")?
        {
            let deleted = conn
                .execute(
                    "DELETE FROM identity WHERE wallet IS NOT NULL
                 AND wallet NOT IN (SELECT seed_hash FROM wallet)",
                    [],
                )
                .migration_err("identity", "delete orphaned identity rows")?;
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
            if self
                .table_exists(conn, table)
                .migration_err(table, "check table existence")?
            {
                let deleted = conn
                    .execute(
                        &format!(
                            "DELETE FROM {table} WHERE {fk_col} NOT IN (SELECT id FROM identity)"
                        ),
                        [],
                    )
                    .migration_err(table, "delete orphaned identity FK rows")?;
                if deleted > 0 {
                    tracing::info!(
                        "Cleaned {deleted} orphaned row(s) from {table} (missing identity)"
                    );
                }
            }
        }

        // --- SET NULL children of identity(id) ---
        if self
            .table_exists(conn, "asset_lock_transaction")
            .migration_err("asset_lock_transaction", "check table existence")?
        {
            conn.execute(
                "UPDATE asset_lock_transaction SET identity_id = NULL
                 WHERE identity_id IS NOT NULL
                   AND identity_id NOT IN (SELECT id FROM identity)",
                [],
            )
            .migration_err("asset_lock_transaction", "nullify orphaned identity_id")?;
            conn.execute(
                "UPDATE asset_lock_transaction SET identity_id_potentially_in_creation = NULL
                 WHERE identity_id_potentially_in_creation IS NOT NULL
                   AND identity_id_potentially_in_creation NOT IN (SELECT id FROM identity)",
                [],
            )
            .migration_err(
                "asset_lock_transaction",
                "nullify orphaned identity_id_potentially_in_creation",
            )?;
        }

        // --- CASCADE children of token(id) ---
        if self
            .table_exists(conn, "identity_token_balances")
            .migration_err("identity_token_balances", "check table existence")?
            && self
                .table_exists(conn, "token")
                .migration_err("token", "check table existence")?
        {
            conn.execute(
                "DELETE FROM identity_token_balances
                 WHERE token_id NOT IN (SELECT id FROM token)",
                [],
            )
            .migration_err("identity_token_balances", "delete orphaned token FK rows")?;
        }
        if self
            .table_exists(conn, "token_order")
            .migration_err("token_order", "check table existence")?
            && self
                .table_exists(conn, "token")
                .migration_err("token", "check table existence")?
        {
            conn.execute(
                "DELETE FROM token_order WHERE token_id NOT IN (SELECT id FROM token)",
                [],
            )
            .migration_err("token_order", "delete orphaned token FK rows")?;
        }

        // --- CASCADE children of contract ---
        if self
            .table_exists(conn, "token")
            .migration_err("token", "check table existence")?
            && self
                .table_exists(conn, "contract")
                .migration_err("contract", "check table existence")?
        {
            conn.execute(
                "DELETE FROM token WHERE (data_contract_id, network)
                 NOT IN (SELECT contract_id, network FROM contract)",
                [],
            )
            .migration_err("token", "delete orphaned contract FK rows")?;
        }

        // --- CASCADE children of contested_name ---
        if self
            .table_exists(conn, "contestant")
            .migration_err("contestant", "check table existence")?
            && self
                .table_exists(conn, "contested_name")
                .migration_err("contested_name", "check table existence")?
        {
            conn.execute(
                "DELETE FROM contestant
                 WHERE (normalized_contested_name, network)
                 NOT IN (SELECT normalized_contested_name, network FROM contested_name)",
                [],
            )
            .migration_err("contestant", "delete orphaned contested_name FK rows")?;
        }

        Ok(())
    }

    /// Log all FK violations to help diagnose SQLITE_CONSTRAINT_FOREIGNKEY errors.
    fn log_fk_violations(conn: &Connection) {
        const MAX_VIOLATIONS_TO_LOG: usize = 50;

        tracing::error!(
            "FK constraint failure detected — running PRAGMA foreign_key_check for diagnostics:"
        );
        let Ok(mut stmt) = conn.prepare("PRAGMA foreign_key_check") else {
            tracing::error!("  failed to prepare PRAGMA foreign_key_check");
            return;
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        }) else {
            tracing::error!("  failed to execute PRAGMA foreign_key_check");
            return;
        };

        let mut count = 0usize;
        let mut errors = 0usize;
        for row in rows {
            match row {
                Ok((table, rowid, parent, fk_idx)) => {
                    count += 1;
                    if count <= MAX_VIOLATIONS_TO_LOG {
                        tracing::error!(
                            "  FK violation: {table} rowid={rowid} -> {parent} (fk_index={fk_idx})"
                        );
                    }
                }
                Err(e) => {
                    errors += 1;
                    if errors <= 3 {
                        tracing::error!("  FK check row decode error: {e}");
                    }
                }
            }
        }
        if count > MAX_VIOLATIONS_TO_LOG {
            tracing::error!(
                "  ... and {} more violation(s) not shown",
                count - MAX_VIOLATIONS_TO_LOG
            );
        }
        if count == 0 && errors == 0 {
            tracing::error!("  no violations found (failure may be from deferred FK check)");
        }
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
    fn rename_network_dash_to_mainnet(&self, conn: &Connection) -> Result<(), MigrationError> {
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
            tracing::debug!("  rename_network: updating {table}");
            conn.execute(
                &format!("UPDATE {table} SET network = 'mainnet' WHERE network = 'dash'"),
                [],
            )
            .migration_err(table, "rename network dash -> mainnet")?;
        }
        Ok(())
    }

    /// Run database consistency checks on startup.
    /// Non-fatal: logs warnings for any issues found but does not fail.
    fn run_consistency_checks(&self) {
        const MAX_ISSUES_TO_LOG: usize = 20;

        let conn = self.conn.lock().unwrap();

        // PRAGMA quick_check can return multiple rows (one per issue).
        match conn.prepare("PRAGMA quick_check") {
            Ok(mut stmt) => match stmt
                .query_map([], |row| row.get::<_, String>(0))
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            {
                Ok(results) if results.len() == 1 && results[0] == "ok" => {
                    tracing::debug!("Database quick_check passed");
                }
                Ok(results) if results.is_empty() => {
                    tracing::warn!("Database quick_check returned no results");
                }
                Ok(results) => {
                    tracing::warn!("Database quick_check found {} issue(s):", results.len());
                    for issue in results.iter().take(MAX_ISSUES_TO_LOG) {
                        tracing::warn!("  {issue}");
                    }
                    if results.len() > MAX_ISSUES_TO_LOG {
                        tracing::warn!(
                            "  ... and {} more issue(s) not shown",
                            results.len() - MAX_ISSUES_TO_LOG
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("Database quick_check failed: {e}");
                }
            },
            Err(e) => {
                tracing::warn!("Database quick_check failed to prepare: {e}");
            }
        }

        // PRAGMA foreign_key_check returns one row per FK violation.
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
                        let mut violations = Vec::new();
                        let mut row_errors = 0usize;
                        for row in rows {
                            match row {
                                Ok(v) => violations.push(v),
                                Err(e) => {
                                    row_errors += 1;
                                    if row_errors <= 3 {
                                        tracing::warn!(
                                            "Database foreign_key_check row decode error: {e}"
                                        );
                                    }
                                }
                            }
                        }
                        if violations.is_empty() && row_errors == 0 {
                            tracing::debug!("Database foreign_key_check passed — no violations");
                        } else {
                            if !violations.is_empty() {
                                tracing::warn!(
                                    "Database foreign_key_check found {} violation(s):",
                                    violations.len()
                                );
                                for (table, rowid, parent, fk_idx) in
                                    violations.iter().take(MAX_ISSUES_TO_LOG)
                                {
                                    tracing::warn!(
                                        "  FK violation: {table} rowid={rowid} -> {parent} (fk_index={fk_idx})"
                                    );
                                }
                                if violations.len() > MAX_ISSUES_TO_LOG {
                                    tracing::warn!(
                                        "  ... and {} more violation(s) not shown",
                                        violations.len() - MAX_ISSUES_TO_LOG
                                    );
                                }
                            }
                            if row_errors > 0 {
                                tracing::warn!(
                                    "Database foreign_key_check had {row_errors} row decode error(s)"
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

    /// Consolidated migration v33 → v34.
    ///
    /// This single step replaces eight intermediate migration steps
    /// (v34-v41) that existed only on this feature branch and were never
    /// released. Starting state: v33 schema with `seed_hash` as the
    /// `wallet` primary key. Ending state: `wallet_id` is the primary key,
    /// `seed_hash` is gone, all child tables key on `wallet_id` with proper
    /// FK constraints, and the DashPay contact-request table has its final
    /// v34 column set.
    ///
    /// Note: Steps 1 and 2 (add `wallet_id` column + backfill no-password wallets)
    /// were extracted to [`Database::ensure_wallet_id_column_and_backfill`] which
    /// is called by `initialize` **before** this transaction starts. This allows
    /// those changes to commit even if this migration rolls back (e.g., when
    /// password-protected wallets need unlocking via `WalletMigrationScreen`).
    ///
    /// Steps:
    /// 1. Guard: abort if any `wallet_id IS NULL` (locked password wallets).
    /// 2. Guard: abort if any duplicate `wallet_id` values.
    /// 3. `PRAGMA foreign_keys = OFF` — about to drop/recreate FK parents.
    /// 4. DELETE all cache-table rows (tolerate missing tables).
    /// 5. DROP and recreate every cache table in final v34 shape.
    /// 6. Ensure `utxos.is_instant_locked` exists.
    /// 7. Recreate `dashpay_contact_requests` with DIP-15 columns.
    /// 8. Rebuild `wallet` with `wallet_id` as PRIMARY KEY, drop `seed_hash`.
    /// 9. Drop `dashpay_address_mappings` and its indexes.
    /// 10. Clear `selected_wallet_hash` in settings.
    /// 11. `PRAGMA foreign_keys = ON`.
    fn migrate_v33_to_v34_consolidated(&self, conn: &Connection) -> Result<(), MigrationError> {
        // Step 1: Guard — refuse if any wallet still has NULL wallet_id.
        //         The sentinel prefix is matched by WalletMigrationScreen.
        let locked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet WHERE wallet_id IS NULL",
                [],
                |row| row.get(0),
            )
            .migration_err("wallet", "check for locked wallets (wallet_id IS NULL)")?;
        if locked > 0 {
            return Err(MigrationError {
                table: Some("wallet".into()),
                details: format!(
                    "locked password wallets require unlock: {locked} wallet(s) \
                     have no wallet_id yet — unlock each wallet via \
                     WalletMigrationScreen, then re-run migration"
                ),
                source: rusqlite::Error::InvalidQuery,
            });
        }

        // Step 4: Guard — wallet_id must be unique (will become PK).
        let dup_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) - COUNT(DISTINCT wallet_id) FROM wallet",
                [],
                |row| row.get(0),
            )
            .migration_err("wallet", "check wallet_id uniqueness")?;
        if dup_count != 0 {
            return Err(MigrationError {
                table: Some("wallet".into()),
                details: format!(
                    "cannot promote wallet_id to PRIMARY KEY: {dup_count} duplicate wallet_id \
                     value(s) detected across wallet rows"
                ),
                source: rusqlite::Error::InvalidQuery,
            });
        }

        // Step 5: Disable FK enforcement while we drop/recreate FK parents.
        conn.execute_batch("PRAGMA foreign_keys = OFF")
            .migration_err("pragma", "foreign_keys=OFF for v34 table rebuild")?;

        // Step 6: Delete all cache-table rows. Use `let _ =` to tolerate
        //         tables that may not exist on all upgrade paths (e.g.
        //         wallet_account_pool_state introduced in v36).
        for table in [
            "identity",
            "wallet_addresses",
            "platform_address_balances",
            "asset_lock_transaction",
            "utxos",
            "dashpay_contacts",
            "dashpay_contact_requests",
            "dashpay_profiles",
            "dashpay_payments",
            "dashpay_address_mappings",
            "dashpay_contact_address_indices",
            "contested_name",
            "contestant",
            "contract",
            "shielded_notes",
            "shielded_wallet_meta",
            "contact_private_info",
            "scheduled_votes",
            "top_up",
            "identity_order",
            "token_order",
            "token",
            "identity_token_balances",
        ] {
            let _ = conn.execute(&format!("DELETE FROM {table}"), []);
        }

        // Step 7: DROP and recreate every cache table in v34 final shape.
        //         All in one execute_batch so DDL lands atomically.
        conn.execute_batch(
            "DROP TABLE IF EXISTS wallet_addresses;
             CREATE TABLE wallet_addresses (
                wallet_id BLOB NOT NULL,
                address TEXT NOT NULL,
                derivation_path TEXT NOT NULL,
                balance INTEGER,
                path_reference INTEGER NOT NULL,
                path_type INTEGER NOT NULL,
                total_received INTEGER DEFAULT 0,
                PRIMARY KEY (wallet_id, address),
                FOREIGN KEY (wallet_id) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference
                 ON wallet_addresses (path_reference);
             CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type
                 ON wallet_addresses (path_type);

             DROP TABLE IF EXISTS platform_address_balances;
             CREATE TABLE platform_address_balances (
                wallet_id BLOB NOT NULL,
                address TEXT NOT NULL,
                balance INTEGER NOT NULL DEFAULT 0,
                nonce INTEGER NOT NULL DEFAULT 0,
                network TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT 0,
                last_full_sync_balance INTEGER DEFAULT NULL,
                PRIMARY KEY (wallet_id, address, network),
                FOREIGN KEY (wallet_id) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );

             DROP TABLE IF EXISTS shielded_notes;
             CREATE TABLE shielded_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                wallet_id BLOB NOT NULL,
                note_data BLOB NOT NULL,
                position INTEGER NOT NULL,
                cmx BLOB NOT NULL,
                nullifier BLOB NOT NULL,
                block_height INTEGER NOT NULL,
                is_spent INTEGER NOT NULL DEFAULT 0,
                value INTEGER NOT NULL,
                network TEXT NOT NULL,
                UNIQUE(wallet_id, nullifier, network),
                FOREIGN KEY (wallet_id) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_shielded_notes_wallet_network
                 ON shielded_notes (wallet_id, network);

             DROP TABLE IF EXISTS shielded_wallet_meta;
             CREATE TABLE shielded_wallet_meta (
                wallet_id BLOB NOT NULL,
                network TEXT NOT NULL,
                last_nullifier_sync_height INTEGER NOT NULL DEFAULT 0,
                last_nullifier_sync_timestamp INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (wallet_id, network),
                FOREIGN KEY (wallet_id) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );

             DROP TABLE IF EXISTS asset_lock_transaction;
             CREATE TABLE asset_lock_transaction (
                tx_id BLOB NOT NULL,
                output_index INTEGER NOT NULL DEFAULT 0,
                transaction_data BLOB NOT NULL,
                amount INTEGER,
                instant_lock_data BLOB,
                chain_locked_height INTEGER,
                identity_id BLOB,
                identity_id_potentially_in_creation BLOB,
                wallet BLOB NOT NULL,
                network TEXT NOT NULL,
                account_index INTEGER NOT NULL DEFAULT 0,
                funding_type INTEGER NOT NULL DEFAULT 0,
                identity_index INTEGER NOT NULL DEFAULT 0,
                proof_data BLOB,
                PRIMARY KEY (tx_id, output_index),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE SET NULL,
                FOREIGN KEY (identity_id_potentially_in_creation)
                    REFERENCES identity(id) ON DELETE SET NULL,
                FOREIGN KEY (wallet) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_asset_lock_transaction_network
                 ON asset_lock_transaction (network);

             DROP TABLE IF EXISTS identity;
             CREATE TABLE identity (
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
                CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL)
                    OR (wallet IS NULL AND wallet_index IS NULL)),
                FOREIGN KEY (wallet) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_identity_local_network_type
                 ON identity (is_local, network, identity_type);

             DROP TABLE IF EXISTS wallet_transactions;
             CREATE TABLE wallet_transactions (
                wallet_id BLOB NOT NULL,
                account_type BLOB NOT NULL,
                txid BLOB NOT NULL,
                network TEXT NOT NULL,
                record BLOB NOT NULL,
                PRIMARY KEY (wallet_id, account_type, txid, network),
                FOREIGN KEY (wallet_id) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_wallet_transactions_wallet_network
                 ON wallet_transactions (wallet_id, network);

             DROP TABLE IF EXISTS wallet_account_pool_state;
             CREATE TABLE wallet_account_pool_state (
                wallet_id BLOB NOT NULL,
                account_type BLOB NOT NULL,
                pool_type INTEGER NOT NULL,
                highest_used INTEGER,
                highest_generated INTEGER,
                PRIMARY KEY (wallet_id, account_type, pool_type),
                FOREIGN KEY (wallet_id) REFERENCES wallet(wallet_id) ON DELETE CASCADE
             );",
        )
        .migration_err(
            "cache tables",
            "recreate all cache tables with wallet_id FK",
        )?;

        // Step 8: Ensure utxos.is_instant_locked exists. The column was
        //         added in v36; older upgrade paths may lack it. The rows
        //         were cleared in step 6 so we only need the schema change.
        let has_instant_locked: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('utxos') WHERE name='is_instant_locked'",
                [],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .migration_err("utxos", "check for is_instant_locked column")?;
        if !has_instant_locked {
            conn.execute(
                "ALTER TABLE utxos ADD COLUMN is_instant_locked INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .migration_err("utxos", "add is_instant_locked column")?;
        }

        // Step 9: Recreate dashpay_contact_requests with DIP-15 crypto
        //         columns and platform_created_at_ms (added in v38/v39),
        //         plus wallet_id (added in MED #15 fix). DROP+CREATE is
        //         safe because step 6 already deleted all rows.
        conn.execute_batch(
            "DROP TABLE IF EXISTS dashpay_contact_requests;
             CREATE TABLE dashpay_contact_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
                wallet_id BLOB,
                network TEXT NOT NULL,
                to_username TEXT,
                account_label TEXT,
                request_type TEXT NOT NULL CHECK (request_type IN ('sent', 'received')),
                status TEXT DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'rejected', 'expired')),
                created_at INTEGER DEFAULT (unixepoch()),
                responded_at INTEGER,
                expires_at INTEGER,
                sender_key_index INTEGER,
                recipient_key_index INTEGER,
                account_reference INTEGER,
                encrypted_public_key BLOB,
                encrypted_account_label_bytes BLOB,
                auto_accept_proof BLOB,
                core_height_created_at INTEGER,
                platform_created_at_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_contact_requests_from
                 ON dashpay_contact_requests(from_identity_id);
             CREATE INDEX IF NOT EXISTS idx_contact_requests_to
                 ON dashpay_contact_requests(to_identity_id);",
        )
        .migration_err("dashpay_contact_requests", "recreate with DIP-15 columns")?;

        // Step 10: Rebuild `wallet` — preserve data, promote wallet_id to
        //          PRIMARY KEY, drop seed_hash column.
        conn.execute_batch(
            "CREATE TABLE wallet_new (
                wallet_id BLOB NOT NULL PRIMARY KEY,
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
                core_wallet_name TEXT DEFAULT NULL,
                primary_identity_id BLOB,
                last_scanned_identity_index INTEGER
             );
             INSERT INTO wallet_new
             SELECT wallet_id, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password,
                    password_hint, network, confirmed_balance, unconfirmed_balance,
                    total_balance, last_platform_full_sync, last_platform_sync_checkpoint,
                    last_terminal_block, core_wallet_name,
                    NULL, NULL
             FROM wallet;
             DROP TABLE wallet;
             ALTER TABLE wallet_new RENAME TO wallet;
             CREATE INDEX idx_wallet_network ON wallet (network);",
        )
        .migration_err("wallet", "rebuild with wallet_id as PRIMARY KEY")?;

        // Step 11: Drop dashpay_address_mappings (removed in what was v35).
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_dashpay_address_mappings_contact;
             DROP INDEX IF EXISTS idx_dashpay_address_mappings_owner;
             DROP TABLE IF EXISTS dashpay_address_mappings;",
        )
        .migration_err("dashpay_address_mappings", "drop table and indexes")?;

        // Step 12: Clear selected_wallet_hash — old seed_hash value won't
        //          match the new wallet_id key.
        let _ = conn.execute("UPDATE settings SET selected_wallet_hash = NULL", []);

        // Step 13: Re-enable FK enforcement.
        conn.execute_batch("PRAGMA foreign_keys = ON")
            .migration_err("pragma", "foreign_keys=ON after v34 table rebuild")?;

        Ok(())
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

    /// Verify the full schema at DEFAULT_DB_VERSION — all tables and
    /// columns introduced by the current migration ladder (v28 through
    /// the latest). Despite the name, this is the "final schema"
    /// assertion kept under its historical name.
    fn assert_v33_schema(conn: &Connection) {
        // wallet.core_wallet_name (v28)
        assert_column_exists(conn, "wallet", "core_wallet_name");
        // wallet.wallet_id is now the PRIMARY KEY (v34); seed_hash is gone.
        assert_column_exists(conn, "wallet", "wallet_id");
        assert_column_not_exists(conn, "wallet", "seed_hash");
        // v34 added persister-owned wallet-level identity fields.
        assert_column_exists(conn, "wallet", "primary_identity_id");
        assert_column_exists(conn, "wallet", "last_scanned_identity_index");

        // shielded_notes table (v29; v34 renames wallet_seed_hash → wallet_id).
        assert_table_exists(conn, "shielded_notes");
        for col in [
            "wallet_id",
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
        assert_column_not_exists(conn, "shielded_notes", "wallet_seed_hash");

        // shielded_wallet_meta table with last_nullifier_sync_timestamp (v30);
        // v34 renames wallet_seed_hash → wallet_id.
        assert_table_exists(conn, "shielded_wallet_meta");
        assert_column_exists(conn, "shielded_wallet_meta", "wallet_id");
        assert_column_not_exists(conn, "shielded_wallet_meta", "wallet_seed_hash");
        assert_column_exists(
            conn,
            "shielded_wallet_meta",
            "last_nullifier_sync_timestamp",
        );

        // `wallet_transactions.status` was introduced in v30 and
        // removed in v37 when the whole table was recreated with
        // per-account attribution. The v37 assertions
        // below cover the new shape.

        // contact_private_info table (v29)
        assert_table_exists(conn, "contact_private_info");

        // dashpay_contact_requests table (pre-existing, but checked for completeness)
        assert_table_exists(conn, "dashpay_contact_requests");

        // dashpay_address_mappings dropped in v35.
        assert_table_not_exists(conn, "dashpay_address_mappings");

        // wallet_account_pool_state introduced in v36, recreated in
        // v34 with `wallet_id` column (was `seed_hash`).
        assert_table_exists(conn, "wallet_account_pool_state");
        for col in [
            "wallet_id",
            "account_type",
            "pool_type",
            "highest_used",
            "highest_generated",
        ] {
            assert_column_exists(conn, "wallet_account_pool_state", col);
        }
        // utxos.is_instant_locked added in v36.
        assert_column_exists(conn, "utxos", "is_instant_locked");

        // wallet_transactions recreated in v37, then
        // again in v34 with `wallet_id` column (was `seed_hash`).
        assert_table_exists(conn, "wallet_transactions");
        for col in ["wallet_id", "account_type", "txid", "network", "record"] {
            assert_column_exists(conn, "wallet_transactions", col);
        }
        // The old columns (timestamp/height/block_hash/net_amount/fee/
        // label/is_ours/raw_transaction/status) must be gone.
        for old_col in [
            "timestamp",
            "height",
            "block_hash",
            "net_amount",
            "fee",
            "label",
            "is_ours",
            "raw_transaction",
            "status",
        ] {
            assert_column_not_exists(conn, "wallet_transactions", old_col);
        }

        // DIP-15 crypto columns added to `dashpay_contact_requests`
        // in v38 (Item 7), plus platform timestamp column in v39
        // (review M2).
        for col in [
            "sender_key_index",
            "recipient_key_index",
            "account_reference",
            "encrypted_public_key",
            "encrypted_account_label_bytes",
            "auto_accept_proof",
            "core_height_created_at",
            "platform_created_at_ms",
        ] {
            assert_column_exists(conn, "dashpay_contact_requests", col);
        }
    }

    fn assert_column_not_exists(conn: &Connection, table: &str, column: &str) {
        let exists: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{column}'"),
                [],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap();
        assert!(
            !exists,
            "column `{column}` should NOT exist in table `{table}`"
        );
    }

    fn assert_table_not_exists(conn: &Connection, table: &str) {
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap();
        assert!(!exists, "table `{table}` should not exist");
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
        assert_eq!(version, 34);

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

            // FK enforcement may be ON after create_tables' v34 rebuild.
            // Disable it for the cross-table DROP/CREATE dance below.
            conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

            // Remove v28+ tables entirely
            conn.execute("DROP TABLE IF EXISTS shielded_notes", [])
                .unwrap();
            conn.execute("DROP TABLE IF EXISTS shielded_wallet_meta", [])
                .unwrap();
            conn.execute("DROP TABLE IF EXISTS contact_private_info", [])
                .unwrap();

            // Rebuild `wallet` in its v27 shape: seed_hash PK, no wallet_id,
            // no core_wallet_name. The fresh-install DB is empty at this
            // point, so no data preservation needed. (v34 drops the
            // seed_hash column — we're recreating the legacy shape here
            // purely to exercise the migration ladder.)
            conn.execute_batch(
                "DROP TABLE IF EXISTS wallet;
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
                 );",
            )
            .unwrap();

            // Rebuild wallet child tables in their pre-v34 shape so the
            // v33 orphan-cleanup migration (`DELETE FROM ... WHERE
            // seed_hash NOT IN (SELECT seed_hash FROM wallet)`) runs
            // against the schema it was written for.
            conn.execute_batch(
                "DROP TABLE IF EXISTS wallet_addresses;
                 CREATE TABLE wallet_addresses (
                    seed_hash BLOB NOT NULL,
                    address TEXT NOT NULL,
                    derivation_path TEXT NOT NULL,
                    balance INTEGER,
                    path_reference INTEGER NOT NULL,
                    path_type INTEGER NOT NULL,
                    PRIMARY KEY (seed_hash, address),
                    FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                 );
                 DROP TABLE IF EXISTS platform_address_balances;
                 CREATE TABLE platform_address_balances (
                    seed_hash BLOB NOT NULL,
                    address TEXT NOT NULL,
                    balance INTEGER NOT NULL DEFAULT 0,
                    nonce INTEGER NOT NULL DEFAULT 0,
                    network TEXT NOT NULL,
                    updated_at INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (seed_hash, address, network),
                    FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                 );
                 DROP TABLE IF EXISTS asset_lock_transaction;
                 CREATE TABLE asset_lock_transaction (
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
                 );
                 DROP TABLE IF EXISTS identity;
                 CREATE TABLE identity (
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
                 );",
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
        assert_eq!(db.db_schema_version().unwrap(), DEFAULT_DB_VERSION);

        // Verify full v33 schema
        let conn = db.conn.lock().unwrap();
        assert_v33_schema(&conn);
    }

    #[test]
    fn test_v33_migration_with_orphaned_fk_rows() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("orphans.db");
        let db = super::Database::new(&db_file_path).unwrap();

        // Build full schema at current version, then recreate
        // wallet_transactions with the pre-migration-37 schema (which
        // had `timestamp`, `status`, etc.) so we can test the v33 FK
        // cleanup against the table shape it was designed for.
        // Migration 37 restructured wallet_transactions to per-account
        // attribution with a `record` blob, dropping `timestamp`.
        db.create_tables().unwrap();
        db.set_default_version().unwrap();

        let valid_seed_hash = vec![0xAAu8; 32];
        let orphan_seed_hash = vec![0xBBu8; 32];

        {
            let conn = db.conn.lock().unwrap();

            // FK enforcement may be ON after create_tables' v34 rebuild.
            // Disable it for the cross-table DROP/CREATE dance that
            // rebuilds legacy schemas (otherwise child table FKs would
            // trip while we recreate their parent).
            conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

            // Rebuild `wallet` in its pre-v34 shape (seed_hash PK, no
            // wallet_id column) — we're exercising the migration ladder
            // starting from v27, and every INSERT below uses seed_hash
            // as the wallet key. The v34 create_tables output is empty
            // at this point, so there's nothing to preserve.
            conn.execute_batch(
                "DROP TABLE IF EXISTS wallet;
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
                 );",
            )
            .unwrap();

            // Recreate wallet_transactions with the pre-migration-37
            // schema so the INSERT below can use `timestamp` + `status`.
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
                    status INTEGER NOT NULL DEFAULT 2,
                    PRIMARY KEY (seed_hash, txid, network)
                 );",
            )
            .unwrap();

            // Drop wallet_account_pool_state — it was created by v34
            // create_tables with FK to wallet(wallet_id), which would
            // be a dangling FK once we roll wallet back to its v27
            // shape (no wallet_id column). The v36 migration will
            // recreate it with the seed_hash FK appropriate for the
            // intermediate state, then v34 rebuild again.
            conn.execute_batch("DROP TABLE IF EXISTS wallet_account_pool_state")
                .unwrap();

            // Rebuild wallet_addresses + platform_address_balances in
            // their pre-v34 shape (seed_hash column, FK to
            // wallet(seed_hash)) so the INSERTs below and the v33
            // orphan-cleanup migration run against the schema they
            // were written against.
            conn.execute_batch(
                "DROP TABLE IF EXISTS wallet_addresses;
                 CREATE TABLE wallet_addresses (
                    seed_hash BLOB NOT NULL,
                    address TEXT NOT NULL,
                    derivation_path TEXT NOT NULL,
                    balance INTEGER,
                    path_reference INTEGER NOT NULL,
                    path_type INTEGER NOT NULL,
                    total_received INTEGER DEFAULT 0,
                    PRIMARY KEY (seed_hash, address),
                    FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                 );
                 DROP TABLE IF EXISTS platform_address_balances;
                 CREATE TABLE platform_address_balances (
                    seed_hash BLOB NOT NULL,
                    address TEXT NOT NULL,
                    balance INTEGER NOT NULL DEFAULT 0,
                    nonce INTEGER NOT NULL DEFAULT 0,
                    network TEXT NOT NULL,
                    updated_at INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (seed_hash, address, network),
                    FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                 );",
            )
            .unwrap();

            // Rebuild asset_lock_transaction in its pre-v34 shape
            // (FK to wallet(seed_hash)) so the INSERTs validate.
            conn.execute_batch(
                "DROP TABLE IF EXISTS asset_lock_transaction;
                 CREATE TABLE asset_lock_transaction (
                    tx_id BLOB NOT NULL,
                    output_index INTEGER NOT NULL DEFAULT 0,
                    transaction_data BLOB NOT NULL,
                    amount INTEGER,
                    instant_lock_data BLOB,
                    chain_locked_height INTEGER,
                    identity_id BLOB,
                    identity_id_potentially_in_creation BLOB,
                    wallet BLOB NOT NULL,
                    network TEXT NOT NULL,
                    account_index INTEGER NOT NULL DEFAULT 0,
                    funding_type INTEGER NOT NULL DEFAULT 0,
                    identity_index INTEGER NOT NULL DEFAULT 0,
                    proof_data BLOB,
                    PRIMARY KEY (tx_id, output_index),
                    FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE SET NULL,
                    FOREIGN KEY (identity_id_potentially_in_creation) REFERENCES identity(id) ON DELETE SET NULL,
                    FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                 );",
            )
            .unwrap();

            // Rebuild identity in its pre-v34 shape.
            conn.execute_batch(
                "DROP TABLE IF EXISTS identity;
                 CREATE TABLE identity (
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
                 );",
            )
            .unwrap();

            // Insert a real wallet with the old network name. The
            // encrypted_seed blob must be 64 bytes for a no-password
            // wallet so the v34 migration can derive wallet_id from
            // it — otherwise v34 aborts with "locked password wallets
            // require unlock" (wallet_id stays NULL).
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, uses_password, network
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    valid_seed_hash,
                    vec![1u8; 64],
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

            // Recreate wallet without core_wallet_name. The source
            // table already has v27 shape (rebuilt above), so it
            // already lacks core_wallet_name — but rebuild anyway to
            // match the test's original intent.
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

        // Run migration with orphaned FK rows present
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
                "SELECT COUNT(*) FROM wallet_transactions WHERE wallet_id = ?1",
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

        // wallet_transactions: migration 37 drops and recreates the table
        // with per-account attribution (different schema), so both valid
        // and orphan rows from the pre-migration-37 schema are gone. The
        // v33 FK cleanup ran correctly at its migration step; we just
        // can't verify survivors here because of the later DROP.
        let valid_txs: i64 = conn
            .query_row("SELECT COUNT(*) FROM wallet_transactions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            valid_txs, 0,
            "wallet_transactions should be empty after migration 37 table recreation"
        );

        // Wallet itself should have mainnet. v34 drops seed_hash,
        // so we key by the single surviving wallet row.
        let wallet_network: String = conn
            .query_row("SELECT network FROM wallet", [], |row| row.get(0))
            .unwrap();
        assert_eq!(wallet_network, "mainnet");

        // v34 migration DELETEs all cache tables (wallet_addresses,
        // asset_lock_transaction, identity, etc.) as part of the
        // seed_hash → wallet_id cache nuke. So every cache table is
        // empty after migration.
        let all_addrs: i64 = conn
            .query_row("SELECT COUNT(*) FROM wallet_addresses", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            all_addrs, 0,
            "wallet_addresses should be empty after v34 cache nuke"
        );

        let all_locks: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset_lock_transaction", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            all_locks, 0,
            "asset_lock_transaction should be empty after v34 cache nuke"
        );
    }

    /// Test migration from v0.9.0 schema (DB version 5) all the way to current.
    /// This is the exact schema shipped in the v0.9.0 release, with realistic
    /// data including wallets, addresses, identities, and asset locks.
    #[test]
    fn test_migration_from_v090_to_current() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("v090.db");
        let db = super::Database::new(&db_file_path).unwrap();

        {
            let conn = db.conn.lock().unwrap();

            // Exact v0.9.0 schema — copied from git show v0.9.0:src/database/initialization.rs
            conn.execute_batch(
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
                );

                CREATE TABLE IF NOT EXISTS wallet (
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
                );

                CREATE TABLE IF NOT EXISTS wallet_addresses (
                    seed_hash BLOB NOT NULL,
                    address TEXT NOT NULL,
                    derivation_path TEXT NOT NULL,
                    balance INTEGER,
                    path_reference INTEGER NOT NULL,
                    path_type INTEGER NOT NULL,
                    PRIMARY KEY (seed_hash, address),
                    FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference
                    ON wallet_addresses (path_reference);
                CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type
                    ON wallet_addresses (path_type);

                CREATE TABLE IF NOT EXISTS utxos (
                    txid BLOB NOT NULL,
                    vout INTEGER NOT NULL,
                    address TEXT NOT NULL,
                    value INTEGER NOT NULL,
                    script_pubkey BLOB NOT NULL,
                    network TEXT NOT NULL,
                    PRIMARY KEY (txid, vout, network)
                );

                CREATE INDEX IF NOT EXISTS idx_utxos_address ON utxos (address);
                CREATE INDEX IF NOT EXISTS idx_utxos_network ON utxos (network);

                CREATE TABLE IF NOT EXISTS asset_lock_transaction (
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
                );

                CREATE TABLE IF NOT EXISTS identity (
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
                    CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL)
                        OR (wallet IS NULL AND wallet_index IS NULL)),
                    FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_identity_local_network_type
                    ON identity (is_local, network, identity_type);

                CREATE TABLE IF NOT EXISTS contested_name (
                    normalized_contested_name TEXT NOT NULL,
                    locked_votes INTEGER,
                    abstain_votes INTEGER,
                    awarded_to BLOB,
                    end_time INTEGER,
                    locked INTEGER NOT NULL DEFAULT 0,
                    last_updated INTEGER,
                    network TEXT NOT NULL,
                    PRIMARY KEY (normalized_contested_name, network)
                );

                CREATE TABLE IF NOT EXISTS contestant (
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
                    FOREIGN KEY (normalized_contested_name, network)
                        REFERENCES contested_name(normalized_contested_name, network)
                        ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS contract (
                    contract_id BLOB,
                    contract BLOB,
                    name TEXT,
                    network TEXT NOT NULL,
                    PRIMARY KEY (contract_id, network)
                );

                CREATE INDEX IF NOT EXISTS idx_name_network ON contract (name, network);",
            )
            .unwrap();

            // v0.9.0 also created these via separate functions
            // proof_log (v2)
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS proof_log (
                    proof_log_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    proof_log BLOB NOT NULL,
                    proof_log_timestamp INTEGER NOT NULL
                );",
            )
            .unwrap();

            // top_up (v4)
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS top_up (
                    identity_id BLOB NOT NULL,
                    top_up_index INTEGER NOT NULL,
                    amount INTEGER NOT NULL,
                    PRIMARY KEY (identity_id, top_up_index),
                    FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
                );",
            )
            .unwrap();

            // scheduled_votes (v5) — v0.9.0 schema had NO network column
            // and NO FK to identity. The v6 migration handles both.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS scheduled_votes (
                    identity_id BLOB NOT NULL,
                    contested_name TEXT NOT NULL,
                    vote_choice TEXT NOT NULL,
                    time INTEGER NOT NULL,
                    executed INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (identity_id, contested_name)
                );",
            )
            .unwrap();

            // Insert settings at version 5
            conn.execute(
                "INSERT INTO settings (id, network, start_root_screen, database_version)
                 VALUES (1, 'dash', 0, 5)",
                [],
            )
            .unwrap();

            // Insert a wallet with some addresses and an identity
            let seed_hash = vec![0xAAu8; 32];
            conn.execute(
                "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, network)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'test-wallet', 1, 0, 'dash')",
                params![
                    seed_hash,
                    vec![1u8; 64],
                    vec![2u8; 16],
                    vec![3u8; 12],
                    vec![4u8; 33]
                ],
            )
            .unwrap();

            conn.execute(
                "INSERT INTO wallet_addresses (seed_hash, address, derivation_path,
                    balance, path_reference, path_type)
                 VALUES (?1, 'yTestAddr1', 'm/44''/1''/0''/0/0', 50000, 0, 0)",
                params![seed_hash],
            )
            .unwrap();

            let identity_id = vec![0xBBu8; 32];
            conn.execute(
                "INSERT INTO identity (id, is_local, alias, wallet, wallet_index,
                    identity_type, network)
                 VALUES (?1, 1, 'my-identity', ?2, 0, 'user', 'dash')",
                params![identity_id, seed_hash],
            )
            .unwrap();

            conn.execute(
                "INSERT INTO asset_lock_transaction (tx_id, transaction_data, amount,
                    identity_id, wallet, network)
                 VALUES (?1, ?2, 100000, ?3, ?4, 'dash')",
                params![vec![0xCCu8; 32], vec![0u8; 50], identity_id, seed_hash],
            )
            .unwrap();

            conn.execute(
                "INSERT INTO contract (contract_id, contract, name, network)
                 VALUES (?1, ?2, 'dpns', 'dash')",
                params![vec![0xDDu8; 32], vec![0u8; 100]],
            )
            .unwrap();
        }

        assert_eq!(db.db_schema_version().unwrap(), 5);

        // Run full migration from v5 to current
        let result = db.try_perform_migration(5, DEFAULT_DB_VERSION);
        assert!(
            result.is_ok(),
            "migration from v0.9.0 (v5) to v{DEFAULT_DB_VERSION} failed: {:?}",
            result.err()
        );

        assert_eq!(db.db_schema_version().unwrap(), DEFAULT_DB_VERSION);

        let conn = db.conn.lock().unwrap();
        assert_v33_schema(&conn);

        // Verify data survived migration. v34 drops the seed_hash
        // column, so we key by the single surviving wallet row instead
        // of by seed_hash. The v34 migration backfills wallet_id on
        // this no-password wallet, and v34 rebuilds the table with
        // wallet_id as the primary key.
        let wallet_network: String = conn
            .query_row("SELECT network FROM wallet", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            wallet_network, "mainnet",
            "wallet network should be renamed"
        );

        // wallet_addresses should have total_received column (added by v17)
        assert_column_exists(&conn, "wallet_addresses", "total_received");

        // wallet should have balance columns (added by v16)
        assert_column_exists(&conn, "wallet", "confirmed_balance");
        assert_column_exists(&conn, "wallet", "total_balance");

        // wallet should have core_wallet_name (added by v33)
        assert_column_exists(&conn, "wallet", "core_wallet_name");

        // v34 migration DELETEs all cache tables (identity,
        // asset_lock_transaction, etc.) as part of the seed_hash →
        // wallet_id cache nuke. Verify they're empty.
        let id_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM identity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(id_count, 0, "identity table should be empty after v34 nuke");

        let lock_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset_lock_transaction", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            lock_count, 0,
            "asset_lock_transaction should be empty after v34 nuke"
        );
    }

    /// The consolidated v34 migration must drop `dashpay_address_mappings`
    /// cleanly even when the table is populated with real data. The
    /// fresh-install test covers the empty case; this one exercises the
    /// upgrade path with rows and indices in place, which is what real
    /// user databases look like.
    #[test]
    fn test_v34_migration_drops_dashpay_address_mappings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("populated_v33.db");
        let db = super::Database::new(&db_file_path).unwrap();

        // Build a full database at the current version then simulate a
        // v33 state: rebuild the wallet table with seed_hash PK (no
        // wallet_id), recreate dashpay_address_mappings, and set the
        // version back to 33 so the v34 migration path is exercised.
        db.create_tables().unwrap();
        db.set_default_version().unwrap();

        {
            let conn = db.conn.lock().unwrap();

            // Disable FK enforcement while we recreate the wallet table
            // in its pre-v34 shape (seed_hash PK, no wallet_id) so that
            // the v34 migration logic sees the schema it was written for.
            conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
            conn.execute_batch(
                "DROP TABLE IF EXISTS wallet;
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
                    last_terminal_block INTEGER DEFAULT 0,
                    core_wallet_name TEXT DEFAULT NULL
                 );",
            )
            .unwrap();

            // Recreate the table with its original v33-era schema.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS dashpay_address_mappings (
                    address TEXT PRIMARY KEY,
                    owner_identity_id BLOB NOT NULL,
                    contact_identity_id BLOB NOT NULL,
                    address_index INTEGER NOT NULL,
                    created_at INTEGER DEFAULT (unixepoch())
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_dashpay_address_mappings_owner
                 ON dashpay_address_mappings(owner_identity_id)",
                [],
            )
            .unwrap();
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_dashpay_address_mappings_contact
                 ON dashpay_address_mappings(owner_identity_id, contact_identity_id)",
                [],
            )
            .unwrap();

            // Populate with representative rows across two owners
            // and three contacts to exercise the indices.
            let owner_a = vec![0x01u8; 32];
            let owner_b = vec![0x02u8; 32];
            let contact_1 = vec![0x11u8; 32];
            let contact_2 = vec![0x12u8; 32];
            let contact_3 = vec![0x13u8; 32];
            let rows = [
                ("addr_a1_0", &owner_a, &contact_1, 0u32),
                ("addr_a1_1", &owner_a, &contact_1, 1),
                ("addr_a2_0", &owner_a, &contact_2, 0),
                ("addr_b1_0", &owner_b, &contact_1, 0),
                ("addr_b3_0", &owner_b, &contact_3, 0),
            ];
            for (addr, owner, contact, idx) in rows {
                conn.execute(
                    "INSERT INTO dashpay_address_mappings
                        (address, owner_identity_id, contact_identity_id, address_index)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![addr, owner, contact, idx],
                )
                .unwrap();
            }

            // Verify we actually populated the table and indices.
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM dashpay_address_mappings", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 5, "populated table must have 5 rows");

            let index_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='index' AND tbl_name='dashpay_address_mappings'
                     AND name LIKE 'idx_%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(index_count, 2, "populated table must have 2 indices");

            // Downgrade the recorded schema version to 33.
            conn.execute("UPDATE settings SET database_version = 33 WHERE id = 1", [])
                .unwrap();
        }

        assert_eq!(db.db_schema_version().unwrap(), 33);

        // Run the migration. This should DROP the table and its
        // indices without error.
        db.try_perform_migration(33, DEFAULT_DB_VERSION)
            .expect("v33 → v34 migration must succeed on populated dashpay_address_mappings table");

        // Verify final version and that the table + indices are gone.
        assert_eq!(db.db_schema_version().unwrap(), DEFAULT_DB_VERSION);
        let conn = db.conn.lock().unwrap();
        assert_table_not_exists(&conn, "dashpay_address_mappings");
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name LIKE 'idx_dashpay_address_mappings%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            index_count, 0,
            "v35 migration must drop all `dashpay_address_mappings` indices"
        );

        // Cross-check that adjacent DashPay tables survived intact
        // (no FK cascade damage). `dashpay_contact_requests` has
        // indices of the same name prefix pattern so we verify it
        // still has rows/structure.
        assert_table_exists(&conn, "dashpay_contact_requests");
        assert_table_exists(&conn, "dashpay_profiles");
        assert_table_exists(&conn, "dashpay_contacts");
        assert_table_exists(&conn, "dashpay_payments");
    }

    /// TC-C1: v33 → v34 migration must abort with the locked-wallet
    /// sentinel when a password-protected wallet has no `wallet_id`
    /// yet. If this guard ever regresses to silently dropping the
    /// wallet during the schema rebuild, the user's encrypted seed
    /// would be lost with no path to recovery.
    #[test]
    fn test_v33_to_v34_aborts_when_locked_password_wallets_present() {
        use crate::database::wallet::tests::{
            build_epk_bytes, insert_password_wallet, make_v33_db,
        };
        use crate::model::wallet::encryption::encrypt_message;

        let db = make_v33_db();
        let seed = [0x77u8; 64];
        let seed_hash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
        let epk = build_epk_bytes(&seed);
        let (encrypted, salt, nonce) = encrypt_message(&seed, "secret").unwrap();

        insert_password_wallet(&db, &seed_hash, &encrypted, &salt, &nonce, &epk);

        // Backfill runs first (idempotent, adds wallet_id column) —
        // it cannot populate wallet_id for password wallets, so the
        // row stays at wallet_id = NULL.
        db.ensure_wallet_id_column_and_backfill().unwrap();

        // Migration must abort with the sentinel. Asserting the
        // exact substring is load-bearing: WalletMigrationScreen
        // matches on this prefix to route the user to the unlock
        // flow rather than showing a generic migration failure.
        let err = db
            .try_perform_migration(33, 34)
            .expect_err("migration must fail while a locked wallet is present");
        let err_str = err.to_string();
        assert!(
            err_str.contains("locked password wallets require unlock"),
            "unexpected migration error: {err_str}"
        );

        // The wallet row must still exist and its encrypted_seed
        // must be byte-identical — no destructive rebuild ran.
        let (stored_seed, stored_salt, stored_nonce): (Vec<u8>, Vec<u8>, Vec<u8>) = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT encrypted_seed, salt, nonce FROM wallet WHERE seed_hash = ?1",
                params![seed_hash.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("wallet row must still exist after guarded migration")
        };
        assert_eq!(
            stored_seed, encrypted,
            "encrypted_seed must survive the aborted migration unchanged"
        );
        assert_eq!(stored_salt, salt);
        assert_eq!(stored_nonce, nonce);
    }
}
