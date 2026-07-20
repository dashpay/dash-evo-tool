//! Post-PR-#860 cold-start migration orchestrator.
//!
//! Drains legacy `data.db` rows that the unwire left behind into the
//! upstream `platform-wallet-storage` k/v store and `SecretStore`.
//! Idempotent: a per-network completion sentinel under
//! [`sentinel_key_for`] in `det-app.sqlite` short-circuits subsequent
//! launches **on the same network**.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::backend_task::dapi_discovery::persist_dapi_addresses;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::{DetScope, KvAdapterError, network_prefix};

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
        network_prefix(network)
    )
}

/// Per-network completion key for automatic DAPI refresh during an upgrade.
///
/// This pass cannot share the wallet-drain sentinel: a prior launch may finish
/// moving wallets while node discovery is temporarily unavailable. Keeping a
/// separate key lets discovery retry without rerunning or gating fund recovery.
pub fn dapi_refresh_sentinel_key_for(network: Network) -> String {
    format!("det:migration:dapi_refresh:{}:v1", network_prefix(network))
}

/// Tables sniffed during detection. Any non-empty row count flips the
/// migration into the `Running` state. Ordered so the cheapest check
/// (the single-row `wallet` table) runs first.
///
/// `scheduled_votes`, `top_up` and `identity` are in the list because an
/// install can hold them with no wallet rows at all — a masternode voter who
/// imported identity keys directly has identities and queued votes but no HD
/// wallet. Omitting them would leave the detection gate closed and drop that
/// user's keys.
const LEGACY_TABLES: &[&str] = &[
    "wallet",
    "single_key_wallet",
    "utxos",
    "scheduled_votes",
    "top_up",
    "identity",
];

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
/// Variants wrap upstream error types via `#[source]`; user-facing messages
/// live on the matching [`TaskError`] variants.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The legacy database predates the oldest layout this migration can read
    /// directly. The numeric fields are retained for diagnostics only.
    #[error(
        "legacy data version {found} is older than the minimum directly migratable version {minimum_supported}"
    )]
    LegacyDataTooOld { found: i64, minimum_supported: i64 },

    /// The legacy database comes from a newer, unknown layout.
    #[error(
        "legacy data version {found} is newer than the maximum directly migratable version {maximum_supported}"
    )]
    LegacyDataTooNew { found: i64, maximum_supported: i64 },

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

    /// The decoded scheduled votes could not be written into the k/v store.
    /// The app-data sentinel stays unwritten so the next launch retries the
    /// idempotent import rather than leaving the votes behind. It never blocks
    /// the wallet drain — [`run`] judges this only after the wallets are safe.
    #[error("could not save scheduled votes from the previous version")]
    ScheduledVotesWrite {
        #[source]
        source: Box<TaskError>,
    },

    /// The app-data pass failed with an error that did not already originate in
    /// the migration layer (a k/v read while checking which votes exist, say).
    /// Wrapped so the combined [`MigrationState::FailedWithUnreadableIdentities`]
    /// banner keeps a typed chain when both DET-owned passes break together.
    #[error("could not import the previous version's app data")]
    AppDataImport {
        #[source]
        source: Box<TaskError>,
    },

    /// A decoded legacy identity could not be written into the identity store.
    /// Hard failure: the identity sentinel stays unwritten so the next launch
    /// retries the idempotent import. Never blocks the wallet drain — [`run`]
    /// judges this only after the seeds are safe.
    #[error("could not save an identity from the previous version")]
    IdentityImportFailed {
        #[source]
        source: Box<TaskError>,
    },

    /// The legacy top-up history could not be carried across — the legacy table
    /// is unreadable, or the k/v store rejected the write. Audit trail, not
    /// funds, so it never blocks the wallet drain; but the app-data sentinel
    /// stays unwritten so the next launch retries the idempotent import rather
    /// than recording the loss as final.
    #[error("could not save the top-up history from the previous version")]
    TopUpHistoryWrite {
        #[source]
        source: Box<TaskError>,
    },

    /// Could not read, write or clear the durable unreadable-vote warning. That
    /// record is what re-raises the warning on every launch until the user
    /// acknowledges it, so a failure here is surfaced rather than dropped — a
    /// silently-lost warning is a silently-missed vote deadline.
    #[error("could not access the unreadable-vote warning")]
    VoteWarningRecord {
        #[source]
        source: KvAdapterError,
    },

    /// Could not read, write or clear the durable unreadable-top-up warning.
    #[error("could not access the unreadable top-up history warning")]
    TopUpWarningRecord {
        #[source]
        source: KvAdapterError,
    },

    /// Could not read, write or clear the durable unreadable-identity warning.
    /// That record is the only thing that survives the identity sentinel, so a
    /// failure here is surfaced rather than dropped — a silently-lost warning is
    /// a user who never learns their signing keys did not come across.
    #[error("could not access the unreadable-identity warning")]
    IdentityWarningRecord {
        #[source]
        source: KvAdapterError,
    },

    /// Could not persist the identity rows already processed by a partial pass.
    #[error("could not record identity migration progress")]
    IdentityProgressRecord {
        #[source]
        source: KvAdapterError,
    },

    /// The wallet backend was not yet wired when the migration ran.
    /// This is a hard configuration bug: the orchestrator runs after
    /// `ensure_wallet_backend`, so this should never fire in
    /// production. Kept as a typed variant so a future regression is
    /// caught immediately instead of silently no-oping.
    #[error("wallet backend not available during migration")]
    WalletBackendUnavailable,

    /// A password-protected wallet needs an egui frame loop to collect its
    /// password, but the caller is a standalone MCP/CLI process.
    #[error(
        "Open the Dash Evo Tool desktop app once to finish the storage update, then try again."
    )]
    InteractivePromptUnavailable,

    /// A pass reported an error that is not a [`MigrationError`]. Every pass is
    /// meant to report through one of the typed variants above; this catch-all
    /// exists so a stray error still reaches a terminal banner. Leaving one
    /// unpublished would strand the status on `Running`, which gates every
    /// wallet-touching task behind `WalletStorageNotReady` and offers no retry.
    #[error("could not finish updating the previous version's data")]
    Unexpected {
        #[source]
        source: Box<TaskError>,
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
    /// the "Retry now" banner) retries the idempotent registration. Migration
    /// collects every protected wallet password before reaching this check.
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

/// Open the pre-update SQLite file with write operations disabled by SQLite.
fn open_legacy_read_only(path: &std::path::Path) -> Result<Connection, MigrationError> {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
        |source| MigrationError::LegacyDbOpen {
            path: path.to_string_lossy().to_string(),
            source,
        },
    )
}

/// Coerce any migration failure into the `Arc<MigrationError>` chain the failure
/// banners render. Total by design: every error the orchestrator can produce must
/// end up publishable, or it leaves the status on `Running` — which gates every
/// wallet-touching task behind `WalletStorageNotReady`, with no retry to escape.
///
/// Task errors that already carry a [`MigrationError`] chain reuse it verbatim,
/// so the banner sees the same typed source. Anything else is wrapped in
/// [`MigrationError::Unexpected`] rather than dropped.
pub(crate) fn migration_error_chain(error: TaskError) -> Arc<MigrationError> {
    match error {
        TaskError::MigrationFailed { source }
        | TaskError::SavedDataTooOld { source }
        | TaskError::SavedDataTooNew { source }
        | TaskError::StorageUpdateNeedsDesktop { source } => source,
        other => Arc::new(MigrationError::Unexpected {
            source: Box::new(other),
        }),
    }
}

fn validate_legacy_database_version(version: i64) -> Result<(), MigrationError> {
    use crate::model::data_migration::{
        DirectMigrationVersion, MAX_DIRECT_MIGRATION_VERSION, MIN_DIRECT_MIGRATION_VERSION,
        classify_direct_migration_version,
    };

    match classify_direct_migration_version(version) {
        DirectMigrationVersion::TooOld => Err(MigrationError::LegacyDataTooOld {
            found: version,
            minimum_supported: MIN_DIRECT_MIGRATION_VERSION,
        }),
        DirectMigrationVersion::Supported => Ok(()),
        DirectMigrationVersion::TooNew => Err(MigrationError::LegacyDataTooNew {
            found: version,
            maximum_supported: MAX_DIRECT_MIGRATION_VERSION,
        }),
    }
}

fn validate_saved_data_for_migration(app_context: &AppContext) -> Result<(), MigrationError> {
    let version = app_context
        .db
        .stored_data_version()
        .map_err(|source| MigrationError::LegacyDbRead {
            table: "settings",
            source,
        })?
        .unwrap_or(0);
    validate_legacy_database_version(version)
}

#[cfg(not(test))]
async fn refresh_dapi_nodes_once(app_context: &Arc<AppContext>) {
    refresh_dapi_nodes_once_with(app_context, |network| async move {
        crate::backend_task::dapi_discovery::discover_and_format(network, None).await
    })
    .await;
}

#[cfg(test)]
async fn refresh_dapi_nodes_once(app_context: &Arc<AppContext>) {
    refresh_dapi_nodes_once_with(app_context, |_| async {
        Err(crate::backend_task::dapi_discovery::DapiDiscoveryError::Timeout)
    })
    .await;
}

/// Runs one best-effort refresh with an injected discovery operation.
///
/// Run-triggered passes still hold their per-context `migration_run`, while migration and manual refresh
/// share a process-wide guard across whole-file config persistence, including different network contexts.
async fn refresh_dapi_nodes_once_with<D, F>(app_context: &Arc<AppContext>, discover: D)
where
    D: FnOnce(Network) -> F,
    F: Future<
        Output = Result<(usize, String), crate::backend_task::dapi_discovery::DapiDiscoveryError>,
    >,
{
    refresh_dapi_nodes_once_with_legacy_check(app_context, discover, detect_legacy_rows).await;
}

async fn refresh_dapi_nodes_once_with_legacy_check<D, F, L>(
    app_context: &Arc<AppContext>,
    discover: D,
    detect_legacy: L,
) where
    D: FnOnce(Network) -> F,
    F: Future<
        Output = Result<(usize, String), crate::backend_task::dapi_discovery::DapiDiscoveryError>,
    >,
    L: FnOnce(&AppContext) -> Result<bool, MigrationError>,
{
    let network = app_context.network;
    let app_kv = app_context.app_kv();
    let sentinel_key = dapi_refresh_sentinel_key_for(network);

    match app_kv.get::<MigrationCompletion>(DetScope::Global, &sentinel_key) {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                target = "migration::finish_unwire",
                ?network,
                ?error,
                "Could not read the automatic DAPI refresh sentinel; node discovery will retry on the next launch",
            );
            return;
        }
    }

    if !matches!(network, Network::Mainnet | Network::Testnet) {
        write_dapi_refresh_completion(&app_kv, &sentinel_key, network, 0);
        return;
    }

    match detect_legacy(app_context) {
        Ok(true) => {}
        Ok(false) => {
            write_dapi_refresh_completion(&app_kv, &sentinel_key, network, 0);
            return;
        }
        Err(error) => {
            tracing::warn!(
                target = "migration::finish_unwire",
                ?network,
                ?error,
                "Could not inspect the previous version's data for automatic DAPI refresh; node discovery will retry on the next launch",
            );
            return;
        }
    }

    let (count, addresses_csv) = match discover(network).await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(
                target = "migration::finish_unwire",
                ?network,
                ?error,
                "Automatic DAPI node discovery failed during migration; it will retry on the next launch",
            );
            return;
        }
    };

    if let Err(error) = persist_dapi_addresses(app_context, addresses_csv) {
        tracing::warn!(
            target = "migration::finish_unwire",
            ?network,
            ?error,
            "Could not persist automatically discovered DAPI nodes; the refresh will retry on the next launch",
        );
        return;
    }

    if let Err(error) = Arc::clone(app_context).reinit_core_client_and_sdk() {
        tracing::warn!(
            target = "migration::finish_unwire",
            ?network,
            ?error,
            "Could not reinitialize network clients after automatic DAPI refresh; the saved addresses will be used on the next launch",
        );
        return;
    }

    tracing::info!(
        target = "migration::finish_unwire",
        ?network,
        count,
        "Automatically refreshed DAPI nodes during migration",
    );
    write_dapi_refresh_completion(&app_kv, &sentinel_key, network, 1);
}

fn write_dapi_refresh_completion(
    app_kv: &crate::wallet_backend::DetKv,
    sentinel_key: &str,
    network: Network,
    network_count: u32,
) {
    if let Err(error) = write_completion_sentinel(app_kv, sentinel_key, network_count) {
        tracing::warn!(
            target = "migration::finish_unwire",
            ?network,
            ?error,
            "Could not write the automatic DAPI refresh sentinel; the pass may retry on the next launch",
        );
    }
}

/// Run the FinishUnwire migration. Idempotent — completes a no-op when
/// the sentinels are already present.
///
/// Returns `true` when this launch actually moved legacy data (rows were
/// detected and drained), and `false` for the no-op paths: the sentinels
/// already existed, or no legacy rows were present. Callers use the flag to
/// decide whether to surface a "storage update complete" banner — a no-op
/// launch must not show one.
///
/// A best-effort DAPI refresh is queued before the migration passes. After the
/// run publishes terminal status and releases its guard, refresh acquires and
/// holds that guard until it finishes. Operations that claim the same guard,
/// including local identity deletion, remain unavailable during that interval.
///
/// Three independent recovery passes, each under its own sentinel, in this order:
///
/// 1. **App data** (scheduled votes, top-up history) — DET-owned rows the
///    wallet drain never touched.
/// 2. **Wallet drain** (single keys, HD seeds, wallet metadata, upstream
///    registration) — the pass that restores access to funds.
/// 3. **Identities** (identity rows and the keys they hold) — last, because it
///    needs the drain's output: a wired backend, a reachable vault, and a
///    hydrated `ctx.wallets` for wallet-derived identity keys to attach to.
///
/// **Neither DET-owned pass gates the other.** The wallet drain runs regardless
/// of the app-data outcome, and the identity import runs regardless of it too —
/// both results are held, never propagated on the spot, and judged together at
/// the end. A legacy vote row that cannot be imported must never stand between
/// the user and their seeds, nor between the user and their identity keys: an
/// app-data failure is deterministic, so letting it short-circuit the identity
/// import would strand those keys outside the vault on every launch, not just
/// this one.
///
/// The wallet drain is the one deliberate prerequisite: a drain failure returns
/// early, so the identity import does not run on that launch. It cannot — that
/// pass needs the drain's output (pass 3 above), and both retry on the next
/// launch, neither sentinel having been written.
///
/// Both DET-owned passes write their sentinel unconditionally, so their counters
/// exist for exactly one launch. What outlives them are the durable
/// [`UnreadableVotesWarning`], [`UnreadableTopUpsWarning`] and
/// [`UnreadableIdentitiesWarning`] records, and
/// those — never the counters — are what this function publishes: each is
/// re-raised on every launch, not only the one that discovered it, until
/// [`acknowledge_unreadable_app_data`] / [`acknowledge_unreadable_identities`]
/// retires it. When both are pending they ride one
/// [`MigrationState::SucceededWithUnreadableData`], so a lone
/// identity warning can never outrank the vote warning and silently cost the
/// user a live vote deadline. If reading the vote-warning record fails, the vote
/// half is withheld until a later launch reads it successfully — the identity
/// half still reaches the user, and neither is reported as a failed migration —
/// a *hard* app-data failure (not merely a failed later read of this record) is
/// the only thing that publishes [`MigrationState::FailedWithUnreadableIdentities`]
/// instead; see the `# Errors` section below.
///
/// # Errors
///
/// [`TaskError::SavedDataTooOld`] before any pass begins when the legacy database
/// predates v0.9.3, or [`TaskError::SavedDataTooNew`] when it comes from an
/// unknown newer layout. [`TaskError::MigrationFailed`] when the wallet drain
/// fails (the completion sentinel stays unwritten, so the next launch retries),
/// or when the wallet drain succeeded but the app-data or identity pass hit a
/// hard failure — an unreadable legacy file, a k/v write error. Undecodable
/// *rows* are not an error: they are counted and reported on
/// [`MigrationState::SucceededWithUnreadableData`], because failing here
/// would wedge the wallet drain behind a row the user cannot repair.
///
/// One case returns `Ok` while publishing a failure banner itself: a hard
/// app-data failure that coincides with unreadable identities on the same
/// launch. Both must reach the user, so the terminal state is
/// [`MigrationState::FailedWithUnreadableIdentities`] (published here, carrying
/// both signals) rather than an `Err` that this boundary would publish as a
/// plain `Failed`, dropping the identity count.
pub async fn run(app_context: &Arc<AppContext>) -> Result<bool, TaskError> {
    let _run_guard = match app_context.migration_run.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let guard = app_context.migration_run.lock().await;
            match app_context.migration_status().state().as_ref() {
                MigrationState::Failed { error } => {
                    let result = Err(super::migration_task_error(Arc::clone(error)));
                    drop(guard);
                    return result;
                }
                MigrationState::Idle => guard,
                _ => {
                    drop(guard);
                    return Ok(false);
                }
            }
        }
    };

    match run_under_guard(app_context).await {
        Ok(did_work) => Ok(did_work),
        Err(task_error) => {
            if !app_context.migration_status().state().is_in_progress() {
                return Err(task_error);
            }
            let source = migration_error_chain(task_error);
            app_context
                .migration_status()
                .set_state(MigrationState::Failed {
                    error: Arc::clone(&source),
                });
            Err(super::migration_task_error(source))
        }
    }
}

/// Waits for the launching pass, then holds `migration_run` through refresh.
/// Operations that claim this guard stay gated until the detached work completes.
fn spawn_dapi_refresh<F>(app_context: &Arc<AppContext>, refresh: F) -> tokio::task::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let ctx = Arc::clone(app_context);
    tokio::spawn(async move {
        let _refresh_guard = ctx.migration_run.lock().await;
        refresh.await;
    })
}

/// Queues refresh before migration so its waiter acquires the guard at completion.
async fn run_under_guard(app_context: &Arc<AppContext>) -> Result<bool, TaskError> {
    let ctx = Arc::clone(app_context);
    std::mem::drop(spawn_dapi_refresh(app_context, async move {
        refresh_dapi_nodes_once(&ctx).await;
    }));
    run_under_guard_with_dapi_refresh(app_context, std::future::ready(())).await
}

async fn run_under_guard_with_dapi_refresh<F>(
    app_context: &Arc<AppContext>,
    dapi_refresh: F,
) -> Result<bool, TaskError>
where
    F: Future<Output = ()>,
{
    validate_saved_data_for_migration(app_context)?;

    dapi_refresh.await;

    let status = app_context.migration_status();

    // Scheduled votes and top-up history carry their own sentinel and run
    // ahead of the wallet-drain gate below: an install that already completed
    // the wallet drain under an earlier build (which had no app-data import)
    // still has those rows in `data.db`, and the wallet sentinel would
    // otherwise short-circuit the launch and strand them.
    status.set_state(MigrationState::Running {
        step: MigrationStep::AppData,
    });
    let app_data = migrate_app_data(app_context);

    // The app-data result is deliberately held, not propagated: the wallet drain
    // is what restores access to funds, so nothing about DET's own rows may gate
    // it. Propagating here would let one bad vote row wedge the drain on every
    // launch, with no user-reachable way out.
    let wallet_moved = match drain_wallets(app_context).await {
        Ok(moved) => moved,
        Err(drain_error) => {
            if let Err(app_data_error) = &app_data {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?app_data_error,
                    "App-data import failed on the same launch as the wallet drain; both retry on the next launch",
                );
            }
            return Err(drain_error);
        }
    };

    // Funds are reachable from here on. The identity import runs next, and its
    // result — like the app-data one — is held rather than propagated, because
    // the two DET-owned passes must not gate each other.
    //
    // The identity pass needs the drain's output (backend wired, vault reachable,
    // `ctx.wallets` hydrated) so a wallet-derived key lands against a wallet that
    // exists. It must NOT wait on the app-data result: a hard app-data failure —
    // one malformed vote-index blob is enough — is deterministic, so unwrapping
    // it first would skip the identity import on this launch *and every retry*
    // (the app-data sentinel is never written, so the failure recurs forever).
    // That would strand a masternode owner's private keys outside the vault
    // permanently, over a corrupt vote queue.
    status.set_state(MigrationState::Running {
        step: MigrationStep::Identities,
    });
    let identities = migrate_identities(app_context);

    // Both DET-owned passes have now run. A hard failure in either still reaches
    // the user's "Retry now" banner, but only after neither could block the
    // other. The identity failure takes precedence when both fail: keys outrank
    // votes.
    let identities = match identities {
        Ok(outcome) => outcome,
        Err(identity_error) => {
            if let Err(app_data_error) = &app_data {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?app_data_error,
                    "App-data import failed on the same launch as the identity import; both retry on the next launch",
                );
            }
            return Err(identity_error);
        }
    };

    // The identity warning is read back from storage rather than taken from this
    // pass's counters: the identity sentinel is written unconditionally, so every
    // launch after the discovery run short-circuits the import and honestly
    // reports zero unreadable rows — while the rows it could not decode still
    // hold keys the user cannot sign with. Re-published until
    // [`acknowledge_unreadable_identities`] retires it.
    //
    // A read failure here is surfaced, never dropped: this record is the only
    // thing standing between the user and the news about their own keys.
    let identities_warning = match read_identities_warning(
        &app_context.app_kv(),
        app_context.network,
    ) {
        Ok(warning) => warning,
        Err(warning_error) => {
            if let Err(app_data_error) = &app_data {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    error = ?app_data_error,
                    "App-data import failed on the launch where the identity-warning record could not be read; the app-data import retries on the next launch",
                );
            }
            return Err(warning_error.into());
        }
    };

    // Unreadable identities outrank a *readable* app-data pass: an identity that
    // did not come across took its keys with it, so the user cannot sign — let
    // alone vote — until it is loaded again. But a *hard* app-data failure on the
    // same launch is not something the identity warning may swallow: it would
    // leave the user no retry for their scheduled votes, silently and every
    // launch (the app-data sentinel is never written, so it recurs). So when both
    // break together, a single combined banner names each — the app-data half
    // retryable — instead of one eating the other.
    //
    // This branch runs BEFORE `app_data` is unwrapped with `?`: unwrapping first
    // would return the deterministic app-data error and permanently mask the
    // identity outcome. `app_data` is moved here by value; the fall-through path
    // below (no pending identity warning) still owns it because this branch always
    // returns.
    if let Some(identities_warning) = identities_warning {
        let unreadable = identities_warning.count;
        match app_data {
            Ok(outcome) => {
                let moved_data = wallet_moved || outcome.moved_data() || identities.moved_data();

                // A pending vote warning must ride along, or the identity signal
                // buries it forever: the identity warning is re-published on every
                // launch until acknowledged, so this branch would otherwise return
                // ahead of the `read_vote_warning` re-publish below. Both counts
                // come from storage, not from this launch's counters: after the
                // discovery run both passes short-circuit on their sentinels and
                // honestly report zero — exactly on the launches where this branch
                // is the only one the user ever sees.
                //
                // A k/v read that itself fails costs only the vote half of the
                // banner, never the identity half: the record is durable and this
                // branch re-runs on every launch (the identity sentinel stays
                // unwritten), so the next successful read re-publishes it. Reporting
                // the read error as a *failure* state instead would tell the user the
                // app-data pass did not finish — it did, and wrote its sentinel — and
                // offer a retry that re-runs nothing.
                match read_unreadable_app_data(&app_context.app_kv(), app_context.network) {
                    Ok(warning) if !warning.is_empty() => {
                        tracing::warn!(
                            target = "migration::finish_unwire",
                            unreadable,
                            imported = identities.imported,
                            votes_unreadable = warning.votes,
                            top_ups_unreadable = warning.top_ups,
                            network = ?app_context.network,
                            "Some legacy identities and some legacy scheduled votes could not be decoded; they stay in the previous version's data.db and must be loaded / scheduled again",
                        );
                        status.set_state(MigrationState::SucceededWithUnreadableData {
                            identities: unreadable,
                            votes: warning.votes,
                            top_ups: warning.top_ups,
                        });
                    }
                    Ok(_) => {
                        tracing::warn!(
                            target = "migration::finish_unwire",
                            unreadable,
                            imported = identities.imported,
                            network = ?app_context.network,
                            "Some legacy identities could not be decoded; they stay in the previous version's data.db and must be loaded again",
                        );
                        status.set_state(MigrationState::SucceededWithUnreadableData {
                            identities: unreadable,
                            votes: 0,
                            top_ups: 0,
                        });
                    }
                    Err(warning_error) => {
                        tracing::warn!(
                            target = "migration::finish_unwire",
                            unreadable,
                            imported = identities.imported,
                            error = ?warning_error,
                            network = ?app_context.network,
                            "Some legacy identities could not be decoded, and the pending vote-warning record could not be read so any vote notice is withheld until the next launch; the identity rows stay in the previous version's data.db and must be loaded again",
                        );
                        status.set_state(MigrationState::SucceededWithUnreadableData {
                            identities: unreadable,
                            votes: 0,
                            top_ups: 0,
                        });
                    }
                }
                return Ok(moved_data);
            }
            Err(app_data_error) => {
                let moved_data = wallet_moved || identities.moved_data();
                tracing::warn!(
                    target = "migration::finish_unwire",
                    unreadable,
                    imported = identities.imported,
                    error = ?app_data_error,
                    network = ?app_context.network,
                    "Both DET-owned passes failed on the same launch: some legacy identities could not be decoded and the app-data import hit a hard error; both retry on the next launch",
                );
                status.set_state(MigrationState::FailedWithUnreadableIdentities {
                    count: unreadable,
                    error: migration_error_chain(app_data_error),
                });
                return Ok(moved_data);
            }
        }
    }

    // No identity warning is pending, so app-data no longer masks anything
    // critical: a hard failure here is the user's "Retry now" banner.
    let app_data = app_data?;
    let moved_data = wallet_moved || app_data.moved_data() || identities.moved_data();

    // The warnings are read back from storage rather than taken from this pass's
    // counters: on every launch after the discovery run the import short-circuits
    // on its sentinel and reports zero, yet unreadable votes still need
    // re-scheduling and unreadable top-up history still needs review. Re-published
    // until the user acknowledges it.
    let warning = read_unreadable_app_data(&app_context.app_kv(), app_context.network)?;
    if !warning.is_empty() {
        tracing::warn!(
            target = "migration::finish_unwire",
            votes_unreadable = warning.votes,
            top_ups_unreadable = warning.top_ups,
            network = ?app_context.network,
            "Some legacy scheduled votes or top-up rows could not be decoded; they stay in the previous version's data.db and require user review",
        );
        status.set_state(MigrationState::SucceededWithUnreadableData {
            identities: 0,
            votes: warning.votes,
            top_ups: warning.top_ups,
        });
        return Ok(moved_data);
    }

    status.set_state(terminal_state(moved_data));
    Ok(moved_data)
}

/// Drain the legacy wallet family — single-key rows, HD wallet seeds, wallet
/// metadata — into the upstream store, register the migrated wallets, then
/// record the per-network completion sentinel.
///
/// Returns `true` when this launch drained wallet rows, `false` for the two
/// no-op paths (sentinel already present, or no legacy rows at all). This is
/// the funds path: [`run`] keeps it free of every DET-owned concern so nothing
/// but a genuine wallet-migration failure can withhold access to a seed.
async fn drain_wallets(app_context: &Arc<AppContext>) -> Result<bool, TaskError> {
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
        "FinishUnwire wallet drain complete",
    );
    Ok(true)
}

/// Terminal state for a launch that reached the end without failing.
/// `Success` raises the completion banner, so it is reserved for launches
/// that actually moved data — a no-op launch becomes `Ready` without raising a
/// completion banner.
fn terminal_state(moved_data: bool) -> MigrationState {
    if moved_data {
        MigrationState::Success
    } else {
        MigrationState::Ready
    }
}

/// Releases, on every exit path, the seed leases this run's password prompts
/// took. Each seed is forgotten as soon as the unlock's own reconciliation
/// subtask is also done with it, so an unlock granted for the storage update
/// never silently outlives the update.
struct RunSeedLeases<'a>(&'a Arc<AppContext>);

impl Drop for RunSeedLeases<'_> {
    fn drop(&mut self) {
        self.0.migration_status().release_seed_leases();
    }
}

/// Re-hydrates just-migrated wallets into `ctx.wallets`, registers open wallets,
/// and waits for the UI to unlock or explicitly skip each protected wallet.
/// [`run`] calls this immediately before [`write_sentinel`], so every open wallet
/// is registered before completion; a skipped wallet remains closed in its
/// legacy protected envelope until a later ordinary unlock reconciles it.
/// Idempotent.
///
/// Each wallet unlocked for this run is bootstrapped twice — once by the unlock
/// gesture's own subtask, once by the `bootstrap_loaded_wallets` pass below —
/// and neither ordering is guaranteed. The run therefore holds its own lease on
/// every seed it prompted for (`WalletUnlockRetention::UntilStorageUpdateComplete`)
/// rather than depending on the unlock subtask still being alive.
async fn register_migrated_wallets(app_context: &Arc<AppContext>) -> Result<(), MigrationError> {
    let _seed_leases = RunSeedLeases(app_context);

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
    // without a restart.
    app_context.bootstrap_loaded_wallets().await;

    app_context
        .migration_status()
        .begin_wallet_password_collection();
    loop {
        let wallets = app_context
            .migration_status()
            .pending_wallet_passwords(app_context.locked_wallet_hashes());
        if wallets.is_empty() {
            break;
        }
        if !app_context.has_interactive_secret_prompt() {
            return Err(MigrationError::InteractivePromptUnavailable);
        }
        app_context
            .migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords { wallets });
        app_context
            .migration_status()
            .wait_for_wallet_password()
            .await;
        app_context.bootstrap_loaded_wallets().await;
    }

    let unregistered = app_context.unregistered_open_wallet_count();
    if unregistered > 0 {
        return Err(MigrationError::RegistrationIncomplete { unregistered });
    }
    Ok(())
}

/// Per-network sentinel for the DET app-data import (scheduled votes and
/// top-up history). Separate from the wallet-drain sentinel on purpose: an
/// install that already completed the wallet drain under an earlier build
/// still has its votes sitting in `data.db`, and a shared sentinel would
/// declare that install "done" and drop them.
pub fn app_data_sentinel_key_for(network: Network) -> String {
    format!("det:migration:app_data:{}:v1", network_prefix(network))
}

/// Per-network key of the un-acknowledged unreadable-vote warning. Distinct
/// from the app-data sentinel: the sentinel records that the import *ran*, this
/// record that the user has not yet been *told* what it could not carry across.
fn vote_warning_key_for(network: Network) -> String {
    format!(
        "det:migration:unreadable_votes:{}:v1",
        network_prefix(network)
    )
}

/// Durable "some scheduled votes could not be read" warning.
///
/// The import runs once (the app-data sentinel short-circuits every later
/// launch), so the pass counters exist for exactly one launch. A user who was
/// away, or who dismissed the banner without reading it, would never hear about
/// it again — while the vote it names may still have a live deadline. This
/// record outlives the pass: [`run`] re-publishes it on every launch until
/// [`acknowledge_unreadable_app_data`] clears it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnreadableVotesWarning {
    /// Legacy vote rows the import could not decode. Never `0` — a zero-count
    /// warning is not written at all.
    pub count: u32,
}

/// The pending unreadable-vote warning for `network`, if the user has not
/// acknowledged it yet.
fn read_vote_warning(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<Option<UnreadableVotesWarning>, MigrationError> {
    app_kv
        .get::<UnreadableVotesWarning>(DetScope::Global, &vote_warning_key_for(network))
        .map_err(|source| MigrationError::VoteWarningRecord { source })
}

/// Record `count` unreadable vote rows as a pending warning. A zero count
/// writes nothing — there is nothing to tell the user.
///
/// Written BEFORE the app-data sentinel: a crash between the two re-runs the
/// idempotent import, whereas the reverse order would lose the warning for good.
fn write_vote_warning(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
    count: u32,
) -> Result<(), MigrationError> {
    if count == 0 {
        return Ok(());
    }
    app_kv
        .put(
            DetScope::Global,
            &vote_warning_key_for(network),
            &UnreadableVotesWarning { count },
        )
        .map_err(|source| MigrationError::VoteWarningRecord { source })
}

/// Retire the unreadable-vote warning for the active network: the user has read
/// it. Clears the durable record so later launches stay quiet, and drops the
/// banner. The legacy rows in `data.db` are untouched — only the notice is
/// retired, so a build with a better decoder can still recover the votes.
pub fn acknowledge_unreadable_app_data(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let network = app_context.network;
    clear_unreadable_app_data(&app_context.app_kv(), network)?;
    tracing::info!(
        target = "migration::finish_unwire",
        network = ?network,
        "User acknowledged the unreadable app-data warning",
    );
    app_context
        .migration_status()
        .set_state(MigrationState::Ready);
    Ok(())
}

fn clear_unreadable_app_data(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<(), MigrationError> {
    app_kv
        .delete(DetScope::Global, &vote_warning_key_for(network))
        .map_err(|source| MigrationError::VoteWarningRecord { source })?;
    app_kv
        .delete(DetScope::Global, &top_up_warning_key_for(network))
        .map_err(|source| MigrationError::TopUpWarningRecord { source })?;
    Ok(())
}

fn top_up_warning_key_for(network: Network) -> String {
    format!(
        "det:migration:unreadable_top_ups:{}:v1",
        network_prefix(network)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnreadableTopUpsWarning {
    /// Legacy top-up rows the import could not decode.
    pub count: u32,
}

fn read_top_up_warning(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<Option<UnreadableTopUpsWarning>, MigrationError> {
    app_kv
        .get::<UnreadableTopUpsWarning>(DetScope::Global, &top_up_warning_key_for(network))
        .map_err(|source| MigrationError::TopUpWarningRecord { source })
}

fn write_top_up_warning(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
    count: u32,
) -> Result<(), MigrationError> {
    if count == 0 {
        return Ok(());
    }
    app_kv
        .put(
            DetScope::Global,
            &top_up_warning_key_for(network),
            &UnreadableTopUpsWarning { count },
        )
        .map_err(|source| MigrationError::TopUpWarningRecord { source })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct UnreadableAppData {
    votes: u32,
    top_ups: u32,
}

impl UnreadableAppData {
    fn is_empty(self) -> bool {
        self.votes == 0 && self.top_ups == 0
    }
}

fn read_unreadable_app_data(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<UnreadableAppData, MigrationError> {
    Ok(UnreadableAppData {
        votes: read_vote_warning(app_kv, network)?.map_or(0, |warning| warning.count),
        top_ups: read_top_up_warning(app_kv, network)?.map_or(0, |warning| warning.count),
    })
}

/// Per-network key of the un-acknowledged unreadable-identity warning. Distinct
/// from the identity sentinel: the sentinel records that the import *ran*, this
/// record that the user has not yet been *told* what it could not carry across.
fn identities_warning_key_for(network: Network) -> String {
    format!(
        "det:migration:unreadable_identities:{}:v1",
        network_prefix(network)
    )
}

/// Durable "some identities could not be read" warning.
///
/// The import runs once (the identity sentinel short-circuits every later
/// launch), so the pass counters exist for exactly one launch. A user who was
/// away, or who dismissed the banner without reading it, would otherwise never
/// hear that the keys those identities held — a masternode's owner / voting key,
/// say — are not loaded. This record outlives the pass: [`run`] re-publishes it
/// on every launch until [`acknowledge_unreadable_identities`] clears it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnreadableIdentitiesWarning {
    /// Legacy identity rows the import could not decode. Never `0` — a
    /// zero-count warning is not written at all.
    pub count: u32,
}

/// The pending unreadable-identity warning for `network`, if the user has not
/// acknowledged it yet.
fn read_identities_warning(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<Option<UnreadableIdentitiesWarning>, MigrationError> {
    app_kv
        .get::<UnreadableIdentitiesWarning>(DetScope::Global, &identities_warning_key_for(network))
        .map_err(|source| MigrationError::IdentityWarningRecord { source })
}

/// Record `count` unreadable identity rows as a pending warning. A zero count
/// writes nothing — there is nothing to tell the user.
///
/// Written BEFORE the identity sentinel: a crash between the two re-runs the
/// idempotent import, whereas the reverse order would lose the warning for good.
fn write_identities_warning(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
    count: u32,
) -> Result<(), MigrationError> {
    if count == 0 {
        return Ok(());
    }
    app_kv
        .put(
            DetScope::Global,
            &identities_warning_key_for(network),
            &UnreadableIdentitiesWarning { count },
        )
        .map_err(|source| MigrationError::IdentityWarningRecord { source })
}

/// Retire the unreadable-identity warning for the active network: the user has
/// read it. Clears the durable record so later launches stay quiet, and drops
/// the banner. The legacy rows in `data.db` are untouched — only the notice is
/// retired, so a build with a better decoder can still recover the identities.
///
/// This gesture does not gate the import loop: [`migrate_identities`] writes its
/// sentinel unconditionally, so the import is already a once-only event whether
/// or not the user ever clicks. Acknowledgement retires the notice, nothing more.
pub fn acknowledge_unreadable_identities(app_context: &Arc<AppContext>) -> Result<(), TaskError> {
    let network = app_context.network;
    app_context
        .app_kv()
        .delete(DetScope::Global, &identities_warning_key_for(network))
        .map_err(|source| MigrationError::IdentityWarningRecord { source })?;
    tracing::info!(
        target = "migration::finish_unwire",
        network = ?network,
        "User acknowledged the unreadable-identity warning",
    );
    app_context
        .migration_status()
        .set_state(MigrationState::Ready);
    Ok(())
}

/// Import the DET-owned rows the wallet drain never touched: scheduled DPNS
/// votes (deadline-critical) and top-up history (audit trail).
///
/// Returns the pass counters, including `votes_unreadable` — rows that could
/// not be decoded. Those are *not* an error: a corrupt row decodes no better on
/// a retry, so failing here would only wedge the launch. [`run`] reports the
/// count to the user instead, and the legacy rows are never deleted.
///
/// Idempotent — votes already in the k/v store are left alone, so a retry can
/// never overwrite a vote the user has since cast with its stale legacy
/// `executed` flag.
///
/// # Errors
///
/// [`TaskError::MigrationFailed`] when the legacy file cannot be opened or read,
/// or the decoded votes cannot be written to the k/v store. The app-data
/// sentinel stays unwritten in those cases, so the next launch retries.
fn migrate_app_data(app_context: &Arc<AppContext>) -> Result<AppDataMigrationOutcome, TaskError> {
    let app_kv = app_context.app_kv();
    let network = app_context.network;
    let sentinel_key = app_data_sentinel_key_for(network);

    let done = app_kv
        .get::<MigrationCompletion>(DetScope::Global, &sentinel_key)
        .map_err(|source| MigrationError::Sentinel { source })?
        .is_some();
    if done {
        return Ok(AppDataMigrationOutcome::default());
    }

    let Some(path) = app_context.db.db_file_path() else {
        // In-memory / headless: no legacy file to import from.
        return Ok(AppDataMigrationOutcome::default());
    };
    if !path.exists() {
        return Ok(AppDataMigrationOutcome::default());
    }
    let conn = open_legacy_read_only(&path)?;

    // Probe before reaching for the wallet backend: an install with nothing to
    // import (a fresh one, or any build after C5 stopped creating these tables)
    // must complete without the backend being wired, or a cold start with no
    // legacy data would fail on a dependency it never actually needs.
    if !table_has_rows(&conn, "scheduled_votes")? && !table_has_rows(&conn, "top_up")? {
        write_completion_sentinel(&app_kv, &sentinel_key, 1)?;
        return Ok(AppDataMigrationOutcome::default());
    }

    // There is data to move, so the k/v store — and therefore the backend —
    // is genuinely required now. An unwired backend is transient: the
    // cold-start dispatcher retries this variant on a later frame.
    app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;

    // Votes already in the k/v store win over their legacy row: a retry must
    // not push a stale `executed = 0` over a vote the user has since cast.
    //
    // The read is typed into `AppDataImport` rather than propagated raw: every
    // error leaving this pass must be a `MigrationError`, or the terminal banner
    // has no typed chain to render.
    let existing_votes: std::collections::BTreeSet<([u8; 32], String)> = app_context
        .get_scheduled_votes()
        .map_err(|source| MigrationError::AppDataImport {
            source: Box::new(source),
        })?
        .into_iter()
        .map(|v| (v.voter_id.to_buffer(), v.contested_name))
        .collect();

    let outcome = migrate_app_data_from_conn(
        &conn,
        network,
        &existing_votes,
        |votes| app_context.insert_scheduled_votes(votes),
        |id, top_ups| app_context.save_top_ups(id, top_ups),
    )?;

    tracing::info!(
        target = "migration::finish_unwire",
        votes_imported = outcome.votes_imported,
        votes_skipped_existing = outcome.votes_skipped_existing,
        votes_unreadable = outcome.votes_unreadable,
        top_up_identities_imported = outcome.top_up_identities_imported,
        top_ups_unreadable = outcome.top_ups_unreadable,
        network = ?network,
        "App-data migration pass complete",
    );

    // Persist the warning before the sentinel: this is the only pass that ever
    // counts the undecodable rows (the sentinel short-circuits every later
    // launch), so the count has to outlive it or the user gets one banner and no
    // second chance. Ordered before the sentinel so a crash in between re-runs the
    // idempotent import rather than losing the warning.
    write_vote_warning(&app_kv, network, outcome.votes_unreadable)?;
    write_top_up_warning(&app_kv, network, outcome.top_ups_unreadable)?;

    // The sentinel is written even when rows were unreadable: every *importable*
    // row is now in the k/v store, and the undecodable ones will never decode. A
    // withheld sentinel would re-run this import on every launch, which would
    // resurrect votes the user has since cast and cleared from the queue.
    write_completion_sentinel(&app_kv, &sentinel_key, 1)?;

    Ok(outcome)
}

/// Record one import pass as complete under `sentinel_key`. The sole writer of
/// [`MigrationCompletion`]: every pass — DAPI refresh, wallet drain, app data,
/// identities — records completion through here, so all sentinels share one
/// codec.
///
/// `network_count` is diagnostic only — logged when the sentinel is read back,
/// never branched on. The wallet drain uses it to tell a no-op launch (`0`) from
/// a real drain (`1`); the other passes always record `1`.
fn write_completion_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
    sentinel_key: &str,
    network_count: u32,
) -> Result<(), MigrationError> {
    let completion = MigrationCompletion {
        completed_at: now_epoch_seconds(),
        sha: env!("CARGO_PKG_VERSION").to_string(),
        network_count,
    };
    app_kv
        .put(DetScope::Global, sentinel_key, &completion)
        .map_err(|source| MigrationError::Sentinel { source })
}

/// Pure app-data migration body (testable without an `AppContext`).
///
/// `existing_votes` holds the `(voter, contested_name)` pairs already in the
/// k/v store; those rows are skipped so a retry cannot overwrite a vote the
/// user has since cast with the stale legacy `executed` flag.
fn migrate_app_data_from_conn<I, T>(
    conn: &Connection,
    network: dash_sdk::dpp::dashcore::Network,
    existing_votes: &std::collections::BTreeSet<([u8; 32], String)>,
    insert_votes: I,
    mut save_top_ups: T,
) -> Result<AppDataMigrationOutcome, MigrationError>
where
    I: FnOnce(&[crate::backend_task::contested_names::ScheduledDPNSVote]) -> Result<(), TaskError>,
    T: FnMut(
        &dash_sdk::platform::Identifier,
        &std::collections::BTreeMap<u32, u64>,
    ) -> Result<(), TaskError>,
{
    let mut outcome = AppDataMigrationOutcome::default();

    let legacy_votes = crate::database::legacy_import::read_scheduled_votes(conn, network)
        .map_err(|source| MigrationError::LegacyDbRead {
            table: "scheduled_votes",
            source,
        })?;
    outcome.votes_unreadable = legacy_votes.unreadable;

    let to_import: Vec<_> = legacy_votes
        .votes
        .into_iter()
        .filter(|v| {
            let known =
                existing_votes.contains(&(v.voter_id.to_buffer(), v.contested_name.clone()));
            if known {
                outcome.votes_skipped_existing = outcome.votes_skipped_existing.saturating_add(1);
            }
            !known
        })
        .collect();

    if !to_import.is_empty() {
        insert_votes(&to_import).map_err(|source| MigrationError::ScheduledVotesWrite {
            source: Box::new(source),
        })?;
        outcome.votes_imported = u32::try_from(to_import.len()).unwrap_or(u32::MAX);
    }

    // Top-ups are audit trail, not funds, so a failure never blocks the votes that
    // already landed — every identity is attempted before the pass gives up. But it
    // is not swallowed either: the caller withholds the app-data sentinel on `Err`,
    // so the next launch retries the idempotent import. Swallowing would freeze a
    // one-off k/v error into permanent loss, because the sentinel short-circuits
    // every later launch. Undecodable *rows* never reach here — the reader skips
    // and logs them — so an error here is structural and a retry is worth taking.
    let failure = match crate::database::legacy_import::read_top_ups(conn, network) {
        Ok(top_ups) => {
            outcome.top_ups_unreadable = top_ups.unreadable;
            let mut first_error = None;
            for (identity_id, history) in top_ups.top_ups {
                let id = dash_sdk::platform::Identifier::from(identity_id);
                match save_top_ups(&id, &history) {
                    Ok(()) => {
                        outcome.top_up_identities_imported =
                            outcome.top_up_identities_imported.saturating_add(1)
                    }
                    Err(e) => {
                        tracing::warn!(
                            target = "migration::finish_unwire",
                            identity = %hex::encode(identity_id),
                            error = ?e,
                            "Could not write top-up history; the import will be retried on the next launch",
                        );
                        first_error.get_or_insert(e);
                    }
                }
            }
            first_error.map(|source| MigrationError::TopUpHistoryWrite {
                source: Box::new(source),
            })
        }
        Err(source) => Some(MigrationError::LegacyDbRead {
            table: "top_up",
            source,
        }),
    };
    if let Some(failure) = failure {
        return Err(failure);
    }

    Ok(outcome)
}

/// Per-network sentinel for the legacy identity import. Distinct from the
/// wallet-drain sentinel on purpose: every install that already drained its
/// wallets under an earlier build (which had no identity import) still has its
/// identities — and their owner / voting keys — sitting in `data.db`. Sharing
/// the drain's sentinel would declare exactly those installs "done" and drop
/// them.
pub fn identities_sentinel_key_for(network: Network) -> String {
    format!("det:migration:identities:{}:v1", network_prefix(network))
}

fn identities_progress_key_for(network: Network) -> String {
    format!(
        "det:migration:identity_progress:{}:v1",
        network_prefix(network)
    )
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct IdentityMigrationProgress {
    processed: BTreeSet<[u8; 32]>,
}

fn read_identity_progress(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<IdentityMigrationProgress, MigrationError> {
    app_kv
        .get(DetScope::Global, &identities_progress_key_for(network))
        .map(|progress| progress.unwrap_or_default())
        .map_err(|source| MigrationError::IdentityProgressRecord { source })
}

fn write_identity_progress(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
    processed: &BTreeSet<[u8; 32]>,
) -> Result<(), MigrationError> {
    app_kv
        .put(
            DetScope::Global,
            &identities_progress_key_for(network),
            &IdentityMigrationProgress {
                processed: processed.clone(),
            },
        )
        .map_err(|source| MigrationError::IdentityProgressRecord { source })
}

/// Record an explicit identity deletion before removing its modern record.
/// A later partial-pass retry then skips the stale legacy row. The caller holds
/// `migration_run` across this marker and the complete modern-store deletion.
pub(crate) fn record_identity_deletion(
    app_context: &AppContext,
    id: [u8; 32],
) -> Result<(), MigrationError> {
    if matches!(
        app_context.migration_status().state().as_ref(),
        MigrationState::Ready
            | MigrationState::Success
            | MigrationState::SucceededWithUnreadableData { .. }
    ) {
        return Ok(());
    }
    let app_kv = app_context.app_kv();
    let mut progress = read_identity_progress(&app_kv, app_context.network)?;
    if progress.processed.insert(id) {
        write_identity_progress(&app_kv, app_context.network, &progress.processed)?;
    }
    Ok(())
}

/// Import the legacy `identity` rows — and the private keys they carry — into
/// the modern identity store.
///
/// Runs after the wallet drain: the k/v store, the secret vault and a hydrated
/// `ctx.wallets` all have to exist first. Idempotent — an identity already in
/// the store is left alone, so a retry can never overwrite an alias the user has
/// since edited with the stale legacy copy.
///
/// Key material is never handled here. Each decoded identity goes straight to
/// [`AppContext::insert_local_qualified_identity`], which routes the keys
/// through the vault seam and writes only `InVault` placeholders to disk.
///
/// Runs exactly once: the sentinel is written even when rows were unreadable, so
/// a deletion the user makes afterwards is durable. Undecodable rows are counted
/// into a durable [`UnreadableIdentitiesWarning`] and left in `data.db`, never
/// deleted — recovering them after a decoder fix is an explicit user gesture
/// (dashpay/dash-evo-tool#889), not an automatic retry.
///
/// # Errors
///
/// [`TaskError::MigrationFailed`] when the legacy file cannot be opened or read,
/// or a decoded identity cannot be written to the store. Undecodable rows are
/// not an error — they are counted and reported to the user by [`run`].
fn migrate_identities(
    app_context: &Arc<AppContext>,
) -> Result<IdentityMigrationOutcome, TaskError> {
    let app_kv = app_context.app_kv();
    let network = app_context.network;
    let sentinel_key = identities_sentinel_key_for(network);

    let done = app_kv
        .get::<MigrationCompletion>(DetScope::Global, &sentinel_key)
        .map_err(|source| MigrationError::Sentinel { source })?
        .is_some();
    if done {
        return Ok(IdentityMigrationOutcome::default());
    }

    let Some(path) = app_context.db.db_file_path() else {
        // In-memory / headless: no legacy file to import from.
        return Ok(IdentityMigrationOutcome::default());
    };
    if !path.exists() {
        return Ok(IdentityMigrationOutcome::default());
    }
    let conn = open_legacy_read_only(&path)?;

    // Own probe rather than the drain's [`detect_legacy_rows`] gate: an
    // identity-only install (a masternode voter with no HD wallet) has to import
    // its identities even though the wallet family is empty. Probing first also
    // keeps a fresh install from needing the backend it has no data for.
    if !table_has_rows(&conn, "identity")? {
        write_completion_sentinel(&app_kv, &sentinel_key, 1)?;
        return Ok(IdentityMigrationOutcome::default());
    }

    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;

    let mut progress = read_identity_progress(&app_kv, network)?;
    let existing_ids = app_context.local_identity_ids().map_err(|source| {
        MigrationError::IdentityImportFailed {
            source: Box::new(source),
        }
    })?;
    for id in existing_ids {
        if app_context
            .has_local_qualified_identity(&id)
            .map_err(|source| MigrationError::IdentityImportFailed {
                source: Box::new(source),
            })?
        {
            progress.processed.insert(id.to_buffer());
        }
    }
    write_identity_progress(&app_kv, network, &progress.processed)?;

    let outcome = migrate_identities_from_conn(
        &conn,
        network,
        &mut progress.processed,
        |processed| write_identity_progress(&app_kv, network, processed),
        |id| app_context.has_local_qualified_identity(&Identifier::from(id)),
        |qi, wallet| {
            // Diagnostic only — the link is imported either way (see the pure
            // body). An absent wallet means it failed to migrate or is locked.
            if let Some((seed_hash, _)) = wallet
                && backend.wallet_meta().get(network, seed_hash).is_none()
            {
                tracing::warn!(
                    target = "migration::finish_unwire",
                    identity = %hex::encode(qi.identity.id().to_buffer()),
                    "Importing an identity whose wallet is not present; the link is kept so it re-attaches when that wallet is restored",
                );
            }
            app_context.insert_local_qualified_identity(qi, wallet)
        },
    )?;

    tracing::info!(
        target = "migration::finish_unwire",
        imported = outcome.imported,
        skipped_existing = outcome.skipped_existing,
        unreadable = outcome.unreadable,
        network = ?network,
        "Identity migration pass complete",
    );

    // Persist the warning before the sentinel: this pass counts the undecodable
    // rows exactly once (the sentinel short-circuits every later launch), so the
    // count has to outlive it or the user gets one banner and no second chance.
    // Ordered before the sentinel so a crash in between re-runs the idempotent
    // import rather than losing the warning.
    write_identities_warning(&app_kv, network, outcome.unreadable)?;

    // The sentinel is written even when rows were unreadable — the same rule the
    // app-data pass follows, for the same reason. Every *importable* row is now
    // in the store, and a withheld sentinel would re-run this import on every
    // launch: harmless for an identity the user has edited (skip-if-present), but
    // it would resurrect one the user has DELETED, restoring its alias and
    // re-writing its legacy plaintext keys into the vault, forever. The
    // undecodable rows stay in `data.db` and are reported by the durable warning
    // above; re-importing them after a decoder fix is an explicit user gesture
    // (dashpay/dash-evo-tool#889), not an automatic retry that costs a deletion.
    write_completion_sentinel(&app_kv, &sentinel_key, 1)?;

    Ok(outcome)
}

/// Pure identity-import body (testable without an `AppContext`).
///
/// `processed` contains identities already imported, already present in modern
/// storage, or explicitly deleted. `has_current_blob` checks storage directly
/// before each pending insert. `record_processed` durably advances the set after
/// an insert or direct-presence discovery, and `insert` is the vault-routing writer.
fn migrate_identities_from_conn<R, H, I>(
    conn: &Connection,
    network: Network,
    processed: &mut BTreeSet<[u8; 32]>,
    mut record_processed: R,
    mut has_current_blob: H,
    mut insert: I,
) -> Result<IdentityMigrationOutcome, MigrationError>
where
    R: FnMut(&BTreeSet<[u8; 32]>) -> Result<(), MigrationError>,
    H: FnMut([u8; 32]) -> Result<bool, TaskError>,
    I: FnMut(&QualifiedIdentity, &Option<(WalletSeedHash, u32)>) -> Result<(), TaskError>,
{
    let import_failed = |source: TaskError| MigrationError::IdentityImportFailed {
        source: Box::new(source),
    };

    let legacy =
        crate::database::legacy_import::read_identities(conn, network).map_err(|source| {
            MigrationError::LegacyDbRead {
                table: "identity",
                source,
            }
        })?;

    let mut outcome = IdentityMigrationOutcome {
        unreadable: legacy.unreadable,
        ..Default::default()
    };

    for row in legacy.identities {
        // Skip an identity already in the store, wholesale. Reconciling
        // legacy-only keys into a present record is deliberately NOT attempted:
        // field absence cannot be told apart from a deliberate removal (a
        // cleared alias, "Remove private key from DET") without provenance the
        // model does not carry, and a plaintext key merged into a protected
        // identity trips the vault-first downgrade guard. The legacy `data.db`
        // is preserved, so those keys are recoverable by a later build. See the
        // known limitation in the design doc (§7) and the tracked follow-up.
        if processed.contains(&row.id) {
            outcome.skipped_existing = outcome.skipped_existing.saturating_add(1);
            continue;
        }

        // The index-derived startup snapshot is only a fast path. Check the
        // blob itself before every pending insert so a stale or incomplete
        // index cannot overwrite a modern record with its legacy copy.
        if has_current_blob(row.id).map_err(import_failed)? {
            processed.insert(row.id);
            record_processed(processed)?;
            outcome.skipped_existing = outcome.skipped_existing.saturating_add(1);
            continue;
        }

        // The wallet link travels verbatim, never nulled — even when that wallet
        // did not come across: it is what re-attaches the identity when the
        // wallet is restored or unlocked later.
        insert(&row.qi, &row.wallet).map_err(import_failed)?;
        processed.insert(row.id);
        record_processed(processed)?;
        outcome.imported = outcome.imported.saturating_add(1);
    }

    Ok(outcome)
}

/// Outcome counters from one [`migrate_identities`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct IdentityMigrationOutcome {
    /// Identities written into the per-network identity store, keys vaulted.
    imported: u32,
    /// Identities already in the store and therefore left untouched.
    skipped_existing: u32,
    /// Legacy rows that could not be decoded. Recorded as a durable
    /// [`UnreadableIdentitiesWarning`] so the user still hears about them, but
    /// never fails the pass — the identities that *did* decode must not be held
    /// hostage by one that did not.
    unreadable: u32,
}

impl IdentityMigrationOutcome {
    /// `true` when this pass actually moved an identity across.
    fn moved_data(&self) -> bool {
        self.imported > 0
    }
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
    let conn = open_legacy_read_only(&path)?;
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

/// Outcome counters from one [`migrate_app_data`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct AppDataMigrationOutcome {
    /// Scheduled votes written into the per-network k/v store.
    votes_imported: u32,
    /// Scheduled votes already present in the k/v store and therefore left
    /// alone. A retry must not resurrect a stale `executed` flag over a vote
    /// the user has since cast.
    votes_skipped_existing: u32,
    /// Legacy vote rows that could not be decoded (corrupt voter id or vote
    /// choice). Non-fatal — a corrupt row decodes no better on a retry, and
    /// failing the pass would wedge the wallet drain behind it. Surfaced to the
    /// user by [`run`] instead, so a vote is never lost in silence.
    votes_unreadable: u32,
    /// Identities whose top-up history was written into the k/v store.
    top_up_identities_imported: u32,
    /// Legacy top-up rows that could not be decoded. Non-fatal and surfaced
    /// through a durable warning.
    top_ups_unreadable: u32,
}

impl AppDataMigrationOutcome {
    /// Whether this pass actually carried data across — the signal [`run`] uses
    /// to decide whether the launch earns a completion banner.
    fn moved_data(&self) -> bool {
        self.votes_imported > 0 || self.top_up_identities_imported > 0
    }
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
    let conn = open_legacy_read_only(&path)?;

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
    crate::database::table_exists(conn, table)
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
    let conn = open_legacy_read_only(&path)?;

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
    let conn = open_legacy_read_only(&path)?;

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
            encrypted_seed: zeroize::Zeroizing::new(encrypted_seed),
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

/// Read the completion sentinel for `network` from `det-app.sqlite`.
fn read_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
) -> Result<Option<MigrationCompletion>, MigrationError> {
    app_kv
        .get::<MigrationCompletion>(DetScope::Global, &sentinel_key_for(network))
        .map_err(|e| MigrationError::Sentinel { source: e })
}

/// Write the wallet-drain completion sentinel for `network`, marking the drain
/// as finished for this network on this install.
fn write_sentinel(
    app_kv: &crate::wallet_backend::DetKv,
    network: Network,
    network_count: u32,
) -> Result<(), MigrationError> {
    write_completion_sentinel(app_kv, &sentinel_key_for(network), network_count)
}

fn now_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl From<MigrationError> for TaskError {
    fn from(source: MigrationError) -> Self {
        super::migration_task_error(Arc::new(source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::dapi_discovery::persist_dapi_addresses_with_hook;
    use crate::config::{CONFIG_ENV_LOCK, Config, NetworkConfig};
    use crate::wallet_backend::DetKv;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    fn dapi_refresh_is_complete(app_context: &AppContext) -> bool {
        app_context
            .app_kv()
            .get::<MigrationCompletion>(
                DetScope::Global,
                &dapi_refresh_sentinel_key_for(app_context.network),
            )
            .expect("read DAPI refresh sentinel")
            .is_some()
    }

    async fn wait_for_dapi_refresh(app_context: &Arc<AppContext>) {
        // `run` deliberately detaches this work; yield so it queues for the guard,
        // then acquire the same guard after the refresh releases it.
        tokio::task::yield_now().await;
        let guard = app_context.migration_run.lock().await;
        drop(guard);
    }

    fn seed_legacy_single_key(app_context: &AppContext) {
        let path = app_context.db.db_file_path().expect("file-backed database");
        let conn = Connection::open(path).expect("open legacy database");
        seed_legacy_row(
            &conn,
            &[7u8; 32],
            &[1u8; 32],
            &[],
            &[],
            "addr",
            None,
            false,
            app_context.network,
        );
    }

    #[tokio::test]
    async fn dapi_refresh_fresh_install_skips_discovery_and_preserves_config() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let original_env = std::fs::read(tmp.path().join(".env")).expect("read original config");
        let original_live_addresses = ctx
            .config
            .read()
            .expect("read original in-memory config")
            .dapi_addresses
            .clone();
        let calls = AtomicUsize::new(0);

        refresh_dapi_nodes_once_with(&ctx, |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((1, "https://unused.example:443".to_string()))
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(dapi_refresh_is_complete(&ctx));
        assert_eq!(
            std::fs::read(tmp.path().join(".env")).expect("read final config"),
            original_env,
        );
        assert_eq!(
            ctx.config
                .read()
                .expect("read in-memory config")
                .dapi_addresses
                .as_ref(),
            original_live_addresses.as_ref(),
        );
    }

    #[tokio::test]
    async fn dapi_refresh_legacy_install_updates_disk_and_live_config() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        seed_legacy_single_key(&ctx);
        let calls = AtomicUsize::new(0);
        let calls_ref = &calls;
        let addresses = "https://one.example:443,https://two.example:443";

        refresh_dapi_nodes_once_with(&ctx, |network| async move {
            assert_eq!(network, Network::Testnet);
            calls_ref.fetch_add(1, Ordering::SeqCst);
            Ok((2, addresses.to_string()))
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(dapi_refresh_is_complete(&ctx));
        let saved = crate::config::Config::load_from(tmp.path()).expect("reload saved config");
        assert_eq!(
            saved
                .config_for_network(Network::Testnet)
                .as_ref()
                .and_then(|config| config.dapi_addresses.as_deref()),
            Some(addresses),
        );
        assert_eq!(
            ctx.config
                .read()
                .expect("read in-memory config")
                .dapi_addresses
                .as_deref(),
            Some(addresses),
        );
    }

    #[test]
    fn dapi_config_persistence_updates_only_selected_key() {
        const CHILD_MARKER: &str = "DET_DAPI_PERSISTENCE_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_none() {
            let status = std::process::Command::new(
                std::env::current_exe().expect("test executable"),
            )
            .arg(
                "backend_task::migration::finish_unwire::tests::dapi_config_persistence_updates_only_selected_key",
            )
            .arg("--exact")
            .env(CHILD_MARKER, "1")
            .env_remove("MAINNET_core_rpc_password")
            .env_remove("TESTNET_wallet_private_key")
            .status()
            .expect("run isolated persistence test");
            assert!(status.success(), "isolated persistence test failed");
            return;
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        let env_path = tmp.path().join(".env");
        let app_context = app_context_for_network(tmp.path(), Network::Mainnet);
        let mainnet_password_before = std::env::var_os("MAINNET_core_rpc_password");
        let testnet_key_before = std::env::var_os("TESTNET_wallet_private_key");
        std::fs::write(
            &env_path,
            "# Preserve operator formatting\n\
             MAINNET_dapi_addresses=https://mainnet-old.example:443\n\
             MAINNET_core_rpc_password='[REDACTED]' # unchanged\n\
             TESTNET_wallet_private_key='[NOT_A_KEY]' # unchanged\n",
        )
        .expect("write initial config");

        persist_dapi_addresses_with_hook(
            &app_context,
            "https://mainnet-new.example:443".to_string(),
            || {
                app_context
                    .config
                    .write()
                    .expect("update live config during persistence")
                    .core_rpc_password = Some("[LIVE_UPDATE]".to_string());
            },
            || {},
        )
        .expect("persist selected DAPI addresses");

        let saved = std::fs::read_to_string(env_path).expect("read updated config");
        assert!(saved.contains("MAINNET_dapi_addresses=https://mainnet-new.example:443\n"));
        assert!(saved.contains("MAINNET_core_rpc_password='[REDACTED]' # unchanged\n"));
        assert!(saved.contains("TESTNET_wallet_private_key='[NOT_A_KEY]' # unchanged\n"));
        assert_eq!(
            std::env::var_os("MAINNET_core_rpc_password"),
            mainnet_password_before,
        );
        assert_eq!(
            std::env::var_os("TESTNET_wallet_private_key"),
            testnet_key_before,
        );
        assert_eq!(
            app_context
                .config
                .read()
                .expect("read live config after persistence")
                .core_rpc_password
                .as_deref(),
            Some("[LIVE_UPDATE]"),
        );
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ConfigPersistenceEvent {
        FirstEntered,
        FirstPausedBeforeSave,
        SecondAttempting,
        SecondEntered,
        SecondSaved,
        FirstSaved,
    }

    #[tokio::test]
    async fn dapi_config_persistence_serializes_across_networks() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        Config {
            mainnet_config: Some(NetworkConfig {
                dapi_addresses: Some("https://mainnet-old.example:443".to_string()),
                ..Default::default()
            }),
            testnet_config: Some(NetworkConfig {
                dapi_addresses: Some("https://testnet-old.example:443".to_string()),
                ..Default::default()
            }),
            devnet_config: None,
            local_config: None,
        }
        .save(tmp.path())
        .expect("write initial config");
        let mainnet_context = app_context_for_network(tmp.path(), Network::Mainnet);
        let testnet_context = app_context_for_network_with_shared_storage(
            tmp.path(),
            Network::Testnet,
            &mainnet_context,
        );

        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
        let first_context = Arc::clone(&mainnet_context);
        let first_before_save_tx = event_tx.clone();
        let first_after_save_tx = event_tx.clone();
        let first = std::thread::spawn(move || {
            persist_dapi_addresses_with_hook(
                &first_context,
                "https://mainnet-new.example:443".to_string(),
                || {
                    first_before_save_tx
                        .send(ConfigPersistenceEvent::FirstEntered)
                        .expect("report first entry");
                    first_before_save_tx
                        .send(ConfigPersistenceEvent::FirstPausedBeforeSave)
                        .expect("report first pause");
                    release_first_rx.recv().expect("release first save");
                },
                || {
                    first_after_save_tx
                        .send(ConfigPersistenceEvent::FirstSaved)
                        .expect("report first save");
                },
            )
            .expect("first persistence");
        });

        assert_eq!(
            event_rx.recv().expect("first entry event"),
            ConfigPersistenceEvent::FirstEntered,
        );
        assert_eq!(
            event_rx.recv().expect("first pause event"),
            ConfigPersistenceEvent::FirstPausedBeforeSave,
        );

        let second_context = Arc::clone(&testnet_context);
        let second_attempt_tx = event_tx.clone();
        let second_before_save_tx = event_tx.clone();
        let second_after_save_tx = event_tx.clone();
        let second = std::thread::spawn(move || {
            second_attempt_tx
                .send(ConfigPersistenceEvent::SecondAttempting)
                .expect("report second attempt");
            persist_dapi_addresses_with_hook(
                &second_context,
                "https://testnet-new.example:443".to_string(),
                || {
                    second_before_save_tx
                        .send(ConfigPersistenceEvent::SecondEntered)
                        .expect("report second entry");
                },
                || {
                    second_after_save_tx
                        .send(ConfigPersistenceEvent::SecondSaved)
                        .expect("report second save");
                },
            )
            .expect("second persistence");
        });

        assert_eq!(
            event_rx.recv().expect("second attempt event"),
            ConfigPersistenceEvent::SecondAttempting,
        );
        let premature_entry = event_rx.recv_timeout(std::time::Duration::from_millis(250));
        let premature_save = if premature_entry.is_ok() {
            Some(
                event_rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .expect("second save before first release"),
            )
        } else {
            None
        };

        release_first_tx.send(()).expect("release first");
        first.join().expect("join first persistence section");
        second.join().expect("join second persistence section");

        assert_eq!(
            premature_entry,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout),
            "the second persistence section entered before the first released",
        );
        assert!(
            premature_save.is_none(),
            "the second persistence section saved before the first released: {premature_save:?}",
        );
        assert_eq!(
            event_rx.try_iter().collect::<Vec<_>>(),
            [
                ConfigPersistenceEvent::FirstSaved,
                ConfigPersistenceEvent::SecondEntered,
                ConfigPersistenceEvent::SecondSaved,
            ],
        );

        let saved = Config::load_from(tmp.path()).expect("load final config");
        assert_eq!(
            saved
                .config_for_network(Network::Mainnet)
                .as_ref()
                .and_then(|config| config.dapi_addresses.as_deref()),
            Some("https://mainnet-new.example:443"),
        );
        assert_eq!(
            saved
                .config_for_network(Network::Testnet)
                .as_ref()
                .and_then(|config| config.dapi_addresses.as_deref()),
            Some("https://testnet-new.example:443"),
        );
        assert_eq!(
            mainnet_context
                .config
                .read()
                .expect("read mainnet live config")
                .dapi_addresses
                .as_deref(),
            Some("https://mainnet-new.example:443"),
        );
        assert_eq!(
            testnet_context
                .config
                .read()
                .expect("read testnet live config")
                .dapi_addresses
                .as_deref(),
            Some("https://testnet-new.example:443"),
        );
    }

    #[tokio::test]
    async fn dapi_config_persistence_reports_save_failure() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        Config {
            mainnet_config: Some(NetworkConfig {
                dapi_addresses: Some("https://mainnet-old.example:443".to_string()),
                ..Default::default()
            }),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
        }
        .save(tmp.path())
        .expect("write initial config");
        let app_context = app_context_for_network(tmp.path(), Network::Mainnet);
        let env_path = tmp.path().join(".env");

        let error = persist_dapi_addresses_with_hook(
            &app_context,
            "https://mainnet-new.example:443".to_string(),
            || {
                std::fs::remove_file(&env_path).expect("remove config file");
                std::fs::create_dir(&env_path).expect("replace config file with directory");
            },
            || {},
        )
        .expect_err("persistence must report the save failure");

        assert!(matches!(
            error,
            TaskError::Config(crate::config::ConfigError::SaveError { .. })
        ));
    }

    #[tokio::test]
    async fn dapi_refresh_reinit_failure_withholds_completion_sentinel() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        seed_legacy_single_key(&ctx);

        refresh_dapi_nodes_once_with(&ctx, |_| async { Ok((1, String::new())) }).await;

        assert!(!dapi_refresh_is_complete(&ctx));
    }

    #[tokio::test]
    async fn dapi_refresh_legacy_detection_failure_retries_then_completes() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        seed_legacy_single_key(&ctx);
        let calls = AtomicUsize::new(0);

        refresh_dapi_nodes_once_with_legacy_check(
            &ctx,
            |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((1, "https://unused.example:443".to_string()))
            },
            |_| {
                Err(MigrationError::LegacyDbRead {
                    table: "wallet",
                    source: rusqlite::Error::InvalidQuery,
                })
            },
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!dapi_refresh_is_complete(&ctx));

        refresh_dapi_nodes_once_with(&ctx, |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((1, "https://retry.example:443".to_string()))
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(dapi_refresh_is_complete(&ctx));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dapi_refresh_discovery_failure_retries_without_failing_migration() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        seed_legacy_wallet(&ctx, &[0x81u8; 64], "funds", ctx.network);
        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");
        let calls = AtomicUsize::new(0);

        let refresh = refresh_dapi_nodes_once_with(&ctx, |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(crate::backend_task::dapi_discovery::DapiDiscoveryError::Timeout)
        });
        let did_work = run_under_guard_with_dapi_refresh(&ctx, refresh)
            .await
            .expect("DAPI discovery must not fail the migration");

        assert!(did_work, "the independent wallet pass still moved data");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!dapi_refresh_is_complete(&ctx));

        refresh_dapi_nodes_once_with(&ctx, |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(crate::backend_task::dapi_discovery::DapiDiscoveryError::Timeout)
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 2, "the next launch retries");
        assert!(!dapi_refresh_is_complete(&ctx));
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dapi_refresh_launches_are_serialized() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let attempts = Arc::new(AtomicUsize::new(0));
        let first_started = Arc::new(tokio::sync::Notify::new());
        let release_first = Arc::new(tokio::sync::Notify::new());

        let first_attempts = Arc::clone(&attempts);
        let first_started_task = Arc::clone(&first_started);
        let release_first_task = Arc::clone(&release_first);
        let first = spawn_dapi_refresh(&ctx, async move {
            first_attempts.fetch_add(1, Ordering::SeqCst);
            first_started_task.notify_one();
            release_first_task.notified().await;
        });
        first_started.notified().await;

        let second_attempts = Arc::clone(&attempts);
        let second_started = Arc::new(tokio::sync::Notify::new());
        let second_started_task = Arc::clone(&second_started);
        let second = spawn_dapi_refresh(&ctx, async move {
            second_attempts.fetch_add(1, Ordering::SeqCst);
            second_started_task.notify_one();
        });

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                second_started.notified(),
            )
            .await
            .is_err(),
            "a second refresh must wait for the first refresh's migration guard",
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        release_first.notify_one();
        first.await.expect("join first refresh");
        second.await.expect("join second refresh");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dapi_refresh_devnet_detection_failure_completes_without_discovery() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = app_context_for_network(tmp.path(), Network::Devnet);
        let calls = AtomicUsize::new(0);

        refresh_dapi_nodes_once_with_legacy_check(
            &ctx,
            |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((1, "https://unused.example:443".to_string()))
            },
            |_| {
                Err(MigrationError::LegacyDbRead {
                    table: "wallet",
                    source: rusqlite::Error::InvalidQuery,
                })
            },
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(dapi_refresh_is_complete(&ctx));
    }

    #[tokio::test]
    async fn dapi_refresh_existing_sentinel_skips_discovery() {
        let _env_guard = CONFIG_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        seed_legacy_single_key(&ctx);
        let calls = AtomicUsize::new(0);

        refresh_dapi_nodes_once_with(&ctx, |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((1, "https://first.example:443".to_string()))
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(dapi_refresh_is_complete(&ctx));

        refresh_dapi_nodes_once_with(&ctx, |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((1, "https://unused.example:443".to_string()))
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(dapi_refresh_is_complete(&ctx));
    }

    #[test]
    fn unreadable_top_up_warning_is_durable_and_acknowledgeable() {
        let app_kv = kv();
        let network = Network::Testnet;

        write_top_up_warning(&app_kv, network, 2).expect("write warning");
        assert_eq!(
            read_top_up_warning(&app_kv, network).expect("read warning"),
            Some(UnreadableTopUpsWarning { count: 2 })
        );

        clear_unreadable_app_data(&app_kv, network).expect("acknowledge warning");
        assert_eq!(
            read_top_up_warning(&app_kv, network).expect("read cleared warning"),
            None
        );
    }

    #[test]
    fn identity_progress_survives_a_failed_pass() {
        let app_kv = kv();
        let network = Network::Testnet;
        let processed = BTreeSet::from([[0x11; 32], [0x22; 32]]);

        write_identity_progress(&app_kv, network, &processed).expect("write progress");

        assert_eq!(
            read_identity_progress(&app_kv, network)
                .expect("read progress")
                .processed,
            processed
        );
    }

    // ── App-data import: scheduled votes + top-up history ────────────

    mod app_data {
        use super::*;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
        use dash_sdk::platform::Identifier;
        use std::cell::RefCell;
        use std::collections::{BTreeMap, BTreeSet};

        const VOTER: [u8; 32] = [0x11u8; 32];

        /// A legacy `data.db` holding one scheduled vote and one top-up for a
        /// testnet identity — the v0.10-dev shape.
        fn legacy_conn() -> Connection {
            let conn = Connection::open_in_memory().expect("open");
            conn.execute_batch(
                "CREATE TABLE scheduled_votes (
                    identity_id BLOB NOT NULL,
                    contested_name TEXT NOT NULL,
                    vote_choice TEXT NOT NULL,
                    time INTEGER NOT NULL,
                    executed INTEGER NOT NULL DEFAULT 0,
                    network TEXT NOT NULL,
                    PRIMARY KEY (identity_id, contested_name)
                 );
                 CREATE TABLE identity (id BLOB PRIMARY KEY, network TEXT NOT NULL);
                 CREATE TABLE top_up (
                    identity_id BLOB NOT NULL,
                    top_up_index INTEGER NOT NULL,
                    amount INTEGER NOT NULL,
                    PRIMARY KEY (identity_id, top_up_index)
                 );",
            )
            .expect("schema");
            conn.execute(
                "INSERT INTO identity (id, network) VALUES (?1, 'testnet')",
                rusqlite::params![VOTER.as_slice()],
            )
            .expect("identity");
            conn.execute(
                "INSERT INTO scheduled_votes
                 (identity_id, contested_name, vote_choice, time, executed, network)
                 VALUES (?1, 'alice', 'Lock', 1700000000, 0, 'testnet')",
                rusqlite::params![VOTER.as_slice()],
            )
            .expect("vote");
            conn.execute(
                "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES (?1, 0, 5000)",
                rusqlite::params![VOTER.as_slice()],
            )
            .expect("top up");
            conn
        }

        /// The core promise: a queued vote survives the upgrade. Losing it
        /// means a masternode voter silently misses a vote window.
        #[test]
        fn imports_scheduled_votes_and_top_ups() {
            let conn = legacy_conn();
            let votes = RefCell::new(Vec::new());
            let top_ups = RefCell::new(Vec::new());

            let outcome = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &BTreeSet::new(),
                |v| {
                    votes.borrow_mut().extend_from_slice(v);
                    Ok(())
                },
                |id, map| {
                    top_ups.borrow_mut().push((*id, map.clone()));
                    Ok(())
                },
            )
            .expect("import");

            assert_eq!(outcome.votes_imported, 1);
            assert_eq!(outcome.votes_unreadable, 0);
            assert_eq!(outcome.top_up_identities_imported, 1);
            assert_eq!(outcome.top_ups_unreadable, 0);

            let votes = votes.borrow();
            assert_eq!(votes.len(), 1);
            assert_eq!(votes[0].contested_name, "alice");
            assert_eq!(votes[0].choice, ResourceVoteChoice::Lock);
            assert_eq!(votes[0].voter_id, Identifier::from(VOTER));
            assert!(!votes[0].executed_successfully);

            let top_ups = top_ups.borrow();
            assert_eq!(top_ups.len(), 1);
            assert_eq!(top_ups[0].1, BTreeMap::from([(0, 5000)]));
        }

        /// A retry must not overwrite a vote the user already cast in the new
        /// build — the legacy row still says `executed = 0`, so re-importing
        /// it would queue the vote a second time.
        #[test]
        fn skips_votes_already_present_in_the_kv_store() {
            let conn = legacy_conn();
            let existing = BTreeSet::from([(VOTER, "alice".to_string())]);
            let votes = RefCell::new(Vec::new());

            let outcome = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &existing,
                |v| {
                    votes.borrow_mut().extend_from_slice(v);
                    Ok(())
                },
                |_, _| Ok(()),
            )
            .expect("import");

            assert_eq!(outcome.votes_imported, 0);
            assert_eq!(outcome.votes_skipped_existing, 1);
            assert!(
                votes.borrow().is_empty(),
                "an already-migrated vote must not be re-queued",
            );
        }

        /// An undecodable vote row is counted and reported — never dropped in
        /// silence, and never fatal. Fatal would be worse than useless: the row
        /// decodes no better on a retry, so it would wedge every launch, and the
        /// wallet drain behind it (QA-101). The readable votes around it still
        /// import.
        #[test]
        fn unreadable_vote_row_is_counted_without_failing_the_import() {
            let conn = legacy_conn();
            conn.execute(
                "INSERT INTO scheduled_votes
                 (identity_id, contested_name, vote_choice, time, executed, network)
                 VALUES (?1, 'corrupt', 'Nonsense', 1, 0, 'testnet')",
                rusqlite::params![VOTER.as_slice()],
            )
            .expect("corrupt vote");

            let votes = RefCell::new(Vec::new());
            let outcome = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &BTreeSet::new(),
                |v| {
                    votes.borrow_mut().extend_from_slice(v);
                    Ok(())
                },
                |_, _| Ok(()),
            )
            .expect("an unreadable vote must not fail the import");

            assert_eq!(outcome.votes_imported, 1, "the readable vote still lands");
            assert_eq!(
                outcome.votes_unreadable, 1,
                "the corrupt row must be reported, not swallowed",
            );
            assert_eq!(votes.borrow().len(), 1);
            assert_eq!(votes.borrow()[0].contested_name, "alice");
        }

        /// A top-up write failure must fail the pass. It is audit trail, so it
        /// never blocks the wallet drain — but swallowing it would let the
        /// app-data sentinel record "done" over a history that never landed,
        /// making a transient k/v error permanent. The votes that already
        /// imported stay imported; the retry is idempotent.
        #[test]
        fn top_up_write_failure_fails_the_pass() {
            let conn = legacy_conn();

            let result = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &BTreeSet::new(),
                |_| Ok(()),
                |_, _| Err(TaskError::WalletNotFound),
            );

            assert!(
                matches!(result, Err(MigrationError::TopUpHistoryWrite { .. })),
                "a top-up write failure must reach the caller so the sentinel is withheld, got {result:?}",
            );
        }

        #[test]
        fn unreadable_top_up_rows_are_counted_without_blocking_readable_history() {
            let conn = legacy_conn();
            conn.execute(
                "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES (?1, 1, -1)",
                rusqlite::params![VOTER.as_slice()],
            )
            .expect("corrupt top up");
            let saved = RefCell::new(Vec::new());

            let outcome = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &BTreeSet::new(),
                |_| Ok(()),
                |_, history| {
                    saved.borrow_mut().push(history.clone());
                    Ok(())
                },
            )
            .expect("row-level damage is non-fatal");

            assert_eq!(outcome.top_ups_unreadable, 1);
            assert_eq!(saved.borrow()[0], BTreeMap::from([(0, 5_000)]));
        }

        /// A structurally unreadable `top_up` table (here: no `amount` column)
        /// fails the pass for the same reason — the caller must withhold the
        /// sentinel rather than declare the history migrated.
        #[test]
        fn top_up_read_failure_fails_the_pass() {
            let conn = Connection::open_in_memory().expect("open");
            conn.execute_batch(
                "CREATE TABLE identity (id BLOB PRIMARY KEY, network TEXT NOT NULL);
                 CREATE TABLE top_up (
                    identity_id BLOB NOT NULL,
                    top_up_index INTEGER NOT NULL
                 );",
            )
            .expect("schema");

            let result = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &BTreeSet::new(),
                |_| Ok(()),
                |_, _| Ok(()),
            );

            assert!(
                matches!(
                    result,
                    Err(MigrationError::LegacyDbRead {
                        table: "top_up",
                        ..
                    })
                ),
                "an unreadable top-up table must reach the caller, got {result:?}",
            );
        }

        /// A fresh install has none of these tables — a no-op, not an error.
        #[test]
        fn missing_tables_are_a_no_op() {
            let conn = Connection::open_in_memory().expect("open");

            let outcome = migrate_app_data_from_conn(
                &conn,
                Network::Testnet,
                &BTreeSet::new(),
                |_| Ok(()),
                |_, _| Ok(()),
            )
            .expect("import");

            assert_eq!(outcome, AppDataMigrationOutcome::default());
        }

        /// The app-data sentinel is per network and distinct from the
        /// wallet-drain sentinel: an install that already drained its wallets
        /// under an earlier build must still import its votes.
        #[test]
        fn sentinel_is_per_network_and_distinct_from_the_wallet_sentinel() {
            let testnet = app_data_sentinel_key_for(Network::Testnet);
            assert_ne!(testnet, app_data_sentinel_key_for(Network::Mainnet));
            assert_ne!(testnet, sentinel_key_for(Network::Testnet));
        }

        /// Votes and top-ups can exist with no wallet rows at all (a
        /// masternode voter who imported identity keys directly), so the
        /// detection gate must sniff their tables too.
        #[test]
        fn detection_gate_covers_the_app_data_tables() {
            assert!(LEGACY_TABLES.contains(&"scheduled_votes"));
            assert!(LEGACY_TABLES.contains(&"top_up"));
        }
    }

    // ── Identity import ──────────────────────────────────────────────

    mod identities {
        use super::*;
        use crate::database::test_helpers::{
            LegacyIdentityFixture, basic_legacy_identity_blob, create_legacy_identity_table,
        };
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use std::cell::RefCell;

        const NETWORK: Network = Network::Testnet;

        pub(super) fn create_identity_table(conn: &Connection) {
            create_legacy_identity_table(conn).expect("create identity table");
        }

        /// A minimal, genuinely-encodable identity blob.
        pub(super) fn identity_blob(id: [u8; 32]) -> Vec<u8> {
            basic_legacy_identity_blob(id, None, NETWORK)
        }

        pub(super) fn insert_identity(
            conn: &Connection,
            id: [u8; 32],
            data: Option<Vec<u8>>,
            is_local: bool,
        ) {
            LegacyIdentityFixture::new(id, data, NETWORK.to_string())
                .with_is_local(is_local)
                .insert(conn)
                .expect("insert identity");
        }

        /// Collects what the importer tried to write, so a test can assert on the
        /// identities that reached the (vault-routing) writer.
        #[derive(Default)]
        struct Recorder {
            imported: RefCell<Vec<[u8; 32]>>,
        }

        /// A blob that will never decode must not take the identities around it
        /// down with it: the readable rows still import, and the failure is
        /// counted so the caller can record a durable warning for the user.
        #[test]
        fn an_undecodable_blob_never_blocks_the_identities_that_do_decode() {
            let conn = Connection::open_in_memory().expect("in-memory db");
            create_identity_table(&conn);
            let good = [0xAA; 32];
            let corrupt = [0xBB; 32];
            insert_identity(&conn, good, Some(identity_blob(good)), true);
            insert_identity(&conn, corrupt, Some(vec![0xFF; 16]), true);

            let recorder = Recorder::default();
            let outcome = migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut BTreeSet::new(),
                |_| Ok(()),
                |_| Ok(false),
                |qi, _| {
                    recorder
                        .imported
                        .borrow_mut()
                        .push(qi.identity.id().to_buffer());
                    Ok(())
                },
            )
            .expect("an undecodable row must not fail the pass");

            assert_eq!(outcome.imported, 1, "the readable identity still imports");
            assert_eq!(outcome.unreadable, 1, "the corrupt row is reported");
            assert_eq!(
                *recorder.imported.borrow(),
                vec![good],
                "only the decodable identity reaches the writer",
            );
            assert!(
                outcome.unreadable > 0,
                "a non-zero `unreadable` is what raises the durable warning that tells \
                 the user which keys did not come across",
            );
        }

        /// An identity already in the store is left untouched — skipped
        /// wholesale, never re-inserted. Re-writing would risk overwriting
        /// whatever the user has done to it since (renamed it, removed a key)
        /// with the stale legacy copy.
        #[test]
        fn an_identity_already_in_the_store_is_never_reimported() {
            let conn = Connection::open_in_memory().expect("in-memory db");
            create_identity_table(&conn);
            let existing = [0xAA; 32];
            let fresh = [0xBB; 32];
            insert_identity(&conn, existing, Some(identity_blob(existing)), true);
            insert_identity(&conn, fresh, Some(identity_blob(fresh)), true);

            let recorder = Recorder::default();
            let mut processed = BTreeSet::from([existing]);
            let outcome = migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut processed,
                |_| Ok(()),
                |_| Ok(false),
                |qi, _| {
                    recorder
                        .imported
                        .borrow_mut()
                        .push(qi.identity.id().to_buffer());
                    Ok(())
                },
            )
            .expect("import");

            assert_eq!(outcome.imported, 1);
            assert_eq!(outcome.skipped_existing, 1);
            assert_eq!(
                *recorder.imported.borrow(),
                vec![fresh],
                "the already-present identity must never reach the insert writer",
            );
        }

        #[test]
        fn direct_blob_presence_skips_and_checkpoints_a_stale_index_snapshot() {
            let conn = Connection::open_in_memory().expect("in-memory db");
            create_identity_table(&conn);
            let existing = [0xAA; 32];
            insert_identity(&conn, existing, Some(identity_blob(existing)), true);

            let recorder = Recorder::default();
            let checkpoints = RefCell::new(Vec::new());
            let mut processed = BTreeSet::new();
            let outcome = migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut processed,
                |processed| {
                    checkpoints.borrow_mut().push(processed.clone());
                    Ok(())
                },
                |id| Ok(id == existing),
                |qi, _| {
                    recorder
                        .imported
                        .borrow_mut()
                        .push(qi.identity.id().to_buffer());
                    Ok(())
                },
            )
            .expect("presence check");

            assert_eq!(outcome.imported, 0);
            assert_eq!(outcome.skipped_existing, 1);
            assert!(recorder.imported.borrow().is_empty());
            assert_eq!(processed, BTreeSet::from([existing]));
            assert_eq!(
                *checkpoints.borrow(),
                vec![BTreeSet::from([existing])],
                "the direct-presence discovery must become durable progress",
            );
        }

        #[test]
        fn a_partial_hard_failure_does_not_reimport_an_earlier_identity() {
            let conn = Connection::open_in_memory().expect("in-memory db");
            create_identity_table(&conn);
            let first = [0x11; 32];
            let failing = [0x22; 32];
            insert_identity(&conn, first, Some(identity_blob(first)), true);
            insert_identity(&conn, failing, Some(identity_blob(failing)), true);

            let attempts = RefCell::new(Vec::new());
            let mut processed = BTreeSet::new();
            let first_run = migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut processed,
                |_| Ok(()),
                |_| Ok(false),
                |qi, _| {
                    let id = qi.identity.id().to_buffer();
                    attempts.borrow_mut().push(id);
                    if id == failing {
                        Err(TaskError::WalletNotFound)
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(matches!(
                first_run,
                Err(MigrationError::IdentityImportFailed { .. })
            ));
            assert!(processed.contains(&first));

            migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut processed,
                |_| Ok(()),
                |_| Ok(false),
                |qi, _| {
                    attempts.borrow_mut().push(qi.identity.id().to_buffer());
                    Ok(())
                },
            )
            .expect("retry");

            assert_eq!(
                attempts.borrow().iter().filter(|id| **id == first).count(),
                1,
                "a previously imported identity must stay skipped even if the user removed it before the retry",
            );
        }

        /// Observed (`is_local = 0`) rows are v0.9.3's lookup cache and NULL-blob
        /// rows have nothing in them. Neither is user data: both are skipped, and
        /// neither counts as unreadable — so neither raises a warning about keys
        /// that were never there.
        #[test]
        fn observed_and_null_blob_rows_are_skipped_without_failing_the_pass() {
            let conn = Connection::open_in_memory().expect("in-memory db");
            create_identity_table(&conn);
            let observed = [0xAA; 32];
            let null_blob = [0xBB; 32];
            insert_identity(&conn, observed, Some(identity_blob(observed)), false);
            insert_identity(&conn, null_blob, None, true);

            let recorder = Recorder::default();
            let outcome = migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut BTreeSet::new(),
                |_| Ok(()),
                |_| Ok(false),
                |qi, _| {
                    recorder
                        .imported
                        .borrow_mut()
                        .push(qi.identity.id().to_buffer());
                    Ok(())
                },
            )
            .expect("import");

            assert_eq!(
                (
                    outcome.imported,
                    outcome.unreadable,
                    outcome.skipped_existing
                ),
                (0, 0, 0),
                "neither row is user data, and neither is a failure",
            );
            assert!(recorder.imported.borrow().is_empty());
        }

        /// A link to a wallet that did not come across is kept verbatim, never
        /// nulled: it is what re-attaches the identity when the user finally
        /// unlocks (or restores) that wallet.
        #[test]
        fn a_link_to_an_absent_wallet_is_preserved_not_nulled() {
            let conn = Connection::open_in_memory().expect("in-memory db");
            create_identity_table(&conn);
            let id = [0xAA; 32];
            let orphan_wallet = [0x77; 32];
            LegacyIdentityFixture::new(id, Some(identity_blob(id)), NETWORK.to_string())
                .with_wallet(orphan_wallet.to_vec(), 4)
                .insert(&conn)
                .expect("insert identity");

            let links: RefCell<Vec<Option<(WalletSeedHash, u32)>>> = RefCell::new(Vec::new());
            let outcome = migrate_identities_from_conn(
                &conn,
                NETWORK,
                &mut BTreeSet::new(),
                |_| Ok(()),
                |_| Ok(false),
                |_, wallet| {
                    links.borrow_mut().push(*wallet);
                    Ok(())
                },
            )
            .expect("import");

            assert_eq!(outcome.imported, 1, "the identity imports anyway");
            assert_eq!(
                *links.borrow(),
                vec![Some((orphan_wallet, 4))],
                "the wallet link must survive verbatim — nulling it would orphan the \
                 identity from its keys permanently",
            );
        }

        /// An identity-only install (a masternode voter with no HD wallet) must
        /// still trip the detection gate, or its keys are never even looked at.
        #[test]
        fn detection_gate_covers_the_identity_table() {
            assert!(LEGACY_TABLES.contains(&"identity"));
        }

        /// The identity sentinel is per network and distinct from the wallet
        /// drain's: reusing the latter would skip the import for every install
        /// that already drained under a build without an identity importer.
        #[test]
        fn sentinel_is_per_network_and_distinct_from_the_other_sentinels() {
            let testnet = identities_sentinel_key_for(Network::Testnet);
            assert_ne!(testnet, identities_sentinel_key_for(Network::Mainnet));
            assert_ne!(testnet, sentinel_key_for(Network::Testnet));
            assert_ne!(testnet, app_data_sentinel_key_for(Network::Testnet));
        }
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

    /// Copying a legacy single key must not change its source table or rows.
    #[test]
    fn single_key_copy_leaves_legacy_table_unchanged() {
        use dash_sdk::dpp::dashcore::Network;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data.db");
        let conn = Connection::open(&path).expect("open legacy db");
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
        drop(conn);
        let before = std::fs::read(&path).expect("snapshot before copy");
        let conn = Connection::open(&path).expect("reopen legacy db");

        let outcome =
            migrate_single_key_rows_from_conn(&conn, |_wif, _alias| Ok(()), Network::Testnet)
                .expect("copy single-key rows");
        assert_eq!(outcome.imported, 1);
        drop(conn);

        assert_eq!(
            std::fs::read(&path).expect("snapshot after copy"),
            before,
            "copying must not mutate the legacy SQLite file",
        );
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
        assert_eq!(got.encrypted_seed.as_slice(), ciphertext);
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
        assert_eq!(got.encrypted_seed.as_slice(), ciphertext);
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
        assert_eq!(got.encrypted_seed.as_slice(), ciphertext);
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
        app_context_for_network(dir, Network::Testnet)
    }

    fn app_context_for_network(dir: &std::path::Path, network: Network) -> Arc<AppContext> {
        crate::app_dir::ensure_env_file(dir);
        let db_file = dir.join("data.db");
        let db = Arc::new(crate::database::Database::new(&db_file).expect("db"));
        db.create_tables(true).expect("create tables");
        db.set_default_version().expect("set version");

        let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
        AppContext::new(
            dir.to_path_buf(),
            network,
            db,
            Default::default(),
            Default::default(),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("AppContext")
    }

    fn app_context_for_network_with_shared_storage(
        dir: &std::path::Path,
        network: Network,
        shared: &AppContext,
    ) -> Arc<AppContext> {
        AppContext::new(
            dir.to_path_buf(),
            network,
            Arc::clone(&shared.db),
            Default::default(),
            Default::default(),
            egui::Context::default(),
            shared.app_kv(),
            shared.secret_store(),
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("AppContext")
    }

    #[test]
    fn identity_deletion_is_rejected_while_migration_is_running() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        ctx.migration_status().set_state(MigrationState::Running {
            step: MigrationStep::Identities,
        });

        assert!(matches!(
            ctx.delete_local_qualified_identity(&Identifier::from([0x11; 32])),
            Err(TaskError::WalletStorageNotReady)
        ));
    }

    #[test]
    fn identity_deletion_is_atomic_with_an_idle_migration_lock_holder() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let deleted = [0x44; 32];
        let _migration_guard = ctx.migration_run.try_lock().expect("claim migration lock");

        assert!(matches!(
            ctx.delete_local_qualified_identity(&Identifier::from(deleted)),
            Err(TaskError::WalletStorageNotReady)
        ));
        assert!(
            !read_identity_progress(&ctx.app_kv(), ctx.network)
                .expect("read progress")
                .processed
                .contains(&deleted),
            "a rejected delete must not leave a tombstone or any other partial write",
        );
    }

    #[tokio::test]
    async fn public_migration_run_waits_behind_idle_deletion_guard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let migration_guard = ctx.migration_run.try_lock().expect("claim migration lock");
        let follower_ctx = Arc::clone(&ctx);
        let follower = tokio::spawn(async move { run(&follower_ctx).await });
        tokio::task::yield_now().await;

        assert!(
            !follower.is_finished(),
            "the public entry point must wait for the shared migration guard",
        );
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Idle),
            "a waiting migration must not publish Running before it owns the guard",
        );

        drop(migration_guard);
        let did_work = follower.await.expect("join").expect("migration task");

        assert!(!did_work, "the fresh-install pass has no legacy work");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "an idle lock holder was deletion, not a completed migration leader",
        );
    }

    #[tokio::test]
    async fn public_migration_follower_returns_the_published_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let migration_guard = ctx.migration_run.try_lock().expect("claim migration lock");
        let source = Arc::new(MigrationError::WalletBackendUnavailable);
        ctx.migration_status().set_state(MigrationState::Failed {
            error: Arc::clone(&source),
        });
        let follower_ctx = Arc::clone(&ctx);
        let follower = tokio::spawn(async move { run(&follower_ctx).await });
        tokio::task::yield_now().await;

        assert!(
            !follower.is_finished(),
            "the follower must wait for the leader"
        );
        drop(migration_guard);
        let error = follower
            .await
            .expect("join")
            .expect_err("the follower must return the leader's failure");

        assert!(matches!(
            error,
            TaskError::MigrationFailed { source: returned }
                if Arc::ptr_eq(&returned, &source)
        ));
    }

    #[test]
    fn identity_deletion_before_migration_is_recorded_for_the_retry_filter() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let deleted = [0x33; 32];

        record_identity_deletion(&ctx, deleted).expect("record deletion");

        assert!(
            read_identity_progress(&ctx.app_kv(), ctx.network)
                .expect("read progress")
                .processed
                .contains(&deleted)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deletion_first_keeps_a_pending_legacy_identity_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");
        let deleted = [0x55; 32];
        let deleted_id = Identifier::from(deleted);
        ctx.delete_local_qualified_identity(&deleted_id)
            .expect("delete through normal path");

        let conn = Connection::open_in_memory().expect("in-memory db");
        identities::create_identity_table(&conn);
        identities::insert_identity(
            &conn,
            deleted,
            Some(identities::identity_blob(deleted)),
            true,
        );
        let mut processed = read_identity_progress(&ctx.app_kv(), ctx.network)
            .expect("read deletion marker")
            .processed;
        let inserted = std::cell::Cell::new(false);
        let outcome = migrate_identities_from_conn(
            &conn,
            ctx.network,
            &mut processed,
            |_| Ok(()),
            |_| Ok(false),
            |_, _| {
                inserted.set(true);
                Ok(())
            },
        )
        .expect("migration pass");

        assert!(
            !inserted.get(),
            "the deleted identity must not be re-imported"
        );
        assert_eq!(outcome.skipped_existing, 1);
        assert!(
            !ctx.has_local_qualified_identity(&deleted_id)
                .expect("read identity store")
        );

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn too_old_database_version_is_rejected_before_migration() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        ctx.db
            .execute("UPDATE settings SET database_version = 10 WHERE id = 1", [])
            .expect("set too-old database version");

        let error = run(&ctx).await.expect_err("version 10 must be rejected");

        assert!(
            matches!(
                &error,
                TaskError::SavedDataTooOld { source }
                    if matches!(
                        source.as_ref(),
                        MigrationError::LegacyDataTooOld {
                            found: 10,
                            minimum_supported: 11,
                        }
                    )
            ),
            "too-old data must keep its typed version diagnostics: {error:?}",
        );
        assert_eq!(
            error.to_string(),
            "This saved data was created by a much older version of Dash Evo Tool and can't be upgraded directly. Please install Dash Evo Tool 0.9.3 first and open your data with it once, then upgrade to this version."
        );
        assert!(
            std::error::Error::source(&error).is_some(),
            "the typed migration source must remain available for diagnostics",
        );
        assert!(
            read_sentinel(&ctx.app_kv(), ctx.network)
                .expect("read sentinel")
                .is_none(),
            "the migration must not write its completion sentinel after rejecting the version",
        );
    }

    #[tokio::test]
    async fn too_new_database_version_is_rejected_before_migration() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        ctx.db
            .execute("UPDATE settings SET database_version = 41 WHERE id = 1", [])
            .expect("set too-new database version");

        let error = run(&ctx).await.expect_err("version 41 must be rejected");

        assert!(
            matches!(
                &error,
                TaskError::SavedDataTooNew { source }
                    if matches!(
                        source.as_ref(),
                        MigrationError::LegacyDataTooNew {
                            found: 41,
                            maximum_supported: 40,
                        }
                    )
            ),
            "too-new data must keep its typed version diagnostics: {error:?}",
        );
        assert_eq!(
            error.to_string(),
            "Your saved data was created by a newer version of Dash Evo Tool. Update to the latest version to open it."
        );
        assert!(
            std::error::Error::source(&error).is_some(),
            "the typed migration source must remain available for diagnostics",
        );
        assert!(
            read_sentinel(&ctx.app_kv(), ctx.network)
                .expect("read sentinel")
                .is_none(),
            "the migration must not write its completion sentinel after rejecting the version",
        );
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Idle),
            "rejecting the version before any pass must not publish a migration state",
        );
    }

    #[test]
    fn v093_database_version_is_accepted_for_direct_migration() {
        validate_legacy_database_version(11).expect("v0.9.3 data must remain migratable");
    }

    #[test]
    fn current_database_version_is_accepted_for_direct_migration() {
        validate_legacy_database_version(i64::from(crate::database::DEFAULT_DB_VERSION))
            .expect("current data must remain migratable");
    }

    /// Upper-accept boundary: the newest supported pre-unwire layout
    /// (`MAX_DIRECT_MIGRATION_VERSION`, currently 40) is the top of the accepted
    /// 11..=40 range and must still migrate, while the first version above it is
    /// rejected as too new. Pins both sides of the ceiling so a silent narrowing
    /// of the range is caught.
    #[test]
    fn max_supported_database_version_is_accepted_for_direct_migration() {
        use crate::model::data_migration::MAX_DIRECT_MIGRATION_VERSION;

        validate_legacy_database_version(MAX_DIRECT_MIGRATION_VERSION)
            .expect("the newest supported pre-unwire data version must remain migratable");
        assert!(
            matches!(
                validate_legacy_database_version(MAX_DIRECT_MIGRATION_VERSION + 1),
                Err(MigrationError::LegacyDataTooNew { .. })
            ),
            "the first version above the supported range must be rejected as too new",
        );
    }

    #[test]
    fn invalid_and_future_database_versions_are_typed() {
        assert!(matches!(
            validate_legacy_database_version(-1),
            Err(MigrationError::LegacyDataTooOld { found: -1, .. })
        ));
        let future_error = validate_legacy_database_version(41)
            .expect_err("the first unknown future version must be rejected");
        assert!(matches!(
            &future_error,
            MigrationError::LegacyDataTooNew { found: 41, .. }
        ));
        let task_error = TaskError::from(future_error);
        assert!(matches!(task_error, TaskError::SavedDataTooNew { .. }));
        assert!(std::error::Error::source(&task_error).is_some());
        assert!(matches!(
            validate_legacy_database_version(i64::MAX),
            Err(MigrationError::LegacyDataTooNew { .. })
        ));
    }

    /// F113 — a launch with no legacy rows must report `did_work = false`
    /// and leave the migration state `Ready`, so the per-frame banner
    /// reconciler never shows a spurious "storage update complete".
    #[tokio::test]
    async fn run_with_no_legacy_rows_is_a_silent_noop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());

        let did_work = run(&ctx).await.expect("run");

        assert!(!did_work, "fresh install moved no data");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "no-op launch must become Ready without publishing Success",
        );
    }

    /// F113 — once the per-network sentinel exists, a subsequent launch is
    /// a no-op: `did_work = false` and the state becomes `Ready` (no banner).
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
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "sentinel short-circuit must become Ready",
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
        assert!(!MigrationError::InteractivePromptUnavailable.is_backend_not_ready());
    }

    /// `migration_error_chain` reuses a `MigrationFailed` chain verbatim (so the
    /// combined banner shows the same typed source, and a wrapped
    /// `WalletBackendUnavailable` still classifies as backend-not-ready), and
    /// wraps any stray non-migration `TaskError` rather than dropping it.
    #[test]
    fn migration_error_chain_preserves_migration_source_and_wraps_others() {
        let inner = Arc::new(MigrationError::WalletBackendUnavailable);
        let chain = migration_error_chain(TaskError::MigrationFailed {
            source: Arc::clone(&inner),
        });
        assert!(
            Arc::ptr_eq(&chain, &inner),
            "a MigrationFailed chain must be reused, not re-wrapped",
        );
        assert!(
            chain.is_backend_not_ready(),
            "the reused chain keeps its backend-not-ready classification",
        );

        let wrapped = migration_error_chain(TaskError::WalletNotFound);
        assert!(
            matches!(*wrapped, MigrationError::Unexpected { .. }),
            "a non-migration error is wrapped, never dropped",
        );
        assert!(
            !wrapped.is_backend_not_ready(),
            "a wrapped stray error is terminal, offered with a retry",
        );
    }

    /// The coercion is TOTAL — every `TaskError` yields a publishable chain that
    /// still carries the original error as its `#[source]`. This is what keeps a
    /// failed migration from stranding the status on `Running`: while it is
    /// running, `run_backend_task` rejects every wallet-touching task with
    /// `WalletStorageNotReady` and the banner offers no retry, so an error that
    /// published nothing would wedge the wallet surface until the app restarts.
    #[test]
    fn every_task_error_yields_a_publishable_chain_with_its_source_intact() {
        for stray in [
            TaskError::WalletNotFound,
            TaskError::WalletStorageNotReady,
            TaskError::IdentityNotFound,
        ] {
            let expected = stray.to_string();
            let chain = migration_error_chain(stray);

            let source = std::error::Error::source(&*chain)
                .expect("the stray error must survive as the chain's source");
            assert_eq!(
                source.to_string(),
                expected,
                "the original error must reach the details panel, not be replaced",
            );
        }
    }

    /// Wire the real wallet seam onto a fixture context. Offline: the backend
    /// builds its sidecars and hydrates from them without touching the network,
    /// so an end-to-end `run()` can be driven to completion in a unit test. The
    /// wallet drain aborts at its first step without a wired backend, so the
    /// funds-reachability tests below cannot use [`fresh_app_context`] alone.
    async fn wire_backend(app_context: &Arc<AppContext>) {
        let (tx, _rx) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, app_context.egui_ctx().clone());
        app_context
            .ensure_wallet_backend(sender)
            .await
            .expect("wallet backend must wire offline");
    }

    /// Stage the v0.10-dev vote queue: the legacy table plus the rows given as
    /// `(contested_name, vote_choice)`. A `vote_choice` the reader cannot parse
    /// is the corrupt row an upgrade has to survive.
    fn seed_legacy_votes(
        app_context: &Arc<AppContext>,
        voter: &[u8; 32],
        rows: &[(&str, &str)],
        network: dash_sdk::dpp::dashcore::Network,
    ) {
        crate::database::test_helpers::create_legacy_scheduled_votes_table(&app_context.db)
            .expect("create legacy scheduled_votes table");
        for (contested_name, vote_choice) in rows {
            crate::database::test_helpers::seed_legacy_scheduled_vote_row(
                &app_context.db,
                voter,
                contested_name,
                vote_choice,
                network,
            )
            .expect("insert legacy scheduled vote row");
        }
    }

    /// Stage a legacy unprotected HD wallet row whose xpub derives from its seed,
    /// so the drain produces a wallet the registration gate accepts. Returns the
    /// seed hash the wallet is keyed by.
    fn seed_legacy_wallet(
        app_context: &Arc<AppContext>,
        seed: &[u8; 64],
        alias: &str,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> crate::model::wallet::WalletSeedHash {
        let seed_hash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(seed);
        let epk = crate::database::test_helpers::legacy_master_epk_bytes(seed, network);
        crate::database::test_helpers::seed_legacy_unprotected_hd_wallet_row(
            &app_context.db,
            &seed_hash,
            seed,
            &epk,
            alias,
            network,
        )
        .expect("insert legacy wallet row");
        seed_hash
    }

    fn seed_legacy_protected_wallet(
        app_context: &Arc<AppContext>,
        seed: &[u8; 64],
        password: &str,
        alias: &str,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> crate::model::wallet::WalletSeedHash {
        use crate::model::wallet::encryption::{EncryptedEnvelope, encrypt_message};

        let seed_hash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(seed);
        let epk = crate::database::test_helpers::legacy_master_epk_bytes(seed, network);
        let EncryptedEnvelope {
            ciphertext,
            salt,
            nonce,
        } = encrypt_message(seed, password).expect("encrypt protected fixture seed");
        crate::database::test_helpers::seed_legacy_protected_hd_wallet_row(
            &app_context.db,
            &seed_hash,
            &ciphertext,
            &salt,
            &nonce,
            &epk,
            alias,
            Some("the saved hint"),
            network,
        )
        .expect("insert protected legacy wallet row");
        seed_hash
    }

    /// A user who skips every protected wallet can finish the migration without
    /// changing that wallet's protection. A later ordinary unlock without
    /// session retention must still populate the upstream id map before dropping
    /// the temporary secret, closing the original WalletNotLoaded failure mode.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skipped_protected_wallet_completes_and_registers_on_later_ordinary_unlock() {
        use crate::context::WalletUnlockRetention;
        use crate::wallet_backend::SecretScope;
        use crate::wallet_backend::poison::RwLockRecover;
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;
        let seed = [0xC5; 64];
        let password = "correct horse battery staple";
        let seed_hash = seed_legacy_protected_wallet(&ctx, &seed, password, "Savings", network);

        ctx.install_secret_prompt(Arc::new(
            crate::wallet_backend::secret_prompt::test_support::TestPrompt::never(),
        ));

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");
        let migration_context = Arc::clone(&ctx);
        let migration = tokio::spawn(async move { run(&migration_context).await });

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !matches!(
            ctx.migration_status().state().as_ref(),
            MigrationState::AwaitingWalletPasswords { wallets } if wallets == &vec![seed_hash]
        ) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "migration must publish the protected wallet password prompt",
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let wallet = ctx
            .wallet_arc(&seed_hash)
            .expect("hydrated protected wallet");
        assert!(!wallet.read_recover().is_open());
        assert!(!backend.is_wallet_registered(&seed_hash));

        ctx.migration_status().skip_wallet(seed_hash);
        assert!(
            migration
                .await
                .expect("migration task must not panic")
                .expect("skipping must not fail migration"),
            "the migration moved legacy wallet data",
        );

        assert_eq!(*ctx.migration_status().state(), MigrationState::Success);
        assert!(
            read_sentinel(&ctx.app_kv(), network)
                .expect("read sentinel")
                .is_some(),
            "skipping every remaining wallet must still write the completion sentinel",
        );
        assert!(!wallet.read_recover().is_open());
        assert_eq!(ctx.unregistered_open_wallet_count(), 0);
        let secret_store = ctx.secret_store();
        let seed_view = crate::wallet_backend::WalletSeedView::new(&secret_store);
        let legacy_envelope = seed_view
            .legacy_envelope_get(&seed_hash)
            .expect("read legacy protected envelope")
            .expect("a skipped wallet must keep its legacy protected envelope");
        assert!(legacy_envelope.uses_password);
        assert!(!legacy_envelope.salt.is_empty());
        assert!(!legacy_envelope.nonce.is_empty());
        assert_eq!(
            seed_view
                .scheme(&seed_hash)
                .expect("current envelope scheme"),
            crate::wallet_backend::secret_seam::SecretScheme::Absent,
            "skipping must not re-encrypt the seed into the current envelope",
        );

        wallet
            .write_recover()
            .wallet_seed
            .open(password)
            .expect("ordinary unlock verifies the saved password");
        ctx.handle_wallet_unlocked(&wallet, password, WalletUnlockRetention::OperationOnly)
            .expect("ordinary unlock must save the seed in the current vault");

        let scope = SecretScope::HdSeed { seed_hash };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let registered = backend.is_wallet_registered(&seed_hash);
            let forgotten = !backend.secret_access().is_session_cached(&scope);
            if registered && forgotten {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "ordinary unlock must register the skipped wallet and then forget its seed",
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        assert_eq!(backend.wallet_count().await, 1);
        assert!(
            seed_view
                .legacy_envelope_get(&seed_hash)
                .expect("read legacy envelope after unlock")
                .is_none(),
            "a later unlock must collect the redundant legacy envelope",
        );
        assert_eq!(
            seed_view.scheme(&seed_hash).expect("scheme after unlock"),
            crate::wallet_backend::secret_seam::SecretScheme::Protected,
        );

        backend.shutdown().await;
    }

    /// A standalone MCP/CLI context has no frame loop capable of rendering the
    /// password prompt. A protected legacy wallet must therefore fail promptly
    /// instead of parking on `Notify` forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_wallet_fails_fast_without_an_interactive_prompt() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        seed_legacy_protected_wallet(
            &ctx,
            &[0xD1; 64],
            "headless-password",
            "Headless savings",
            Network::Testnet,
        );
        wire_backend(&ctx).await;

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ctx.run_migration_task(crate::backend_task::migration::MigrationTask::FinishUnwire),
        )
        .await
        .expect("headless storage update must never wait for a person")
        .expect_err("a protected wallet requires the desktop app");

        match &result {
            TaskError::StorageUpdateNeedsDesktop { source } => assert!(matches!(
                source.as_ref(),
                MigrationError::InteractivePromptUnavailable
            )),
            other => panic!("expected the dedicated headless storage-update error, got {other:?}"),
        }
        assert!(
            result
                .to_string()
                .contains("Open the Dash Evo Tool desktop app once")
        );
        assert!(
            !matches!(
                ctx.migration_status().state().as_ref(),
                MigrationState::AwaitingWalletPasswords { .. }
            ),
            "a headless context must never publish a prompt nobody can render",
        );

        ctx.wallet_backend()
            .expect("backend wired")
            .shutdown()
            .await;
    }

    /// The old SQLite file is a recovery artifact, not migration-owned state.
    /// A complete run that unlocks one protected wallet and skips another must
    /// leave the file byte-for-byte unchanged, and both copied legacy envelopes
    /// must remain available after the run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn full_unlock_and_skip_run_leaves_legacy_database_unchanged() {
        use crate::context::WalletUnlockRetention;
        use crate::wallet_backend::poison::RwLockRecover;
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let unlock_password = "unlock-this-wallet";
        let unlock_hash = seed_legacy_protected_wallet(
            &ctx,
            &[0xD2; 64],
            unlock_password,
            "Unlock me",
            Network::Testnet,
        );
        let skip_hash = seed_legacy_protected_wallet(
            &ctx,
            &[0xD3; 64],
            "skip-this-wallet",
            "Skip me",
            Network::Testnet,
        );
        let legacy_path = tmp.path().join("data.db");
        let before = std::fs::read(&legacy_path).expect("snapshot legacy database before run");

        ctx.install_secret_prompt(Arc::new(
            crate::wallet_backend::secret_prompt::test_support::TestPrompt::never(),
        ));
        wire_backend(&ctx).await;
        let migration_context = Arc::clone(&ctx);
        let migration = tokio::spawn(async move { run(&migration_context).await });

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let MigrationState::AwaitingWalletPasswords { wallets } =
                ctx.migration_status().state().as_ref()
                && wallets.contains(&unlock_hash)
                && wallets.contains(&skip_hash)
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "migration must publish both protected wallets",
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let unlock_wallet = ctx.wallet_arc(&unlock_hash).expect("wallet to unlock");
        unlock_wallet
            .write_recover()
            .wallet_seed
            .open(unlock_password)
            .expect("fixture password opens wallet");
        ctx.handle_wallet_unlocked(
            &unlock_wallet,
            unlock_password,
            WalletUnlockRetention::UntilAppClose,
        )
        .expect("unlocked seed must land in the current vault");
        ctx.migration_status().skip_wallet(skip_hash);
        ctx.migration_status().notify_wallet_password_submitted();

        assert!(
            migration
                .await
                .expect("migration task must not panic")
                .expect("unlock plus skip must complete"),
        );
        let after = std::fs::read(&legacy_path).expect("snapshot legacy database after run");
        assert_eq!(after, before, "the legacy database bytes must never change");

        let store = ctx.secret_store();
        let view = crate::wallet_backend::WalletSeedView::new(&store);
        assert!(
            view.legacy_envelope_get(&unlock_hash)
                .expect("read unlocked legacy envelope")
                .is_none(),
            "unlocking must collect the copied legacy recovery envelope",
        );
        assert!(
            view.legacy_envelope_get(&skip_hash)
                .expect("read skipped legacy envelope")
                .is_some(),
            "skipping must retain the copied legacy recovery envelope",
        );

        ctx.wallet_backend()
            .expect("backend wired")
            .shutdown()
            .await;
    }

    /// Stage an identity row whose blob will never decode, so `read_identities`
    /// counts it unreadable. Written into the *modern* `identity` table the app
    /// context already created — not the legacy fixture shape — so the NULL wallet
    /// link is what satisfies that table's both-or-neither `CHECK`.
    fn insert_corrupt_identity_row(conn: &Connection, id: [u8; 32]) {
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, network)
             VALUES (?1, ?2, 2, 1, 'testnet')",
            rusqlite::params![id.as_slice(), vec![0xFFu8; 8]],
        )
        .expect("corrupt identity row");
    }

    /// QA-101 — the headline funds regression. A single undecodable legacy vote
    /// row must NOT stand between the user and their wallet: the drain runs to
    /// completion (seeds copied, wallet hydrated AND upstream-registered, the
    /// completion sentinel written), while the corrupt row is still surfaced —
    /// counted on the terminal state and left in `data.db` — rather than
    /// silently dropped. Before the fix the vote import ran first, unconditionally
    /// and fatally, so this wallet stayed unreachable on every launch forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_completes_the_wallet_drain_despite_an_unreadable_vote_row() {
        use crate::wallet_backend::poison::RwLockRecover;
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        let seed_hash = seed_legacy_wallet(&ctx, &[0xA3u8; 64], "funds", network);
        let voter = [0x11u8; 32];
        seed_legacy_votes(
            &ctx,
            &voter,
            &[("alice", "Lock"), ("corrupt", "Nonsense")],
            network,
        );

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");
        assert!(
            !backend.is_wallet_registered(&seed_hash),
            "precondition: the legacy wallet is not migrated yet",
        );

        run(&ctx)
            .await
            .expect("an unreadable vote row must not fail the wallet migration");

        // Funds first: hydrated into `ctx.wallets`, registered in the same
        // `id_map` that `resolve_wallet` consults, and the drain recorded as done.
        assert!(
            ctx.wallets.read_recover().contains_key(&seed_hash),
            "the migrated wallet must be visible after the migration",
        );
        assert!(
            backend.is_wallet_registered(&seed_hash),
            "the migrated wallet must be reachable — a corrupt vote row must never \
             block access to funds",
        );
        assert!(
            read_sentinel(&ctx.app_kv(), network)
                .expect("read sentinel")
                .is_some(),
            "the completion sentinel must be written once the drain succeeds",
        );

        // No silent loss: the readable vote came across, and the unreadable one is
        // reported on the terminal state (the banner the user sees), not swallowed.
        let votes = ctx.get_scheduled_votes().expect("read scheduled votes");
        assert_eq!(votes.len(), 1, "the readable vote must still be imported");
        assert_eq!(votes[0].contested_name, "alice");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 0,
                votes: 1,
                top_ups: 0,
            },
            "the corrupt vote row must be surfaced to the user, not dropped in silence",
        );

        // The legacy rows survive, so a build with a better decoder can still get
        // the vote back.
        let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM scheduled_votes", [], |r| r.get(0))
            .expect("count legacy votes");
        assert_eq!(remaining, 2, "the migration must never delete legacy rows");

        backend.shutdown().await;
    }

    /// TC-MIG-009 — end-to-end idempotency: `run()` called TWICE on the same
    /// `AppContext` must make the second launch a true no-op. Not just "the
    /// sentinel reads back" — the second pass must re-fire no side effect, which
    /// this proves by clearing the vote queue between the runs (what the app does
    /// once a vote has been cast) and requiring the re-run NOT to resurrect it
    /// from the legacy row that is still sitting in `data.db`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn second_run_on_the_same_context_re_fires_nothing() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        let seed_hash = seed_legacy_wallet(&ctx, &[0xB4u8; 64], "funds", network);
        seed_legacy_votes(&ctx, &[0x22u8; 32], &[("alice", "Lock")], network);

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        assert!(
            run(&ctx).await.expect("first run"),
            "the first launch moves legacy data",
        );
        wait_for_dapi_refresh(&ctx).await;
        assert!(backend.is_wallet_registered(&seed_hash));
        assert_eq!(ctx.get_scheduled_votes().expect("read votes").len(), 1);
        let sentinel_after_first = read_sentinel(&ctx.app_kv(), network)
            .expect("read sentinel")
            .expect("sentinel written by the first run");

        // The user casts the vote, so the app drops it from the queue. The legacy
        // row still says `executed = 0` — a re-import would queue it a second time.
        ctx.clear_all_scheduled_votes().expect("clear vote queue");

        let did_work = run(&ctx).await.expect("second run");

        assert!(!did_work, "the second launch must move nothing");
        assert!(
            ctx.get_scheduled_votes().expect("read votes").is_empty(),
            "a re-run must not resurrect a vote the user has already dealt with",
        );
        assert_eq!(
            read_sentinel(&ctx.app_kv(), network)
                .expect("read sentinel")
                .expect("sentinel still present"),
            sentinel_after_first,
            "a no-op launch must not rewrite the completion sentinel",
        );
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "a no-op launch must not publish a completion banner",
        );

        backend.shutdown().await;
    }

    /// A top-up import failure must leave the app-data sentinel unwritten, so
    /// the next launch retries the (idempotent) import. Writing the sentinel
    /// anyway would make a one-off failure permanent: the fast path short-circuits
    /// on it forever and the history never comes across. Staged with a legacy
    /// `top_up` table that has no `amount` column, which is what a structurally
    /// damaged legacy file looks like from the reader's side.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn app_data_sentinel_is_withheld_when_top_ups_cannot_be_imported() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;
        let identity = [0x44u8; 32];

        {
            // `identity` comes from the modern schema; only `top_up` is staged, and
            // without its `amount` column — the reader's prepare fails on it.
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            conn.execute_batch(
                "CREATE TABLE top_up (
                    identity_id BLOB NOT NULL,
                    top_up_index INTEGER NOT NULL
                 );",
            )
            .expect("broken legacy top-up schema");
            conn.execute(
                "INSERT INTO top_up (identity_id, top_up_index) VALUES (?1, 0)",
                rusqlite::params![identity.as_slice()],
            )
            .expect("top-up row");
        }

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        let result = migrate_app_data(&ctx);

        assert!(
            result.is_err(),
            "an unimportable top-up history must fail the app-data pass, got {result:?}",
        );
        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &app_data_sentinel_key_for(network))
                .expect("read app-data sentinel")
                .is_none(),
            "the app-data sentinel must not be written when the import failed — \
             the next launch has to retry it",
        );

        backend.shutdown().await;
    }

    /// Both DET-owned passes broken on the SAME launch must surface BOTH signals,
    /// not have one silently eat the other. Here an undecodable identity row
    /// (unreadable identities) and a structurally-damaged legacy `top_up` table
    /// (a hard app-data failure) coincide. Before the fix the run published
    /// `SucceededWithUnreadableIdentities` and returned `Ok`, so the app-data
    /// failure never reached a banner — no retry, swallowed every launch. Now the
    /// combined `FailedWithUnreadableIdentities` carries the identity count AND
    /// the app-data error chain: the user sees both, with a retry for the
    /// app-data half. Funds are still safe (the wallet drain ran regardless), and
    /// neither DET-owned sentinel is written, so both retry on the next launch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn both_app_data_and_identity_failures_surface_together() {
        use crate::wallet_backend::poison::RwLockRecover;
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        // Funds: a normal legacy wallet, so the drain completes and `moved_data`.
        let seed_hash = seed_legacy_wallet(&ctx, &[0xD6u8; 64], "funds", network);

        {
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            insert_corrupt_identity_row(&conn, [0x44u8; 32]);
            // A structurally-damaged legacy `top_up` (no `amount` column) with a
            // row → the app-data pass hard-fails when its reader's prepare fails.
            conn.execute_batch(
                "CREATE TABLE top_up (
                    identity_id BLOB NOT NULL,
                    top_up_index INTEGER NOT NULL
                 );",
            )
            .expect("broken legacy top-up schema");
            conn.execute(
                "INSERT INTO top_up (identity_id, top_up_index) VALUES (?1, 0)",
                rusqlite::params![[0x44u8; 32].as_slice()],
            )
            .expect("top-up row");
        }

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        let did_work = run(&ctx)
            .await
            .expect("both failures are non-fatal to the run — it publishes its own terminal state");
        assert!(did_work, "the wallet drain moved data");

        // Both signals in one terminal state: the identity count AND the app-data
        // error chain. The pre-fix `SucceededWithUnreadableIdentities` would have
        // hidden the app-data failure — `matches!` here would fail on it.
        assert!(
            matches!(
                &*ctx.migration_status().state(),
                MigrationState::FailedWithUnreadableIdentities { count: 1, .. }
            ),
            "both failures must surface together, got {:?}",
            ctx.migration_status().state(),
        );

        // Funds stay safe: the drain ran despite both DET-owned passes breaking.
        assert!(
            ctx.wallets.read_recover().contains_key(&seed_hash),
            "the migrated wallet must be visible — neither DET-owned failure may block funds",
        );
        assert!(
            backend.is_wallet_registered(&seed_hash),
            "the migrated wallet must be reachable",
        );

        // The app-data pass hard-failed, so its sentinel stays unwritten and the
        // import retries next launch. The identity pass did NOT fail — it imported
        // what decoded and counted what did not — so it completes, and the
        // undecodable row is carried forward by its durable warning instead of by
        // an endlessly-retried import that would resurrect deleted identities.
        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &app_data_sentinel_key_for(network))
                .expect("read app-data sentinel")
                .is_none(),
            "the app-data sentinel must stay unwritten so the failure retries",
        );
        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &identities_sentinel_key_for(network))
                .expect("read identity sentinel")
                .is_some(),
            "an unreadable row is not a pass failure: the import completes, and the row is \
             reported by the durable warning rather than retried forever",
        );

        backend.shutdown().await;
    }

    /// A failed *vote-warning read* is not an app-data failure, and the banner may
    /// not say it is. Here the app-data pass SUCCEEDS (its sentinel is written) and
    /// only the follow-up warning-record read fails, alongside an undecodable
    /// identity row. `FailedWithUnreadableIdentities` tells the user "updating the
    /// rest of your previous data did not finish" and offers a "Retry now" that
    /// re-runs a pass which already completed — false on both counts. The identity
    /// signal is the one the user must act on, so the run falls through to the
    /// honest `SucceededWithUnreadableIdentities` and the unreadable notice record
    /// is left for the next launch to re-read.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unreadable_vote_warning_record_does_not_claim_the_app_data_pass_failed() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        // Funds: a normal legacy wallet, so the drain completes and `moved_data`.
        seed_legacy_wallet(&ctx, &[0xD7u8; 64], "funds", network);

        {
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            // A corrupt identity blob → unreadable == 1, which is what steers the run
            // into the branch under test. The legacy `top_up` table is left alone, so
            // the app-data pass runs clean and writes its sentinel.
            insert_corrupt_identity_row(&conn, [0x45u8; 32]);
        }

        // Poison the warning record: a unit value encodes to an empty bincode body,
        // so decoding it back as an `UnreadableVotesWarning` hits an unexpected end
        // of input. The app-data pass decodes zero unreadable votes and therefore
        // never overwrites it.
        ctx.app_kv()
            .put(DetScope::Global, &vote_warning_key_for(network), &())
            .expect("poison the vote-warning record");
        assert!(
            read_vote_warning(&ctx.app_kv(), network).is_err(),
            "precondition: the poisoned record must make the warning read fail, \
             otherwise this test would pass for the wrong reason",
        );

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        let did_work = run(&ctx).await.expect("a failed notice read is not fatal");
        assert!(did_work, "the wallet drain moved data");

        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 0,
                top_ups: 0,
            },
            "an unreadable notice record must not be reported as an app-data failure",
        );

        // The fact the old banner denied: the app-data pass really did finish.
        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &app_data_sentinel_key_for(network))
                .expect("read app-data sentinel")
                .is_some(),
            "the app-data sentinel proves the pass completed — a banner offering to \
             'finish updating' it would be lying",
        );

        backend.shutdown().await;
    }

    /// Fix-8 — the unreadable-vote warning is durable. The discovery run records
    /// it; every later launch re-publishes it from that record even though both
    /// sentinels short-circuit the passes, so a user who was away when the
    /// migration finished still learns that a vote with a live deadline needs
    /// re-scheduling. Only an explicit acknowledgement retires it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_vote_warning_is_republished_until_acknowledged() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        seed_legacy_wallet(&ctx, &[0xC5u8; 64], "funds", network);
        seed_legacy_votes(&ctx, &[0x33u8; 32], &[("corrupt", "Nonsense")], network);

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        run(&ctx).await.expect("first launch");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 0,
                votes: 1,
                top_ups: 0,
            },
            "the discovery run surfaces the warning",
        );

        // A later launch starts from a fresh in-memory status, and both sentinels
        // now short-circuit their passes — the warning must come from storage.
        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("second launch");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 0,
                votes: 1,
                top_ups: 0,
            },
            "a warning the user may have missed must survive a restart",
        );

        acknowledge_unreadable_app_data(&ctx).expect("acknowledge");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "acknowledging clears the banner immediately",
        );

        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("third launch");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "an acknowledged warning must never come back",
        );

        backend.shutdown().await;
    }

    /// An unreadable identity must not permanently hide an unreadable *vote*.
    /// Both damaged on the same launch is the trap: the identity branch returns
    /// early, ahead of the durable vote-warning read, so a lone identity signal
    /// would bury the vote half — and the user would silently miss a vote whose
    /// deadline is still live. Both counts ride one terminal state instead, both
    /// halves survive a restart (each re-read from its own durable record, since
    /// both sentinels short-circuit their passes by then), and acknowledging the
    /// vote half retires only that half.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_identities_do_not_hide_the_unreadable_vote_warning() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        seed_legacy_wallet(&ctx, &[0xE7u8; 64], "funds", network);
        // One vote decodes, one does not → the app-data pass succeeds AND records
        // a durable unreadable-vote warning.
        seed_legacy_votes(
            &ctx,
            &[0x55u8; 32],
            &[("alice", "Lock"), ("corrupt", "Nonsense")],
            network,
        );
        {
            // A corrupt identity blob → the identity pass counts it unreadable and
            // records a durable warning, so the identity branch recurs on every launch.
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            insert_corrupt_identity_row(&conn, [0x66u8; 32]);
        }

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        run(&ctx).await.expect("neither damaged row is fatal");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 1,
                top_ups: 0,
            },
            "the discovery run must name BOTH remedies — the identity warning may \
             not swallow a vote with a live deadline",
        );

        // The readable vote still came across; only the corrupt one did not.
        let votes = ctx.get_scheduled_votes().expect("read scheduled votes");
        assert_eq!(votes.len(), 1, "the readable vote must still be imported");

        // Both sentinels are now written, which is what makes the next launch a
        // real test of the durable reads: both passes short-circuit and report
        // zero unreadable rows, so *neither* half of the banner can come from a
        // counter — each must be re-read from its own durable warning record.
        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &app_data_sentinel_key_for(network))
                .expect("read app-data sentinel")
                .is_some(),
            "precondition: the app-data pass completed, so its counters go quiet from now on",
        );
        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &identities_sentinel_key_for(network))
                .expect("read identity sentinel")
                .is_some(),
            "precondition: the identity pass completed too — a withheld sentinel would re-run \
             the import forever and resurrect identities the user has deleted",
        );

        // Later launch: both sentinels short-circuit their passes, so both halves
        // of the combined banner must come from the durable records, not counters.
        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("second launch");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 1,
                top_ups: 0,
            },
            "the vote warning must survive a restart even while identities stay unreadable",
        );

        // Acknowledging retires the vote half only: the identities are still
        // unreadable, so their warning must keep coming back on its own.
        acknowledge_unreadable_app_data(&ctx).expect("acknowledge");
        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("third launch");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 0,
                top_ups: 0,
            },
            "an acknowledged vote warning must not come back, but the identity one must",
        );

        backend.shutdown().await;
    }

    /// The resurrection regression. One genuinely corrupt legacy row must not
    /// make the identity import re-run forever: while it did, an identity the
    /// user had deliberately DELETED — a routine security gesture, offered on
    /// both the Identities and the Masternode screens — was silently re-imported
    /// on the very next launch, its alias restored and its legacy plaintext keys
    /// re-written into the vault. The skip-if-present rule cannot help here: a
    /// deleted identity is, by construction, no longer present.
    ///
    /// The import now writes its sentinel unconditionally, exactly like the
    /// app-data pass, so it is a once-only event and a deletion is durable. The
    /// unreadable row is still reported — from a record that outlives the pass.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_deleted_identity_is_not_resurrected_by_an_unreadable_sibling_row() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        seed_legacy_wallet(&ctx, &[0x9Au8; 64], "funds", network);

        // One identity that decodes, and one genuinely corrupt row beside it —
        // the row that used to hold the sentinel hostage forever.
        let deleted = [0x11u8; 32];
        {
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            identities::insert_identity(
                &conn,
                deleted,
                Some(identities::identity_blob(deleted)),
                true,
            );
            identities::insert_identity(&conn, [0x22u8; 32], Some(vec![0xFFu8; 8]), true);
        }

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        run(&ctx).await.expect("first launch");
        wait_for_dapi_refresh(&ctx).await;

        let deleted_id = Identifier::from(deleted);
        assert!(
            ctx.has_local_qualified_identity(&deleted_id)
                .expect("read identity store"),
            "precondition: the readable identity is imported by the discovery run",
        );
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 0,
                top_ups: 0,
            },
            "precondition: the corrupt row is reported to the user",
        );

        // The user deliberately removes the identity — the gesture whose entire
        // point is that those keys stop living in this install.
        ctx.delete_local_qualified_identity(&deleted_id)
            .expect("delete identity");
        assert!(
            !ctx.has_local_qualified_identity(&deleted_id)
                .expect("read identity store"),
            "precondition: the delete took effect",
        );

        // The next launch, with the corrupt row still sitting in `data.db`.
        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("second launch");

        assert!(
            !ctx.has_local_qualified_identity(&deleted_id)
                .expect("read identity store"),
            "a deleted identity must STAY deleted: re-importing it would restore the alias \
             the user cleared and re-write their legacy plaintext keys into the vault, on \
             every launch, with no banner to explain it and no way to stop it",
        );

        // Closing the retry door must not cost the user the notice about the row
        // that never decoded.
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 0,
                top_ups: 0,
            },
            "the unreadable row is still reported once the import itself is complete",
        );

        backend.shutdown().await;
    }

    /// The unreadable-identity warning is acknowledgeable, and durable until it
    /// is. Before, it was a sticky banner with no action button: the user could
    /// not retire it, and the only thing keeping it on screen was an import that
    /// re-ran forever. The record now outlives the (once-only) import, is
    /// re-published on every launch, and only an explicit acknowledgement stops
    /// it — the legacy rows themselves are never touched.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_identity_warning_is_republished_until_acknowledged() {
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());
        let network = Network::Testnet;

        seed_legacy_wallet(&ctx, &[0xB4u8; 64], "funds", network);
        {
            let conn = Connection::open(tmp.path().join("data.db")).expect("open data.db");
            identities::insert_identity(&conn, [0x44u8; 32], Some(vec![0xFFu8; 8]), true);
        }

        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend wired");

        run(&ctx).await.expect("first launch");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 0,
                top_ups: 0,
            },
            "the discovery run surfaces the warning",
        );

        assert!(
            ctx.app_kv()
                .get::<MigrationCompletion>(DetScope::Global, &identities_sentinel_key_for(network))
                .expect("read identity sentinel")
                .is_some(),
            "the import completes even with an unreadable row: a withheld sentinel would \
             re-run it on every launch and resurrect identities the user has deleted",
        );

        // The import is done, so the pass reports zero from here on — the warning
        // has to come from storage or the user never hears it again.
        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("second launch");
        assert_eq!(
            *ctx.migration_status().state(),
            MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 0,
                top_ups: 0,
            },
            "a warning the user may have missed must survive a restart",
        );

        acknowledge_unreadable_identities(&ctx).expect("acknowledge");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "acknowledging clears the banner immediately",
        );

        ctx.migration_status().set_state(MigrationState::Idle);
        run(&ctx).await.expect("third launch");
        assert!(
            matches!(*ctx.migration_status().state(), MigrationState::Ready),
            "an acknowledged warning must never come back",
        );

        backend.shutdown().await;
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

    /// A failed migration must always leave a terminal state behind. `Running`
    /// makes `run_backend_task` reject every wallet-touching task with
    /// `WalletStorageNotReady`, and the banner offers a retry only from
    /// `Failed` — so a failure that published nothing would wedge wallets,
    /// identities and sends until the app is restarted, with no way out.
    #[tokio::test]
    async fn a_failed_migration_never_strands_the_status_on_running() {
        use crate::backend_task::migration::MigrationTask;
        use dash_sdk::dpp::dashcore::Network;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = fresh_app_context(tmp.path());

        // Same shape as the abort above: a legacy row trips detection, and the
        // unwired backend fails the first backend-dependent step.
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

        let result = ctx.run_migration_task(MigrationTask::FinishUnwire).await;
        assert!(result.is_err(), "the migration must report its failure");

        let state = ctx.migration_status().state();
        assert!(
            !state.is_executing(),
            "a failed migration left the status on `Running`, which gates every \
             wallet-touching task with no retry to escape it",
        );
        assert!(
            matches!(*state, MigrationState::Failed { .. }),
            "the failure must reach the user as a retryable banner",
        );
    }
}
