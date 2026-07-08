//! Post-PR-#860 cold-start migration orchestrator.
//!
//! Drains legacy `data.db` rows that the unwire left behind into the
//! upstream `platform-wallet-storage` k/v store and `SecretStore`.
//! Idempotent: a per-network completion sentinel under
//! [`sentinel_key_for`] in `det-app.sqlite` short-circuits subsequent
//! launches **on the same network**.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::wallet_backend::{DetScope, KvAdapterError};

/// Sentinel key format string. The migration body filters every
/// legacy table by `WHERE network = ?1`, so the sentinel must mirror
/// that scope — otherwise an upgrade on mainnet writes the sentinel
/// and a later switch to testnet skips the migration even though
/// testnet wallets are still in the legacy file. Versioned (`:v1`) so
/// a future format change bumps the key rather than re-interpreting
/// the existing payload.
const SENTINEL_KEY_PREFIX: &str = "det:migration:finish_unwire";
const SENTINEL_KEY_VERSION: &str = "v1";

/// Per-network sentinel key. The migration filters legacy rows by
/// `WHERE network = ?1`, so the sentinel scope must match. A previous
/// global key let an upgrade on mainnet hide all testnet wallets after
/// a network switch.
pub fn sentinel_key_for(network: Network) -> String {
    format!(
        "{SENTINEL_KEY_PREFIX}:{}:{SENTINEL_KEY_VERSION}",
        network_token(network)
    )
}

/// Stable, lowercase ASCII network token used in sentinel keys. Kept
/// distinct from `Network::to_string()` so a future upstream change to
/// the `Display` impl cannot invalidate existing sentinels.
fn network_token(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

// TODO: App settings, top-up history, and scheduled DPNS votes all reset/empty on
//   upgrade — confirmed real data loss per v0.9.3 cross-check; follow-up priority:
//   scheduled votes (vote-window deadline risk) > app settings (UX friction) > top-up
//   history (audit trail). Migration to be handled in a separate PR.
/// Tables sniffed during detection. Any non-empty row count flips the
/// migration into the `Running` state. Ordered so the cheapest check
/// (the single-row `wallet` table) runs first.
const LEGACY_TABLES: &[&str] = &["wallet", "single_key_wallet", "utxos"];

/// Persisted sentinel payload. Lives in `det-app.sqlite` under the
/// per-network sentinel key returned by [`sentinel_key_for`].
/// `network_count` is informational — kept for diagnostics so older
/// payloads still round-trip cleanly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationCompletion {
    /// Unix-epoch seconds at completion. Used for diagnostics — never
    /// parsed back into business logic.
    pub completed_at: i64,
    /// Git SHA / version tag of the running build. Lets a future
    /// reader correlate the sentinel with the binary that produced it.
    pub sha: String,
    /// How many network entries the migration walked on this pass.
    /// Always `0` or `1` now that the sentinel is per-network — kept
    /// in the payload so a forward-compatible reader can re-aggregate
    /// across networks without a schema bump.
    pub network_count: u32,
}

/// Domain error envelope for the migration orchestrator.
///
/// Variants wrap upstream error types via `#[source]`; the
/// user-facing message lives on [`TaskError::MigrationFailed`].
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// Could not open the legacy `data.db` SQLite file to sniff for
    /// legacy rows.
    #[error("could not open legacy data.db at {path}")]
    LegacyDbOpen {
        path: String,
        #[source]
        source: rusqlite::Error,
    },

    /// A SQL query against legacy `data.db` failed (table missing,
    /// truncated blob, etc.). Distinct from `LegacyDbOpen` so the UI
    /// can attribute partially-readable corruption separately from
    /// inaccessible-file errors.
    #[error("could not read legacy table `{table}`")]
    LegacyDbRead {
        table: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    /// Could not read or write the migration sentinel in
    /// `det-app.sqlite`. Treated as fatal — without the sentinel the
    /// migration would re-run on every launch.
    #[error("could not access migration sentinel")]
    Sentinel {
        #[source]
        source: KvAdapterError,
    },

    /// At least one legacy `single_key_wallet` row failed to migrate.
    /// Fatal only when `failed > 0`; password-protected rows count as
    /// `skipped_password_protected`, not as failures.
    #[error("could not finish single-key migration: {failed} row(s) failed")]
    SingleKeyPartialFailure {
        /// Number of rows successfully imported (or already present
        /// idempotent re-runs) in the secret store.
        imported: u32,
        /// Number of `uses_password=1` rows the migration skipped
        /// because it has no password material here — T-SK-03's UX
        /// prompt will resolve these.
        skipped_password_protected: u32,
        /// Number of rows that could not be decoded into a usable
        /// private key (corrupt blob, wrong size). Triggers the error
        /// path — sentinel is not written so a re-run can retry.
        failed: u32,
    },

    /// At least one legacy `wallet` row could not be migrated into the
    /// DET wallet-metadata sidecar in this run. Captures the imported /
    /// failed counters so the orchestrator can decide whether to write
    /// the sentinel. Fatal only when `failed > 0`.
    #[error("could not finish wallet-meta migration: {failed} row(s) failed")]
    WalletMetaPartialFailure {
        /// Rows whose meta blob was written to the sidecar (or
        /// idempotently overwritten on a re-run).
        imported: u32,
        /// Rows that could not be decoded (seed_hash wrong size, etc.).
        /// Triggers the error path — sentinel stays unwritten.
        failed: u32,
    },

    /// At least one legacy HD wallet seed row could not be migrated
    /// into the upstream secret vault in this run. The migrator copies
    /// the full envelope verbatim — no decryption — so the only way
    /// to land here is a malformed legacy row (wrong `seed_hash` size,
    /// unreadable SQLite blob, etc.). Fatal whenever `failed > 0`;
    /// the sentinel stays unwritten so a re-run can retry.
    #[error("could not finish wallet-seed migration: {failed} row(s) failed")]
    WalletSeedsPartialFailure {
        /// Rows whose envelope was written to the vault (or
        /// idempotently overwritten on a re-run).
        imported: u32,
        /// Rows that could not be decoded (seed_hash wrong size,
        /// corrupted blob, etc.). Triggers the error path.
        failed: u32,
    },

    /// The wallet backend was not yet wired when the migration ran.
    /// This is a hard configuration bug: the orchestrator runs after
    /// `ensure_wallet_backend`, so this should never fire in
    /// production. Kept as a typed variant so a future regression is
    /// caught immediately instead of silently no-oping.
    #[error("wallet backend not available during migration")]
    WalletBackendUnavailable,

    /// Returned by [`guard_single_key_table_droppable`] when dropping the
    /// legacy single-key table would destroy a password-protected key that
    /// has no copy anywhere else. `remaining` is the un-restored row count.
    #[error(
        "could not drop legacy single-key table: {remaining} protected key(s) not yet restored"
    )]
    ProtectedSingleKeysNotRestored {
        /// Number of `uses_password=1` rows still present and not yet
        /// restored into the modern vault.
        remaining: u32,
    },

    /// Post-migration re-hydration of `ctx.wallets` from the freshly
    /// populated sidecars failed, so the migrated wallets were not
    /// reconstructed in memory and could not be registered upstream. The
    /// completion sentinel is withheld so the next cold boot — which
    /// re-hydrates from the same sidecars during backend construction —
    /// retries.
    #[error("could not re-hydrate migrated wallets")]
    Hydration {
        #[source]
        source: Box<TaskError>,
    },

    /// At least one open (resolvable) wallet was migrated but did not land
    /// in the upstream wallet store after bootstrap registration. The
    /// completion sentinel is withheld so a re-run (the next cold start, or
    /// the "Retry now" banner) retries the idempotent registration. Locked
    /// password-protected wallets are excluded — they register on their
    /// unlock gesture — so this never fires for a protected-only install.
    #[error("could not finish wallet registration: {unregistered} wallet(s) not yet registered")]
    RegistrationIncomplete {
        /// Number of currently-open wallets still missing from the upstream
        /// store after the bootstrap registration pass.
        unregistered: usize,
    },
}

impl MigrationError {
    /// `true` for failures that clear themselves once the wallet backend
    /// finishes wiring. The cold-start dispatcher retries these on a later
    /// frame instead of burning the per-network guard and stranding the
    /// network's wallets behind a manual "Retry now".
    pub fn is_backend_not_ready(&self) -> bool {
        matches!(self, MigrationError::WalletBackendUnavailable)
    }
}

/// Run the FinishUnwire migration. Idempotent — completes a no-op when
/// the sentinel is already present.
///
/// Returns `true` when this launch actually moved legacy data (rows were
/// detected and drained), and `false` for the two no-op paths: the
/// sentinel already existed, or no legacy rows were present. Callers use
/// the flag to decide whether to surface a "storage update complete"
/// banner — a no-op launch must not show one.
///
/// Drains single-key wallet rows, HD wallet seeds, and wallet metadata
/// into the upstream store, registers the migrated wallets, then writes
/// the completion sentinel.
pub async fn run(app_context: &Arc<AppContext>) -> Result<bool, TaskError> {
    let status = app_context.migration_status();
    let app_kv = app_context.app_kv();
    let network = app_context.network;

    // Idempotency: if the sentinel for *this network* already exists,
    // this launch has nothing to do. The sentinel is per-network
    // because every migration body filters legacy rows by `WHERE
    // network = ?1` — a shared sentinel would let an upgrade on
    // mainnet silently skip the testnet migration after a switch.
    if let Some(completion) = read_sentinel(&app_kv, network)? {
        tracing::info!(
            target = "migration::finish_unwire",
            network = ?network,
            completed_at = completion.completed_at,
            sha = %completion.sha,
            network_count = completion.network_count,
            "FinishUnwire already completed for this network — skipping",
        );
        // No-op launch: the sentinel was already written by a prior run, so
        // nothing moved this time. Stay `Idle` so the per-frame banner
        // reconciler never surfaces a spurious "storage update complete".
        status.set_state(MigrationState::Idle);
        return Ok(false);
    }

    status.set_state(MigrationState::Running {
        step: MigrationStep::Detecting,
    });

    let legacy_present = detect_legacy_rows(app_context)?;
    if !legacy_present {
        tracing::info!(
            target = "migration::finish_unwire",
            network = ?network,
            "No legacy data.db rows detected — writing sentinel without migration",
        );
        write_sentinel(&app_kv, network, 0)?;
        // No legacy rows to move (e.g. a fresh install): record the sentinel
        // but stay `Idle` so no completion banner appears for a launch that
        // did no work.
        status.set_state(MigrationState::Idle);
        return Ok(false);
    }

    tracing::info!(
        target = "migration::finish_unwire",
        "Legacy data.db rows detected — beginning migration",
    );

    status.set_state(MigrationState::Running {
        step: MigrationStep::SingleKey,
    });
    migrate_single_key_rows(app_context).await?;

    // Copy every legacy HD wallet seed envelope into
    // the upstream encrypted vault. The envelope bytes travel verbatim
    // (no decryption); password-protected and unprotected rows take
    // the same path so the per-wallet password UX stays intact.
    status.set_state(MigrationState::Running {
        step: MigrationStep::WalletSeeds,
    });
    migrate_wallet_seeds_rows(app_context)?;

    // T-W-00 — mirror legacy `wallet` rows (alias / `is_main` /
    // `core_wallet_name` / master xpub) into the DET wallet-metadata
    // sidecar so the wallet picker keeps the names a user already
    // chose and can render at cold boot without unlocking seeds.
    // Idempotent.
    status.set_state(MigrationState::Running {
        step: MigrationStep::WalletMeta,
    });
    migrate_wallet_meta_rows(app_context)?;

    status.set_state(MigrationState::Running {
        step: MigrationStep::Finalize,
    });

    // Register the migrated wallets upstream BEFORE the completion sentinel,
    // so the sentinel can never claim "done" while a migratable unprotected
    // wallet is still absent from `spv/<net>/platform-wallet.sqlite`. On failure
    // this returns `Err` (the sentinel is skipped) so the next cold start — or
    // the "Retry now" banner — re-runs the idempotent migration.
    register_migrated_wallets(app_context).await?;

    write_sentinel(&app_kv, network, 1)?;

    tracing::info!(
        target = "migration::finish_unwire",
        network = ?network,
        "FinishUnwire migration complete",
    );
    status.set_state(MigrationState::Success);
    Ok(true)
}

/// Re-hydrates just-migrated wallets into `ctx.wallets` and registers the
/// resolvable (open/unprotected) ones upstream. [`run`] calls this
/// immediately before [`write_sentinel`], so completion can never be
/// recorded while a migratable unprotected wallet is still unregistered.
/// Locked protected wallets and genuinely-unusable rows are excluded —
/// both register or land safely elsewhere. Idempotent.
async fn register_migrated_wallets(app_context: &Arc<AppContext>) -> Result<(), MigrationError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;

    // `WalletBackend::new` ran `hydrate_context_wallets` earlier this boot
    // against the then-EMPTY sidecars; the migration just populated them, so
    // re-hydrate to make the wallets visible without a restart. A hydration
    // failure means the wallets are not reconstructed, so do not claim
    // completion — the next cold boot re-hydrates from the same sidecars.
    backend
        .hydrate_context_wallets(app_context)
        .map_err(|source| MigrationError::Hydration {
            source: Box::new(source),
        })?;

    // Re-run the cold-boot W2 bridge now that `ctx.wallets` is populated, so the
    // just-migrated open wallets are registered upstream (`id_map` + persistor)
    // without a restart. Idempotent and prompt-free; locked protected wallets
    // are skipped and register on their unlock gesture.
    app_context.bootstrap_loaded_wallets().await;

    let unregistered = app_context.unregistered_open_wallet_count();
    if unregistered > 0 {
        return Err(MigrationError::RegistrationIncomplete { unregistered });
    }
    Ok(())
}

/// Returns `true` when any of the [`LEGACY_TABLES`] holds at least one
/// row. Missing tables are treated as empty: a freshly-installed
/// `data.db` already lacks the dropped tables, and that is correct.
fn detect_legacy_rows(app_context: &AppContext) -> Result<bool, MigrationError> {
    let Some(path) = app_context.db.db_file_path() else {
        // In-memory DBs (tests, headless) have no legacy file to drain.
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }
    let path_str = path.to_string_lossy().to_string();
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path_str,
        source: e,
    })?;
    for &table in LEGACY_TABLES {
        if table_has_rows(&conn, table)? {
            tracing::debug!(
                target = "migration::finish_unwire",
                table,
                "Legacy table holds rows",
            );
            return Ok(true);
        }
    }
    Ok(false)
}

/// `SELECT 1 FROM <table> LIMIT 1` — returns `false` for missing
/// tables. Uses a typed `legacy_table_exists` pre-check so the missing
/// table case never reaches `conn.prepare` — any `rusqlite` error from
/// here on is a hard error, not a "table missing" string-parsed branch.
fn table_has_rows(conn: &Connection, table: &'static str) -> Result<bool, MigrationError> {
    if !legacy_table_exists_named(conn, table)? {
        return Ok(false);
    }
    // Caller passes a static identifier from `LEGACY_TABLES`, so the
    // `format!` here cannot interpolate user input. SQLite parameter
    // binding does not support table names, so this is the canonical
    // shape.
    let sql = format!("SELECT 1 FROM {table} LIMIT 1");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| MigrationError::LegacyDbRead { table, source: e })?;
    let mut rows = stmt
        .query([])
        .map_err(|e| MigrationError::LegacyDbRead { table, source: e })?;
    let has_row = rows
        .next()
        .map_err(|e| MigrationError::LegacyDbRead { table, source: e })?
        .is_some();
    Ok(has_row)
}

/// Outcome counters from one [`migrate_single_key_rows`] pass. Public
/// to the test module so partial-failure semantics can be asserted
/// without invoking the AppContext-bound orchestrator.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SingleKeyMigrationOutcome {
    /// Rows whose raw private key was written to the secret store
    /// (or were already present — idempotent re-runs land here too).
    imported: u32,
    /// Rows the migration skipped because they were encrypted with a
    /// per-wallet password we do not have on this code path. T-SK-03's
    /// UX prompt will resolve them later — they do not count as a
    /// failure so a pure password-protected install still writes the
    /// sentinel on the next launch.
    skipped_password_protected: u32,
    /// Rows that could not be decoded into 32 raw private-key bytes
    /// (wrong blob length, sqlite read error, address-derivation
    /// failure). Triggers the error path — sentinel stays unwritten.
    failed: u32,
}

/// Walks the legacy `single_key_wallet` table for `network` and imports
/// every `uses_password=0` row into the secret store under the canonical
/// `single_key_priv.<addr>` label. Idempotent. Password-protected rows
/// are skipped and reported separately, not as failures.
async fn migrate_single_key_rows(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;

    let Some(path) = app_context.db.db_file_path() else {
        // In-memory / headless: nothing to migrate.
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let path_str = path.to_string_lossy().to_string();
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path_str,
        source: e,
    })?;

    let view = backend.single_key();
    let outcome = migrate_single_key_rows_from_conn(
        &conn,
        |wif, alias| view.import_wif(wif, alias).map(|_| ()),
        app_context.network,
    )?;
    tracing::info!(
        target = "migration::finish_unwire",
        imported = outcome.imported,
        skipped_password_protected = outcome.skipped_password_protected,
        failed = outcome.failed,
        network = ?app_context.network,
        "Single-key migration pass complete",
    );

    if outcome.failed > 0 {
        return Err(MigrationError::SingleKeyPartialFailure {
            imported: outcome.imported,
            skipped_password_protected: outcome.skipped_password_protected,
            failed: outcome.failed,
        }
        .into());
    }
    Ok(())
}

/// Pure migration body (testable without an `AppContext`). Decodes every
/// `uses_password=0` row at `conn` into a WIF and imports it via `import`.
/// Returns counters rather than erroring on partial readability. A
/// missing table is not an error — a fresh install has none.
fn migrate_single_key_rows_from_conn<F>(
    conn: &Connection,
    mut import: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<SingleKeyMigrationOutcome, MigrationError>
where
    F: FnMut(&str, Option<String>) -> Result<(), TaskError>,
{
    use dash_sdk::dpp::dashcore::PrivateKey;

    if !legacy_table_exists_named(conn, "single_key_wallet")? {
        return Ok(SingleKeyMigrationOutcome::default());
    }
    let sql = "SELECT encrypted_private_key, alias, uses_password \
               FROM single_key_wallet WHERE network = ?1";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;

    let rows = stmt
        .query_map(rusqlite::params![network.to_string()], |row| {
            let encrypted: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?;
            let uses_password: i32 = row.get(2)?;
            Ok((encrypted, alias, uses_password))
        })
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;

    let mut outcome = SingleKeyMigrationOutcome::default();
    for row in rows {
        let (encrypted, alias, uses_password) = match row {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?e,
                    "Skipping unreadable single_key_wallet row",
                );
                outcome.failed = outcome.failed.saturating_add(1);
                continue;
            }
        };

        if uses_password != 0 {
            // Password-protected rows need the user's password. T-SK-03
            // will surface a one-time UX prompt; until then count and
            // skip — they do not block the rest of the migration.
            tracing::warn!(
                target = "migration::finish_unwire",
                "Skipping password-protected single_key_wallet row (T-SK-03 UX prompt deferred)",
            );
            outcome.skipped_password_protected =
                outcome.skipped_password_protected.saturating_add(1);
            continue;
        }

        // Per legacy schema: `uses_password=0` rows store the raw
        // 32-byte private key directly in `encrypted_private_key`
        // (salt/nonce are empty). See `model/wallet/single_key.rs`
        // `SingleKeyData::open_no_password`.
        let key_bytes: [u8; 32] = match encrypted.as_slice().try_into() {
            Ok(b) => b,
            Err(_) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    blob_len = encrypted.len(),
                    "Skipping single_key_wallet row with non-32-byte raw key blob",
                );
                outcome.failed = outcome.failed.saturating_add(1);
                continue;
            }
        };

        let priv_key = match PrivateKey::from_byte_array(&key_bytes, network) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = %e,
                    "Skipping single_key_wallet row — invalid private key bytes",
                );
                outcome.failed = outcome.failed.saturating_add(1);
                continue;
            }
        };
        let wif = priv_key.to_wif();

        // `import` is `SingleKeyView::import_wif` in production; the
        // view writes to the secret store under the canonical
        // `single_key_priv.<addr>` label and seeds the in-memory
        // index. Re-import on the same address overwrites the same
        // bytes — idempotent (TC-SK-002).
        match import(&wif, alias) {
            Ok(_) => outcome.imported = outcome.imported.saturating_add(1),
            Err(e) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?e,
                    "Failed to import single_key_wallet row into secret store",
                );
                outcome.failed = outcome.failed.saturating_add(1);
            }
        }
    }

    Ok(outcome)
}

/// Count legacy `single_key_wallet` rows for `network` that are
/// password-protected (`uses_password=1`) and have NOT yet been restored
/// into the modern vault.
///
/// **Data-loss gate (S3).** A protected row holds a private key encrypted
/// under the user's OLD legacy password. Until the user supplies that
/// password and the key is re-encrypted into the modern secret-store
/// vault (T-SK-03), the legacy row is the ONLY copy. Dropping the table
/// while any such row remains permanently destroys the key.
///
/// A row counts as **restored** when `is_restored(address)` returns
/// `true` — in production that closure checks the modern single-key
/// sidecar for a matching entry at the same address. The closure shape
/// (mirroring [`migrate_single_key_rows_from_conn`]) keeps this body
/// testable without standing up a `WalletBackend`.
///
/// **Missing table is not a hazard** — a fresh install (or one whose
/// table was already cleaned up after all rows were restored) returns
/// `0`. Rows with an unreadable `address`/`uses_password` are
/// conservatively counted as un-restored so a corrupt row can never let
/// the table be dropped.
fn count_unrestored_protected_single_keys<F>(
    conn: &Connection,
    mut is_restored: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<u32, MigrationError>
where
    F: FnMut(&str) -> bool,
{
    if !legacy_table_exists_named(conn, "single_key_wallet")? {
        return Ok(0);
    }
    let sql = "SELECT address, uses_password FROM single_key_wallet WHERE network = ?1";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;
    let rows = stmt
        .query_map(rusqlite::params![network.to_string()], |row| {
            let address: Option<String> = row.get(0)?;
            let uses_password: i32 = row.get(1)?;
            Ok((address, uses_password))
        })
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;

    let mut remaining: u32 = 0;
    for row in rows {
        let (address, uses_password) = match row {
            Ok(t) => t,
            Err(e) => {
                // An unreadable row can't be proven restored — count it
                // so the table stays put rather than risk a silent drop.
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?e,
                    "Counting unreadable single_key_wallet row as un-restored (drop guard)",
                );
                remaining = remaining.saturating_add(1);
                continue;
            }
        };
        if uses_password == 0 {
            // Unprotected rows migrate without the user's password and
            // carry no data-loss hazard — they are out of scope here.
            continue;
        }
        match address {
            Some(addr) if is_restored(&addr) => {}
            _ => remaining = remaining.saturating_add(1),
        }
    }
    Ok(remaining)
}

/// Data-loss gate: returns `Ok(())` only when the legacy
/// `single_key_wallet` table for `network` may be safely dropped — i.e.
/// every password-protected row has been restored into the modern vault.
/// Otherwise returns [`MigrationError::ProtectedSingleKeysNotRestored`]
/// with the remaining count.
///
/// **Every cleanup path that drops the legacy single-key table MUST call
/// this first and abort on error.** This is the single structural
/// chokepoint that prevents the permanent-key-loss scenario described on
/// [`MigrationError::ProtectedSingleKeysNotRestored`] (Smythe S3).
fn guard_single_key_table_droppable<F>(
    conn: &Connection,
    is_restored: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<(), MigrationError>
where
    F: FnMut(&str) -> bool,
{
    let remaining = count_unrestored_protected_single_keys(conn, is_restored, network)?;
    if remaining > 0 {
        tracing::warn!(
            target = "migration::finish_unwire",
            remaining,
            network = ?network,
            "Refusing to drop legacy single-key table — protected keys not yet restored",
        );
        return Err(MigrationError::ProtectedSingleKeysNotRestored { remaining });
    }
    Ok(())
}

/// Drop the legacy `single_key_wallet` table for `network`, but ONLY
/// after [`guard_single_key_table_droppable`] confirms no protected row
/// remains un-restored. This is the one sanctioned way to remove the
/// legacy single-key table; the drop is unconditionally gated so a
/// future cleanup path cannot bypass the data-loss check (Smythe S3).
///
/// `is_restored` is the same predicate the guard uses — in production it
/// checks the modern single-key sidecar for a matching restored entry.
/// The table is dropped with `DROP TABLE IF EXISTS` so a re-run after a
/// successful drop is a no-op.
fn drop_legacy_single_key_table<F>(
    conn: &Connection,
    is_restored: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<(), MigrationError>
where
    F: FnMut(&str) -> bool,
{
    guard_single_key_table_droppable(conn, is_restored, network)?;
    conn.execute("DROP TABLE IF EXISTS single_key_wallet", [])
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;
    tracing::info!(
        target = "migration::finish_unwire",
        network = ?network,
        "Dropped legacy single-key table (all protected keys restored)",
    );
    Ok(())
}

/// Salt expected by Argon2id during the legacy AES-GCM seed encryption
/// (16 bytes, see `src/model/wallet/encryption.rs`).
const LEGACY_SALT_LEN: usize = 16;

/// GCM nonce expected by AES-256-GCM during the legacy seed encryption
/// (12 bytes, see `src/model/wallet/encryption.rs`).
const LEGACY_NONCE_LEN: usize = 12;

/// Row-level length guard for the password-related crypto
/// fields on a legacy `wallet` row. Password-protected rows must carry a
/// 16-byte salt and a 12-byte nonce; unprotected rows must carry empty
/// fields (the legacy DB writer bypasses encryption when
/// `uses_password = false`). Anything else is corruption — the caller
/// skips the row and logs.
fn crypto_field_lengths_ok(salt: &[u8], nonce: &[u8], uses_password: bool) -> bool {
    if uses_password {
        salt.len() == LEGACY_SALT_LEN && nonce.len() == LEGACY_NONCE_LEN
    } else {
        salt.is_empty() && nonce.is_empty()
    }
}

/// Expected plaintext length of a BIP-39 seed (64 bytes, PBKDF2 output),
/// mirroring `wallet_backend::hydration::EXPECTED_SEED_LEN`. An unprotected
/// wallet whose `encrypted_seed` is a different length is rejected by
/// hydration, so the copy step rejects it too — see [`hd_seed_row_is_hydratable`].
const HYDRATABLE_SEED_LEN: usize = 64;

/// Whether a legacy `wallet` row will survive cold-boot hydration (mirrors
/// `wallet_backend::hydration`: master xpub must decode, and an unprotected
/// row's seed must be exactly 64 bytes). Copy-acceptance must be a subset
/// of hydration-acceptance, or an unusable row could pass the copy step yet
/// stay invisible to the registration gate, letting the sentinel falsely
/// read "done".
fn hd_seed_row_is_hydratable(
    uses_password: bool,
    encrypted_seed: &[u8],
    xpub_encoded: &[u8],
) -> bool {
    use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey;
    if xpub_encoded.is_empty() || ExtendedPubKey::decode(xpub_encoded).is_err() {
        return false;
    }
    if !uses_password && encrypted_seed.len() != HYDRATABLE_SEED_LEN {
        return false;
    }
    true
}

/// Returns `true` when `table` exists in the SQLite schema at `conn`.
/// Propagates a typed static table name into the error variant so
/// partial-failure paths stay attributable. Used by [`table_has_rows`]
/// and the per-domain `migrate_*_rows_from_conn` bodies as a typed
/// pre-check that replaces the previous `msg.contains("no such table")`
/// arms.
fn legacy_table_exists_named(
    conn: &Connection,
    table: &'static str,
) -> Result<bool, MigrationError> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        rusqlite::params![table],
        |row| row.get::<_, i64>(0).map(|c| c > 0),
    )
    .map_err(|e| MigrationError::LegacyDbRead { table, source: e })
}

/// Outcome counters from one [`migrate_wallet_meta_rows`] pass.
/// `imported` includes idempotent re-imports — re-running the migration
/// after success is a per-row `set()` overwrite, not a no-op skip, so
/// the counter is meaningful even when the sidecar already holds the
/// same value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct WalletMetaMigrationOutcome {
    /// Rows for `app_context.network` written into the wallet-meta
    /// sidecar. A re-run with the same legacy rows lands here again —
    /// `set` upserts.
    imported: u32,
    /// Rows that could not be decoded (seed-hash wrong length).
    /// Triggers the error path — sentinel stays unwritten.
    failed: u32,
}

/// Copies legacy `wallet` rows (alias / `is_main` / `core_wallet_name`)
/// into the DET wallet-metadata sidecar. Idempotent. `core_wallet_name`
/// is optional — a recent legacy schema drop means older installs may
/// still have it, so the reader probes for it at row-read time.
fn migrate_wallet_meta_rows(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;

    let Some(path) = app_context.db.db_file_path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let path_str = path.to_string_lossy().to_string();
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path_str,
        source: e,
    })?;

    let view = backend.wallet_meta();
    let outcome = migrate_wallet_meta_rows_from_conn(
        &conn,
        |seed_hash, meta| view.set(app_context.network, &seed_hash, &meta),
        app_context.network,
    )?;
    tracing::info!(
        target = "migration::finish_unwire",
        imported = outcome.imported,
        failed = outcome.failed,
        network = ?app_context.network,
        "Wallet-meta migration pass complete",
    );

    if outcome.failed > 0 {
        return Err(MigrationError::WalletMetaPartialFailure {
            imported: outcome.imported,
            failed: outcome.failed,
        }
        .into());
    }
    Ok(())
}

/// Pure migration body (testable without an `AppContext`). Forwards each
/// `(seed_hash, meta)` row at `conn` to `set`; returns counters. A missing
/// table or missing `core_wallet_name` column is not an error.
fn migrate_wallet_meta_rows_from_conn<F>(
    conn: &Connection,
    mut set: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<WalletMetaMigrationOutcome, MigrationError>
where
    F: FnMut(
        crate::model::wallet::WalletSeedHash,
        crate::model::wallet::meta::WalletMeta,
    ) -> Result<(), TaskError>,
{
    if !legacy_table_exists_named(conn, "wallet")? {
        return Ok(WalletMetaMigrationOutcome::default());
    }
    // `core_wallet_name` is the only optional column, so it is probed and
    // NULL-substituted. `uses_password`/`password_hint` are read unprobed —
    // the seed migration selects them unconditionally and runs first over
    // the same table, so a schema lacking them already fails there.
    let core_wallet_name_present = wallet_table_has_core_wallet_name(conn)?;
    let sql = if core_wallet_name_present {
        "SELECT seed_hash, alias, is_main, core_wallet_name, master_ecdsa_bip44_account_0_epk, \
         uses_password, password_hint \
         FROM wallet WHERE network = ?1"
    } else {
        "SELECT seed_hash, alias, is_main, NULL AS core_wallet_name, \
         master_ecdsa_bip44_account_0_epk, uses_password, password_hint \
         FROM wallet WHERE network = ?1"
    };

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "wallet",
            source: e,
        })?;

    let rows = stmt
        .query_map(rusqlite::params![network.to_string()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?;
            let is_main: Option<bool> = row.get(2)?;
            let core_wallet_name: Option<String> = row.get(3)?;
            let xpub_encoded: Vec<u8> = row.get(4)?;
            let uses_password: bool = row.get(5)?;
            let password_hint: Option<String> = row.get(6)?;
            Ok((
                seed_hash,
                alias,
                is_main,
                core_wallet_name,
                xpub_encoded,
                uses_password,
                password_hint,
            ))
        })
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "wallet",
            source: e,
        })?;

    let mut outcome = WalletMetaMigrationOutcome::default();
    for row in rows {
        let (
            seed_hash_bytes,
            alias,
            is_main,
            core_wallet_name,
            xpub_encoded,
            uses_password,
            password_hint,
        ) = match row {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?e,
                    "Skipping unreadable wallet row",
                );
                outcome.failed = outcome.failed.saturating_add(1);
                continue;
            }
        };

        let seed_hash: crate::model::wallet::WalletSeedHash =
            match seed_hash_bytes.as_slice().try_into() {
                Ok(b) => b,
                Err(_) => {
                    tracing::warn!(
                        target = "migration::finish_unwire",
                        blob_len = seed_hash_bytes.len(),
                        "Skipping wallet row with non-32-byte seed_hash",
                    );
                    outcome.failed = outcome.failed.saturating_add(1);
                    continue;
                }
            };

        let meta = crate::model::wallet::meta::WalletMeta {
            alias: alias.unwrap_or_default(),
            is_main: is_main.unwrap_or(false),
            core_wallet_name,
            xpub_encoded,
            // Carry the legacy `wallet` row's password flag/hint straight into
            // WalletMeta so the persisted metadata is accurate from cold-start:
            // a protected wallet stays `uses_password = true` (Tier-2 keeps the
            // password; nothing downgrades it), keeping the metadata and the
            // at-rest scheme always in agreement.
            uses_password,
            password_hint,
        };

        match set(seed_hash, meta) {
            Ok(()) => outcome.imported = outcome.imported.saturating_add(1),
            Err(e) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?e,
                    "Failed to write wallet-meta entry",
                );
                outcome.failed = outcome.failed.saturating_add(1);
            }
        }
    }

    Ok(outcome)
}

/// Probe whether the legacy `wallet` table still carries
/// `core_wallet_name`. A recent legacy schema migration drops the
/// column, so older installs may still have it while freshly-migrated
/// ones will not. Missing table reads as "column absent" — the caller
/// short-circuits via the typed `legacy_table_exists_named` pre-check
/// before this probe runs.
fn wallet_table_has_core_wallet_name(conn: &Connection) -> Result<bool, MigrationError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet') WHERE name = 'core_wallet_name'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "wallet",
            source: e,
        })?;
    Ok(count > 0)
}

/// Outcome counters from one [`migrate_wallet_seeds_rows`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct WalletSeedsMigrationOutcome {
    /// Rows whose full envelope was written to the upstream vault.
    imported: u32,
    /// Rows that would not survive cold-boot hydration (see
    /// [`hd_seed_row_is_hydratable`]); non-fatal, logged and counted rather
    /// than silently copied. The seed stays in legacy `data.db` — this is
    /// exclusion, not data loss.
    skipped_malformed: u32,
    /// Rows that could not be decoded (seed_hash wrong size, blob
    /// length wrong, etc.). Triggers the error path.
    failed: u32,
}

/// Copies each legacy `wallet` row's full encrypted envelope (ciphertext +
/// salt + nonce + flags + xpub) into the upstream vault via
/// [`WalletSeedView`](crate::wallet_backend::WalletSeedView) without
/// decrypting it, so protected and unprotected rows take the same path.
/// Idempotent: re-running overwrites the same envelope under the same
/// `WalletId`.
fn migrate_wallet_seeds_rows(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;

    let Some(path) = app_context.db.db_file_path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let path_str = path.to_string_lossy().to_string();
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path_str,
        source: e,
    })?;

    let view = backend.wallet_seeds();
    let outcome = migrate_wallet_seeds_rows_from_conn(
        &conn,
        |seed_hash, envelope| view.set(&seed_hash, &envelope),
        app_context.network,
    )?;

    tracing::info!(
        target = "migration::finish_unwire",
        imported = outcome.imported,
        skipped_malformed = outcome.skipped_malformed,
        failed = outcome.failed,
        network = ?app_context.network,
        "Wallet-seed migration pass complete",
    );

    if outcome.failed > 0 {
        return Err(MigrationError::WalletSeedsPartialFailure {
            imported: outcome.imported,
            failed: outcome.failed,
        }
        .into());
    }
    Ok(())
}

/// Pure wallet-seed migration body — readable without an `AppContext`.
/// Walks the `wallet` table at `conn` filtered to `network` and forwards
/// each `(seed_hash, envelope)` pair to `set`. Returns counters; never
/// errors on partial readability so the caller can decide the policy.
///
/// **Missing table is not an error** — a freshly-installed `data.db`
/// (no legacy rows at all) returns the zero outcome.
fn migrate_wallet_seeds_rows_from_conn<F>(
    conn: &Connection,
    mut set: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<WalletSeedsMigrationOutcome, MigrationError>
where
    F: FnMut(
        crate::model::wallet::WalletSeedHash,
        crate::model::wallet::seed_envelope::StoredSeedEnvelope,
    ) -> Result<(), TaskError>,
{
    if !legacy_table_exists_named(conn, "wallet")? {
        return Ok(WalletSeedsMigrationOutcome::default());
    }
    let sql = "SELECT seed_hash, encrypted_seed, salt, nonce, password_hint, \
               uses_password, master_ecdsa_bip44_account_0_epk \
               FROM wallet WHERE network = ?1";

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "wallet",
            source: e,
        })?;

    let rows = stmt
        .query_map(rusqlite::params![network.to_string()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let encrypted_seed: Vec<u8> = row.get(1)?;
            let salt: Vec<u8> = row.get(2)?;
            let nonce: Vec<u8> = row.get(3)?;
            let password_hint: Option<String> = row.get(4)?;
            let uses_password: bool = row.get(5)?;
            let xpub_encoded: Vec<u8> = row.get(6)?;
            Ok((
                seed_hash,
                encrypted_seed,
                salt,
                nonce,
                password_hint,
                uses_password,
                xpub_encoded,
            ))
        })
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "wallet",
            source: e,
        })?;

    let mut outcome = WalletSeedsMigrationOutcome::default();
    for row in rows {
        let (seed_hash_bytes, encrypted_seed, salt, nonce, password_hint, uses_password, xpub) =
            match row {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        target = "migration::finish_unwire",
                        error = ?e,
                        "Skipping unreadable wallet row during seed migration",
                    );
                    outcome.failed = outcome.failed.saturating_add(1);
                    continue;
                }
            };

        let seed_hash: crate::model::wallet::WalletSeedHash =
            match seed_hash_bytes.as_slice().try_into() {
                Ok(b) => b,
                Err(_) => {
                    tracing::warn!(
                        target = "migration::finish_unwire",
                        blob_len = seed_hash_bytes.len(),
                        "Skipping wallet row with non-32-byte seed_hash during seed migration",
                    );
                    outcome.failed = outcome.failed.saturating_add(1);
                    continue;
                }
            };

        // Salt/nonce length sanity. AES-GCM requires a
        // 16-byte Argon2 salt and a 12-byte GCM nonce when the row is
        // password-protected; when it isn't, both fields must be
        // empty. Anything else is row-level corruption — skip and
        // log, do NOT abort the whole migration.
        if !crypto_field_lengths_ok(&salt, &nonce, uses_password) {
            tracing::warn!(
                target = "migration::finish_unwire",
                seed_hash = %hex::encode(seed_hash),
                salt_len = salt.len(),
                nonce_len = nonce.len(),
                uses_password,
                "Skipping wallet row with corrupted crypto field lengths during seed migration",
            );
            outcome.failed = outcome.failed.saturating_add(1);
            continue;
        }

        // Hydration symmetry. The copy step must not accept a row that
        // cold-boot hydration would silently drop, or the migrated wallet would
        // be in neither the registered nor the gate-counted set and the sentinel
        // would falsely read "done". A row that fails the shared hydratability
        // check is genuinely unusable (no derivable xpub / corrupt seed); skip
        // and surface it (non-fatal) rather than copy it. The seed stays in
        // legacy `data.db`, which the migration never deletes.
        if !hd_seed_row_is_hydratable(uses_password, &encrypted_seed, &xpub) {
            tracing::warn!(
                target = "migration::finish_unwire",
                seed_hash = %hex::encode(seed_hash),
                uses_password,
                seed_len = encrypted_seed.len(),
                xpub_len = xpub.len(),
                "Skipping wallet row that cold-boot hydration would drop (xpub absent/undecodable or unprotected seed length != 64); seed retained in legacy data.db",
            );
            outcome.skipped_malformed = outcome.skipped_malformed.saturating_add(1);
            continue;
        }

        let envelope = crate::model::wallet::seed_envelope::StoredSeedEnvelope {
            encrypted_seed,
            salt,
            nonce,
            password_hint,
            uses_password,
            xpub_encoded: xpub,
        };

        match set(seed_hash, envelope) {
            Ok(()) => outcome.imported = outcome.imported.saturating_add(1),
            Err(e) => {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    seed_hash = %hex::encode(seed_hash),
                    error = ?e,
                    "Failed to write wallet-seed envelope entry",
                );
                outcome.failed = outcome.failed.saturating_add(1);
            }
        }
    }

    Ok(outcome)
}

/// Public data-loss gate for the future legacy single-key table cleanup
/// (T7). Returns `Ok(())` only when the legacy `single_key_wallet` table
/// for the active network may be safely dropped — i.e. every
/// password-protected row has a matching restored entry in the modern
/// single-key sidecar. Otherwise returns
/// [`TaskError::MigrationFailed`] wrapping
/// [`MigrationError::ProtectedSingleKeysNotRestored`].
///
/// **Any cleanup path that removes the legacy single-key table MUST call
/// this first and abort on error** (Smythe S3). The production
/// `is_restored` predicate consults the modern single-key index: a
/// legacy protected address counts as restored once an
/// [`ImportedKey`](crate::model::single_key::ImportedKey) exists at the
/// same address — regardless of whether the user re-protected it with a
/// new passphrase. A key restored without a new passphrase is just as
/// recovered as one with, so keying on address presence (not
/// `has_passphrase`) is what makes the gate eventually release.
pub fn ensure_legacy_single_key_table_droppable(
    app_context: &Arc<AppContext>,
) -> Result<(), TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;
    let Some(path) = app_context.db.db_file_path() else {
        // In-memory / headless: no legacy file, nothing to gate.
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path.to_string_lossy().to_string(),
        source: e,
    })?;

    // Snapshot every restored address once so the per-row predicate is a
    // cheap set lookup. Presence in the modern index — not the passphrase
    // flag — is the restored signal: the import path always mirrors the
    // recovered key into the index whether or not the user chose a new
    // passphrase.
    let restored: std::collections::BTreeSet<String> = backend
        .single_key()
        .list()
        .into_iter()
        .map(|k| k.address)
        .collect();

    guard_single_key_table_droppable(&conn, |addr| restored.contains(addr), app_context.network)?;
    Ok(())
}

/// The ONE sanctioned production path to remove the legacy
/// `single_key_wallet` table (future T7 cleanup). It drops the table ONLY
/// after the data-loss gate confirms every protected row is restored; the
/// gate is run inside this function, so a future cleanup caller cannot
/// forget it (Smythe S3 / S5). On a blocked drop it returns
/// [`TaskError::MigrationFailed`] wrapping
/// [`MigrationError::ProtectedSingleKeysNotRestored`] and leaves the
/// table — and every key — intact.
///
/// A re-run after a successful drop is a no-op (`DROP TABLE IF EXISTS`),
/// and an in-memory / absent legacy file is a no-op success.
pub fn drop_legacy_single_key_table_when_safe(
    app_context: &Arc<AppContext>,
) -> Result<(), TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;
    let Some(path) = app_context.db.db_file_path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path.to_string_lossy().to_string(),
        source: e,
    })?;
    let restored: std::collections::BTreeSet<String> = backend
        .single_key()
        .list()
        .into_iter()
        .map(|k| k.address)
        .collect();
    drop_legacy_single_key_table(&conn, |addr| restored.contains(addr), app_context.network)?;
    Ok(())
}

/// Read the completion sentinel for `network` from `det-app.sqlite`.
fn read_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<Option<MigrationCompletion>, MigrationError> {
    app_kv
        .get::<MigrationCompletion>(DetScope::Global, &sentinel_key_for(network))
        .map_err(|e| MigrationError::Sentinel { source: e })
}

/// Write the completion sentinel for `network`, marking the migration
/// as finished for this network on this install.
fn write_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
    network_count: u32,
) -> Result<(), MigrationError> {
    let completion = MigrationCompletion {
        completed_at: now_epoch_seconds(),
        sha: env!("CARGO_PKG_VERSION").to_string(),
        network_count,
    };
    app_kv
        .put(DetScope::Global, &sentinel_key_for(network), &completion)
        .map_err(|e| MigrationError::Sentinel { source: e })
}

fn now_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl From<MigrationError> for TaskError {
    fn from(source: MigrationError) -> Self {
        TaskError::MigrationFailed {
            source: Arc::new(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::DetKv;
    use crate::wallet_backend::kv_test_support::InMemoryKv;

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    /// TC-MIG-009 — calling the migration when the sentinel for the
    /// active network is already present must be a no-op. The
    /// orchestrator must not consult legacy `data.db`, must not move
    /// state into `Running`, and must leave the sentinel untouched.
    #[test]
    fn sentinel_short_circuits_run() {
        use dash_sdk::dpp::dashcore::Network;

        let kv = kv();
        let original = MigrationCompletion {
            completed_at: 1234,
            sha: "test-sha".into(),
            network_count: 1,
        };
        kv.put(
            DetScope::Global,
            &sentinel_key_for(Network::Testnet),
            &original,
        )
        .expect("seed sentinel");

        // Reading the sentinel back via the same path the orchestrator
        // uses is the contractual short-circuit hook. If this returns
        // `Some`, the orchestrator skips legacy detection entirely.
        let observed: Option<MigrationCompletion> =
            read_sentinel(&kv, Network::Testnet).expect("read sentinel");
        assert_eq!(observed, Some(original));
    }

    /// Round-trip: writing the sentinel and reading it back yields the
    /// same payload. Guards the codec from accidental shape drift.
    #[test]
    fn sentinel_round_trip() {
        use dash_sdk::dpp::dashcore::Network;

        let kv = kv();
        write_sentinel(&kv, Network::Mainnet, 1).expect("write sentinel");
        let completion = read_sentinel(&kv, Network::Mainnet)
            .expect("read")
            .expect("present");
        assert_eq!(completion.network_count, 1);
        assert!(completion.completed_at > 0);
        assert_eq!(completion.sha, env!("CARGO_PKG_VERSION"));
    }

    /// The sentinel is scoped per network — writing the mainnet sentinel
    /// must not satisfy a subsequent testnet read, or a network switch
    /// would leave testnet wallets permanently unmigrated behind a
    /// stale-looking global sentinel.
    #[test]
    fn sentinel_is_per_network_mainnet_then_testnet() {
        use dash_sdk::dpp::dashcore::Network;

        let kv = kv();
        // Step 1: simulate a successful mainnet migration.
        write_sentinel(&kv, Network::Mainnet, 1).expect("write mainnet sentinel");
        assert!(
            read_sentinel(&kv, Network::Mainnet)
                .expect("read mainnet")
                .is_some(),
            "mainnet sentinel must be visible to a mainnet read",
        );
        // Step 2: switching to testnet must NOT short-circuit. The
        // testnet read returns `None` so the orchestrator proceeds to
        // detect-and-drain legacy testnet rows.
        assert!(
            read_sentinel(&kv, Network::Testnet)
                .expect("read testnet")
                .is_none(),
            "mainnet sentinel must not satisfy a testnet read — \
             per-network sentinel regression",
        );
        // Step 3: a clean testnet migration writes its own sentinel
        // without touching the mainnet one. Both then short-circuit
        // their respective networks.
        write_sentinel(&kv, Network::Testnet, 1).expect("write testnet sentinel");
        assert!(read_sentinel(&kv, Network::Mainnet).unwrap().is_some());
        assert!(read_sentinel(&kv, Network::Testnet).unwrap().is_some());
        // And the devnet / regtest sentinels are still independent.
        assert!(read_sentinel(&kv, Network::Devnet).unwrap().is_none());
        assert!(read_sentinel(&kv, Network::Regtest).unwrap().is_none());
    }

    /// Per-network sentinel keys are stable, distinct, and prefixed —
    /// guards against accidental key collisions or `Display` drift on
    /// upstream `Network`.
    #[test]
    fn sentinel_key_format_is_per_network() {
        use dash_sdk::dpp::dashcore::Network;

        let mainnet = sentinel_key_for(Network::Mainnet);
        let testnet = sentinel_key_for(Network::Testnet);
        let devnet = sentinel_key_for(Network::Devnet);
        let regtest = sentinel_key_for(Network::Regtest);

        assert_eq!(mainnet, "det:migration:finish_unwire:mainnet:v1");
        assert_eq!(testnet, "det:migration:finish_unwire:testnet:v1");
        assert_eq!(devnet, "det:migration:finish_unwire:devnet:v1");
        assert_eq!(regtest, "det:migration:finish_unwire:regtest:v1");
        // All four are distinct — a misencoded network would collapse
        // the sentinels and re-introduce the cross-network leak.
        let set: std::collections::HashSet<_> = [&mainnet, &testnet, &devnet, &regtest]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 4, "every network must get a unique sentinel key");
    }

    // ─────────────────────────────────────────────────────────────────
    // Helpers for the single-key migration tests below.
    // Mirror the legacy schema shape from `database/single_key_wallet.rs`
    // — the migration reads `encrypted_private_key`, `alias`,
    // `uses_password`, and `network`. The other columns are seeded so
    // the legacy table looks realistic, but the migration ignores them.
    // ─────────────────────────────────────────────────────────────────

    fn create_legacy_table(conn: &Connection) {
        conn.execute(
            "CREATE TABLE single_key_wallet (
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
                total_balance INTEGER DEFAULT 0,
                core_wallet_name TEXT DEFAULT NULL
            )",
            [],
        )
        .expect("create legacy table");
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_legacy_row(
        conn: &Connection,
        key_hash: &[u8; 32],
        encrypted_private_key: &[u8],
        salt: &[u8],
        nonce: &[u8],
        address: &str,
        alias: Option<&str>,
        uses_password: bool,
        network: dash_sdk::dpp::dashcore::Network,
    ) {
        conn.execute(
            "INSERT INTO single_key_wallet (
                key_hash, encrypted_private_key, salt, nonce, public_key,
                address, alias, uses_password, network
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                key_hash.as_slice(),
                encrypted_private_key,
                salt,
                nonce,
                // The migration does not consult `public_key`; seed an
                // empty blob to keep the column NOT NULL constraint
                // satisfied without dragging in PublicKey derivation.
                Vec::<u8>::new(),
                address,
                alias,
                uses_password as i32,
                network.to_string(),
            ],
        )
        .expect("insert legacy row");
    }

    /// Bare `SingleKeyView`-compatible fixture: no `WalletBackend`, just
    /// a file-backed secret store and an in-memory address index. This
    /// is what the migration body needs and lets the test assert on the
    /// real `SecretStore` writes without standing up an SDK.
    fn view_fixture(
        dir: &std::path::Path,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> (
        Arc<platform_wallet_storage::secrets::SecretStore>,
        std::sync::RwLock<
            std::collections::BTreeMap<String, crate::model::single_key::ImportedKey>,
        >,
        dash_sdk::dpp::dashcore::Network,
    ) {
        let store = Arc::new(
            crate::wallet_backend::single_key::open_secret_store(&dir.join("secrets.pwsvault"))
                .expect("open vault"),
        );
        let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
        (store, index, network)
    }

    /// TC-SK-001 — a legacy `uses_password=0` row gets imported into the
    /// secret store under the canonical `single_key_priv.<addr>` label.
    /// This is the post-upgrade "previously imported key still visible"
    /// regression guard: if this fails, the user sees nothing on the
    /// imported-keys list after the migration.
    #[test]
    fn tc_sk_001_uses_password_zero_row_migrates_to_secret_store() {
        use crate::wallet_backend::single_key::{
            SingleKeyView, label_for_address, single_key_namespace_id,
        };
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_table(&conn);

        // Build a known private key + derived address so the test can
        // assert on the canonical label.
        let mut raw = [0u8; 32];
        raw[31] = 7;
        let priv_key = PrivateKey::from_byte_array(&raw, Network::Testnet).expect("priv");
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&Secp256k1::new()),
        };
        let address = Address::p2pkh(&pub_key, Network::Testnet).to_string();
        let key_hash = crate::model::wallet::single_key::ClosedSingleKey::compute_key_hash(&raw);
        seed_legacy_row(
            &conn,
            &key_hash,
            &raw,
            &[],
            &[],
            &address,
            Some("paycheque"),
            false,
            Network::Testnet,
        );

        let (store, index, network) = view_fixture(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };

        let outcome = migrate_single_key_rows_from_conn(
            &conn,
            |wif, alias| view.import_wif(wif, alias).map(|_| ()),
            Network::Testnet,
        )
        .expect("migrate");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.skipped_password_protected, 0);
        assert_eq!(outcome.failed, 0);

        // The canonical secret-store label is present and decodes as
        // an unprotected SingleKeyEntry whose plaintext is 32 bytes
        // (with per-key passphrases the in-vault payload is the versioned
        // entry shape rather than the bare 32 raw bytes).
        let label = label_for_address(&address);
        let secret = store
            .get(&single_key_namespace_id(), &label)
            .expect("read secret")
            .expect("secret present");
        let entry =
            crate::wallet_backend::single_key_entry::SingleKeyEntry::decode(secret.expose_secret())
                .expect("decode entry");
        assert!(!entry.has_passphrase);
        let raw = entry.decrypt(None).expect("plaintext");
        assert_eq!(raw.len(), 32);

        // The view's index reflects the imported key (TC-SK-001's
        // "Imported key — <X[0..6]…>" UI surface relies on this).
        assert_eq!(view.list().len(), 1);
        assert_eq!(view.list()[0].address, address);
        assert_eq!(view.list()[0].alias.as_deref(), Some("paycheque"));
    }

    /// TC-SK-002 — running the migration twice is a no-op on the second
    /// pass. The label collision overwrites the same bytes (cheap
    /// idempotency) and the in-memory index entry count stays at 1.
    #[test]
    fn tc_sk_002_re_run_is_idempotent_and_does_not_duplicate() {
        use crate::wallet_backend::single_key::SingleKeyView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_table(&conn);

        let mut raw = [0u8; 32];
        raw[31] = 9;
        let key_hash = crate::model::wallet::single_key::ClosedSingleKey::compute_key_hash(&raw);
        seed_legacy_row(
            &conn,
            &key_hash,
            &raw,
            &[],
            &[],
            // Address derivation in the migration body is what matters
            // — the legacy column is informational. Pass a placeholder
            // that the NOT NULL constraint accepts.
            "placeholder",
            None,
            false,
            Network::Testnet,
        );

        let (store, index, network) = view_fixture(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };

        let first = migrate_single_key_rows_from_conn(
            &conn,
            |wif, alias| view.import_wif(wif, alias).map(|_| ()),
            Network::Testnet,
        )
        .expect("first pass");
        let second = migrate_single_key_rows_from_conn(
            &conn,
            |wif, alias| view.import_wif(wif, alias).map(|_| ()),
            Network::Testnet,
        )
        .expect("second pass");

        assert_eq!(first.imported, 1);
        assert_eq!(second.imported, 1, "re-import is reported as success");
        // The index does not duplicate — the address key is stable.
        assert_eq!(view.list().len(), 1, "re-run must not duplicate index");
    }

    /// TC-SK-006 — partial failures (corrupt blob + password-protected
    /// row) do not crash the migration. The good row still imports, the
    /// password-protected row counts as deferred, and the corrupt row
    /// is the sole `failed`. The fatal-vs-deferred split lets the
    /// orchestrator skip the sentinel write when `failed > 0` while
    /// pure password-protected runs still make forward progress.
    #[test]
    fn tc_sk_006_partial_failure_does_not_crash_run() {
        use crate::wallet_backend::single_key::SingleKeyView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_table(&conn);

        // Good row.
        let mut good = [0u8; 32];
        good[31] = 21;
        let good_hash = crate::model::wallet::single_key::ClosedSingleKey::compute_key_hash(&good);
        seed_legacy_row(
            &conn,
            &good_hash,
            &good,
            &[],
            &[],
            "good",
            None,
            false,
            Network::Testnet,
        );

        // Corrupt row: 16-byte blob — wrong size, drops into the
        // `failed` bucket without aborting the loop.
        let mut bad_hash = [0u8; 32];
        bad_hash[0] = 0xCC;
        seed_legacy_row(
            &conn,
            &bad_hash,
            &[0u8; 16],
            &[],
            &[],
            "bad",
            None,
            false,
            Network::Testnet,
        );

        // Password-protected row: `uses_password=1` — deferred to
        // T-SK-03, does not count as a failure.
        let mut pw_hash = [0u8; 32];
        pw_hash[0] = 0xAA;
        seed_legacy_row(
            &conn,
            &pw_hash,
            &[0xDE; 48], // ciphertext shape — never decoded here
            &[0x01; 16],
            &[0x02; 12],
            "pw",
            Some("locked"),
            true,
            Network::Testnet,
        );

        let (store, index, network) = view_fixture(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };

        let outcome = migrate_single_key_rows_from_conn(
            &conn,
            |wif, alias| view.import_wif(wif, alias).map(|_| ()),
            Network::Testnet,
        )
        .expect("partial failure must not abort the loop");

        assert_eq!(outcome.imported, 1, "the good row must still land");
        assert_eq!(
            outcome.skipped_password_protected, 1,
            "password-protected rows are deferred, not failed",
        );
        assert_eq!(
            outcome.failed, 1,
            "the 16-byte blob is the only true failure"
        );
    }

    /// Missing legacy table is not an error — a fresh install of the
    /// app reaches the single-key step with no `single_key_wallet`
    /// table at all and must report a clean zero outcome.
    #[test]
    fn missing_single_key_table_yields_zero_outcome() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open empty db");

        let outcome =
            migrate_single_key_rows_from_conn(&conn, |_wif, _alias| Ok(()), Network::Testnet)
                .expect("missing table is benign");

        assert_eq!(outcome, SingleKeyMigrationOutcome::default());
    }

    // ─────────────────────────────────────────────────────────────────
    // T-SK-03 / S3 — legacy single-key table DROP data-loss gate.
    // A password-protected (`uses_password=1`) row holds a key encrypted
    // under the user's OLD legacy password. Dropping the table before
    // that row is restored into the modern vault destroys the key
    // permanently. These tests pin the gate that forbids the drop while
    // any protected row remains un-restored.
    // ─────────────────────────────────────────────────────────────────

    /// Seed one protected and one unprotected legacy single-key row for
    /// the same network so the gate tests operate on a realistic table.
    fn seed_protected_and_unprotected(
        conn: &Connection,
        network: dash_sdk::dpp::dashcore::Network,
    ) {
        create_legacy_table(conn);
        // Protected row — encrypted under the legacy password (salt/nonce
        // present). The blob contents are irrelevant to the gate, which
        // only reads `address` + `uses_password`.
        seed_legacy_row(
            conn,
            &[1u8; 32],
            &[0xAB; 48],
            &[0x11; 16],
            &[0x22; 12],
            "yProtectedAddr",
            Some("protected"),
            true,
            network,
        );
        // Unprotected row — out of scope for the gate.
        seed_legacy_row(
            conn,
            &[2u8; 32],
            &[0xCD; 32],
            &[],
            &[],
            "yOpenAddr",
            Some("open"),
            false,
            network,
        );
    }

    /// S3 (must fail before the fix) — with a protected row present and
    /// NOT restored, the drop guard refuses, the typed error reports the
    /// remaining count, and the legacy rows survive the attempted drop.
    #[test]
    fn protected_row_blocks_table_drop_and_rows_survive() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("open legacy db");
        seed_protected_and_unprotected(&conn, Network::Testnet);

        // Nothing restored yet.
        let nothing_restored = |_addr: &str| false;

        let err = drop_legacy_single_key_table(&conn, nothing_restored, Network::Testnet)
            .expect_err("drop must be blocked while a protected row is un-restored");
        match err {
            MigrationError::ProtectedSingleKeysNotRestored { remaining } => {
                assert_eq!(remaining, 1, "exactly one protected row outstanding");
            }
            other => panic!("expected ProtectedSingleKeysNotRestored, got {other:?}"),
        }

        // The table — and crucially the protected row — must still be
        // present. A premature drop here would be permanent key loss.
        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM single_key_wallet", [], |r| r.get(0))
            .expect("table still exists after blocked drop");
        assert_eq!(row_count, 2, "no rows may be destroyed by a blocked drop");
        let protected_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM single_key_wallet WHERE uses_password = 1",
                [],
                |r| r.get(0),
            )
            .expect("query protected rows");
        assert_eq!(protected_count, 1, "the protected row must survive");
    }

    /// Once every protected row is restored (the predicate returns
    /// `true` for its address), the guard permits the drop and the table
    /// is removed.
    #[test]
    fn drop_succeeds_after_all_protected_rows_restored() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("open legacy db");
        seed_protected_and_unprotected(&conn, Network::Testnet);

        // The protected address is now present in the modern vault.
        let restored = |addr: &str| addr == "yProtectedAddr";

        drop_legacy_single_key_table(&conn, restored, Network::Testnet)
            .expect("drop allowed once protected rows are restored");

        let still_there: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='single_key_wallet'",
                [],
                |r| r.get::<_, i64>(0).map(|c| c > 0),
            )
            .expect("query sqlite_master");
        assert!(!still_there, "table must be dropped after restore");
    }

    /// A network with only unprotected rows carries no data-loss hazard,
    /// so the guard permits the drop even though nothing is "restored".
    #[test]
    fn unprotected_only_table_is_droppable_without_restore() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("open legacy db");
        create_legacy_table(&conn);
        seed_legacy_row(
            &conn,
            &[3u8; 32],
            &[0xCD; 32],
            &[],
            &[],
            "yOpenOnly",
            None,
            false,
            Network::Testnet,
        );

        assert_eq!(
            count_unrestored_protected_single_keys(&conn, |_| false, Network::Testnet)
                .expect("count"),
            0,
            "unprotected rows never count against the drop guard"
        );
        drop_legacy_single_key_table(&conn, |_| false, Network::Testnet)
            .expect("unprotected-only table is freely droppable");
    }

    /// A protected row on a DIFFERENT network must not block dropping the
    /// active network's table — the gate is per-network, matching the
    /// per-network migration scope.
    #[test]
    fn protected_row_on_other_network_does_not_block_drop() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("open legacy db");
        create_legacy_table(&conn);
        // Protected on mainnet, but we gate testnet.
        seed_legacy_row(
            &conn,
            &[4u8; 32],
            &[0xAB; 48],
            &[0x11; 16],
            &[0x22; 12],
            "XMainnetProtected",
            Some("mainnet"),
            true,
            Network::Mainnet,
        );

        assert_eq!(
            count_unrestored_protected_single_keys(&conn, |_| false, Network::Testnet)
                .expect("count"),
            0,
            "a mainnet protected row must not count against a testnet drop"
        );
    }

    /// A missing table is not a hazard — the guard reports zero remaining
    /// and the drop is a no-op success (fresh install / already cleaned).
    #[test]
    fn missing_table_is_droppable_no_op() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("empty.db")).expect("open empty db");
        assert_eq!(
            count_unrestored_protected_single_keys(&conn, |_| false, Network::Testnet)
                .expect("count"),
            0
        );
        drop_legacy_single_key_table(&conn, |_| false, Network::Testnet)
            .expect("missing table drop is a benign no-op");
    }

    /// `table_has_rows` returns `false` for a missing table rather than
    /// erroring — a fresh install lacks the legacy tables entirely.
    #[test]
    fn table_has_rows_treats_missing_table_as_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("legacy.db");
        let conn = Connection::open(&path).expect("open empty db");
        // No tables created — every legacy table is "missing".
        for &table in LEGACY_TABLES {
            assert!(
                !table_has_rows(&conn, table).expect("missing table is not an error"),
                "missing table `{table}` should report no rows",
            );
        }
    }

    // Wallet-meta migration fixtures + tests. Mirrors the legacy `wallet`
    // schema columns the migrator reads; both pre-drop and post-drop
    // shapes (with/without `core_wallet_name`) are covered.

    /// Legacy `wallet` schema including `core_wallet_name` (pre-drop).
    /// Matches the columns DET writes in `database/wallet.rs`'s
    /// `INSERT INTO wallet`.
    fn create_legacy_wallet_table_with_core_name(conn: &Connection) {
        conn.execute(
            "CREATE TABLE wallet (
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
                core_wallet_name TEXT DEFAULT NULL
            )",
            [],
        )
        .expect("create legacy wallet table");
    }

    /// Legacy `wallet` schema without `core_wallet_name` (post-drop —
    /// after the recent legacy schema migration removed the column).
    fn create_legacy_wallet_table_without_core_name(conn: &Connection) {
        conn.execute(
            "CREATE TABLE wallet (
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
        )
        .expect("create legacy wallet table");
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_legacy_wallet_row(
        conn: &Connection,
        seed_hash: &[u8; 32],
        alias: Option<&str>,
        is_main: bool,
        network: dash_sdk::dpp::dashcore::Network,
        core_wallet_name: Option<&str>,
        has_core_name_col: bool,
    ) {
        if has_core_name_col {
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, password_hint, network, core_wallet_name
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    alias,
                    is_main as i32,
                    0_i32,
                    Option::<String>::None,
                    network.to_string(),
                    core_wallet_name,
                ],
            )
            .expect("insert legacy wallet row");
        } else {
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, password_hint, network
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    alias,
                    is_main as i32,
                    0_i32,
                    Option::<String>::None,
                    network.to_string(),
                ],
            )
            .expect("insert legacy wallet row");
        }
    }

    /// In-memory wallet-meta view fixture using the same `InMemoryKv`
    /// fixture the other migration tests reuse — the wallet-meta sidecar
    /// writes through the global `det-app.sqlite` k/v, so a single shared
    /// store backs both the migrator and the reader.
    fn wallet_meta_view(kv: Arc<DetKv>) -> Arc<DetKv> {
        kv
    }

    /// TC-W-009 — a legacy `wallet` row's alias / `is_main` /
    /// `core_wallet_name` lands in the sidecar verbatim. This is the
    /// "name preserved across migration" regression guard: if this
    /// fails the wallet picker shows "Unnamed wallet" post-upgrade.
    #[test]
    fn tc_w_009_legacy_wallet_row_alias_preserved_in_meta() {
        use crate::model::wallet::meta::WalletMeta;
        use crate::wallet_backend::WalletMetaView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_with_core_name(&conn);

        let seed: crate::model::wallet::WalletSeedHash = [0x11; 32];
        seed_legacy_wallet_row(
            &conn,
            &seed,
            Some("paycheque"),
            true,
            Network::Testnet,
            Some("dev-dashd"),
            true,
        );
        // Foreign-network row must not bleed into the testnet pass.
        let other_seed: crate::model::wallet::WalletSeedHash = [0x22; 32];
        seed_legacy_wallet_row(
            &conn,
            &other_seed,
            Some("mainnet wallet"),
            true,
            Network::Mainnet,
            None,
            true,
        );

        let kv = wallet_meta_view(Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default()))));
        let view = WalletMetaView::new(&kv);

        let outcome = migrate_wallet_meta_rows_from_conn(
            &conn,
            |seed_hash, meta| view.set(Network::Testnet, &seed_hash, &meta),
            Network::Testnet,
        )
        .expect("migrate");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(
            view.get(Network::Testnet, &seed),
            Some(WalletMeta {
                alias: "paycheque".into(),
                is_main: true,
                core_wallet_name: Some("dev-dashd".into()),
                xpub_encoded: Vec::new(),
                uses_password: false,
                password_hint: None,
            })
        );
        // Mainnet row must not be visible on testnet.
        assert_eq!(view.get(Network::Testnet, &other_seed), None);
    }

    /// TC-W-001 (storage half) — the migrator writes a row that the
    /// listing path then surfaces. End-to-end (HD-wallet-visible) is
    /// verified after T-W-01 cuts the reader, but this guards the
    /// storage half today.
    #[test]
    fn tc_w_001_storage_half_listing_sees_migrated_row() {
        use crate::wallet_backend::WalletMetaView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_with_core_name(&conn);

        let seed: crate::model::wallet::WalletSeedHash = [0x33; 32];
        seed_legacy_wallet_row(
            &conn,
            &seed,
            Some("savings"),
            false,
            Network::Testnet,
            None,
            true,
        );

        let kv = wallet_meta_view(Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default()))));
        let view = WalletMetaView::new(&kv);
        let outcome = migrate_wallet_meta_rows_from_conn(
            &conn,
            |seed_hash, meta| view.set(Network::Testnet, &seed_hash, &meta),
            Network::Testnet,
        )
        .expect("migrate");
        assert_eq!(outcome.imported, 1);

        let listed = view.list(Network::Testnet);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, seed);
        assert_eq!(listed[0].1.alias, "savings");
    }

    /// TC-W-008-adjacent — running the migrator twice is a no-op on the
    /// second pass; the alias is upserted with the same bytes (cheap
    /// idempotency) and the listing still returns one entry.
    #[test]
    fn tc_w_008_re_run_is_idempotent_and_does_not_duplicate() {
        use crate::wallet_backend::WalletMetaView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_with_core_name(&conn);

        let seed: crate::model::wallet::WalletSeedHash = [0x44; 32];
        seed_legacy_wallet_row(
            &conn,
            &seed,
            Some("paycheque"),
            true,
            Network::Testnet,
            None,
            true,
        );

        let kv = wallet_meta_view(Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default()))));
        let view = WalletMetaView::new(&kv);
        let first = migrate_wallet_meta_rows_from_conn(
            &conn,
            |seed_hash, meta| view.set(Network::Testnet, &seed_hash, &meta),
            Network::Testnet,
        )
        .expect("first pass");
        let second = migrate_wallet_meta_rows_from_conn(
            &conn,
            |seed_hash, meta| view.set(Network::Testnet, &seed_hash, &meta),
            Network::Testnet,
        )
        .expect("second pass");

        assert_eq!(first.imported, 1);
        assert_eq!(second.imported, 1, "re-import is reported as success");
        assert_eq!(view.list(Network::Testnet).len(), 1, "no duplicate entry");
    }

    /// Missing legacy `wallet` table is benign — fresh installs that
    /// never wrote a wallet must reach the wallet-meta step with no
    /// table at all and return the zero outcome.
    #[test]
    fn missing_legacy_wallet_table_yields_zero_outcome() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open empty db");
        // No table created — the migrator must not error out.

        let outcome =
            migrate_wallet_meta_rows_from_conn(&conn, |_seed_hash, _meta| Ok(()), Network::Testnet)
                .expect("missing table is benign");
        assert_eq!(outcome, WalletMetaMigrationOutcome::default());
    }

    /// Post-drop schema: a legacy `wallet` table without
    /// `core_wallet_name` is the runtime reality after the recent
    /// schema migration. The migrator must keep working and store
    /// `core_wallet_name = None` in the sidecar.
    #[test]
    fn post_drop_schema_without_core_wallet_name_falls_back_to_none() {
        use crate::wallet_backend::WalletMetaView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);

        let seed: crate::model::wallet::WalletSeedHash = [0x55; 32];
        seed_legacy_wallet_row(
            &conn,
            &seed,
            Some("paycheque"),
            true,
            Network::Testnet,
            None,
            false,
        );

        let kv = wallet_meta_view(Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default()))));
        let view = WalletMetaView::new(&kv);
        let outcome = migrate_wallet_meta_rows_from_conn(
            &conn,
            |seed_hash, meta| view.set(Network::Testnet, &seed_hash, &meta),
            Network::Testnet,
        )
        .expect("migrate");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.failed, 0);
        let meta = view.get(Network::Testnet, &seed).expect("present");
        assert_eq!(meta.alias, "paycheque");
        assert!(meta.is_main);
        assert!(meta.core_wallet_name.is_none());
    }

    /// A corrupt row (16-byte `seed_hash` instead of 32) lands in the
    /// `failed` bucket without aborting the loop, so a sibling good
    /// row still imports.
    #[test]
    fn partial_failure_does_not_crash_wallet_meta_run() {
        use crate::wallet_backend::WalletMetaView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_with_core_name(&conn);

        let good_seed: crate::model::wallet::WalletSeedHash = [0x77; 32];
        seed_legacy_wallet_row(
            &conn,
            &good_seed,
            Some("good"),
            false,
            Network::Testnet,
            None,
            true,
        );
        // Corrupt: insert directly with a 16-byte seed_hash. SQLite
        // doesn't enforce blob length, so this is a legitimate way to
        // simulate a wedged row.
        conn.execute(
            "INSERT INTO wallet (
                seed_hash, encrypted_seed, salt, nonce,
                master_ecdsa_bip44_account_0_epk, alias, is_main,
                uses_password, password_hint, network, core_wallet_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                vec![0xCC_u8; 16].as_slice(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Option::<String>::None,
                0_i32,
                0_i32,
                Option::<String>::None,
                Network::Testnet.to_string(),
                Option::<String>::None,
            ],
        )
        .expect("insert corrupt row");

        let kv = wallet_meta_view(Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default()))));
        let view = WalletMetaView::new(&kv);
        let outcome = migrate_wallet_meta_rows_from_conn(
            &conn,
            |seed_hash, meta| view.set(Network::Testnet, &seed_hash, &meta),
            Network::Testnet,
        )
        .expect("partial failure does not abort the loop");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.failed, 1);
        assert!(view.get(Network::Testnet, &good_seed).is_some());
    }

    /// Insert a wallet row with the caller's chosen envelope columns —
    /// the full surface T-W-00.5-v2's seed migrator reads. Re-uses the
    /// `has_core_name_col = false` legacy schema (the post-drop shape)
    /// because the seed migration ignores `core_wallet_name`.
    #[allow(clippy::too_many_arguments)]
    fn seed_legacy_wallet_seed_row(
        conn: &Connection,
        seed_hash: &[u8; 32],
        encrypted_seed: &[u8],
        salt: &[u8],
        nonce: &[u8],
        master_xpub: &[u8],
        password_hint: Option<&str>,
        uses_password: bool,
        network: dash_sdk::dpp::dashcore::Network,
    ) {
        conn.execute(
            "INSERT INTO wallet (
                seed_hash, encrypted_seed, salt, nonce,
                master_ecdsa_bip44_account_0_epk, alias, is_main,
                uses_password, password_hint, network
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                seed_hash.as_slice(),
                encrypted_seed,
                salt,
                nonce,
                master_xpub,
                Option::<String>::None,
                0_i32,
                uses_password as i32,
                password_hint,
                network.to_string(),
            ],
        )
        .expect("insert legacy wallet row");
    }

    /// TC-W-001 storage half — a legacy `wallet` row's full envelope
    /// (ciphertext + salt + nonce + flags + xpub) round-trips through
    /// the view. The migrator never decrypts; the vault layer wraps
    /// the envelope with its own at-rest crypto.
    #[test]
    fn tc_w_001_envelope_round_trips_through_view() {
        use crate::wallet_backend::wallet_seed_store::WalletSeedView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);

        let seed_hash: crate::model::wallet::WalletSeedHash = [0xAA; 32];
        let ciphertext: [u8; 80] = [0x11; 80];
        let salt: [u8; 16] = [0x01; 16];
        let nonce: [u8; 12] = [0x02; 12];
        let xpub = valid_xpub([0x99u8; 64], Network::Testnet);
        seed_legacy_wallet_seed_row(
            &conn,
            &seed_hash,
            &ciphertext,
            &salt,
            &nonce,
            &xpub,
            Some("granny's birthday"),
            true,
            Network::Testnet,
        );

        let store = Arc::new(
            crate::wallet_backend::single_key::open_secret_store(
                &dir.path().join("secrets.pwsvault"),
            )
            .expect("open vault"),
        );
        let view = WalletSeedView::new(&store);

        let outcome = migrate_wallet_seeds_rows_from_conn(
            &conn,
            |seed_hash, envelope| view.set(&seed_hash, &envelope),
            Network::Testnet,
        )
        .expect("migrate");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.failed, 0);

        let got = view.get(&seed_hash).expect("get").expect("entry present");
        assert!(got.uses_password);
        assert_eq!(got.encrypted_seed, ciphertext.to_vec());
        assert_eq!(got.salt, salt.to_vec());
        assert_eq!(got.nonce, nonce.to_vec());
        assert_eq!(got.password_hint.as_deref(), Some("granny's birthday"));
        assert_eq!(got.xpub_encoded, xpub);
    }

    /// TC-W-002 — running the seed migration twice is idempotent: the
    /// second pass sees the same import count and the vault still
    /// holds the original envelope (upstream `set` upserts).
    #[test]
    fn tc_w_002_seed_migration_is_idempotent() {
        use crate::wallet_backend::wallet_seed_store::WalletSeedView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);

        let seed_hash: crate::model::wallet::WalletSeedHash = [0xBB; 32];
        let ciphertext: [u8; 64] = [0x22; 64];
        let xpub = valid_xpub([0x88u8; 64], Network::Testnet);
        seed_legacy_wallet_seed_row(
            &conn,
            &seed_hash,
            &ciphertext,
            &[],
            &[],
            &xpub,
            None,
            false,
            Network::Testnet,
        );

        let store = Arc::new(
            crate::wallet_backend::single_key::open_secret_store(
                &dir.path().join("secrets.pwsvault"),
            )
            .expect("open vault"),
        );
        let view = WalletSeedView::new(&store);

        let first = migrate_wallet_seeds_rows_from_conn(
            &conn,
            |seed_hash, envelope| view.set(&seed_hash, &envelope),
            Network::Testnet,
        )
        .expect("first pass");
        assert_eq!(first.imported, 1);

        let second = migrate_wallet_seeds_rows_from_conn(
            &conn,
            |seed_hash, envelope| view.set(&seed_hash, &envelope),
            Network::Testnet,
        )
        .expect("second pass");
        assert_eq!(second.imported, 1);
        assert_eq!(second.failed, 0);

        let got = view.get(&seed_hash).unwrap().unwrap();
        assert_eq!(got.encrypted_seed, ciphertext.to_vec());
        assert!(!got.uses_password);
    }

    /// A password-protected row travels through the vault verbatim:
    /// the migrator never decrypts, the ciphertext lands in the
    /// envelope, and `uses_password = true` is preserved so the
    /// unlock UI keeps prompting.
    #[test]
    fn password_protected_envelope_round_trips() {
        use crate::wallet_backend::wallet_seed_store::WalletSeedView;
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);

        let seed_hash: crate::model::wallet::WalletSeedHash = [0xCC; 32];
        let ciphertext: [u8; 80] = [0xFF; 80];
        let xpub = valid_xpub([0x77u8; 64], Network::Testnet);
        seed_legacy_wallet_seed_row(
            &conn,
            &seed_hash,
            &ciphertext,
            &[0x01; 16],
            &[0x02; 12],
            &xpub,
            Some("locked"),
            true,
            Network::Testnet,
        );

        let store = Arc::new(
            crate::wallet_backend::single_key::open_secret_store(
                &dir.path().join("secrets.pwsvault"),
            )
            .expect("open vault"),
        );
        let view = WalletSeedView::new(&store);

        let outcome = migrate_wallet_seeds_rows_from_conn(
            &conn,
            |seed_hash, envelope| view.set(&seed_hash, &envelope),
            Network::Testnet,
        )
        .expect("migrate");

        assert_eq!(
            outcome.imported, 1,
            "encrypted row migrates without decryption"
        );
        assert_eq!(outcome.failed, 0);

        let got = view.get(&seed_hash).expect("get").expect("present");
        assert!(got.uses_password);
        assert_eq!(got.encrypted_seed, ciphertext.to_vec());
        assert_eq!(got.password_hint.as_deref(), Some("locked"));
    }

    /// A row whose `seed_hash` blob is not 32 bytes counts as a
    /// failure rather than a silent overwrite. Catches schema drift /
    /// corrupt rows before they reach the vault.
    #[test]
    fn non_32_byte_seed_hash_is_failed_not_imported() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);

        // SQLite doesn't enforce blob length, so we can insert a wedged
        // 16-byte `seed_hash` to exercise the failed-decode path.
        conn.execute(
            "INSERT INTO wallet (
                seed_hash, encrypted_seed, salt, nonce,
                master_ecdsa_bip44_account_0_epk, alias, is_main,
                uses_password, password_hint, network
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                vec![0xCC_u8; 16].as_slice(),
                vec![0x00_u8; 64].as_slice(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Option::<String>::None,
                0_i32,
                0_i32,
                Option::<String>::None,
                Network::Testnet.to_string(),
            ],
        )
        .expect("insert corrupt row");

        let outcome = migrate_wallet_seeds_rows_from_conn(&conn, |_, _| Ok(()), Network::Testnet)
            .expect("migrate");
        assert_eq!(outcome.imported, 0);
        assert_eq!(outcome.failed, 1);
    }

    /// Foreign-network rows are partitioned by the `WHERE network = ?`
    /// filter. A mainnet row must not leak into a testnet seed
    /// migration pass.
    #[test]
    fn seed_migration_filters_by_network() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);

        let testnet_seed: crate::model::wallet::WalletSeedHash = [0xEE; 32];
        let testnet_xpub = valid_xpub([0x55u8; 64], Network::Testnet);
        seed_legacy_wallet_seed_row(
            &conn,
            &testnet_seed,
            &[0x33; 64],
            &[],
            &[],
            &testnet_xpub,
            None,
            false,
            Network::Testnet,
        );
        let mainnet_seed: crate::model::wallet::WalletSeedHash = [0xEF; 32];
        let mainnet_xpub = valid_xpub([0x66u8; 64], Network::Mainnet);
        seed_legacy_wallet_seed_row(
            &conn,
            &mainnet_seed,
            &[0x44; 64],
            &[],
            &[],
            &mainnet_xpub,
            None,
            false,
            Network::Mainnet,
        );

        let mut imported: Vec<crate::model::wallet::WalletSeedHash> = Vec::new();
        let outcome = migrate_wallet_seeds_rows_from_conn(
            &conn,
            |seed_hash, _| {
                imported.push(seed_hash);
                Ok(())
            },
            Network::Testnet,
        )
        .expect("migrate");

        assert_eq!(outcome.imported, 1);
        assert_eq!(imported, vec![testnet_seed]);
    }

    /// Missing `wallet` table is the freshly-installed shape — the
    /// migrator returns the zero outcome rather than erroring out.
    #[test]
    fn seed_migration_treats_missing_table_as_empty() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("data.db");
        let conn = Connection::open(&db_path).expect("open legacy db");

        let outcome = migrate_wallet_seeds_rows_from_conn(&conn, |_, _| Ok(()), Network::Testnet)
            .expect("migrate");

        assert_eq!(outcome.imported, 0);
        assert_eq!(outcome.failed, 0);
    }

    /// Encode a genuinely-decodable BIP44 master xpub from a seed, so a copy
    /// test can feed `hd_seed_row_is_hydratable`'s `ExtendedPubKey::decode` a
    /// real value (a random 78-byte blob would not validate).
    fn valid_xpub(seed: [u8; 64], network: dash_sdk::dpp::dashcore::Network) -> Vec<u8> {
        crate::model::wallet::Wallet::new_from_seed(seed, network, None, None)
            .expect("wallet from seed")
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec()
    }

    /// Regression: the copy step must reject exactly what cold-boot
    /// hydration would drop (empty/undecodable xpub, non-64-byte seed) —
    /// counted as `skipped_malformed`, never imported — while a well-formed
    /// sibling row still imports.
    #[test]
    fn qa_001_unhydratable_unprotected_rows_are_skipped_not_copied() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);
        let network = Network::Testnet;

        // Good row: valid 64-byte seed + decodable xpub → hydrates → imports.
        let good_hash: crate::model::wallet::WalletSeedHash = [0x01; 32];
        let good_xpub = valid_xpub([0x42u8; 64], network);
        seed_legacy_wallet_seed_row(
            &conn,
            &good_hash,
            &[0x42u8; 64],
            &[],
            &[],
            &good_xpub,
            None,
            false,
            network,
        );

        // Adversarial (a): empty master xpub — copy accepts it today, hydration
        // drops it via `reconstruct_from_envelope`.
        let empty_xpub_hash: crate::model::wallet::WalletSeedHash = [0x02; 32];
        seed_legacy_wallet_seed_row(
            &conn,
            &empty_xpub_hash,
            &[0x55u8; 64],
            &[],
            &[],
            &[],
            None,
            false,
            network,
        );

        // Adversarial (b): decodable xpub but a 32-byte unprotected seed —
        // hydration drops it via `wallet_from_envelope`'s length check.
        let short_seed_hash: crate::model::wallet::WalletSeedHash = [0x03; 32];
        let short_xpub = valid_xpub([0x11u8; 64], network);
        seed_legacy_wallet_seed_row(
            &conn,
            &short_seed_hash,
            &[0x66u8; 32],
            &[],
            &[],
            &short_xpub,
            None,
            false,
            network,
        );

        let mut imported = Vec::new();
        let outcome = migrate_wallet_seeds_rows_from_conn(
            &conn,
            |seed_hash, _envelope| {
                imported.push(seed_hash);
                Ok(())
            },
            network,
        )
        .expect("migrate");

        assert_eq!(
            outcome.imported, 1,
            "only the well-formed row may be copied"
        );
        assert_eq!(
            outcome.skipped_malformed, 2,
            "both unhydratable rows are skipped, not silently copied",
        );
        assert_eq!(
            outcome.failed, 0,
            "skips are non-fatal — no abort, no per-boot wedge",
        );
        assert_eq!(
            imported,
            vec![good_hash],
            "the skipped rows never reached the vault",
        );
    }

    /// A protected row with a decodable xpub is NOT seed-length-checked (it
    /// hydrates closed from its public xpub), so it copies even with an
    /// arbitrary-length ciphertext — mirroring hydration, which only
    /// length-checks unprotected seeds.
    #[test]
    fn qa_001_protected_row_with_valid_xpub_is_not_seed_length_checked() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("open legacy db");
        create_legacy_wallet_table_without_core_name(&conn);
        let network = Network::Testnet;

        let hash: crate::model::wallet::WalletSeedHash = [0x04; 32];
        let xpub = valid_xpub([0x77u8; 64], network);
        // Protected: 80-byte ciphertext, 16-byte salt, 12-byte nonce.
        seed_legacy_wallet_seed_row(
            &conn,
            &hash,
            &[0xABu8; 80],
            &[0x01; 16],
            &[0x02; 12],
            &xpub,
            Some("locked"),
            true,
            network,
        );

        let outcome =
            migrate_wallet_seeds_rows_from_conn(&conn, |_, _| Ok(()), network).expect("migrate");

        assert_eq!(
            outcome.imported, 1,
            "a protected row with a valid xpub copies"
        );
        assert_eq!(outcome.skipped_malformed, 0);
        assert_eq!(outcome.failed, 0);
    }

    /// Build a minimal, backend-unwired `AppContext` over a fresh `data.db`
    /// (legacy wallet-family tables present but empty). Enough to drive the
    /// two `run()` no-op paths, which return before touching the wallet
    /// backend.
    fn fresh_app_context(dir: &std::path::Path) -> Arc<AppContext> {
        use dash_sdk::dpp::dashcore::Network;

        crate::app_dir::ensure_env_file(dir);
        let db_file = dir.join("data.db");
        let db = Arc::new(crate::database::Database::new(&db_file).expect("db"));
        db.create_tables(true).expect("create tables");
        db.set_default_version().expect("set version");

        let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
        AppContext::new(
            dir.to_path_buf(),
            Network::Testnet,
            db,
            Default::default(),
            Default::default(),
            egui::Context::default(),
            app_kv,
            secret_store,
        )
        .expect("AppContext")
    }

    /// F113 — a launch with no legacy rows must report `did_work = false`
    /// and leave the migration state `Idle`, so the per-frame banner
    /// reconciler never shows a spurious "storage update complete".
    #[tokio::test]
    async fn run_with_no_legacy_rows_is_a_silent_noop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());

        let did_work = run(&ctx).await.expect("run");

        assert!(!did_work, "fresh install moved no data");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Idle),
            "no-op launch must stay Idle, not publish Success",
        );
    }

    /// F113 — once the per-network sentinel exists, a subsequent launch is
    /// a no-op: `did_work = false` and the state stays `Idle` (no banner).
    #[tokio::test]
    async fn run_with_sentinel_present_is_a_silent_noop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());

        // First launch writes the sentinel.
        run(&ctx).await.expect("first run");
        // Second launch short-circuits on the sentinel.
        let did_work = run(&ctx).await.expect("second run");

        assert!(!did_work, "sentinel-present launch moved no data");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Idle),
            "sentinel short-circuit must stay Idle",
        );
    }

    /// Fix-2 transient classification: only `WalletBackendUnavailable` is the
    /// retryable "backend not yet wired" condition that the cold-start
    /// dispatcher auto-retries. Every other variant is terminal — surfaced with
    /// a "Retry now" banner and never auto-looped — so a genuinely-failing
    /// registration or hydration cannot spin forever.
    #[test]
    fn only_wallet_backend_unavailable_is_backend_not_ready() {
        assert!(MigrationError::WalletBackendUnavailable.is_backend_not_ready());
        assert!(
            !MigrationError::RegistrationIncomplete { unregistered: 1 }.is_backend_not_ready(),
            "incomplete registration is terminal, not an auto-retry",
        );
        assert!(
            !MigrationError::Hydration {
                source: Box::new(TaskError::WalletNotFound),
            }
            .is_backend_not_ready(),
            "hydration failure is terminal, not an auto-retry",
        );
        assert!(
            !MigrationError::SingleKeyPartialFailure {
                imported: 0,
                skipped_password_protected: 0,
                failed: 1,
            }
            .is_backend_not_ready(),
        );
        assert!(
            !MigrationError::ProtectedSingleKeysNotRestored { remaining: 1 }.is_backend_not_ready(),
        );
    }

    /// Funds-safety invariant (Fix #3): a run that cannot finish — here because
    /// no wallet backend is wired in the fixture — MUST return `Err` and MUST
    /// NOT write the completion sentinel, so the migration retries on a later
    /// launch instead of falsely recording "done". This is the structural
    /// guarantee that `write_sentinel` runs only after the backend-dependent
    /// steps (registration included) succeed.
    #[tokio::test]
    async fn run_without_wired_backend_does_not_write_sentinel() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());

        // Seed a legacy single-key row so detection trips and the run advances
        // to the first backend-dependent step. `fresh_app_context` wires no
        // backend, so that step aborts with `WalletBackendUnavailable`. The
        // fixture's `single_key_wallet` schema matches `create_legacy_table`.
        {
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            seed_legacy_row(
                &conn,
                &[7u8; 32],
                &[1u8; 32],
                &[],
                &[],
                "addr",
                None,
                false,
                Network::Testnet,
            );
        }

        let result = run(&ctx).await;
        assert!(
            result.is_err(),
            "a run that cannot reach the wallet backend must fail, not complete",
        );

        let app_kv = ctx.app_kv();
        assert!(
            read_sentinel(&app_kv, ctx.network)
                .expect("read sentinel")
                .is_none(),
            "the completion sentinel must not be written when the migration aborts",
        );
    }
}
