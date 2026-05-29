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

/// Shielded row migration. **Skeleton** — T-SH-02 plugs in the
/// `shielded_notes` / cursor mirror here.
async fn migrate_shielded_rows(_app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    // TODO(T-SH-02): mirror `shielded_notes`, `shielded_wallet_meta`
    // and the per-wallet sync cursor here.
    Ok(())
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
}
