use crate::database::Database;
use chrono::Utc;
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

pub const DEFAULT_DB_VERSION: u16 = 39;

/// Minimal view of `.env` values the v34 migration needs.
struct V34EnvSnapshot {
    developer_mode: bool,
    /// True if any `{NETWORK}_core_rpc_password` is set and non-empty.
    has_any_rpc_password: bool,
}

/// Parse `<data_dir>/.env` via `dotenvy::from_path_iter` to decide the v34
/// migration outcome. The iterator API does not mutate process env (unlike
/// `Config::load_from` / `dotenvy::from_path_override`), so this is safe to
/// call from inside the migration transaction and from parallel test runs.
fn read_env_file_for_v34_migration(data_dir: &Path) -> std::io::Result<V34EnvSnapshot> {
    let env_path = data_dir.join(".env");
    let iter = dotenvy::from_path_iter(&env_path).map_err(std::io::Error::other)?;

    let mut developer_mode = false;
    let mut has_any_rpc_password = false;
    for item in iter {
        let (key, value) = item.map_err(std::io::Error::other)?;
        if key.eq_ignore_ascii_case("DEVELOPER_MODE") {
            developer_mode = matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes");
        } else if key.to_ascii_lowercase().ends_with("core_rpc_password") && !value.is_empty() {
            has_any_rpc_password = true;
        }
    }

    Ok(V34EnvSnapshot {
        developer_mode,
        has_any_rpc_password,
    })
}

/// Migration helper: replace the legacy `devnet:` / `local` network
/// labels with the modern `devnet` / `regtest` spellings across every
/// table that still carries a `network` column.
///
/// Pre-C7 this lived as `Database::fix_identity_devnet_network_name` in
/// `database/identities.rs`. The file is gone — the helper is now a free
/// function so it can be called from the migration ladder without
/// reintroducing a domain-specific module.
fn fix_devnet_network_name_in_legacy_tables(conn: &Connection) -> rusqlite::Result<()> {
    // `scheduled_votes` lingers on pre-C5 installs but is no longer
    // created on fresh installs; the per-table existence check below
    // skips it transparently on the new schema.
    const TABLES: [&str; 11] = [
        "asset_lock_transaction",
        "contestant",
        "contested_name",
        "contract",
        "identity",
        "identity_token_balances",
        "scheduled_votes",
        "settings",
        "token",
        "utxos",
        "wallet",
    ];

    for t in TABLES {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [t],
            |row| row.get(0),
        )?;
        if !exists {
            continue;
        }
        // Pre-C5 `scheduled_votes` (v5 schema) had no `network` column;
        // the v6 schema upgrade that added it was unwired in C5, so
        // legacy DBs that never ran a later migration still have the
        // old shape. Skip the UPDATE on those — the table is orphaned
        // either way.
        let has_network: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name='network'",
            [t],
            |row| row.get::<_, i32>(0).map(|c| c > 0),
        )?;
        if !has_network {
            continue;
        }
        conn.execute(
            &format!("UPDATE {t} SET network = 'devnet' WHERE network = 'devnet:'"),
            [],
        )?;
        conn.execute(
            &format!("UPDATE {t} SET network = 'regtest' WHERE network = 'local'"),
            [],
        )?;
    }

    tracing::debug!("Updated network names in database");
    Ok(())
}

/// Detect whether this on-disk DB carries any legacy DET wallet state.
///
/// Returns true when any of the canary legacy tables (`wallet`,
/// `wallet_addresses`, `single_key_wallet`, `utxos`, `shielded_notes`)
/// exists and contains at least one row. Truly-fresh installs — empty
/// `data.db` or DB without those tables — return false, so the gated
/// CREATE TABLE statements in [`Database::create_tables`] are skipped
/// and the wallet state lives entirely in `platform-wallet.sqlite`.
///
/// The check is best-effort: any sqlite read error is treated as
/// "no legacy detected" so a malformed/locked DB does not accidentally
/// recreate the dormant schema on a fresh install.
fn legacy_detected(conn: &Connection) -> bool {
    const TARGETS: [&str; 5] = [
        "wallet",
        "wallet_addresses",
        "single_key_wallet",
        "utxos",
        "shielded_notes",
    ];
    for table in TARGETS {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !exists {
            continue;
        }
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        if count > 0 {
            return true;
        }
    }
    false
}

impl Database {
    pub fn initialize(&self, db_file_path: &Path) -> rusqlite::Result<()> {
        // First, ensure all required columns exist in tables that may have been
        // created with an older schema. This must happen before any queries that
        // depend on these columns (like db_schema_version which needs database_version).
        {
            let conn = self.locked_conn();
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
            // Detect legacy DET wallet state on the same DB file. Truly-fresh
            // installs skip the wallet/utxos/single_key_wallet/wallet_transactions/
            // shielded_notes/shielded_wallet_meta CREATE TABLE statements — that
            // state now lives in `platform-wallet.sqlite`. Pre-existing installs
            // (settings row missing but wallet rows present, an unusual but
            // possible recovery shape) still get the legacy tables so the
            // migration ladder has something to upgrade.
            let include_legacy = {
                let conn = self.locked_conn();
                legacy_detected(&conn)
            };
            self.create_tables(include_legacy)?;
            self.set_default_version()?;
        } else {
            self.run_consistency_checks();

            let current_version = self.db_schema_version()?;
            if current_version != DEFAULT_DB_VERSION {
                self.backup_db(db_file_path)?;
                // Migrations may need to read `.env` from the data directory
                // (see v34 for one such arm). The DB file typically lives at
                // `<data_dir>/<db_file>`, so its parent is the data dir.
                let data_dir = db_file_path.parent();
                if let Err(e) =
                    self.try_perform_migration(current_version, DEFAULT_DB_VERSION, data_dir)
                {
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

    fn apply_version_changes(
        &self,
        version: u16,
        tx: &Connection,
        data_dir: Option<&Path>,
    ) -> Result<(), MigrationError> {
        match version {
            39 => {
                Self::initialize_forgotten_identities_table(tx).migration_err(
                    "forgotten_identities",
                    "v39: create forgotten identity markers",
                )?;
            }
            38 => {
                // Drop the retired `core_backend_mode` settings column. The
                // RPC/SPV backend selector it held was unwired in C3 (user
                // prefs moved to the upstream k/v store) and chain sync is
                // SPV-only now, so the column is permanent dead weight.
                // Existence-guarded and idempotent; mutates ONLY the settings
                // table and preserves every other column and value.
                self.drop_core_backend_mode_column(tx)
                    .migration_err("settings", "v38: drop core_backend_mode column")?;
            }
            37 => {
                // Retire DET's home-grown shielded subsystem: the upstream
                // `platform-wallet` coordinator owns all Orchard state now.
                // No released build ever persisted shielded rows (v0.9.3 ships
                // zero shielded code), so dropping the dead tables is safe and
                // loses no user data. Existence-guarded via IF EXISTS.
                tx.execute_batch(
                    "DROP TABLE IF EXISTS shielded_notes;\n\
                     DROP TABLE IF EXISTS shielded_wallet_meta;",
                )
                .migration_err("shielded_notes", "v37: drop dead shielded tables")?;
            }
            36 => {
                // Drop the orphaned `dashpay_dip14_quarantine_active`
                // settings column left behind by an early P3a build and the
                // withdrawn quarantine apparatus. Existence-guarded and
                // idempotent; mutates ONLY the settings table.
                self.drop_dead_settings_columns(tx)
                    .migration_err("settings", "v36: drop dead settings columns")?;
            }
            35 => {
                // Platform-wallet migration scaffolding (Stage A + Stage B)
                // has been removed. v34 users advance through this arm with
                // no schema or marker writes — the data.db file is left
                // dormant and the future migration tool covers the wallet
                // seeds. The bare version bump preserves the ladder so the
                // v36+ arms keep running.
                let _ = tx;
            }
            34 => {
                // SPV is now the default Core-level backend. Users who have a
                // configured local Dash Core node (Expert mode + at least one
                // network's `core_rpc_password` set) keep their existing mode;
                // everyone else is pinned to SPV. No new persisted flag — the
                // DB version bump itself is the one-shot gate.
                let migrate_to_spv = match data_dir {
                    Some(dir) => match read_env_file_for_v34_migration(dir) {
                        Ok(env_snapshot) => {
                            !(env_snapshot.developer_mode && env_snapshot.has_any_rpc_password)
                        }
                        Err(e) => {
                            tracing::warn!(
                                "v34 migration: .env unreadable ({e}); defaulting to SPV"
                            );
                            true
                        }
                    },
                    // Tests or headless contexts without a data dir: safest default is SPV.
                    None => true,
                };

                // The `core_backend_mode` column itself was unwired in C3
                // (user prefs moved to the upstream k/v store) — only a DB
                // that was created before that change still has it. Guard
                // the legacy update so synthetic v33 DBs created from the
                // post-C3 schema are not rejected.
                let has_legacy_column: bool = tx
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='core_backend_mode'",
                        [],
                        |row| row.get::<_, i32>(0).map(|c| c > 0),
                    )
                    .unwrap_or(false);
                if has_legacy_column {
                    if migrate_to_spv {
                        tx.execute("UPDATE settings SET core_backend_mode = 1 WHERE id = 1", [])
                            .migration_err("settings", "v34: pin SPV as default backend")?;
                    } else {
                        tracing::info!(
                            "v34 migration: preserving existing core_backend_mode \
                             (local Dash Core node configured)"
                        );
                    }
                } else {
                    tracing::debug!(
                        "v34 migration: legacy core_backend_mode column absent — no-op"
                    );
                }
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
                // Legacy v33 also created `contact_private_info` — the
                // table was retired in D4d (private memos now live in the
                // per-network k/v sidecar). Pre-D4d installs keep the
                // dormant row set; fresh installs never create the table.
                //
                // The old `shielded_notes` / `shielded_wallet_meta` tables are
                // no longer created here: DET's shielded subsystem was retired
                // and the v37 migration drops them. A DB stepping through v33
                // simply skips the create; v37 then drops any legacy copies.
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
                // Legacy v13 created the DashPay tables (dashpay_profiles,
                // dashpay_contacts, dashpay_contact_requests,
                // dashpay_payments, dashpay_contact_address_indices,
                // dashpay_address_mappings). All six were retired in D4d
                // — upstream `ManagedIdentity` and the k/v sidecar now own
                // the state. Pre-D4d installs keep the dormant rows; fresh
                // installs never reach this arm because they jump to
                // `DEFAULT_DB_VERSION` directly.
            }
            12 => {
                self.add_disable_zmq_column(tx)
                    .migration_err("settings", "add disable_zmq column")?;
            }
            11 => {
                // Rename `is_in_creation` to `status` on the legacy
                // identity table. Pre-C7 this method lived on `Database`;
                // it is inlined here now that `database/identities.rs`
                // is gone.
                tx.execute(
                    "ALTER TABLE identity RENAME COLUMN is_in_creation TO status",
                    [],
                )
                .migration_err("identity", "rename is_in_creation to status")?;
                tx.execute("UPDATE identity SET status = 2 WHERE status = 0", [])
                    .migration_err("identity", "remap is_in_creation values")?;
            }
            10 => {
                self.add_theme_preference_column(tx)
                    .migration_err("settings", "add theme_preference column")?;
            }
            9 => {
                // The `identity` table is kept (empty CREATE) so legacy
                // rows can still be cleaned up here. Fresh installs have
                // an empty table — the DELETE is a no-op.
                tx.execute(
                    "DELETE FROM identity WHERE (network LIKE 'devnet%' OR network = 'regtest')",
                    [],
                )
                .migration_err("identity", "delete devnet/regtest identities")?;
                // The `token` table was unwired in C7 — fresh installs
                // do not create it. Legacy DBs still have it and pay
                // the cost of the DELETE once.
                if self
                    .table_exists(tx, "token")
                    .migration_err("token", "check table existence")?
                {
                    tx.execute(
                        "DELETE FROM token WHERE network LIKE 'devnet%' OR network = 'regtest'",
                        [],
                    )
                    .migration_err("token", "delete devnet/regtest tokens")?;
                }
                // `asset_lock_transaction` was unwired — fresh installs do
                // not create the table, so the devnet/regtest sweep is
                // skipped when the table is absent. Legacy DBs still have
                // it and pay the cost of the DELETE once.
                if self
                    .table_exists(tx, "asset_lock_transaction")
                    .migration_err("asset_lock_transaction", "check table existence")?
                {
                    tx.execute(
                        "DELETE FROM asset_lock_transaction \
                         WHERE network LIKE 'devnet%' OR network = 'regtest'",
                        [],
                    )
                    .migration_err(
                        "asset_lock_transaction",
                        "clear devnet/regtest asset lock identity IDs",
                    )?;
                }
                // The `contract` table was unwired in C6 — on fresh
                // installs it does not exist, so we skip the devnet/regtest
                // sweep when the table is absent. Legacy DBs still have it
                // and pay the cost of the DELETE once.
                if self
                    .table_exists(tx, "contract")
                    .migration_err("contract", "check table existence")?
                {
                    tx.execute(
                        "DELETE FROM contract WHERE network LIKE 'devnet%' OR network = 'regtest'",
                        [],
                    )
                    .migration_err("contract", "delete devnet/regtest contracts")?;
                }
                fix_devnet_network_name_in_legacy_tables(tx)
                    .migration_err("identity", "fix devnet network name")?;
            }
            8 => {
                // The `contract` table was unwired in C6 — on fresh
                // installs it does not exist, so we skip the column
                // rename when the table is absent.
                if self
                    .table_exists(tx, "contract")
                    .migration_err("contract", "check table existence")?
                {
                    let name_column_exists: bool = tx
                        .query_row(
                            "SELECT COUNT(*) FROM pragma_table_info('contract') WHERE name='name'",
                            [],
                            |row| Ok(row.get::<_, i64>(0)? > 0),
                        )
                        .migration_err("contract", "inspect table columns")?;
                    if name_column_exists {
                        tx.execute("ALTER TABLE contract RENAME COLUMN name TO alias", [])
                            .migration_err("contract", "rename name to alias")?;
                    }
                }
            }
            7 => {
                // `asset_lock_transaction` was unwired — fresh installs do
                // not create the table, so the FK migration is skipped
                // when absent. Legacy DBs still rebuild the FK once.
                if self
                    .table_exists(tx, "asset_lock_transaction")
                    .migration_err("asset_lock_transaction", "check table existence")?
                {
                    Self::migrate_asset_lock_fk_to_set_null(tx)
                        .migration_err("asset_lock_transaction", "migrate FK to SET NULL")?;
                }
            }
            6 => {
                // Pre-C5: `scheduled_votes` schema upgrade. The table is no longer
                // created/managed; pre-C5 installs keep the orphaned table dormant.
                //
                // Pre-C7: this arm used to (re)create the `token`,
                // `identity_token_balances`, `identity_order` and
                // `token_order` tables. All four were unwired in C7
                // (tokens + identity registry moved to the per-network
                // k/v store). Fresh installs no longer create them and
                // legacy installs keep the dormant rows.
                let _ = tx;
            }
            5 => {
                // Pre-C5: `scheduled_votes` create. Removed in C5; the table
                // is no longer created on fresh installs.
            }
            4 => {
                // Pre-C5: `top_up` create. Removed in C5.
            }
            3 => {
                self.add_custom_dash_qt_columns(tx)
                    .migration_err("settings", "add custom dash_qt columns")?;
            }
            2 => {
                // Pre-C5: `proof_log` create. Removed in C5; proof errors now
                // surface via structured tracing only.
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
        data_dir: Option<&Path>,
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
                let mut conn = self.locked_conn();

                for version in (original_version + 1)..=to_version {
                    tracing::debug!("Applying migration v{version}");
                    let tx = conn.transaction().map_err(|e| MigrationError {
                        table: None,
                        details: format!("v{version}: begin transaction"),
                        source: e,
                    })?;
                    let result = self
                        .apply_version_changes(version, &tx, data_dir)
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
        let conn = self.locked_conn();

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

    /// Reads the saved data version as SQLite stores it.
    ///
    /// `None` means the settings table is absent. An existing table without its
    /// singleton row is version `0`, which predates every supported migration.
    pub(crate) fn stored_data_version(&self) -> rusqlite::Result<Option<i64>> {
        let conn = self.locked_conn();
        if !self.table_exists(&conn, "settings")? {
            return Ok(None);
        }

        match conn.query_row(
            "SELECT database_version FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        ) {
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Some(0)),
            result => result.map(Some),
        }
    }

    /// Checks the version used by the writable legacy migration ladder.
    ///
    /// Versions above the current default are returned unchanged so callers can
    /// detect data written by a newer build.
    fn db_schema_version(&self) -> rusqlite::Result<u16> {
        let version = self.stored_data_version()?.unwrap_or(0);
        u16::try_from(version).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, version))
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
    ///
    /// `include_legacy` controls whether the wallet-family tables
    /// (`wallet`, `wallet_addresses`, `utxos`, `single_key_wallet`,
    /// `wallet_transactions`, `shielded_notes`, `shielded_wallet_meta`)
    /// are created. Truly-fresh DET installs pass `false` so these dormant
    /// schemas never appear in `data.db`; legacy installs and the migration
    /// ladder still pass `true` so upgrade arms keep working. Always-present
    /// tables (`settings`, `forgotten_identities`,
    /// `platform_address_balances`) are created regardless.
    pub(crate) fn create_tables(&self, include_legacy: bool) -> rusqlite::Result<()> {
        let conn = self.locked_conn();
        // Create the settings table.
        //
        // User-preference columns (network, theme, ZMQ, evonode tools, …)
        // were unwired in C3 of the data.db unwire and moved to the
        // upstream k/v store. The selected-wallet pointer
        // (`selected_wallet_hash`, `selected_single_key_hash`) was
        // unwired in C4 and moved to the per-network wallet k/v store.
        // What survives here is the migration runner's version counter.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            database_version INTEGER NOT NULL
        )",
            [],
        )?;
        Self::initialize_forgotten_identities_table(&conn)?;

        if include_legacy {
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
        }

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

        if include_legacy {
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
        }

        // `asset_lock_transaction` was unwired entirely — fresh installs
        // no longer create the table. Legacy installs keep the dormant
        // rows; the migration tool drains them via git history.

        if include_legacy {
            // The local identity registry lives in the per-network wallet
            // k/v store. The legacy `identity` table is created only for
            // legacy installs and tests. Its sole live reader, `get_wallets`
            // in `database/wallet.rs`, is a legacy-only path that errors on
            // the missing `wallet` table before reaching `identity` on a
            // fresh install, so omitting the table here is safe.
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
        }

        // contested_name / contestant tables removed in C6 — DPNS contest
        // cache now lives in the per-network wallet k/v store. Legacy
        // installs keep the dormant rows; fresh installs never create the
        // tables.

        // The user-contract registry moved to the per-network wallet k/v
        // store in C6. The `token` table was removed in C7, so nothing
        // references `contract` any more — the empty placeholder is no
        // longer created on fresh installs. Legacy installs keep the
        // dormant rows.

        // Token registry, per-identity token balances, identity ordering
        // and token ordering all moved to the per-network wallet k/v
        // store in C7. Nothing else references these tables, so they
        // are no longer created on fresh installs. Legacy installs keep
        // the dormant rows.

        // DashPay tables and `contact_private_info` were retired in D4d.
        // Upstream `ManagedIdentity` now owns contact / profile / payment
        // state, and a per-network k/v sidecar owns DET-only overlays
        // (private memo, blocked / rejected markers, timestamps, address
        // index, address mapping). Fresh installs no longer create the
        // tables; legacy installs keep the dormant rows.

        if include_legacy {
            // Initialize single key wallet table
            self.initialize_single_key_wallet_table(&conn)?;

            // The shielded pool tables (`shielded_notes` /
            // `shielded_wallet_meta`) are intentionally NOT created: DET's
            // shielded subsystem was retired and the upstream coordinator owns
            // all Orchard state. The v37 migration drops any legacy copies.
        }

        Ok(())
    }

    /// Ensures that the default database version is set in the settings table.
    pub(crate) fn set_default_version(&self) -> rusqlite::Result<()> {
        // TODO: Discuss migration approach with the team.
        // Suggested approach:
        // we don't change `create_tables`, we just add migrations
        // and rely on it to bring the database to the latest version.
        // It means that we put `1` in the `settings` table as the initial version
        self.set_db_version(DEFAULT_DB_VERSION)
    }
    fn set_db_version(&self, version: u16) -> rusqlite::Result<()> {
        // User-preference columns moved to the upstream k/v store (C3).
        // Initialising the row only seeds the singleton primary key and
        // the migration runner's version counter — everything else lives
        // in `det:settings:v1` now.
        self.execute(
            "INSERT INTO settings (id, database_version)
             VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET database_version = excluded.database_version",
            params![version],
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
        // The `token` and `identity_token_balances` tables were unwired
        // in C7 — fresh installs do not create them, so we skip the
        // index creation when the underlying table is absent. Legacy
        // installs still have the tables and pick up the index.
        if self.table_exists(conn, "token")? {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_token_network ON token (network)",
                [],
            )?;
        }
        if self.table_exists(conn, "identity_token_balances")? {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_identity_token_balances_network ON identity_token_balances (network)",
                [],
            )?;
        }
        // The `scheduled_votes` table was unwired in C5; the index it carried
        // is no longer maintained on fresh installs. Existing pre-C5 installs
        // keep the orphaned table and its index dormant. Older pre-v6 shapes
        // also lack the `network` column entirely — skip in that case.
        if self.table_exists(conn, "scheduled_votes")? {
            let has_network: bool = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('scheduled_votes') WHERE name='network'",
                [],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )?;
            if has_network {
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_scheduled_votes_network ON scheduled_votes (network)",
                    [],
                )?;
            }
        }
        // The `asset_lock_transaction` table was unwired — fresh installs
        // do not create it, so we skip the index when the table is absent.
        // Legacy installs still have the table and pick up the index.
        if self.table_exists(conn, "asset_lock_transaction")? {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_asset_lock_transaction_network ON asset_lock_transaction (network)",
                [],
            )?;
        }
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

    // DET's shielded subsystem was retired (Phase D); the old shielded table
    // helpers and the `database::shielded` module were deleted. The v37
    // migration drops any legacy `shielded_notes` / `shielded_wallet_meta`.

    /// Rebuild legacy `asset_lock_transaction` rows so both `identity_id`
    /// FKs use `ON DELETE SET NULL` instead of `ON DELETE CASCADE`.
    ///
    /// Inlined here when the `database/asset_lock_transaction` module was
    /// deleted; only reachable from the v7 migration arm under a
    /// `table_exists` guard. Safe to run multiple times: if the table
    /// already has the correct FKs it exits early.
    fn migrate_asset_lock_fk_to_set_null(conn: &Connection) -> rusqlite::Result<()> {
        {
            let mut pragma = conn.prepare("PRAGMA foreign_key_list('asset_lock_transaction')")?;
            let fk_rows = pragma
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(2)?, // table
                        row.get::<_, String>(6)?, // on_delete action
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let needs_migration = fk_rows
                .iter()
                .filter(|(tbl, _)| tbl == "identity")
                .any(|(_, action)| action.to_uppercase() != "SET NULL");

            if !needs_migration {
                return Ok(());
            }
        }

        conn.execute("PRAGMA foreign_keys = OFF", [])?;

        conn.execute(
            "ALTER TABLE asset_lock_transaction RENAME TO asset_lock_transaction_old",
            [],
        )?;

        conn.execute(
            "CREATE TABLE asset_lock_transaction (
                tx_id BLOB PRIMARY KEY,
                transaction_data BLOB NOT NULL,
                amount INTEGER,
                instant_lock_data BLOB,
                chain_locked_height INTEGER,
                identity_id BLOB,
                identity_id_potentially_in_creation BLOB,
                wallet BLOB NOT NULL,
                network TEXT NOT NULL,
                FOREIGN KEY (identity_id)
                    REFERENCES identity(id) ON DELETE SET NULL,
                FOREIGN KEY (identity_id_potentially_in_creation)
                    REFERENCES identity(id) ON DELETE SET NULL,
                FOREIGN KEY (wallet)
                    REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "INSERT INTO asset_lock_transaction
              (tx_id, transaction_data, amount, instant_lock_data,
               chain_locked_height, identity_id, identity_id_potentially_in_creation,
               wallet, network)
             SELECT tx_id, transaction_data, amount, instant_lock_data,
                    chain_locked_height, identity_id,
                    identity_id_potentially_in_creation, wallet, network
             FROM asset_lock_transaction_old",
            [],
        )?;

        conn.execute("DROP TABLE asset_lock_transaction_old", [])?;

        conn.execute("PRAGMA foreign_keys = ON", [])?;

        Ok(())
    }

    /// Remove orphaned child rows left behind when parent rows were deleted
    /// while FK enforcement was off (system SQLite before bundled build).
    /// Bundled SQLite enables FK checks by default, so any subsequent UPDATE
    /// on these rows triggers re-validation and fails. Covers all FK
    /// relationships in the schema: wallet→children, identity→children,
    /// token→children, contract→children, contested_name→children.
    fn clean_orphaned_fk_rows(&self, conn: &Connection) -> Result<(), MigrationError> {
        // --- CASCADE children of wallet(seed_hash) ---
        let wallet_fk_delete: &[(&str, &str)] = &[
            ("wallet_addresses", "seed_hash"),
            ("wallet_transactions", "seed_hash"),
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
    pub(crate) fn table_exists(&self, conn: &Connection, table: &str) -> rusqlite::Result<bool> {
        crate::database::table_exists(conn, table)
    }

    /// Migration 29: rename network value `"dash"` to `"mainnet"` in all tables.
    ///
    /// Upstream `dashcore` renamed `Network::Dash` to `Network::Mainnet`,
    /// changing the `Display`/`FromStr` representation. This migration updates
    /// every table that stores the network as a string column.
    fn rename_network_dash_to_mainnet(&self, conn: &Connection) -> Result<(), MigrationError> {
        // The `settings` table dropped its `network` column in C3 — the
        // active-network pointer now lives in `AppSettings` in the
        // upstream k/v store. Every other domain table still keys rows
        // by a `network` string and needs the rename.
        let tables = [
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
            // `scheduled_votes` was unwired in C5; on fresh installs the
            // table is absent (skip) and on legacy v5-shaped installs the
            // column may be missing (also skip). The table is orphaned
            // either way once `scheduled_votes` lives in k/v.
            let exists = self
                .table_exists(conn, table)
                .migration_err(table, "check table existence")?;
            if !exists {
                continue;
            }
            let has_network: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name='network'",
                    [table],
                    |row| row.get::<_, i32>(0).map(|c| c > 0),
                )
                .migration_err(table, "check for network column")?;
            if !has_network {
                continue;
            }
            conn.execute(
                &format!("UPDATE {table} SET network = 'mainnet' WHERE network = 'dash'"),
                [],
            )
            .migration_err(table, "rename network dash -> mainnet")?;
        }
        // The legacy `settings.network` column may still exist in DBs that
        // pre-date C3. Update it defensively — `UPDATE` against a missing
        // column would error, so we gate on existence.
        let settings_has_network: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='network'",
                [],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap_or(false);
        if settings_has_network {
            conn.execute(
                "UPDATE settings SET network = 'mainnet' WHERE network = 'dash'",
                [],
            )
            .migration_err("settings", "rename network dash -> mainnet")?;
        }
        Ok(())
    }

    /// Run database consistency checks on startup.
    /// Non-fatal: logs warnings for any issues found but does not fail.
    fn run_consistency_checks(&self) {
        const MAX_ISSUES_TO_LOG: usize = 20;

        let conn = self.locked_conn();

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

        // The shielded tables were retired in Phase D: v33 no longer creates
        // them and the v37 migration drops any legacy copies, so after a full
        // migration they must be absent.
        assert_table_absent(conn, "shielded_notes");
        assert_table_absent(conn, "shielded_wallet_meta");

        // wallet_transactions.status (v30)
        assert_column_exists(conn, "wallet_transactions", "status");

        // contact_private_info and dashpay_contact_requests were retired
        // in D4d — fresh installs no longer create them. Pre-D4d installs
        // keep the dormant rows, but the fresh-install path tested here
        // intentionally skips them.
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

    // Given a database whose on-disk schema version is higher than the
    // build supports,
    // when I call `try_perform_migration`,
    // then it returns an error and leaves the persisted version untouched
    // (no row is mutated).
    //
    // Originally this lane simulated a v9 mid-flight failure by dropping
    // `asset_lock_transaction`; that module is gone and every surviving
    // migration arm is idempotent + `table_exists`-guarded, so we can no
    // longer reliably provoke an intra-arm failure without contrived
    // fixtures. The `Greater`-version refusal is the only failure path
    // that is stable across the consolidated ladder, and it exercises the
    // same "error returned, DB untouched" contract.
    #[test]
    fn test_migration_failure_rolls_back() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("test_data.db");
        let db = super::Database::new(&db_file_path).unwrap();

        const NETWORK: &str = "regtest";

        db.create_tables(true).unwrap();
        db.set_default_version().unwrap();

        // Seed an identity so we can prove no DB mutation occurred.
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO identity (id, is_local, alias, network) VALUES (?, ?, ?, ?)",
            rusqlite::params![vec![1u8; 32], 1, "test_identity", NETWORK],
        )
        .expect("insert test identity");
        drop(conn);

        // Pin the version one past what this build supports.
        let future_version = DEFAULT_DB_VERSION + 1;
        db.set_db_version(future_version).unwrap();

        // The `Greater` arm must refuse and not touch the DB.
        let result = db.try_perform_migration(future_version, DEFAULT_DB_VERSION, None);
        assert!(result.is_err(), "expected refusal");
        println!("Migration failed as expected: {}", result.unwrap_err());

        let version: u16 = db.db_schema_version().unwrap();
        assert_eq!(version, future_version, "version must be untouched");

        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM identity WHERE network = ?",
                params![NETWORK],
                |row| row.get(0),
            )
            .expect("count identities");
        assert_eq!(
            count, 1,
            "Identity must survive the rejected migration attempt"
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

        // Post-T-DEV-01: truly-fresh installs no longer create the
        // wallet-family tables — those live in `platform-wallet.sqlite`
        // now. `assert_v33_schema` only applies to upgrade-replay DBs,
        // so it has moved to `test_v33_migration_from_v27`. Here we
        // only need to confirm the settings row is in place.
    }

    #[test]
    fn test_v33_migration_from_v27() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_file_path = temp_dir.path().join("v27.db");
        let db = super::Database::new(&db_file_path).unwrap();

        // Build a full database then strip v28+ additions to simulate v27.
        db.create_tables(true).unwrap();
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
        let result = db.try_perform_migration(27, DEFAULT_DB_VERSION, None);
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

        // Build full schema at current version
        db.create_tables(true).unwrap();
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

            // The `asset_lock_transaction` table is no longer created on
            // fresh installs, but this test exercises the legacy-shape
            // orphan cleanup that v33 performs on installs that still
            // carry it. Recreate the legacy schema manually so the v27
            // synthetic fixture matches reality for pre-unwire DBs.
            conn.execute_batch(
                "CREATE TABLE asset_lock_transaction (
                    tx_id BLOB PRIMARY KEY,
                    transaction_data BLOB NOT NULL,
                    amount INTEGER,
                    instant_lock_data BLOB,
                    chain_locked_height INTEGER,
                    identity_id BLOB,
                    identity_id_potentially_in_creation BLOB,
                    wallet BLOB NOT NULL,
                    network TEXT NOT NULL,
                    FOREIGN KEY (identity_id)
                        REFERENCES identity(id) ON DELETE SET NULL,
                    FOREIGN KEY (identity_id_potentially_in_creation)
                        REFERENCES identity(id) ON DELETE SET NULL,
                    FOREIGN KEY (wallet)
                        REFERENCES wallet(seed_hash) ON DELETE CASCADE
                );",
            )
            .unwrap();

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
            // Remove shielded tables — Phase D drops them (v37); not recreated
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

        // Run migration with orphaned FK rows present
        let result = db.try_perform_migration(27, DEFAULT_DB_VERSION, None);
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
        // Phase D retired the shielded tables — v33 no longer creates them and
        // v37 drops any legacy copies, so they must be absent post-migration.
        assert_table_absent(&conn, "shielded_notes");
        assert_table_absent(&conn, "shielded_wallet_meta");

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

            // A queued DPNS vote and a top-up: the non-wallet rows the unwire
            // left behind. The v0.9.0 `scheduled_votes` shape has no `network`
            // column, so these also cover the pre-v6 reader path.
            conn.execute(
                "INSERT INTO scheduled_votes
                    (identity_id, contested_name, vote_choice, time, executed)
                 VALUES (?1, 'quantum', 'Lock', 1700000000, 0)",
                params![identity_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO top_up (identity_id, top_up_index, amount)
                 VALUES (?1, 0, 100000)",
                params![identity_id],
            )
            .unwrap();
        }

        assert_eq!(db.db_schema_version().unwrap(), 5);

        // Run full migration from v5 to current
        let result = db.try_perform_migration(5, DEFAULT_DB_VERSION, None);
        assert!(
            result.is_ok(),
            "migration from v0.9.0 (v5) to v{DEFAULT_DB_VERSION} failed: {:?}",
            result.err()
        );

        assert_eq!(db.db_schema_version().unwrap(), DEFAULT_DB_VERSION);

        let conn = db.conn.lock().unwrap();
        assert_v33_schema(&conn);

        // Verify data survived migration
        let wallet_network: String = conn
            .query_row(
                "SELECT network FROM wallet WHERE seed_hash = ?1",
                params![vec![0xAAu8; 32]],
                |row| row.get(0),
            )
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

        // Identity should survive with network renamed
        let id_network: String = conn
            .query_row(
                "SELECT network FROM identity WHERE id = ?1",
                params![vec![0xBBu8; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id_network, "mainnet");

        // Asset lock should survive with identity_id intact
        let lock_identity: Option<Vec<u8>> = conn
            .query_row(
                "SELECT identity_id FROM asset_lock_transaction WHERE tx_id = ?1",
                params![vec![0xCCu8; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lock_identity, Some(vec![0xBBu8; 32]));

        // The ladder must leave the non-wallet rows readable for the import
        // that carries them into the k/v store. A dropped scheduled vote is a
        // missed vote window, so this is asserted end-to-end against the real
        // post-migration schema rather than a hand-built fixture.
        use crate::database::legacy_import::{
            read_app_settings, read_scheduled_votes, read_top_ups,
        };
        use dash_sdk::dpp::dashcore::Network;

        let votes = read_scheduled_votes(&conn, Network::Mainnet).unwrap();
        assert_eq!(votes.unreadable, 0);
        assert_eq!(votes.votes.len(), 1, "the scheduled vote must survive");
        assert_eq!(votes.votes[0].contested_name, "quantum");
        assert!(!votes.votes[0].executed_successfully);

        let top_ups = read_top_ups(&conn, Network::Mainnet).unwrap();
        assert_eq!(top_ups.unreadable, 0);
        assert_eq!(top_ups.top_ups.len(), 1);
        assert_eq!(top_ups.top_ups[0].1.get(&0), Some(&100_000));

        let settings = read_app_settings(&conn)
            .unwrap()
            .expect("the settings row must survive the ladder");
        assert_eq!(
            settings.network,
            Network::Mainnet,
            "the saved network must survive; resetting it relaunches the user elsewhere",
        );
    }

    // ── v34 migration: SPV-default backend ──────────────────────────
    //
    // The migration reads `.env` via `dotenvy::from_path_iter`, which does
    // not mutate the process environment, so these tests are self-contained
    // and can run in parallel with each other and the rest of the suite.
    mod v34 {
        fn write_env(dir: &std::path::Path, contents: &str) {
            std::fs::write(dir.join(".env"), contents).expect("write .env");
        }

        /// Set up a fresh v33 database in `dir` with `core_backend_mode = 0` (RPC),
        /// returning the `Database`.
        ///
        /// The v33 schema predates C3 — it still has the legacy
        /// `core_backend_mode` column on the settings table — so the
        /// fixture backfills it directly after `create_tables` to faithfully
        /// reproduce the on-disk shape of a real pre-C3 install.
        fn fresh_v33_db(dir: &std::path::Path) -> super::super::Database {
            let db_file = dir.join("test_data.db");
            let db = super::super::Database::new(&db_file).unwrap();
            db.create_tables(true).unwrap();
            {
                let conn = db.conn.lock().unwrap();
                conn.execute(
                    "ALTER TABLE settings ADD COLUMN core_backend_mode INTEGER DEFAULT 1",
                    [],
                )
                .unwrap();
            }
            db.set_default_version().unwrap();
            // Set starting state: v33 with the legacy RPC default.
            db.set_db_version(33).unwrap();
            {
                let conn = db.conn.lock().unwrap();
                conn.execute("UPDATE settings SET core_backend_mode = 0 WHERE id = 1", [])
                    .unwrap();
            }
            db
        }

        fn read_core_backend_mode(db: &super::super::Database) -> u8 {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT core_backend_mode FROM settings WHERE id = 1",
                [],
                |row| row.get::<_, u8>(0),
            )
            .unwrap()
        }

        /// developer_mode=true + a non-empty core_rpc_password on at least one
        /// network → migration leaves the saved mode untouched.
        #[test]
        fn v34_preserves_mode_when_local_core_configured() {
            let tmp = tempfile::tempdir().unwrap();
            let db = fresh_v33_db(tmp.path());
            write_env(
                tmp.path(),
                "DEVELOPER_MODE=true\n\
                 MAINNET_core_rpc_password=hunter2\n",
            );

            let result = db.try_perform_migration(33, 34, Some(tmp.path()));
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 34);
            // Mode preserved as RPC (0) — user's existing choice respected.
            assert_eq!(read_core_backend_mode(&db), 0);
        }

        /// Same as `v34_preserves_mode_when_local_core_configured`, but the
        /// password key is fully uppercase (`MAINNET_CORE_RPC_PASSWORD`). The
        /// suffix match must be case-insensitive so these users are not
        /// silently flipped to SPV during the v34 migration.
        #[test]
        fn v34_preserves_mode_with_uppercase_password_key() {
            let tmp = tempfile::tempdir().unwrap();
            let db = fresh_v33_db(tmp.path());
            write_env(
                tmp.path(),
                "DEVELOPER_MODE=true\n\
                 MAINNET_CORE_RPC_PASSWORD=hunter2\n",
            );

            let result = db.try_perform_migration(33, 34, Some(tmp.path()));
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 34);
            // Mode preserved as RPC (0) — uppercase password key must count.
            assert_eq!(read_core_backend_mode(&db), 0);
        }

        /// developer_mode=true but no password on any network → SPV.
        #[test]
        fn v34_migrates_to_spv_when_no_rpc_password() {
            let tmp = tempfile::tempdir().unwrap();
            let db = fresh_v33_db(tmp.path());
            write_env(tmp.path(), "DEVELOPER_MODE=true\n");

            let result = db.try_perform_migration(33, 34, Some(tmp.path()));
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 34);
            assert_eq!(read_core_backend_mode(&db), 1);
        }

        /// developer_mode=false (regardless of password) → SPV.
        #[test]
        fn v34_migrates_to_spv_when_developer_mode_off() {
            let tmp = tempfile::tempdir().unwrap();
            let db = fresh_v33_db(tmp.path());
            write_env(
                tmp.path(),
                "DEVELOPER_MODE=false\n\
                 MAINNET_core_rpc_password=hunter2\n",
            );

            let result = db.try_perform_migration(33, 34, Some(tmp.path()));
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 34);
            assert_eq!(read_core_backend_mode(&db), 1);
        }

        /// No `.env` at all (or unreadable) → SPV (safest default).
        #[test]
        fn v34_migrates_to_spv_when_env_missing() {
            let tmp = tempfile::tempdir().unwrap();
            let db = fresh_v33_db(tmp.path());
            // Deliberately do not write `.env`.

            let result = db.try_perform_migration(33, 34, Some(tmp.path()));
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 34);
            assert_eq!(read_core_backend_mode(&db), 1);
        }

        /// Re-running the migration on an already-migrated DB is a no-op.
        #[test]
        fn v34_rerun_is_noop() {
            let tmp = tempfile::tempdir().unwrap();
            let db = fresh_v33_db(tmp.path());
            write_env(tmp.path(), "DEVELOPER_MODE=false\n");

            // First run: 33 -> 34
            db.try_perform_migration(33, 34, Some(tmp.path())).unwrap();
            assert_eq!(db.db_schema_version().unwrap(), 34);

            // Second run: 34 -> 34 is a no-op.
            let result = db.try_perform_migration(34, 34, Some(tmp.path()));
            assert!(result.is_ok(), "re-run should be no-op: {:?}", result.err());
            assert!(
                !result.unwrap(),
                "try_perform_migration should report no migration needed"
            );
            assert_eq!(db.db_schema_version().unwrap(), 34);
        }
    }

    // ── v38 migration: drop the retired core_backend_mode column ─────
    mod v38 {
        fn settings_column_exists(db: &super::super::Database, column: &str) -> bool {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name = ?1",
                [column],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap()
        }

        /// Build a v37 DB whose `settings` table still carries the legacy
        /// `core_backend_mode` column plus a second legacy column
        /// (`disable_zmq`) with a distinctive value, so the migration can be
        /// shown to drop ONLY the target column and preserve the rest.
        fn v37_db_with_legacy_columns(dir: &std::path::Path) -> super::super::Database {
            let db_file = dir.join("test_data.db");
            let db = super::super::Database::new(&db_file).unwrap();
            db.create_tables(true).unwrap();
            {
                let conn = db.conn.lock().unwrap();
                conn.execute(
                    "ALTER TABLE settings ADD COLUMN core_backend_mode INTEGER DEFAULT 1",
                    [],
                )
                .unwrap();
                conn.execute(
                    "ALTER TABLE settings ADD COLUMN disable_zmq INTEGER DEFAULT 0",
                    [],
                )
                .unwrap();
            }
            db.set_default_version().unwrap();
            db.set_db_version(37).unwrap();
            {
                let conn = db.conn.lock().unwrap();
                conn.execute(
                    "UPDATE settings SET core_backend_mode = 0, disable_zmq = 1 WHERE id = 1",
                    [],
                )
                .unwrap();
            }
            db
        }

        /// A pre-v38 DB with the legacy column migrates cleanly: the
        /// `core_backend_mode` column is dropped, the version advances to 38,
        /// and every other settings value survives.
        #[test]
        fn v38_drops_column_and_preserves_other_settings() {
            let tmp = tempfile::tempdir().unwrap();
            let db = v37_db_with_legacy_columns(tmp.path());
            assert!(settings_column_exists(&db, "core_backend_mode"));

            let result = db.try_perform_migration(37, 38, None);
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 38);

            // Target column gone.
            assert!(
                !settings_column_exists(&db, "core_backend_mode"),
                "core_backend_mode must be dropped"
            );
            // Sibling settings survive untouched.
            assert!(settings_column_exists(&db, "disable_zmq"));
            let disable_zmq: i64 = {
                let conn = db.conn.lock().unwrap();
                conn.query_row("SELECT disable_zmq FROM settings WHERE id = 1", [], |row| {
                    row.get(0)
                })
                .unwrap()
            };
            assert_eq!(disable_zmq, 1, "unrelated settings must be preserved");
        }

        /// A DB that never had the column (fresh post-C3 schema) migrates to
        /// v38 without error — the drop is a guarded no-op.
        #[test]
        fn v38_is_noop_when_column_absent() {
            let tmp = tempfile::tempdir().unwrap();
            let db_file = tmp.path().join("fresh.db");
            let db = super::super::Database::new(&db_file).unwrap();
            db.create_tables(false).unwrap();
            db.set_default_version().unwrap();
            db.set_db_version(37).unwrap();
            assert!(!settings_column_exists(&db, "core_backend_mode"));

            let result = db.try_perform_migration(37, 38, None);
            assert!(result.is_ok(), "migration failed: {:?}", result.err());
            assert_eq!(db.db_schema_version().unwrap(), 38);
            assert!(!settings_column_exists(&db, "core_backend_mode"));
        }

        /// Re-running the migration on an already-migrated DB is a no-op.
        #[test]
        fn v38_rerun_is_noop() {
            let tmp = tempfile::tempdir().unwrap();
            let db = v37_db_with_legacy_columns(tmp.path());

            db.try_perform_migration(37, 38, None).unwrap();
            assert_eq!(db.db_schema_version().unwrap(), 38);

            let result = db.try_perform_migration(38, 38, None);
            assert!(result.is_ok(), "re-run should be no-op: {:?}", result.err());
            assert!(
                !result.unwrap(),
                "try_perform_migration should report no migration needed"
            );
            assert_eq!(db.db_schema_version().unwrap(), 38);
        }
    }

    mod v39 {
        #[test]
        fn v39_creates_forgotten_identity_markers_for_existing_databases() {
            let tmp = tempfile::tempdir().unwrap();
            let db = super::super::Database::new(tmp.path().join("v38.db")).unwrap();
            db.execute(
                "CREATE TABLE settings (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    database_version INTEGER NOT NULL
                )",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO settings (id, database_version) VALUES (1, 38)",
                [],
            )
            .unwrap();
            assert!(
                !db.table_exists(&db.locked_conn(), "forgotten_identities")
                    .unwrap()
            );

            db.try_perform_migration(38, 39, None).unwrap();

            assert_eq!(db.db_schema_version().unwrap(), 39);
            assert!(
                db.table_exists(&db.locked_conn(), "forgotten_identities")
                    .unwrap()
            );
            let columns: i64 = db
                .locked_conn()
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('forgotten_identities')
                     WHERE name IN ('network', 'identity_id')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(columns, 2);
        }
    }

    // ---------- T-DEV-01: legacy CREATE TABLE gating ----------

    /// Helper: assert that a table does NOT exist in the database.
    fn assert_table_absent(conn: &Connection, table: &str) {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                params![table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists, "table `{table}` must NOT exist on a fresh install");
    }

    /// TC-DEV-006 — Truly-fresh install creates no wallet-family tables.
    ///
    /// The gated targets (`wallet`, `wallet_addresses`, `utxos`,
    /// `single_key_wallet`, `wallet_transactions`, `shielded_notes`,
    /// `shielded_wallet_meta`, `identity`) are legacy schema that lives in
    /// `platform-wallet.sqlite` or the per-network k/v store now. Only
    /// `settings` (the migration version counter) is always created.
    #[test]
    fn tc_dev_006_fresh_install_omits_legacy_tables() {
        let tmp = tempfile::tempdir().unwrap();
        let db_file = tmp.path().join("fresh.db");
        let db = super::Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();

        let conn = db.conn.lock().unwrap();

        // Always-present
        assert_table_exists(&conn, "settings");

        // Gated targets must be absent
        for t in [
            "wallet",
            "wallet_addresses",
            "utxos",
            "single_key_wallet",
            "wallet_transactions",
            "shielded_notes",
            "shielded_wallet_meta",
            "identity",
        ] {
            assert_table_absent(&conn, t);
        }
    }

    /// TC-MIG-006 — Existing install with legacy rows triggers full schema
    /// creation so the migration ladder has tables to upgrade.
    ///
    /// Simulates an unusual recovery shape: a DB where the `settings` row
    /// was wiped (so `is_first_time_setup` reports true) but the
    /// `wallet` table still carries rows. `legacy_detected` returns true,
    /// so `initialize` re-creates the wallet-family schema.
    #[test]
    fn tc_mig_006_legacy_rows_trigger_full_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let db_file = tmp.path().join("legacy.db");
        let db = super::Database::new(&db_file).unwrap();

        // Pre-seed a legacy `wallet` row before `initialize` runs.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "CREATE TABLE wallet (
                    seed_hash BLOB NOT NULL PRIMARY KEY,
                    encrypted_seed BLOB NOT NULL,
                    salt BLOB NOT NULL,
                    nonce BLOB NOT NULL,
                    master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
                    uses_password INTEGER NOT NULL,
                    network TEXT NOT NULL
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, uses_password, network
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    vec![1u8; 32],
                    vec![2u8; 16],
                    vec![3u8; 16],
                    vec![4u8; 12],
                    vec![5u8; 33],
                    0i32,
                    "mainnet",
                ],
            )
            .unwrap();
            assert!(super::legacy_detected(&conn));
        }

        db.initialize(&db_file).unwrap();

        let conn = db.conn.lock().unwrap();
        // Upgrade-replay path: wallet-family tables present.
        assert_table_exists(&conn, "wallet");
        assert_table_exists(&conn, "wallet_addresses");
        assert_table_exists(&conn, "utxos");
        assert_table_exists(&conn, "single_key_wallet");
        assert_table_exists(&conn, "wallet_transactions");
        // Phase D retired the shielded tables — they are dropped by v37.
        assert_table_absent(&conn, "shielded_notes");
        assert_table_absent(&conn, "shielded_wallet_meta");
    }

    /// TC-MIG-008 (partial) — Fresh install and the `data.db` file.
    ///
    /// Truly-fresh installs land at version `DEFAULT_DB_VERSION` with no
    /// wallet-family tables. The file itself still appears on disk
    /// because `Database::new` opens the SQLite connection eagerly; fully
    /// suppressing the file is T-DEV-02 territory.
    // TODO(T-DEV-02): suppress `data.db` file creation when no DET state
    // needs to be persisted to it.
    #[test]
    fn tc_mig_008_fresh_install_file_state() {
        let tmp = tempfile::tempdir().unwrap();
        let db_file = tmp.path().join("fresh.db");
        assert!(!db_file.exists(), "precondition: file absent");

        let db = super::Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();

        // Partial pass: file exists (Database::new opens an empty SQLite
        // connection) but carries no wallet-family schema.
        assert!(db_file.exists(), "Database::new opens the file eagerly");
        let conn = db.conn.lock().unwrap();
        assert_table_exists(&conn, "settings");
        for t in [
            "wallet",
            "wallet_addresses",
            "utxos",
            "single_key_wallet",
            "wallet_transactions",
            "shielded_notes",
            "shielded_wallet_meta",
        ] {
            assert_table_absent(&conn, t);
        }
    }

    /// `legacy_detected` returns false on an empty DB.
    #[test]
    fn legacy_detected_false_on_empty_db() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(!super::legacy_detected(&conn));
    }

    /// `legacy_detected` returns false when a target table exists but is empty.
    #[test]
    fn legacy_detected_false_on_empty_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE wallet (seed_hash BLOB PRIMARY KEY)", [])
            .unwrap();
        assert!(!super::legacy_detected(&conn));
    }
}
