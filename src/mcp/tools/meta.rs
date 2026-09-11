//! Meta MCP tools: `tool_describe` and `app_storage_status`.

use std::borrow::Cow;
use std::path::Path;
use std::time::Duration;

use dash_sdk::dpp::dashcore::Network;
use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

use crate::backend_task::migration::MigrationError;
use crate::backend_task::migration::finish_unwire::{
    MigrationCompletion, app_data_sentinel_key_for, dapi_refresh_sentinel_key_for,
    identities_sentinel_key_for, sentinel_key_for,
};
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::database::DEFAULT_DB_VERSION;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::{DashMcpService, network_display_name};
use crate::mcp::tools::{NetworkParams, ToolNameParams};
use crate::wallet_backend::{DetKv, DetScope};

/// Return the full tool definition (schema, annotations, description) for a tool.
pub struct DescribeTool;

/// Wrapper for serializing an rmcp `Tool` as JSON output.
///
/// We use `serde_json::Value` because rmcp's `Tool` does not implement `JsonSchema`.
/// The `transform` override emits `"type": "object"` instead of bare `true`,
/// which some MCP clients (e.g. Claude Code) reject during schema validation.
#[derive(Serialize, schemars::JsonSchema)]
pub struct DescribeToolOutput {
    #[schemars(transform = tool_field_to_object)]
    tool: serde_json::Value,
}

fn tool_field_to_object(schema: &mut schemars::Schema) {
    *schema =
        serde_json::from_value(serde_json::json!({ "type": "object" })).expect("static schema");
}

impl ToolBase for DescribeTool {
    type Parameter = ToolNameParams;
    type Output = DescribeToolOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "tool_describe".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Return the full MCP tool definition (schema, annotations, description) \
             for a given tool name."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true))
    }
}

impl AsyncTool<DashMcpService> for DescribeTool {
    async fn invoke(
        service: &DashMcpService,
        param: ToolNameParams,
    ) -> Result<DescribeToolOutput, McpToolError> {
        // Deliberately: tool_describe uses meta naming rather than domain_object_action
        // convention — it's a meta-tool that describes other tools, not a domain operation.
        let tool_def =
            service
                .tool_router
                .get(&param.name)
                .ok_or_else(|| McpToolError::InvalidParam {
                    message: format!("Tool '{}' not found", param.name),
                })?;
        let value = serde_json::to_value(tool_def)
            .map_err(|e| McpToolError::Internal(format!("serialization: {e}")))?;
        Ok(DescribeToolOutput { tool: value })
    }
}

// ---------------------------------------------------------------------------
// AppStorageStatus
// ---------------------------------------------------------------------------

/// Every network whose upgrade markers live in the cross-network app store.
const ALL_NETWORKS: [Network; 4] = [
    Network::Mainnet,
    Network::Testnet,
    Network::Devnet,
    Network::Regtest,
];

/// The persister keeps `det-app.sqlite` open, so wait for a concurrent write
/// rather than reporting a false "no schema history".
const SCHEMA_HISTORY_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Report the on-disk storage state that an upgrade is expected to change.
pub struct AppStorageStatus;

/// One completion marker as the migration wrote it.
#[derive(Serialize, schemars::JsonSchema)]
pub struct SentinelRecord {
    /// Unix-epoch seconds at which the pass completed.
    completed_at: i64,
    /// Version of the build that wrote the marker.
    sha: String,
    network_count: u32,
}

/// The four independent upgrade passes, per network. `None` means the pass has
/// not completed for that network — it will run (or re-run) on the next launch.
#[derive(Serialize, schemars::JsonSchema)]
pub struct NetworkSentinels {
    network: String,
    wallet_drain: Option<SentinelRecord>,
    app_data: Option<SentinelRecord>,
    identities: Option<SentinelRecord>,
    dapi_refresh: Option<SentinelRecord>,
}

/// Legacy rows a completed upgrade could not decode.
#[derive(Serialize, schemars::JsonSchema)]
pub struct UnreadableCounts {
    identities: u32,
    votes: u32,
    top_ups: u32,
}

/// The live migration state machine, flattened for a JSON client.
#[derive(Serialize, schemars::JsonSchema)]
pub struct MigrationSummary {
    /// Machine-readable state name, e.g. `idle`, `running`, `success`, `failed`.
    state: String,
    /// Which pass is executing, set only while `state` is `running`.
    step: Option<String>,
    /// Failure text, set only for the two failed states.
    error: Option<String>,
    unreadable: Option<UnreadableCounts>,
}

/// Newest migration applied to the upstream wallet-storage schema.
#[derive(Serialize, schemars::JsonSchema)]
pub struct WalletStorageLineage {
    version: i64,
    name: String,
    checksum: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct AppStorageStatusOutput {
    /// Version of the running build.
    app_version: String,
    active_network: String,
    /// `data.db` schema version as stored; `null` when the settings table is absent.
    data_db_version: Option<i64>,
    /// Version this build's schema ladder brings `data.db` to.
    data_db_expected_version: u16,
    data_db_up_to_date: bool,
    /// Identifies which upstream schema line the profile is on. `null` when the
    /// app store has no migration history yet — a fresh profile, or one written
    /// by a build that predates the upstream store.
    wallet_storage_lineage: Option<WalletStorageLineage>,
    /// In-memory state of the process answering this call, so a one-shot CLI
    /// invocation reports `idle`. The durable record is `sentinels`.
    migration: MigrationSummary,
    sentinels: Vec<NetworkSentinels>,
}

impl ToolBase for AppStorageStatus {
    type Parameter = NetworkParams;
    type Output = AppStorageStatusOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "app_storage_status".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Report the local storage state: the app database schema version against \
             the version this build expects, the upstream wallet-storage schema lineage, \
             the live migration state, and each network's upgrade completion markers. \
             Diagnostic only — it reads on-disk state without starting or advancing an upgrade."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(false))
    }
}

impl AsyncTool<DashMcpService> for AppStorageStatus {
    async fn invoke(
        service: &DashMcpService,
        param: NetworkParams,
    ) -> Result<AppStorageStatusOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;
        resolve::verify_network(&ctx, param.network.as_deref())?;
        // Deliberately no `ensure_wallets_hydrated` / `ensure_spv_synced`: this
        // tool must observe the upgrade, not trigger it. Everything it reads is
        // reachable straight off `AppContext`.

        let data_db_version = ctx
            .db
            .stored_data_version()
            .map_err(|e| McpToolError::Internal(format!("read data.db version: {e}")))?;

        let app_kv = ctx.app_kv();
        let mut sentinels = Vec::with_capacity(ALL_NETWORKS.len());
        for network in ALL_NETWORKS {
            sentinels.push(NetworkSentinels {
                network: network_display_name(network).to_owned(),
                wallet_drain: read_sentinel(&app_kv, &sentinel_key_for(network))?,
                app_data: read_sentinel(&app_kv, &app_data_sentinel_key_for(network))?,
                identities: read_sentinel(&app_kv, &identities_sentinel_key_for(network))?,
                dapi_refresh: read_sentinel(&app_kv, &dapi_refresh_sentinel_key_for(network))?,
            });
        }

        Ok(AppStorageStatusOutput {
            app_version: crate::VERSION.to_owned(),
            active_network: network_display_name(ctx.network()).to_owned(),
            data_db_version,
            data_db_expected_version: DEFAULT_DB_VERSION,
            data_db_up_to_date: data_db_version == Some(i64::from(DEFAULT_DB_VERSION)),
            wallet_storage_lineage: wallet_storage_lineage(ctx.data_dir()),
            migration: summarize_migration(&ctx.migration_status().state()),
            sentinels,
        })
    }
}

/// Read one completion marker through the same k/v access the upgrade uses.
fn read_sentinel(app_kv: &DetKv, key: &str) -> Result<Option<SentinelRecord>, McpToolError> {
    app_kv
        .get::<MigrationCompletion>(DetScope::Global, key)
        .map(|found| {
            found.map(|completion| SentinelRecord {
                completed_at: completion.completed_at,
                sha: completion.sha,
                network_count: completion.network_count,
            })
        })
        .map_err(|source| McpToolError::TaskFailed(MigrationError::Sentinel { source }.into()))
}

/// Newest applied upstream migration in `det-app.sqlite`, or `None` when the
/// profile has no migration history to report.
///
/// Best-effort by design: a profile the upstream store has never opened has no
/// history table at all, which is an answer rather than a failure — a
/// diagnostic tool that refused to report anything else in that case would be
/// useless on exactly the profiles worth diagnosing.
fn wallet_storage_lineage(data_dir: &Path) -> Option<WalletStorageLineage> {
    match read_wallet_storage_lineage(data_dir) {
        Ok(lineage) => lineage,
        Err(error) => {
            tracing::debug!(%error, "No upstream schema history to report for det-app.sqlite");
            None
        }
    }
}

fn read_wallet_storage_lineage(data_dir: &Path) -> rusqlite::Result<Option<WalletStorageLineage>> {
    let conn = Connection::open_with_flags(
        data_dir.join("det-app.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    conn.busy_timeout(SCHEMA_HISTORY_BUSY_TIMEOUT)?;
    conn.query_row(
        "SELECT version, name, checksum FROM refinery_schema_history \
         ORDER BY version DESC LIMIT 1",
        [],
        |row| {
            Ok(WalletStorageLineage {
                version: row.get(0)?,
                name: row.get(1)?,
                checksum: row.get(2)?,
            })
        },
    )
    .optional()
}

/// Flatten the migration state machine. Matched exhaustively so a new state or
/// step fails the build here instead of silently reporting the wrong thing.
fn summarize_migration(state: &MigrationState) -> MigrationSummary {
    let summary = |state: &str| MigrationSummary {
        state: state.to_owned(),
        step: None,
        error: None,
        unreadable: None,
    };

    match state {
        MigrationState::Idle => summary("idle"),
        MigrationState::Ready => summary("ready"),
        MigrationState::Running { step } => MigrationSummary {
            step: Some(step_name(*step).to_owned()),
            ..summary("running")
        },
        MigrationState::AwaitingWalletPasswords { .. } => summary("awaiting_wallet_passwords"),
        MigrationState::Success => summary("success"),
        MigrationState::SucceededWithUnreadableData {
            identities,
            votes,
            top_ups,
        } => MigrationSummary {
            unreadable: Some(UnreadableCounts {
                identities: *identities,
                votes: *votes,
                top_ups: *top_ups,
            }),
            ..summary("succeeded_with_unreadable_data")
        },
        MigrationState::FailedWithUnreadableIdentities { count, error } => MigrationSummary {
            error: Some(error.to_string()),
            unreadable: Some(UnreadableCounts {
                identities: *count,
                votes: 0,
                top_ups: 0,
            }),
            ..summary("failed_with_unreadable_identities")
        },
        MigrationState::Failed { error } => MigrationSummary {
            error: Some(error.to_string()),
            ..summary("failed")
        },
    }
}

fn step_name(step: MigrationStep) -> &'static str {
    match step {
        MigrationStep::Wiring => "wiring",
        MigrationStep::Detecting => "detecting",
        MigrationStep::AppData => "app_data",
        MigrationStep::SingleKey => "single_key",
        MigrationStep::Shielded => "shielded",
        MigrationStep::WalletSeeds => "wallet_seeds",
        MigrationStep::WalletMeta => "wallet_meta",
        MigrationStep::Identities => "identities",
        MigrationStep::Finalize => "finalize",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn failure() -> Arc<MigrationError> {
        Arc::new(MigrationError::WalletBackendUnavailable)
    }

    /// Every step needs its own name, or a caller reading `step` cannot tell two
    /// passes apart.
    #[test]
    fn every_migration_step_has_a_distinct_name() {
        let names: BTreeSet<&str> = MigrationStep::ALL.iter().map(|s| step_name(*s)).collect();

        assert_eq!(names.len(), MigrationStep::ALL.len());
        assert!(names.iter().all(|name| !name.is_empty()));
    }

    /// An executing upgrade names the pass it is on; a terminal one has no pass.
    #[test]
    fn running_reports_its_step_and_a_terminal_state_does_not() {
        let running = summarize_migration(&MigrationState::Running {
            step: MigrationStep::WalletSeeds,
        });
        assert_eq!(running.state, "running");
        assert_eq!(running.step.as_deref(), Some("wallet_seeds"));
        assert!(running.error.is_none());

        let success = summarize_migration(&MigrationState::Success);
        assert_eq!(success.state, "success");
        assert!(success.step.is_none());
        assert!(success.unreadable.is_none());
    }

    /// Failure states carry the typed error's own text — never a message this
    /// tool invents — and the partial one also reports the rows left behind.
    #[test]
    fn failure_states_carry_error_text_and_unreadable_counts() {
        let expected = failure().to_string();

        let failed = summarize_migration(&MigrationState::Failed { error: failure() });
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.error.as_deref(), Some(expected.as_str()));
        assert!(failed.unreadable.is_none());

        let partial = summarize_migration(&MigrationState::FailedWithUnreadableIdentities {
            count: 3,
            error: failure(),
        });
        assert_eq!(partial.state, "failed_with_unreadable_identities");
        assert_eq!(partial.error.as_deref(), Some(expected.as_str()));
        assert_eq!(partial.unreadable.expect("counts").identities, 3);
    }

    /// A completed-with-damage upgrade reports each counter separately, so a
    /// reader can tell which kind of row was lost.
    #[test]
    fn succeeded_with_unreadable_data_reports_every_counter() {
        let summary = summarize_migration(&MigrationState::SucceededWithUnreadableData {
            identities: 1,
            votes: 2,
            top_ups: 3,
        });

        assert_eq!(summary.state, "succeeded_with_unreadable_data");
        assert!(summary.error.is_none());
        let counts = summary.unreadable.expect("counts");
        assert_eq!((counts.identities, counts.votes, counts.top_ups), (1, 2, 3));
    }

    /// A profile with no app store is a legitimate "nothing applied yet", not a
    /// failure — the lineage read must report `None` rather than propagate.
    #[test]
    fn missing_app_store_reports_no_lineage() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(wallet_storage_lineage(dir.path()).is_none());
    }
}
