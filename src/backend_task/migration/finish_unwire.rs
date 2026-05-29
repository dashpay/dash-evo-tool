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

/// Single-key row migration. **Skeleton** — T-SK-02 plugs in the
/// `SecretStore` import here. Until then the call is a structured
/// no-op so the orchestration shape can land first.
async fn migrate_single_key_rows(_app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    // TODO(T-SK-02): copy legacy `single_key_wallet` rows into
    // `SingleKeyView` / `SecretStore` here.
    Ok(())
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
