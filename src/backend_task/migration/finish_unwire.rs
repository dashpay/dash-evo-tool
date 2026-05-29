//! Post-PR-#860 cold-start migration orchestrator.
//!
//! Drains legacy `data.db` rows that the unwire left behind into the
//! upstream `platform-wallet-storage` k/v store and `SecretStore`.
//! Idempotent: a completion sentinel under
//! [`SENTINEL_KEY`] in `det-app.sqlite` short-circuits subsequent
//! launches. Per-domain row-copy bodies are filled in by T-SK-02
//! (single-key wallets) and T-SH-02 (shielded rows); this scaffold
//! wires the orchestration, status reporting, and sentinel I/O.

use std::sync::Arc;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::wallet_backend::KvAdapterError;

/// Key in the shared `det-app.sqlite` k/v store under which the
/// completion sentinel is written. Versioned so a future format change
/// (e.g. additional checksum fields) bumps the key rather than
/// re-interpreting the existing payload.
pub const SENTINEL_KEY: &str = "det:migration:finish_unwire:v1";

/// Tables sniffed during detection. Any non-empty row count flips the
/// migration into the `Running` state. Ordered so the cheapest check
/// (the single-row `wallet` table) runs first.
const LEGACY_TABLES: &[&str] = &["wallet", "single_key_wallet", "shielded_notes", "utxos"];

/// Persisted sentinel payload. Lives in `det-app.sqlite` under
/// [`SENTINEL_KEY`]. `network_count` is informational — the migration
/// is global, not per-network, so a single row is enough.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationCompletion {
    /// Unix-epoch seconds at completion. Used for diagnostics — never
    /// parsed back into business logic.
    pub completed_at: i64,
    /// Git SHA / version tag of the running build. Lets a future
    /// reader correlate the sentinel with the binary that produced it.
    pub sha: String,
    /// How many network entries the migration walked. `0` means
    /// detection found no legacy rows on first launch.
    pub network_count: u32,
}

/// Domain error envelope for the migration orchestrator.
///
/// Variants wrap upstream error types via `#[source]`; the
/// user-facing message lives on [`TaskError::MigrationFailed`]. Adding
/// a row-copy body (T-SK-02 / T-SH-02) typically extends this enum
/// with a per-domain variant that wraps the relevant adapter error.
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

    /// At least one legacy `single_key_wallet` row could not be migrated
    /// in this run. Captures the imported / skipped / errored counts so
    /// the orchestrator can decide whether to write the sentinel —
    /// password-protected rows count as `skipped_password_protected`
    /// (T-SK-03 will surface a UX prompt to resolve them) while
    /// genuinely unreadable rows count as `failed`. Fatal only when
    /// `failed > 0`; pure password-protected runs leave the sentinel
    /// in place so the next launch picks them up after the user has
    /// supplied the password.
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

    /// The wallet backend was not yet wired when the migration ran.
    /// This is a hard configuration bug: the orchestrator runs after
    /// `ensure_wallet_backend`, so this should never fire in
    /// production. Kept as a typed variant so a future regression is
    /// caught immediately instead of silently no-oping.
    #[error("wallet backend not available during migration")]
    WalletBackendUnavailable,
}

/// Run the FinishUnwire migration. Idempotent — completes a no-op when
/// the sentinel is already present.
///
/// This is the orchestration skeleton. T-SK-02 plugs in the
/// single-key row-copy step; T-SH-02 plugs in the shielded mirror
/// step. Both hook in by adding their bodies to the `SingleKey` /
/// `Shielded` branches below and (if needed) extending
/// [`MigrationError`].
pub async fn run(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let status = app_context.migration_status();
    let app_kv = app_context.app_kv();

    // Idempotency: if the sentinel already exists, this launch has
    // nothing to do. Surface `Success` so the UI does not flash a
    // stale "in progress" banner from a previous frame.
    if let Some(completion) = read_sentinel(&app_kv)? {
        tracing::info!(
            target = "migration::finish_unwire",
            completed_at = completion.completed_at,
            sha = %completion.sha,
            network_count = completion.network_count,
            "FinishUnwire already completed — skipping",
        );
        status.set_state(MigrationState::Success);
        return Ok(());
    }

    status.set_state(MigrationState::Running {
        step: MigrationStep::Detecting,
    });

    let legacy_present = detect_legacy_rows(app_context)?;
    if !legacy_present {
        tracing::info!(
            target = "migration::finish_unwire",
            "No legacy data.db rows detected — writing sentinel without migration",
        );
        write_sentinel(&app_kv, 0)?;
        status.set_state(MigrationState::Success);
        return Ok(());
    }

    tracing::info!(
        target = "migration::finish_unwire",
        "Legacy data.db rows detected — beginning migration",
    );

    // T-SK-02 fills in the SecretStore row copy here.
    status.set_state(MigrationState::Running {
        step: MigrationStep::SingleKey,
    });
    migrate_single_key_rows(app_context).await?;

    // T-SH-02 fills in the shielded mirror here.
    status.set_state(MigrationState::Running {
        step: MigrationStep::Shielded,
    });
    migrate_shielded_rows(app_context).await?;

    // T-W-00 — mirror legacy `wallet` rows (alias / `is_main` /
    // `core_wallet_name`) into the DET wallet-metadata sidecar so the
    // wallet picker keeps the names a user already chose. Idempotent.
    status.set_state(MigrationState::Running {
        step: MigrationStep::WalletMeta,
    });
    migrate_wallet_meta_rows(app_context)?;

    status.set_state(MigrationState::Running {
        step: MigrationStep::Finalize,
    });
    write_sentinel(&app_kv, 1)?;
    tracing::info!(
        target = "migration::finish_unwire",
        "FinishUnwire migration complete",
    );
    status.set_state(MigrationState::Success);
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
/// tables. Catches `no such table` explicitly so a fresh install does
/// not trigger a migration loop.
fn table_has_rows(conn: &Connection, table: &'static str) -> Result<bool, MigrationError> {
    // Caller passes a static identifier from `LEGACY_TABLES`, so the
    // `format!` here cannot interpolate user input. SQLite parameter
    // binding does not support table names, so this is the canonical
    // shape.
    let sql = format!("SELECT 1 FROM {table} LIMIT 1");
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => {
            return Ok(false);
        }
        Err(rusqlite::Error::SqlInputError { msg, .. }) if msg.contains("no such table") => {
            return Ok(false);
        }
        Err(e) => return Err(MigrationError::LegacyDbRead { table, source: e }),
    };
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

/// Single-key row migration. Walks the legacy `single_key_wallet`
/// table for `network` and imports every `uses_password=0` row into
/// the upstream secret store under the canonical
/// `single_key_priv.<addr>` label. Idempotent: a re-run that sees the
/// same address as an existing secret-store entry is a no-op success
/// (covers TC-SK-002 — repeated launches must not duplicate).
///
/// Password-protected rows are deferred to T-SK-03's UX prompt —
/// they cannot be resolved without the user's password and so are
/// reported separately from genuine failures.
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

/// Pure migration body — readable without an `AppContext`. Walks the
/// `single_key_wallet` table at `conn`, decodes every row whose
/// `uses_password=0` blob is a 32-byte raw key, and imports the
/// derived WIF through `import` (kept as a closure so the test path
/// can drive a bare `SingleKeyView` without building a full
/// `WalletBackend`). Returns counters; never errors on partial
/// readability so the caller can decide the policy.
///
/// **Missing table is not an error** — a freshly-installed `data.db`
/// (no legacy rows at all) returns the zero outcome.
fn migrate_single_key_rows_from_conn<F>(
    conn: &Connection,
    mut import: F,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<SingleKeyMigrationOutcome, MigrationError>
where
    F: FnMut(&str, Option<String>) -> Result<(), TaskError>,
{
    use dash_sdk::dpp::dashcore::PrivateKey;

    let sql = "SELECT encrypted_private_key, alias, uses_password \
               FROM single_key_wallet WHERE network = ?1";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => {
            return Ok(SingleKeyMigrationOutcome::default());
        }
        Err(rusqlite::Error::SqlInputError { msg, .. }) if msg.contains("no such table") => {
            return Ok(SingleKeyMigrationOutcome::default());
        }
        Err(e) => {
            return Err(MigrationError::LegacyDbRead {
                table: "single_key_wallet",
                source: e,
            });
        }
    };

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

/// Counters from a single [`migrate_shielded_step`] pass. Internal so
/// the orchestrator can assert row-count parity post-mirror without
/// exposing the shape to other modules.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ShieldedMigrationOutcome {
    /// `shielded_notes` rows present in the sidecar for this network
    /// **after** the mirror — equal to the number of distinct
    /// `(wallet_seed_hash, nullifier)` legacy rows on the same network.
    notes_in_sidecar: u32,
    /// `shielded_wallet_meta` rows mirrored into the sidecar for this
    /// network.
    cursors_in_sidecar: u32,
}

/// Shielded row migration. Mirrors legacy `shielded_notes` +
/// `shielded_wallet_meta` rows for `app_context.network` into the
/// per-network sidecar exposed by [`WalletBackend::shielded`].
/// Idempotent: re-running with the same legacy rows is a silent no-op
/// (`INSERT OR IGNORE` on notes, `INSERT OR REPLACE` on cursors).
async fn migrate_shielded_rows(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;
    let Some(legacy_path) = app_context.db.db_file_path() else {
        return Ok(());
    };
    if !legacy_path.exists() {
        return Ok(());
    }
    let network_str = app_context.network.to_string();
    let outcome = migrate_shielded_step(backend.shielded(), &legacy_path, &network_str)?;
    tracing::info!(
        target = "migration::finish_unwire",
        notes = outcome.notes_in_sidecar,
        cursors = outcome.cursors_in_sidecar,
        network = %network_str,
        "Shielded mirror pass complete",
    );
    Ok(())
}

/// Pure shielded mirror — takes the sidecar view, the legacy `data.db`
/// path, and the network filter. Materialises the sidecar (writes go
/// through the view's open-or-create path), opens the legacy file
/// read-only via URI, ATTACHes it, and copies the filtered rows.
///
/// Missing legacy tables are not an error — a freshly-installed
/// install (or one that never created shielded rows) returns the zero
/// outcome.
fn migrate_shielded_step(
    sidecar: &crate::wallet_backend::ShieldedView,
    legacy_db_path: &std::path::Path,
    network: &str,
) -> Result<ShieldedMigrationOutcome, MigrationError> {
    // Pre-flight on a throwaway read-only conn: if the legacy file has
    // neither shielded table, bail out without touching the sidecar.
    // T-SH-01's lazy provisioning then leaves the sidecar absent on
    // disk for zero-shielded-activity users (FR-3.3 / TC-SH-003).
    {
        let probe = Connection::open(legacy_db_path).map_err(|e| MigrationError::LegacyDbOpen {
            path: legacy_db_path.to_string_lossy().to_string(),
            source: e,
        })?;
        let notes = legacy_table_exists(&probe, "shielded_notes")?;
        let meta = legacy_table_exists(&probe, "shielded_wallet_meta")?;
        if !notes && !meta {
            return Ok(ShieldedMigrationOutcome::default());
        }
    }

    // Force the sidecar file + schema into existence so we can open it
    // as the writable destination connection below.
    sidecar
        .ensure_materialized()
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "shielded_wallet_meta",
            source: e,
        })?;

    // Open the sidecar as the *destination* (writable) main connection
    // and ATTACH the legacy `data.db` read-only. SQLite makes the
    // attached database inherit the main connection's write capability,
    // so this orientation is required for `INSERT INTO main … SELECT
    // FROM legacy.…`. Mirrors the pattern in
    // `context/shielded.rs::migrate_commitment_tree_if_needed`.
    let dest = Connection::open(sidecar.path()).map_err(|e| MigrationError::LegacyDbOpen {
        path: sidecar.path().to_string_lossy().to_string(),
        source: e,
    })?;

    let legacy_path_str = legacy_db_path
        .to_str()
        .ok_or_else(|| MigrationError::LegacyDbRead {
            table: "shielded_notes",
            source: rusqlite::Error::InvalidParameterName(
                "legacy data.db path is not valid UTF-8".to_string(),
            ),
        })?;
    // `?mode=ro` keeps the migrator from acquiring a write lock on the
    // legacy file — a concurrent reader/writer in DET (shielded sync
    // still on the legacy path until T-SH-03) is therefore unaffected.
    let legacy_uri = format!("file:{legacy_path_str}?mode=ro");
    dest.execute_batch("PRAGMA foreign_keys = OFF")
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "shielded_notes",
            source: e,
        })?;
    dest.execute(
        "ATTACH DATABASE ?1 AS legacy",
        rusqlite::params![&legacy_uri],
    )
    .map_err(|e| MigrationError::LegacyDbRead {
        table: "shielded_notes",
        source: e,
    })?;

    let result: Result<ShieldedMigrationOutcome, MigrationError> = (|| {
        // Re-check existence against the *legacy* schema view now that
        // it's attached — the `main` view is the sidecar (always has
        // the tables).
        let legacy_notes_present = legacy_table_exists_in(&dest, "legacy", "shielded_notes")?;
        let legacy_meta_present = legacy_table_exists_in(&dest, "legacy", "shielded_wallet_meta")?;

        let mut outcome = ShieldedMigrationOutcome::default();
        if legacy_notes_present {
            // `INSERT OR IGNORE` honours the sidecar's
            // UNIQUE(wallet_seed_hash, nullifier, network) so a re-run
            // is a silent no-op. `id` is omitted — the sidecar
            // auto-increments fresh row ids on its own counter.
            dest.execute(
                "INSERT OR IGNORE INTO main.shielded_notes
                     (wallet_seed_hash, note_data, position, cmx, nullifier,
                      block_height, is_spent, value, network)
                     SELECT wallet_seed_hash, note_data, position, cmx, nullifier,
                            block_height, is_spent, value, network
                     FROM legacy.shielded_notes
                     WHERE network = ?1",
                rusqlite::params![network],
            )
            .map_err(|e| MigrationError::LegacyDbRead {
                table: "shielded_notes",
                source: e,
            })?;
            outcome.notes_in_sidecar = dest
                .query_row(
                    "SELECT COUNT(*) FROM main.shielded_notes WHERE network = ?1",
                    rusqlite::params![network],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| MigrationError::LegacyDbRead {
                    table: "shielded_notes",
                    source: e,
                })? as u32;
        }
        if legacy_meta_present {
            // `INSERT OR REPLACE` upserts on the
            // PRIMARY KEY(wallet_seed_hash, network) so re-runs with a
            // newer sync cursor monotonically advance the sidecar.
            dest.execute(
                "INSERT OR REPLACE INTO main.shielded_wallet_meta
                     (wallet_seed_hash, network,
                      last_nullifier_sync_height, last_nullifier_sync_timestamp)
                     SELECT wallet_seed_hash, network,
                            last_nullifier_sync_height, last_nullifier_sync_timestamp
                     FROM legacy.shielded_wallet_meta
                     WHERE network = ?1",
                rusqlite::params![network],
            )
            .map_err(|e| MigrationError::LegacyDbRead {
                table: "shielded_wallet_meta",
                source: e,
            })?;
            outcome.cursors_in_sidecar = dest
                .query_row(
                    "SELECT COUNT(*) FROM main.shielded_wallet_meta WHERE network = ?1",
                    rusqlite::params![network],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| MigrationError::LegacyDbRead {
                    table: "shielded_wallet_meta",
                    source: e,
                })? as u32;
        }
        Ok(outcome)
    })();

    // DETACH unconditionally so the dest connection is left clean even
    // on a partial-copy error. Swallow the detach error — the copy
    // error (if any) is the one we want to surface.
    let _ = dest.execute_batch("DETACH DATABASE legacy");
    result
}

/// `SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1` —
/// returns `false` for missing tables. Distinct from
/// [`table_has_rows`] so the shielded migrator can skip ATTACH entirely
/// when neither legacy table exists.
fn legacy_table_exists(conn: &Connection, name: &str) -> Result<bool, MigrationError> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        rusqlite::params![name],
        |row| row.get::<_, i64>(0).map(|c| c > 0),
    )
    .map_err(|e| MigrationError::LegacyDbRead {
        // `name` is a `&str`, but the variant wants `&'static str`. The
        // probe always runs against one of the two known table names —
        // surface the more user-meaningful "shielded_notes" here.
        table: "shielded_notes",
        source: e,
    })
}

/// Same as [`legacy_table_exists`] but addresses a specific schema by
/// name — used to probe the ATTACHed `legacy` database from the
/// destination connection in the shielded migrator.
fn legacy_table_exists_in(
    conn: &Connection,
    schema: &str,
    name: &str,
) -> Result<bool, MigrationError> {
    // SQLite parameter binding does not support schema names, so the
    // `format!` is the canonical shape. `schema` here is a static
    // string from the migrator (`"legacy"`).
    let sql = format!("SELECT COUNT(*) FROM {schema}.sqlite_master WHERE type='table' AND name=?1");
    conn.query_row(&sql, rusqlite::params![name], |row| {
        row.get::<_, i64>(0).map(|c| c > 0)
    })
    .map_err(|e| MigrationError::LegacyDbRead {
        table: "shielded_notes",
        source: e,
    })
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

/// T-W-00 wallet-meta migration. Copies legacy `wallet` rows (alias /
/// `is_main` / `core_wallet_name`) into the DET wallet-metadata sidecar
/// for `app_context.network`. Idempotent (per-row `set` upserts).
///
/// `core_wallet_name` is treated as optional at the schema level — a
/// recent legacy schema migration drops the column from the `wallet`
/// table, so older installs may still have it while freshly-migrated
/// ones will not. The probe at row-read time keeps the migrator
/// compatible with both shapes.
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

/// Pure wallet-meta migration body — readable without an `AppContext`.
/// Walks the `wallet` table at `conn` filtered to `network` and forwards
/// each `(seed_hash, meta)` pair to `set`. Returns counters; never
/// errors on partial readability so the caller can decide the policy.
///
/// **Missing table is not an error** — a freshly-installed `data.db`
/// (no legacy rows at all) returns the zero outcome.
/// **Missing `core_wallet_name` column is not an error** — the
/// recent legacy schema migration drops it; we fall back to `None`.
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
    let core_wallet_name_present = wallet_table_has_core_wallet_name(conn)?;
    let sql = if core_wallet_name_present {
        "SELECT seed_hash, alias, is_main, core_wallet_name \
         FROM wallet WHERE network = ?1"
    } else {
        "SELECT seed_hash, alias, is_main, NULL AS core_wallet_name \
         FROM wallet WHERE network = ?1"
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => {
            return Ok(WalletMetaMigrationOutcome::default());
        }
        Err(rusqlite::Error::SqlInputError { msg, .. }) if msg.contains("no such table") => {
            return Ok(WalletMetaMigrationOutcome::default());
        }
        Err(e) => {
            return Err(MigrationError::LegacyDbRead {
                table: "wallet",
                source: e,
            });
        }
    };

    let rows = stmt
        .query_map(rusqlite::params![network.to_string()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?;
            let is_main: Option<bool> = row.get(2)?;
            let core_wallet_name: Option<String> = row.get(3)?;
            Ok((seed_hash, alias, is_main, core_wallet_name))
        })
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "wallet",
            source: e,
        })?;

    let mut outcome = WalletMetaMigrationOutcome::default();
    for row in rows {
        let (seed_hash_bytes, alias, is_main, core_wallet_name) = match row {
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
/// then short-circuits via the prepared-statement `no such table`
/// branch.
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

/// NFR-4 pre-flight gate: returns `true` when the legacy `data.db`
/// holds at least one `shielded_notes` row for `network` **and** the
/// per-network sidecar is still absent. T-W-01's future wallet-state
/// cutover MUST consult this predicate and defer when it returns
/// `true` — surfaces via the migration banner per Diziet J-3
/// ("Verifying shielded balance…").
///
/// Returns `false` (proceed) when:
///   * no legacy `data.db` exists,
///   * the `shielded_notes` table is absent or empty for `network`,
///   * the sidecar file already exists on disk (which post-T-SH-02
///     means the mirror has run at least once).
pub fn legacy_shielded_present_but_sidecar_empty(
    app_context: &Arc<AppContext>,
) -> Result<bool, TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;
    let Some(legacy_path) = app_context.db.db_file_path() else {
        return Ok(false);
    };
    if !legacy_path.exists() {
        return Ok(false);
    }
    // Sidecar already on disk → mirror has run; nothing to gate on.
    if backend.shielded().path().exists() {
        return Ok(false);
    }
    let network_str = app_context.network.to_string();
    let conn = Connection::open(&legacy_path).map_err(|e| MigrationError::LegacyDbOpen {
        path: legacy_path.to_string_lossy().to_string(),
        source: e,
    })?;
    if !legacy_table_exists(&conn, "shielded_notes")? {
        return Ok(false);
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM shielded_notes WHERE network = ?1",
            rusqlite::params![network_str],
            |row| row.get(0),
        )
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "shielded_notes",
            source: e,
        })?;
    Ok(count > 0)
}

/// Read the completion sentinel from `det-app.sqlite`.
fn read_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
) -> Result<Option<MigrationCompletion>, MigrationError> {
    app_kv
        .get::<MigrationCompletion>(None, SENTINEL_KEY)
        .map_err(|e| MigrationError::Sentinel { source: e })
}

/// Write the completion sentinel, marking the migration as finished
/// for this install.
fn write_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
    network_count: u32,
) -> Result<(), MigrationError> {
    let completion = MigrationCompletion {
        completed_at: now_epoch_seconds(),
        sha: env!("CARGO_PKG_VERSION").to_string(),
        network_count,
    };
    app_kv
        .put(None, SENTINEL_KEY, &completion)
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
            source: Box::new(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::WalletSeedHash;
    use crate::wallet_backend::DetKv;
    use platform_wallet::wallet::platform_wallet::WalletId;
    use platform_wallet_storage::{KvError, KvStore};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Minimal in-memory `KvStore` that mirrors the shape used by the
    /// real `SqlitePersister` for adapter tests.
    #[derive(Default)]
    struct InMemoryKv {
        global: Mutex<BTreeMap<String, Vec<u8>>>,
        per_wallet: Mutex<BTreeMap<(WalletId, String), Vec<u8>>>,
    }

    impl KvStore for InMemoryKv {
        fn get(&self, wallet_id: Option<&WalletId>, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            match wallet_id {
                None => Ok(self.global.lock().unwrap().get(key).cloned()),
                Some(id) => Ok(self
                    .per_wallet
                    .lock()
                    .unwrap()
                    .get(&(*id, key.to_string()))
                    .cloned()),
            }
        }
        fn put(
            &self,
            wallet_id: Option<&WalletId>,
            key: &str,
            value: &[u8],
        ) -> Result<(), KvError> {
            match wallet_id {
                None => {
                    self.global
                        .lock()
                        .unwrap()
                        .insert(key.to_string(), value.to_vec());
                }
                Some(id) => {
                    self.per_wallet
                        .lock()
                        .unwrap()
                        .insert((*id, key.to_string()), value.to_vec());
                }
            }
            Ok(())
        }
        fn delete(&self, wallet_id: Option<&WalletId>, key: &str) -> Result<(), KvError> {
            match wallet_id {
                None => {
                    self.global.lock().unwrap().remove(key);
                }
                Some(id) => {
                    self.per_wallet
                        .lock()
                        .unwrap()
                        .remove(&(*id, key.to_string()));
                }
            }
            Ok(())
        }
        fn list_keys(
            &self,
            wallet_id: Option<&WalletId>,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            let take_prefixed = |keys: Vec<String>| -> Vec<String> {
                match prefix {
                    Some(p) => keys.into_iter().filter(|k| k.starts_with(p)).collect(),
                    None => keys,
                }
            };
            match wallet_id {
                None => Ok(take_prefixed(
                    self.global.lock().unwrap().keys().cloned().collect(),
                )),
                Some(id) => Ok(take_prefixed(
                    self.per_wallet
                        .lock()
                        .unwrap()
                        .keys()
                        .filter(|(wid, _)| wid == id)
                        .map(|(_, k)| k.clone())
                        .collect(),
                )),
            }
        }
    }

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    /// TC-MIG-009 — calling the migration when the sentinel is already
    /// present must be a no-op. The orchestrator must not consult
    /// legacy `data.db`, must not move state into `Running`, and must
    /// leave the sentinel untouched.
    #[test]
    fn sentinel_short_circuits_run() {
        let kv = kv();
        let original = MigrationCompletion {
            completed_at: 1234,
            sha: "test-sha".into(),
            network_count: 3,
        };
        kv.put(None, SENTINEL_KEY, &original)
            .expect("seed sentinel");

        // Reading the sentinel back via the same path the orchestrator
        // uses is the contractual short-circuit hook. If this returns
        // `Some`, the orchestrator skips legacy detection entirely.
        let observed: Option<MigrationCompletion> = read_sentinel(&kv).expect("read sentinel");
        assert_eq!(observed, Some(original));
    }

    /// Round-trip: writing the sentinel and reading it back yields the
    /// same payload. Guards the codec from accidental shape drift.
    #[test]
    fn sentinel_round_trip() {
        let kv = kv();
        write_sentinel(&kv, 7).expect("write sentinel");
        let completion = read_sentinel(&kv).expect("read").expect("present");
        assert_eq!(completion.network_count, 7);
        assert!(completion.completed_at > 0);
        assert_eq!(completion.sha, env!("CARGO_PKG_VERSION"));
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

        // The canonical secret-store label is present and holds 32 bytes.
        let label = label_for_address(&address);
        let secret = store
            .get(&single_key_namespace_id(), &label)
            .expect("read secret")
            .expect("secret present");
        assert_eq!(secret.expose_secret().len(), 32);

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

    // ─────────────────────────────────────────────────────────────────
    // T-SH-02 shielded mirror fixtures + tests.
    // Mirrors the legacy schema from `src/database/shielded.rs` so the
    // ATTACH+INSERT pass exercised below operates against the same
    // shape a real legacy `data.db` would expose.
    // ─────────────────────────────────────────────────────────────────

    fn create_legacy_shielded_tables(conn: &Connection) {
        conn.execute(
            "CREATE TABLE shielded_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                wallet_seed_hash BLOB NOT NULL,
                note_data BLOB NOT NULL,
                position INTEGER NOT NULL,
                cmx BLOB NOT NULL,
                nullifier BLOB NOT NULL,
                block_height INTEGER NOT NULL,
                is_spent INTEGER NOT NULL DEFAULT 0,
                value INTEGER NOT NULL,
                network TEXT NOT NULL,
                UNIQUE(wallet_seed_hash, nullifier, network)
            )",
            [],
        )
        .expect("create legacy shielded_notes");
        conn.execute(
            "CREATE TABLE shielded_wallet_meta (
                wallet_seed_hash BLOB NOT NULL,
                network TEXT NOT NULL,
                last_nullifier_sync_height INTEGER NOT NULL DEFAULT 0,
                last_nullifier_sync_timestamp INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (wallet_seed_hash, network)
            )",
            [],
        )
        .expect("create legacy shielded_wallet_meta");
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_legacy_note(
        conn: &Connection,
        seed: &[u8; 32],
        position: u64,
        nullifier_seed: u8,
        value: u64,
        is_spent: bool,
        network: &str,
    ) {
        conn.execute(
            "INSERT INTO shielded_notes
             (wallet_seed_hash, note_data, position, cmx, nullifier,
              block_height, is_spent, value, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                seed.as_slice(),
                vec![0xAA_u8; 16],
                position as i64,
                vec![position as u8; 32],
                vec![nullifier_seed; 32],
                100_i64 + position as i64,
                is_spent as i32,
                value as i64,
                network,
            ],
        )
        .expect("insert legacy note");
    }

    fn seed_legacy_meta(conn: &Connection, seed: &[u8; 32], network: &str, h: u64, ts: u64) {
        conn.execute(
            "INSERT INTO shielded_wallet_meta
             (wallet_seed_hash, network,
              last_nullifier_sync_height, last_nullifier_sync_timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![seed.as_slice(), network, h as i64, ts as i64],
        )
        .expect("insert legacy meta");
    }

    fn shielded_view(spv_dir: &std::path::Path) -> crate::wallet_backend::ShieldedView {
        std::fs::create_dir_all(spv_dir).expect("create spv dir");
        crate::wallet_backend::ShieldedView::new(spv_dir)
    }

    /// TC-SH-001 — legacy `shielded_notes` rows for the active network
    /// land in the sidecar with matching balances. Notes from a foreign
    /// network MUST stay behind so per-network isolation (TC-SH-009) is
    /// not regressed by the mirror.
    #[test]
    fn tc_sh_001_legacy_notes_mirror_into_sidecar_balances_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy_path = dir.path().join("data.db");
        let conn = Connection::open(&legacy_path).expect("open legacy db");
        create_legacy_shielded_tables(&conn);

        let seed: WalletSeedHash = [0x42; 32];
        seed_legacy_note(&conn, &seed, 0, 0xA1, 10, false, "testnet");
        seed_legacy_note(&conn, &seed, 1, 0xA2, 25, false, "testnet");
        seed_legacy_note(&conn, &seed, 2, 0xA3, 7, true, "testnet");
        // Foreign-network row must NOT be copied.
        seed_legacy_note(&conn, &seed, 3, 0xB1, 99, false, "mainnet");
        drop(conn);

        let sidecar = shielded_view(&dir.path().join("spv").join("testnet"));
        let outcome =
            migrate_shielded_step(&sidecar, &legacy_path, "testnet").expect("mirror runs");

        // 3 testnet rows mirrored — the mainnet row stays in the legacy
        // file. `notes_in_sidecar` counts post-copy sidecar rows.
        assert_eq!(outcome.notes_in_sidecar, 3);

        let unspent = sidecar
            .get_unspent_shielded_notes(&seed, "testnet")
            .expect("read unspent");
        assert_eq!(unspent.len(), 2, "spent note must not appear in unspent");

        let balance = sidecar
            .get_shielded_balance(&seed, "testnet")
            .expect("balance");
        assert_eq!(balance, 35, "balance equals sum of unspent values (10+25)");

        let all = sidecar
            .get_all_shielded_notes(&seed, "testnet")
            .expect("read all");
        assert_eq!(all.len(), 3, "all three testnet notes mirrored");
        assert!(
            sidecar
                .get_all_shielded_notes(&seed, "mainnet")
                .expect("read mainnet")
                .is_empty(),
            "mainnet row must not have leaked into the testnet sidecar",
        );
    }

    /// TC-SH-002 — the legacy `shielded_wallet_meta` cursor (sync
    /// height + timestamp) is preserved verbatim in the sidecar so the
    /// rewired sync path (T-SH-03) does not re-scan from zero.
    #[test]
    fn tc_sh_002_sync_cursor_preserved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy_path = dir.path().join("data.db");
        let conn = Connection::open(&legacy_path).expect("open legacy db");
        create_legacy_shielded_tables(&conn);

        let seed: WalletSeedHash = [0x55; 32];
        seed_legacy_meta(&conn, &seed, "testnet", 1_234_567, 1_700_000_000);
        // Foreign-network cursor — must not bleed into the testnet
        // mirror.
        seed_legacy_meta(&conn, &seed, "mainnet", 9_999_999, 1_800_000_000);
        drop(conn);

        let sidecar = shielded_view(&dir.path().join("spv").join("testnet"));
        let outcome =
            migrate_shielded_step(&sidecar, &legacy_path, "testnet").expect("mirror runs");
        assert_eq!(outcome.cursors_in_sidecar, 1);

        let (h, ts) = sidecar
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("read cursor");
        assert_eq!(h, 1_234_567);
        assert_eq!(ts, 1_700_000_000);

        // The mainnet cursor MUST be invisible from the testnet sidecar.
        let (mh, mt) = sidecar
            .get_nullifier_sync_info(&seed, "mainnet")
            .expect("read mainnet cursor");
        assert_eq!(
            (mh, mt),
            (0, 0),
            "foreign-network cursor must not appear in the testnet sidecar",
        );

        // Re-running the mirror is a no-op (idempotency) — the cursor
        // stays at the same value.
        let again = migrate_shielded_step(&sidecar, &legacy_path, "testnet").expect("re-run");
        assert_eq!(again.cursors_in_sidecar, 1);
        let (h2, ts2) = sidecar
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("read cursor post re-run");
        assert_eq!((h2, ts2), (h, ts));
    }

    /// TC-SH-008 — NFR-4 pre-flight: when the legacy `data.db` holds
    /// shielded rows but the sidecar is empty, the gate predicate
    /// returns `true` so the future T-W-01 wallet-state cutover defers
    /// until the mirror completes. After the mirror runs (sidecar
    /// materialised), the gate flips to `false` — proceed.
    #[test]
    fn tc_sh_008_nfr4_preflight_gates_wallet_cutover() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy_path = dir.path().join("data.db");
        let conn = Connection::open(&legacy_path).expect("open legacy db");
        create_legacy_shielded_tables(&conn);

        let seed: WalletSeedHash = [0x66; 32];
        seed_legacy_note(&conn, &seed, 0, 0xCC, 50, false, "testnet");
        drop(conn);

        let spv_dir = dir.path().join("spv").join("testnet");
        let sidecar = shielded_view(&spv_dir);
        // Sidecar file does not yet exist on disk (lazy provisioning).
        assert!(!sidecar.path().exists(), "sidecar absent before mirror");

        // Gate predicate (inlined to avoid building a full AppContext):
        // legacy file present + shielded_notes row count > 0 + sidecar
        // file absent ⇒ defer.
        let legacy = Connection::open(&legacy_path).expect("open legacy ro");
        let legacy_count: i64 = legacy
            .query_row(
                "SELECT COUNT(*) FROM shielded_notes WHERE network = ?1",
                rusqlite::params!["testnet"],
                |row| row.get(0),
            )
            .expect("count legacy");
        drop(legacy);
        let gate_before = legacy_count > 0 && !sidecar.path().exists() && legacy_path.exists();
        assert!(
            gate_before,
            "pre-flight gate must defer the wallet cutover when shielded data is present",
        );

        // Run the mirror — sidecar is materialised and rows land.
        let outcome =
            migrate_shielded_step(&sidecar, &legacy_path, "testnet").expect("mirror runs");
        assert_eq!(outcome.notes_in_sidecar, 1, "the one legacy note mirrors");
        assert!(
            sidecar.path().exists(),
            "mirror materialises the sidecar file",
        );

        // Post-mirror: gate flips to false (sidecar now present).
        let gate_after = legacy_count > 0 && !sidecar.path().exists() && legacy_path.exists();
        assert!(
            !gate_after,
            "post-mirror the gate must release the wallet cutover",
        );
    }

    /// Missing legacy shielded tables are not an error — a fresh
    /// install (or a wallet with no shielded activity) returns the
    /// zero outcome without touching the sidecar.
    #[test]
    fn missing_legacy_shielded_tables_yield_zero_outcome() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy_path = dir.path().join("data.db");
        Connection::open(&legacy_path).expect("create empty db");

        let sidecar = shielded_view(&dir.path().join("spv").join("testnet"));
        let outcome = migrate_shielded_step(&sidecar, &legacy_path, "testnet").expect("benign");
        assert_eq!(outcome, ShieldedMigrationOutcome::default());
    }

    // ─────────────────────────────────────────────────────────────────
    // T-W-00 wallet-meta migration fixtures + tests.
    //
    // Mirrors the legacy `wallet` schema from
    // `src/database/initialization.rs` for the columns the migrator
    // reads: `seed_hash`, `alias`, `is_main`, `network`,
    // `core_wallet_name`. The migrator drives the schema variant via
    // `wallet_table_has_core_wallet_name` so both the pre-drop and
    // post-drop shapes are covered.
    // ─────────────────────────────────────────────────────────────────

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
}
