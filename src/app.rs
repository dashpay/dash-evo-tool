mod reconcilers;
use reconcilers::{
    AccessibilityActivator, ConnectionBanner, MigrationReconciler, SpvBlockReconciler,
};

#[cfg(not(feature = "testing"))]
use crate::app_dir::data_file_path;
#[cfg(feature = "testing")]
use crate::app_dir::{app_user_data_dir_path, ensure_data_dir_exists, ensure_env_file};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::connection_status::{ConnectionStatus, OverallConnectionState};
use crate::context::feature_gate::FeatureGate;
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::database::Database;
use crate::model::settings::AppSettings;
use crate::ui::components::passphrase_modal;
use crate::ui::components::secret_prompt_host::{ActivePrompt, EguiSecretPromptHost, QueuedPrompt};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt, ProgressOverlay};
use crate::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen;
use crate::ui::dashpay::{DashPayScreen, DashPaySubscreen, ProfileSearchScreen};
use crate::ui::dpns::dpns_contested_names_screen::{DPNSScreen, DPNSSubscreen};
use crate::ui::identities::identities_screen::IdentitiesScreen;
use crate::ui::network_chooser_screen::{NetworkChooserScreen, chooser_network_label};
use crate::ui::theme::ThemeMode;
use crate::ui::tokens::tokens_screen::{TokensScreen, TokensSubscreen};
use crate::ui::tools::address_balance_screen::AddressBalanceScreen;
use crate::ui::tools::contract_visualizer_screen::ContractVisualizerScreen;
use crate::ui::tools::document_visualizer_screen::DocumentVisualizerScreen;
use crate::ui::tools::grovestark_screen::GroveSTARKScreen;
use crate::ui::tools::platform_info_screen::PlatformInfoScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::tools::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::wallets::wallets_screen::WalletsBalancesScreen;
use crate::ui::welcome_screen::WelcomeScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use crate::utils::egui_mpsc::{self, EguiMpscAsync};
use crate::utils::tasks::{TaskManager, TaskShutdownOutcome};
use crate::wallet_backend::{DetScope, WalletBackend};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Identifier;
use eframe::{App, egui};
use platform_wallet_storage::secrets::SecretStore;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::BitOrAssign;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::vec;
use tokio::sync::mpsc as tokiompsc;

/// Banner action id pushed when the user clicks "Retry now" on the
/// migration-failure banner. The app loop matches this id and
/// re-dispatches the FinishUnwire backend task. Kept as a `const` so a
/// future second migration variant can pick a distinct id without
/// risking a typo collision. Exposed for kittest coverage.
pub const MIGRATION_RETRY_ACTION_ID: &str = "migration:retry:finish_unwire";

/// Banner action id pushed when the user acknowledges the unreadable-vote
/// warning. Until it fires, the warning is re-raised on every launch — a
/// dismissed banner is not an acknowledgement, because the vote it names may
/// still have a live deadline. Exposed for kittest coverage.
pub const MIGRATION_VOTES_ACK_ACTION_ID: &str = "migration:ack:unreadable_votes";

/// Banner action id pushed when the user acknowledges the unreadable-identity
/// warning. Until it fires, the warning is re-raised on every launch — a
/// dismissed banner is not an acknowledgement, because the identities it names
/// hold keys the user cannot sign with until they are loaded again. Exposed for
/// kittest coverage.
pub const MIGRATION_IDENTITIES_ACK_ACTION_ID: &str = "migration:ack:unreadable_identities";

/// Banner action id pushed when the user acknowledges the combined warning — the
/// launch where both unreadable identities and unreadable votes were left behind.
/// One banner names both problems, so its single acknowledgement retires both
/// records: re-raising either half after the user has read and dismissed the
/// sentence describing it would be a notice they have already acted on. Exposed
/// for kittest coverage.
pub const MIGRATION_UNREADABLE_ACK_ACTION_ID: &str =
    "migration:ack:unreadable_identities_and_votes";

const WALLET_BACKEND_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_DEADLINE_MARGIN: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownOutcome {
    Complete,
    TaskManagerFailed,
    BackendTasksTimedOut,
    WalletBackendTimedOut,
}

fn shutdown_hard_deadline() -> Duration {
    TaskManager::graceful_shutdown_budget()
        + WALLET_BACKEND_SHUTDOWN_TIMEOUT
        + SHUTDOWN_DEADLINE_MARGIN
}

trait ShutdownWalletBackend: Send + Sync {
    fn forget_all_secrets(&self);
    fn shutdown(&self) -> futures::future::BoxFuture<'_, ()>;
}

impl ShutdownWalletBackend for WalletBackend {
    fn forget_all_secrets(&self) {
        WalletBackend::forget_all_secrets(self);
    }

    fn shutdown(&self) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(WalletBackend::shutdown(self))
    }
}

fn migration_allows_scheduled_vote_sweep(state: &MigrationState) -> bool {
    matches!(
        state,
        MigrationState::Ready
            | MigrationState::Success
            | MigrationState::SucceededWithUnreadableData { .. }
    )
}

pub(crate) fn scheduled_vote_sweep_is_quiet(error: &TaskError) -> bool {
    matches!(
        error,
        TaskError::ScheduledVoteSweepFailed { source, .. }
            if matches!(source.as_ref(), TaskError::NoVotingIdentity { .. })
    )
}

const LEGACY_SETTINGS_IMPORT_WARNING: &str = "The app could not confirm that your network preference was restored from the previous version. Check the selected network before using the application.";

fn show_legacy_settings_import_warning(ctx: &egui::Context, error: &impl std::fmt::Debug) {
    let handle =
        MessageBanner::set_global(ctx, LEGACY_SETTINGS_IMPORT_WARNING, MessageType::Warning);
    handle.disable_auto_dismiss();
    handle.with_details(error);
}

fn legacy_settings_import_requires_network_selection(
    _error: &crate::backend_task::migration::legacy_settings::SettingsImportError,
) -> bool {
    true
}

fn initial_root_screen(
    persisted: RootScreenType,
    persisted_is_registered: bool,
    network_selection_required: bool,
) -> RootScreenType {
    if network_selection_required {
        RootScreenType::RootScreenNetworkChooser
    } else if persisted_is_registered {
        persisted
    } else {
        FALLBACK_ROOT_SCREEN
    }
}

fn show_welcome_screen(onboarding_completed: bool, network_selection_required: bool) -> bool {
    !onboarding_completed && !network_selection_required
}

fn network_selection_allows_root(
    network_selection_required: bool,
    root_screen: RootScreenType,
) -> bool {
    !network_selection_required || root_screen == RootScreenType::RootScreenNetworkChooser
}

fn network_selection_allows_action(network_selection_required: bool, action: &AppAction) -> bool {
    !network_selection_required || matches!(action, AppAction::None | AppAction::SwitchNetwork(_))
}

fn boot_auto_start_spv(
    onboarding_completed: bool,
    auto_start_spv: bool,
    network_selection_required: bool,
) -> bool {
    onboarding_completed && auto_start_spv && !network_selection_required
}

fn clear_scheduled_vote_sweep_guard_on_error(
    in_progress: &mut BTreeSet<Network>,
    context: &BackendTaskContext,
    error: &TaskError,
) {
    let network = match (context, error) {
        (_, TaskError::ScheduledVoteSweepFailed { network, .. }) => Some(*network),
        (_, TaskError::ScheduledVoteSweepAllAddressesExhausted { network, .. }) => Some(*network),
        (
            BackendTaskContext::ScheduledVoteSweep { network },
            TaskError::BackendTaskFailed { .. },
        ) => Some(*network),
        _ => None,
    };
    if let Some(network) = network {
        in_progress.remove(&network);
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn clear_confirmed_vote_recovery_cutoff(
    deferred: &mut BTreeMap<Network, u64>,
    network: Network,
    confirmed_cutoff: Option<u64>,
) -> bool {
    if confirmed_cutoff.is_some() && deferred.get(&network).copied() == confirmed_cutoff {
        deferred.remove(&network);
        true
    } else {
        false
    }
}

/// Action id for the SPV-sync block's "Continue in the background" escape button.
/// SPV sync is **unbounded** — with no peers it stays Connecting/Syncing forever
/// with no terminal signal — so a button-less hard block would trap the user
/// (violating the overlay's C1/C2 contract). This escape lowers the block while
/// sync continues safely in the background — a read-only operation that strands
/// nothing if backgrounded. It is also designated the block's single
/// keyboard-reachable escape (`with_keyboard_escape`), so a keyboard-only /
/// assistive-tech user can activate it with Enter or Space.
/// Colon-namespaced per the overlay action-id convention. Exposed for kittest
/// coverage.
pub const SPV_CONTINUE_BACKGROUND_ACTION: &str = "spv:sync:continue_background";

/// The root screen every fallback route lands on: an unregistered persisted
/// screen at startup, and live de-gating of a role-gated tab.
///
/// It must be a screen the left nav actually carries, and carries at every role
/// — a user dropped onto a screen with no nav entry has no highlighted tab and
/// no way onward. `left_panel::tests::fallback_root_screen_has_an_ungated_nav_entry`
/// locks that invariant.
pub(crate) const FALLBACK_ROOT_SCREEN: RootScreenType = RootScreenType::RootScreenIdentityHub;

fn identity_hub_is_visible(selected: RootScreenType, screen_stack_is_empty: bool) -> bool {
    selected == RootScreenType::RootScreenIdentityHub && screen_stack_is_empty
}

/// Plain, jargon-free descriptions for the SPV-sync block (Everyday-User rule:
/// no "SPV"/"headers"/"masternodes"/raw heights/percentages — the jargon-free
/// "Step N of 5" counter carries the granularity). Complete sentences (NFR-2).
const SPV_CONNECTING_DESCRIPTION: &str = "Connecting to the Dash network.";
const SPV_SYNCING_DESCRIPTION: &str = "Syncing with the Dash network.";

/// What the per-frame SPV-sync block driver should do with the overlay this
/// frame, given whether a startup/Connect sync is **armed**, whether the user
/// chose to continue in the background, and the current connection state. Pure so
/// the policy is unit-testable in isolation from `AppState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpvBlockStep {
    /// Armed, not dismissed, still connecting/syncing: raise (or keep + update).
    Block,
    /// Armed episode reached a terminal state (Synced/Error): lower the block and
    /// DISARM, so subsequent ambient Connecting/Syncing (reconnect, per-block
    /// catch-up) never re-blocks (F-SPV-A).
    Disarm,
    /// Armed but the user chose to continue in the background: keep the block
    /// lowered without ending the episode (C2 escape).
    Stand,
    /// Not armed (ambient sync, or already disarmed): ensure no block is shown.
    Idle,
}

#[cfg(test)]
mod backend_task_join_tests {
    use super::*;
    use crate::backend_task::BackendTaskContext;
    use crate::backend_task::tokens::TokenTask;
    use crate::utils::egui_mpsc::SenderAsync;

    #[test]
    fn backend_task_error_retains_originating_context() {
        let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));

        let result = TaskResult::from_backend_task_result(
            BackendTaskContext::from(&task),
            Err(TaskError::NoIdentitiesFound),
        );

        let TaskResult::Error {
            context,
            error: TaskError::NoIdentitiesFound,
        } = result
        else {
            panic!("expected an attributed backend-task error");
        };
        assert_eq!(context, BackendTaskContext::TokenBalanceRefresh);
        assert_eq!(
            BackendTaskContext::from(&BackendTask::None),
            BackendTaskContext::Other
        );
    }

    #[test]
    fn backend_task_success_retains_originating_context() {
        let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));

        let result = TaskResult::from_backend_task_result(
            BackendTaskContext::from(&task),
            Ok(BackendTaskSuccessResult::FetchedTokenBalances),
        );

        let TaskResult::Success { context, result } = result else {
            panic!("expected an attributed backend-task success");
        };
        assert_eq!(context, BackendTaskContext::TokenBalanceRefresh);
        assert!(matches!(
            *result,
            BackendTaskSuccessResult::FetchedTokenBalances
        ));
    }

    #[test]
    fn unattributed_error_has_unknown_context() {
        let result = TaskResult::unattributed_error(TaskError::NoIdentitiesFound);

        let TaskResult::Error { context, .. } = result else {
            panic!("expected an unattributed task error");
        };
        assert_eq!(context, BackendTaskContext::Unknown);
    }

    #[tokio::test]
    async fn panicking_backend_task_is_forwarded_as_typed_error() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sender = SenderAsync::new(tx, egui::Context::default());
        let join_handle = tokio::task::spawn_blocking(|| panic!("backend task panic"));

        forward_backend_task_join_error(
            join_handle.await,
            sender,
            None,
            BackendTaskContext::Unknown,
        )
        .await;

        let result = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("join failure must be reported promptly")
            .expect("join failure result must be sent");
        let TaskResult::Error {
            error: error @ TaskError::BackendTaskFailed { .. },
            ..
        } = result
        else {
            panic!("expected typed backend task failure, got {result:?}");
        };
        assert!(
            !format!("{error:?}").contains("backend task panic"),
            "panic payload must be redacted from diagnostics"
        );
    }

    #[tokio::test]
    async fn panicking_scheduled_vote_sweep_clears_in_progress_guard() {
        let network = Network::Testnet;
        let unrelated_network = Network::Regtest;
        let mut in_progress = BTreeSet::from([network, unrelated_network]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sender = SenderAsync::new(tx, egui::Context::default());
        let join_handle = tokio::task::spawn_blocking(|| panic!("scheduled sweep panic"));

        forward_backend_task_join_error(
            join_handle.await,
            sender,
            None,
            BackendTaskContext::ScheduledVoteSweep { network },
        )
        .await;

        let TaskResult::Error { context, error } = rx
            .recv()
            .await
            .expect("join failure result must be forwarded")
        else {
            panic!("expected a scheduled-vote sweep error");
        };
        assert!(matches!(error, TaskError::BackendTaskFailed { .. }));

        clear_scheduled_vote_sweep_guard_on_error(&mut in_progress, &context, &error);

        assert!(
            !in_progress.contains(&network),
            "a terminal panic must allow the next scheduled-vote sweep"
        );
        assert!(
            in_progress.contains(&unrelated_network),
            "a terminal panic must not release another network's sweep guard"
        );
    }

    #[tokio::test]
    async fn panicking_paid_contact_action_keeps_its_request_correlation() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sender = SenderAsync::new(tx, egui::Context::default());
        let request_id = Identifier::from([0x44; 32]);
        let join_handle = tokio::task::spawn_blocking(|| panic!("backend task panic"));

        forward_backend_task_join_error(
            join_handle.await,
            sender,
            Some(request_id),
            BackendTaskContext::Unknown,
        )
        .await;

        let result = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("join failure must be reported promptly")
            .expect("join failure result must be sent");
        assert!(matches!(
            result,
            TaskResult::Error {
                error: TaskError::DashPayContactRequestActionFailed {
                    request_id: correlated,
                    source,
                },
                ..
            } if correlated == request_id
                && matches!(source.as_ref(), TaskError::BackendTaskFailed { .. })
        ));
    }

    #[test]
    fn only_missing_local_identity_sweep_errors_are_quiet() {
        let missing_identity = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::NoVotingIdentity {
                identity_id: "voter-id".to_string(),
            }),
        };
        assert!(scheduled_vote_sweep_is_quiet(&missing_identity));

        let unavailable_result = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::ScheduledVoteResultUnavailable),
        };
        assert!(!scheduled_vote_sweep_is_quiet(&unavailable_result));

        let nested_sweep = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::ScheduledVoteSweepFailed {
                network: Network::Regtest,
                source: Box::new(TaskError::NoVotingIdentity {
                    identity_id: "voter-id".to_string(),
                }),
            }),
        };
        assert!(!scheduled_vote_sweep_is_quiet(&nested_sweep));
        assert!(!scheduled_vote_sweep_is_quiet(
            &TaskError::NoVotingIdentity {
                identity_id: "voter-id".to_string(),
            }
        ));
    }
}

/// Pure SPV-sync block policy (F-SPV-A scope gate + C1/C2). The block is **scoped
/// to user-initiated sync** — armed only on startup auto-start and the Connect
/// button — so an ambient reconnect or the SPV engine flipping Synced→Syncing on
/// each new block never hard-blocks a working user. Once an armed episode reaches
/// a terminal state it disarms and stays disarmed until the next Connect/startup.
fn spv_block_step(armed: bool, dismissed: bool, state: OverallConnectionState) -> SpvBlockStep {
    use OverallConnectionState as S;
    if !armed {
        return SpvBlockStep::Idle;
    }
    match state {
        // Terminal for an armed episode: lower and disarm (banner surfaces Error).
        S::Synced | S::Error => SpvBlockStep::Disarm,
        // Still getting connected/synced for this episode: block unless the user
        // is waiting in the background. Disconnected stays blocking while armed —
        // it just means we are still trying to connect.
        S::Connecting | S::Syncing | S::Disconnected => {
            if dismissed {
                SpvBlockStep::Stand
            } else {
                SpvBlockStep::Block
            }
        }
    }
}

/// One-sentence user-facing label for an in-progress migration step.
/// Mirrors Diziet §2.2 D-1 banner copy — single complete sentence per
/// variant so i18n extraction is trivial. Exposed for kittest
/// coverage so a regression in the label table fails the test suite.
pub fn migration_running_text(step: MigrationStep) -> &'static str {
    match step {
        MigrationStep::Detecting => "The app is checking your wallet data.",
        MigrationStep::AppData => "The app is restoring your scheduled votes.",
        MigrationStep::SingleKey => "The app is updating your imported keys.",
        MigrationStep::Shielded => "The app is verifying your shielded balance.",
        MigrationStep::WalletSeeds => "The app is moving your wallets into secure storage.",
        MigrationStep::WalletMeta => "The app is updating your wallet names.",
        MigrationStep::Identities => "The app is restoring your identities and their keys.",
        MigrationStep::Finalize => "The app is finishing the storage update.",
    }
}

/// User-facing banner copy for a migration that finished the wallet drain but
/// left `count` undecodable scheduled votes behind. The votes stay in the
/// previous version's storage (nothing is deleted), but they will not be cast,
/// so the sentence names the one action that recovers them. No "Retry now" —
/// a corrupt row decodes no better on a second pass. Exposed for kittest
/// coverage.
pub fn migration_unreadable_votes_text(count: u32) -> String {
    format!(
        "Some scheduled votes from the previous version could not be read and were not carried \
         over ({count} in total). Schedule them again on the Scheduled Votes screen."
    )
}

/// User-facing banner copy for a migration that finished the wallet drain but
/// could not decode `count` identities. Their keys are therefore not loaded, so
/// the sentence names both the screen and the control that restore them — a user
/// who has never opened that flow cannot act on "load them again" alone. Kept
/// separate from the scheduled-votes copy because the remedy is different — load
/// an identity, not re-schedule a vote. The previous version's data is never
/// deleted, so the re-import is always possible. Exposed for kittest coverage.
pub fn migration_unreadable_identities_text(count: u32) -> String {
    format!(
        "Some identities from the previous version could not be read and were not carried over \
         ({count} in total). Your previous data is untouched. For a user identity, choose Load \
         Identity on the Identities screen. For a masternode or evonode identity, choose + Load \
         on the Masternodes tab."
    )
}

/// User-facing banner copy for the launch where both DET-owned passes left rows
/// behind: `identities` identities and `votes` scheduled votes could not be read.
/// One sentence per problem, each naming its own remedy — the remedies differ
/// (load an identity vs re-schedule a vote), and the identity warning recurs on
/// every launch, so it must never be the reason the deadline-critical vote notice
/// goes unseen. No "Retry now": neither corrupt row decodes better on a second
/// pass. Exposed for kittest coverage.
pub fn migration_unreadable_identities_and_votes_text(identities: u32, votes: u32) -> String {
    format!(
        "Some identities ({identities} in total) and some scheduled votes ({votes} in total) from \
         the previous version could not be read and were not carried over. Your previous data is \
         untouched. For a user identity, choose Load Identity on the Identities screen. For a \
         masternode or evonode identity, choose + Load on the Masternodes tab. Schedule the votes \
         again on the Scheduled Votes screen."
    )
}

/// User-facing banner copy for the rare launch where both DET-owned passes broke:
/// `count` identities could not be read AND updating the rest of the previous
/// version's data (such as scheduled votes) hit a hard error. Names each problem
/// in its own sentence and offers the retry the app-data half needs — the
/// identity half recovers by loading the identities again. The previous version's
/// data is never deleted, so both are recoverable. Exposed for kittest coverage.
pub fn migration_failed_with_unreadable_identities_text(count: u32) -> String {
    format!(
        "Some identities from the previous version could not be read and were not carried over \
         ({count} in total), and updating the rest of your previous data did not finish. Your \
         previous data is untouched. Choose Retry now to finish updating. For a user identity, \
         choose Load Identity on the Identities screen. For a masternode or evonode identity, \
         choose + Load on the Masternodes tab."
    )
}

/// User-facing banner copy for every non-empty combination of unreadable
/// legacy identity, scheduled-vote and top-up rows.
pub fn migration_unreadable_data_text(identities: u32, votes: u32, top_ups: u32) -> String {
    match (identities > 0, votes > 0, top_ups > 0) {
        (true, false, false) => migration_unreadable_identities_text(identities),
        (false, true, false) => migration_unreadable_votes_text(votes),
        (true, true, false) => {
            migration_unreadable_identities_and_votes_text(identities, votes)
        }
        (false, false, true) => format!(
            "Some records of earlier additions to identity balances from the previous version \
             could not be read and were not carried over ({top_ups} in total). Check each \
             identity's balance history before adding more funds."
        ),
        (true, false, true) => format!(
            "Some identities ({identities} in total) and some records of earlier additions to \
             identity balances ({top_ups} in total) from the previous version could not be read \
             and were not carried over. For a user identity, choose Load Identity on the \
             Identities screen. For a masternode or evonode identity, choose + Load on the \
             Masternodes tab. Check each identity's balance history before adding more funds."
        ),
        (false, true, true) => format!(
            "Some scheduled votes ({votes} in total) and some records of earlier additions to \
             identity balances ({top_ups} in total) from the previous version could not be read \
             and were not carried over. Schedule the votes again on the Scheduled Votes screen. \
             Check each identity's balance history before adding more funds."
        ),
        (true, true, true) => format!(
            "Some identities ({identities} in total), some scheduled votes ({votes} in total), \
             and some records of earlier additions to identity balances ({top_ups} in total) \
             from the previous version could not be read and were not carried over. For a user \
             identity, choose Load Identity on the Identities screen. For a masternode or \
             evonode identity, choose + Load on the Masternodes tab. Schedule the votes again on \
             the Scheduled Votes screen, and check each identity's balance history before adding \
             more funds."
        ),
        (false, false, false) => {
            "Some data from the previous version could not be read. Check your identities, scheduled votes, and identity balance history before continuing.".to_string()
        }
    }
}

/// How long the cold-start readiness gate waits for the wallet backend to wire
/// before it stops retrying silently and surfaces a visible, actionable banner.
///
/// Wiring is a local, non-network operation (open the SQLite sidecar, hydrate
/// wallets, bootstrap addresses) that normally completes within a few frames of
/// boot / a network switch — sub-second in the common case. 30 seconds is ~two
/// orders of magnitude past the expected completion, generous enough never to
/// false-positive on a slow disk or a large wallet set, yet short enough that a
/// genuinely wedged backend surfaces within half a minute instead of never. It
/// sits well below the network-bound waits (the 120 s SPV no-progress watchdog,
/// the 10 min MCP sync gate), matching that this wait is local, not on the wire.
const COLD_START_BACKEND_READY_TIMEOUT: Duration = Duration::from_secs(30);

/// User-facing banner shown when the wallet backend never finishes wiring within
/// [`COLD_START_BACKEND_READY_TIMEOUT`], so the cold-start migration can never
/// run. Everyday-User copy (no "backend"/"wiring"/"SPV" jargon): what happened +
/// a self-serviceable action. Complete sentences so i18n extracts it as one unit.
const COLD_START_STUCK_MESSAGE: &str =
    "We couldn't finish preparing your wallet. Try restarting the app.";

/// Decide whether to dispatch the cold-start migration for the active network
/// this frame: only when it has not already been dispatched AND its wallet
/// backend is wired. The migration's first step needs a wired backend; firing
/// before it is wired aborts with a transient `WalletBackendUnavailable` and
/// burns the per-network dispatch guard, so the readiness gate keeps the guard
/// pending and retries on a later frame once the backend wires.
fn should_dispatch_cold_start(already_dispatched: bool, backend_ready: bool) -> bool {
    !already_dispatched && backend_ready
}

/// Whether the readiness gate has been waiting on an unwired wallet backend long
/// enough to surface the stuck-preparation banner. Pure so the timeout is
/// unit-testable with synthetic durations. `waited == None` means we are not (or
/// no longer) waiting — that never times out.
fn cold_start_backend_wait_timed_out(waited: Option<Duration>, timeout: Duration) -> bool {
    waited.is_some_and(|elapsed| elapsed >= timeout)
}

#[derive(Debug)]
pub enum TaskResult {
    Repaint,
    Refresh,
    Success {
        context: BackendTaskContext,
        result: Box<BackendTaskSuccessResult>,
    },
    Error {
        context: BackendTaskContext,
        error: TaskError,
    },
}

impl TaskResult {
    fn from_backend_task_result(
        context: BackendTaskContext,
        value: Result<BackendTaskSuccessResult, TaskError>,
    ) -> Self {
        match value {
            Ok(value) => TaskResult::Success {
                context,
                result: Box::new(value),
            },
            Err(error) => TaskResult::Error { context, error },
        }
    }

    pub(crate) fn unattributed_success(result: BackendTaskSuccessResult) -> Self {
        Self::Success {
            context: BackendTaskContext::Unknown,
            result: Box::new(result),
        }
    }

    pub(crate) fn unattributed_error(error: TaskError) -> Self {
        Self::Error {
            context: BackendTaskContext::Unknown,
            error,
        }
    }
}

async fn forward_backend_task_join_error(
    join_result: Result<(), tokio::task::JoinError>,
    sender: egui_mpsc::SenderAsync<TaskResult>,
    request_id: Option<Identifier>,
    context: BackendTaskContext,
) {
    if let Err(source) = join_result {
        let stopped = TaskError::BackendTaskFailed {
            source: source.into(),
        };
        let error = match request_id {
            Some(request_id) => TaskError::DashPayContactRequestActionFailed {
                request_id,
                source: Box::new(stopped),
            },
            None => stopped,
        };
        if let Err(error) = sender.send(TaskResult::Error { context, error }).await {
            tracing::error!(%error, "Failed to report a stopped backend task");
        }
    }
}

struct ThemeState {
    preference: ThemeMode,
    resolved: ThemeMode,
    last_applied: Option<ThemeMode>,
    last_checked: Instant,
}

impl ThemeState {
    fn new(preference: ThemeMode) -> Self {
        Self {
            resolved: crate::ui::theme::resolve_theme_mode(preference),
            last_applied: None,
            last_checked: Instant::now(),
            preference,
        }
    }

    /// Polls the OS for system theme changes (throttled to every 2s) and
    /// applies the theme if it changed. Returns `true` if the theme was applied.
    fn poll_and_apply(&mut self, ctx: &egui::Context) -> bool {
        if self.preference == ThemeMode::System {
            let now = Instant::now();
            if now.duration_since(self.last_checked) >= Duration::from_secs(2) {
                self.last_checked = now;
                if let Some(detected) = crate::ui::theme::try_detect_system_theme()
                    && detected != self.resolved
                {
                    self.resolved = detected;
                }
            }
        }
        if self.last_applied != Some(self.resolved) {
            crate::ui::theme::apply_theme(ctx, self.resolved);
            self.last_applied = Some(self.resolved);
            true
        } else {
            false
        }
    }

    fn apply_new_preference(&mut self, ctx: &egui::Context, new_theme: ThemeMode) -> bool {
        self.preference = new_theme;
        let mut detection_failed = false;
        self.resolved = if new_theme == ThemeMode::System {
            match crate::ui::theme::try_detect_system_theme() {
                Some(detected) => detected,
                None => {
                    detection_failed = true;
                    self.resolved
                }
            }
        } else {
            new_theme
        };
        self.last_checked = Instant::now();
        crate::ui::theme::apply_theme(ctx, self.resolved);
        self.last_applied = Some(self.resolved);
        detection_failed
    }
}

pub struct AppState {
    pub main_screens: BTreeMap<RootScreenType, Screen>,
    pub selected_main_screen: RootScreenType,
    pub screen_stack: Vec<Screen>,
    pub chosen_network: Network,
    pub connection_status: Arc<ConnectionStatus>,
    pub network_contexts: BTreeMap<Network, Arc<AppContext>>,
    /// Network whose context is being created asynchronously. While `Some`,
    /// the UI shows a progress banner and ignores further switch requests.
    network_switch_pending: Option<Network>,
    /// Progress banner displayed while a network switch is in progress.
    network_switch_banner: Option<BannerHandle>,
    /// Whether boot must remain on the network chooser until the user confirms a network.
    network_selection_required: bool,
    pub task_result_sender: egui_mpsc::SenderAsync<TaskResult>, // Channel sender for sending task results
    pub task_result_receiver: tokiompsc::Receiver<TaskResult>, // Channel receiver for receiving task results
    theme: ThemeState,
    last_scheduled_vote_check: Instant, // Last time we checked if there are scheduled masternode votes to cast
    /// Per-network start of a migration wait that deferred scheduled-vote casting.
    scheduled_vote_sweep_deferred_since_ms: BTreeMap<Network, u64>,
    /// Networks with a scheduled-vote sweep currently running.
    scheduled_vote_sweeps_in_progress: BTreeSet<Network>,
    /// Last recovery-sweep attempt per network, used to throttle retries while
    /// retaining the original eligibility cutoff.
    scheduled_vote_recovery_last_attempt: BTreeMap<Network, Instant>,
    last_repaint_request: Instant, // Throttle periodic repaint scheduling to once per second
    pub subtasks: Arc<TaskManager>, // Subtasks manager for graceful shutdown
    /// Whether to show the welcome/onboarding screen
    pub show_welcome_screen: bool,
    /// The welcome screen instance (only created if needed)
    pub welcome_screen: Option<WelcomeScreen>,
    /// Connection-status banner reconciler (state-transition driven).
    connection_banner: ConnectionBanner,
    /// Blocking SPV-sync overlay reconciler. Hard-blocks the UI while a
    /// **user-initiated** sync (startup auto-start / Connect) connects, until
    /// the chain becomes usable (Synced) or fails (Error), or the user chooses
    /// to continue in the background. Ambient reconnects are never armed, so
    /// they never hard-block a working user (F-SPV-A).
    spv_block: SpvBlockReconciler,
    /// Data-migration banner + cold-start `FinishUnwire` dispatch reconciler.
    migration: MigrationReconciler,
    /// Async shutdown receiver. `Some` until a graceful shutdown reaches a
    /// terminal state; the viewport is closed once the receiver resolves.
    shutdown_receiver: Option<tokio::sync::oneshot::Receiver<ShutdownOutcome>>,
    /// Timestamp when the async shutdown was initiated, used as a hard deadline
    /// to force-close the viewport if the shutdown task stalls.
    shutdown_started: Option<std::time::Instant>,
    /// True once every managed shutdown stage reached a terminal outcome.
    shutdown_finished: bool,
    /// Platform-level accessibility (AccessKit) activation reconciler.
    accessibility: AccessibilityActivator,
    /// Shared MCP context -- follows network switches via `ArcSwap`.
    #[cfg(feature = "mcp")]
    pub mcp_app_context: Option<Arc<arc_swap::ArcSwap<AppContext>>>,
    /// MCP configuration held until a required boot-time network selection succeeds.
    #[cfg(feature = "mcp")]
    mcp_server_pending_config: Option<crate::mcp::McpConfig>,
    /// Receives just-in-time passphrase requests enqueued by the egui secret
    /// prompt host. Drained once per frame in [`Self::update`]; the active
    /// request becomes [`Self::active_secret_prompt`].
    secret_prompt_receiver: tokiompsc::UnboundedReceiver<QueuedPrompt>,
    /// The passphrase prompt currently shown, if any. Exactly one is active at
    /// a time; further requests wait in `secret_prompt_receiver` (FIFO).
    active_secret_prompt: Option<ActivePrompt>,
    /// Whether a blocking passphrase prompt owned the previous frame. Drives the
    /// one-shot pointer-drop on the frame a prompt first becomes active — see
    /// [`passphrase_modal::drop_activation_frame_pointer_click`].
    prompt_was_blocking: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesiredAppAction {
    None,
    Refresh,
    AddScreenType(Box<ScreenType>),
    BackendTask(Box<BackendTask>),
    BackendTasks(Vec<BackendTask>, BackendTasksExecutionMode),
    Custom(String),
}

impl DesiredAppAction {
    pub fn create_action(&self, app_context: &Arc<AppContext>) -> AppAction {
        match self {
            DesiredAppAction::None => AppAction::None,
            DesiredAppAction::Refresh => AppAction::Refresh,
            DesiredAppAction::Custom(message) => AppAction::Custom(message.clone()),
            DesiredAppAction::AddScreenType(screen_type) => {
                AppAction::AddScreen(screen_type.create_screen(app_context))
            }
            DesiredAppAction::BackendTask(backend_task) => {
                AppAction::BackendTask((**backend_task).clone())
            }
            DesiredAppAction::BackendTasks(tasks, mode) => {
                AppAction::BackendTasks(tasks.clone(), mode.clone())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendTasksExecutionMode {
    Sequential,
    Concurrent,
}

#[derive(Debug, PartialEq)]
pub enum AppAction {
    None,
    Refresh,
    PopScreen,
    PopScreenAndRefresh,
    GoToMainScreen,
    SwitchNetwork(Network),
    SetMainScreen(RootScreenType),
    SetMainScreenThenPopScreen(RootScreenType),
    SetMainScreenThenGoToMainScreen(RootScreenType),
    AddScreen(Screen),
    PopThenAddScreenToMainScreen(RootScreenType, Screen),
    BackendTask(BackendTask),
    BackendTaskWithContext {
        task: BackendTask,
        context: BackendTaskContext,
    },
    BackendTasks(Vec<BackendTask>, BackendTasksExecutionMode),
    /// Wire the wallet backend (if needed) and start chain sync for the active
    /// context. Handled by the update loop, which owns the `TaskResult` sender
    /// the wiring step requires. Used by the manual Connect button so a click
    /// during the brief not-yet-wired window lazily wires-then-starts instead
    /// of silently fast-failing.
    StartSpv,
    /// Stop chain sync and unwire the wallet backend for the active context.
    /// Handled by the update loop because the teardown is async. Used by the
    /// manual Disconnect button; the next Connect rebuilds the backend.
    StopSpv,
    Custom(String),
    /// Mark onboarding as complete, hide welcome screen, and optionally navigate
    OnboardingComplete {
        /// The main screen to show
        main_screen: RootScreenType,
        /// Optional sub-screen to push onto the stack
        add_screen: Option<Box<crate::ui::ScreenType>>,
    },
    /// Switch the active sub-tab inside the Identity Hub root screen. Emitted
    /// by in-hub deep links (e.g. the Home tab's "See all activity" link, the
    /// Contacts gate's "Add a display name" CTA). Handled by `AppState::update`
    /// which looks up the visible `IdentityHubScreen` and calls `select_tab`.
    SwitchIdentityHubTab(crate::ui::identity::IdentityHubTab),
}

impl BitOrAssign for AppAction {
    fn bitor_assign(&mut self, rhs: Self) {
        if matches!(rhs, AppAction::None) {
            // If rhs is None, keep the current value.
            return;
        }

        // Otherwise, assign rhs to self.
        *self = rhs;
    }
}

/// Why the wallet backend is being wired, selecting the spawned task's label
/// and its log/banner wording. The single shape behind boot, network switch,
/// post-onboarding auto-start, and the manual Connect button (see
/// [`AppState::spawn_backend_init`]).
#[derive(Debug, Clone, Copy)]
enum BackendInitReason {
    /// Eager per-network wiring at process start.
    Boot,
    /// Eager wiring after a network switch.
    NetworkSwitch,
    /// Post-onboarding chain-sync opt-in.
    OnboardingAutoStart,
    /// The manual Connect button.
    ManualConnect,
}

impl BackendInitReason {
    /// Label for the spawned subtask.
    fn task_name(self) -> &'static str {
        match self {
            BackendInitReason::Boot | BackendInitReason::NetworkSwitch => {
                "wallet-backend-eager-init"
            }
            BackendInitReason::OnboardingAutoStart => "spv_auto_start",
            BackendInitReason::ManualConnect => "spv_manual_start",
        }
    }

    /// Log a successful chain-sync start.
    fn log_spv_started(self, app_ctx: &AppContext, already_running: bool) {
        let network = app_ctx.network;
        match self {
            BackendInitReason::Boot => {
                tracing::info!(?network, "SPV sync started automatically at boot");
            }
            BackendInitReason::NetworkSwitch if already_running => {
                tracing::info!(
                    ?network,
                    "Chain sync already running on the switched-to context"
                );
            }
            BackendInitReason::NetworkSwitch => {
                tracing::info!(?network, "Chain sync started after network switch");
            }
            BackendInitReason::OnboardingAutoStart => {
                tracing::info!(?network, "SPV sync started automatically after onboarding");
            }
            // The manual Connect button is silent on success — the SPV-sync
            // block conveys progress and the indicator flips to connected.
            BackendInitReason::ManualConnect => {}
        }
    }

    /// Handle a failed wire+start. Every path except `ManualConnect` warns and
    /// relies on the lazy backend-task fallback to retry; the manual Connect
    /// surfaces an actionable error banner because the user is waiting.
    fn on_spv_start_error(self, egui_ctx: &egui::Context, error: &TaskError) {
        match self {
            BackendInitReason::Boot => {
                tracing::warn!(error = %error, "eager wallet-backend init + SPV auto-start failed; SDK proof verification will retry once the lazy backend-task fallback fires");
            }
            BackendInitReason::NetworkSwitch => {
                tracing::warn!(error = %error, "eager wallet-backend init + SPV auto-start after network switch failed; lazy fallback will retry");
            }
            BackendInitReason::OnboardingAutoStart => {
                tracing::warn!(error = %error, "Failed to auto-start SPV sync after onboarding");
            }
            BackendInitReason::ManualConnect => {
                // The chokepoint already flipped the SPV indicator to Error;
                // the user pressed Connect and is waiting, so also surface an
                // actionable error banner here.
                let handle = MessageBanner::set_global(
                    egui_ctx,
                    "Could not start network sync. Check your connection and try again.",
                    MessageType::Error,
                );
                handle.disable_auto_dismiss();
                handle.with_details(error);
            }
        }
    }

    /// Handle a failed wire-only init (no chain-sync start requested).
    fn on_wire_error(self, error: &TaskError) {
        match self {
            BackendInitReason::Boot => {
                tracing::warn!(error = %error, "eager wallet-backend init failed; SDK proof verification will retry once the lazy backend-task fallback fires");
            }
            // Only Boot / NetworkSwitch ever wire without starting SPV; the
            // opt-in paths always start.
            _ => {
                tracing::warn!(error = %error, "eager wallet-backend init after network switch failed; lazy fallback will retry");
            }
        }
    }
}

impl AppState {
    /// Creates a new `AppState`, opening the seed vault keyless.
    ///
    /// Database selection is delegated to [`Self::boot_inputs`], which is
    /// feature-gated so that the `testing` build can never touch the
    /// production database. The keyless open aborts on a passphrase-protected
    /// legacy vault; the GUI binary boots through
    /// [`BootApp`](crate::boot::BootApp) instead, which prompts for the
    /// passphrase rather than aborting.
    pub fn new(ctx: egui::Context) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (data_dir, db) = Self::boot_inputs()?;
        let secret_store = AppContext::open_secret_store(&data_dir)?;
        Self::new_inner(ctx, db, data_dir, secret_store)
    }

    /// Prepare the boot inputs (data dir, env file, logging, database).
    ///
    /// The non-testing build opens an existing pre-update database read-only.
    /// A fresh install may create its empty compatibility database; the
    /// `testing` build substitutes an in-memory database so tests never read or
    /// write production data.
    #[cfg(not(feature = "testing"))]
    pub(crate) fn boot_inputs()
    -> Result<(PathBuf, Arc<Database>), Box<dyn std::error::Error + Send + Sync>> {
        let data_dir = crate::boot::prepare_environment()?;
        let db_file_path = data_file_path(&data_dir, "data.db")?;
        let db = if db_file_path.exists() {
            Arc::new(Database::open_legacy_read_only(&db_file_path)?)
        } else {
            let db = Arc::new(Database::new(&db_file_path)?);
            db.initialize(&db_file_path)?;
            db
        };
        Ok((data_dir, db))
    }

    #[cfg(feature = "testing")]
    pub(crate) fn boot_inputs()
    -> Result<(PathBuf, Arc<Database>), Box<dyn std::error::Error + Send + Sync>> {
        let data_dir = app_user_data_dir_path()?;
        ensure_data_dir_exists(&data_dir)?;
        ensure_env_file(&data_dir);

        let db = Arc::new(
            crate::database::test_helpers::create_test_database()
                .map_err(|e| format!("Failed to create test database: {}", e))?,
        );
        Ok((data_dir, db))
    }

    pub(crate) fn new_inner(
        ctx: egui::Context,
        db: Arc<Database>,
        data_dir: PathBuf,
        secret_store: Arc<SecretStore>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Boot path now reads preferences from the shared app k/v store
        // (`<data_dir>/det-app.sqlite`). The store is opened once here and
        // handed to every per-network `AppContext`. The seed vault was opened
        // by the caller (keyless, or with a recovered legacy passphrase).
        let app_kv = AppContext::open_app_kv(&data_dir)?;

        // Carry an upgrading user's preferences (network, theme, onboarding)
        // out of legacy `data.db` before they are read below. This has to run
        // here, ahead of the read: the active network is chosen from the blob
        // a few lines down, and booting a testnet user onto mainnet is a
        // safety hazard. Read/write failures force explicit network selection;
        // the (unwritten) sentinel makes the next launch retry.
        let mut network_selection_required =
            match crate::backend_task::migration::legacy_settings::import_legacy_settings(
                &app_kv, &db,
            ) {
                Ok(outcome) => {
                    tracing::debug!(?outcome, "Legacy settings import");
                    false
                }
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        "Could not import preferences from the previous version — using defaults; \
                         the next launch retries",
                    );
                    show_legacy_settings_import_warning(&ctx, &e);
                    legacy_settings_import_requires_network_selection(&e)
                }
            };

        let settings = match app_kv.get::<AppSettings>(DetScope::Global, AppSettings::KV_KEY) {
            Ok(Some(s)) => s,
            Ok(None) => AppSettings::default(),
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "Failed to read AppSettings at boot — using defaults"
                );
                show_legacy_settings_import_warning(&ctx, &e);
                network_selection_required = true;
                AppSettings::default()
            }
        };
        let theme_preference = settings.theme_mode;
        let onboarding_completed = settings.onboarding_completed;

        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());

        let saved_network = settings.network;

        // App-global user role, shared into every per-network context (including
        // any created later by a network switch) so a live change is observed
        // everywhere without a restart. Seeded below from `get_app_settings()`
        // once the active context exists — the single persisted source of truth.
        let user_role = crate::model::user_role::UserRoleCell::default();

        // Build a helper to create AppContext for a given network.
        let make_context = |network: Network| -> Option<Arc<AppContext>> {
            AppContext::new(
                data_dir.clone(),
                network,
                db.clone(),
                subtasks.clone(),
                connection_status.clone(),
                ctx.clone(),
                Arc::clone(&app_kv),
                Arc::clone(&secret_store),
                user_role.clone(),
            )
        };

        // Only create the saved/active network eagerly; defer ALL others
        // (including mainnet) until the user switches to them. This avoids
        // DAPI discovery + SDK init for networks the user may never use.
        //
        // If the saved network fails (e.g., no DAPI addresses configured),
        // try other networks before giving up. The user can fix the config
        // via the "Fetch Node List" button in Network Settings.
        let mut network_contexts = BTreeMap::new();
        let try_order = std::iter::once(saved_network).chain(
            [
                Network::Mainnet,
                Network::Testnet,
                Network::Devnet,
                Network::Regtest,
            ]
            .into_iter()
            .filter(|n| *n != saved_network),
        );
        for net in try_order {
            if let Some(ctx) = make_context(net) {
                network_contexts.insert(net, ctx);
                break;
            }
            if net == saved_network {
                tracing::warn!(
                    "Could not create context for saved network {:?}. \
                     Check your node addresses. Trying other networks...",
                    saved_network
                );
            }
        }
        if network_contexts.is_empty() {
            return Err(
                "No network could be initialized. Check that at least one network has \
                 DAPI node addresses configured in your settings file. You can use the \
                 \"Fetch Node List\" button in Network Settings to get addresses."
                    .into(),
            );
        }
        let chosen_network = *network_contexts
            .keys()
            .next()
            .expect("invariant: network_contexts is non-empty after the emptiness check above");
        let active_context = network_contexts
            .get(&chosen_network)
            .expect("invariant: chosen_network was just taken from network_contexts")
            .clone();

        // Seed the shared role cell from AppSettings — the single source of truth —
        // publishing it to every context holding the cell. A settings read that
        // fails here seeds the least-privileged role rather than `WHEN_UNSET`; see
        // `seed_user_role_from_settings`.
        active_context.seed_user_role_from_settings();

        // load fonts
        ctx.set_fonts(crate::bundled::fonts().expect("failed to load fonts"));

        // Force-enable AccessKit so the accessibility tree is populated every
        // frame, even without VoiceOver or other assistive technology running.
        // Without this flag, AccessKit activates lazily when a real assistive
        // client connects (which is the normal behavior).
        // Gated behind DASH_EVO_TOOL_ACCESSIBILITY=1 to avoid per-frame cost
        // when not needed for automation tooling.
        let accessibility_enforced =
            std::env::var("DASH_EVO_TOOL_ACCESSIBILITY").unwrap_or_default() == "1";
        if accessibility_enforced {
            ctx.enable_accesskit();
        }

        // All screens are initialized with the active context (chosen_network).
        // They will get the right context via change_context() on network switch.
        let identities_screen = IdentitiesScreen::new(&active_context);
        let dpns_active_contests_screen = DPNSScreen::new(&active_context, DPNSSubscreen::Active);
        let dpns_past_contests_screen = DPNSScreen::new(&active_context, DPNSSubscreen::Past);
        let dpns_my_usernames_screen = DPNSScreen::new(&active_context, DPNSSubscreen::Owned);
        let dpns_scheduled_votes_screen =
            DPNSScreen::new(&active_context, DPNSSubscreen::ScheduledVotes);
        let transition_visualizer_screen = TransitionVisualizerScreen::new(&active_context);
        let proof_visualizer_screen = ProofVisualizerScreen::new(&active_context);
        let document_visualizer_screen = DocumentVisualizerScreen::new(&active_context);
        let contract_visualizer_screen = ContractVisualizerScreen::new(&active_context);
        let platform_info_screen = PlatformInfoScreen::new(&active_context);
        let address_balance_screen = AddressBalanceScreen::new(&active_context);
        let grovestark_screen = GroveSTARKScreen::new(&active_context);
        let document_query_screen = DocumentQueryScreen::new(&active_context);
        let tokens_balances_screen = TokensScreen::new(&active_context, TokensSubscreen::MyTokens);
        let token_search_screen = TokensScreen::new(&active_context, TokensSubscreen::SearchTokens);
        let token_creator_screen =
            TokensScreen::new(&active_context, TokensSubscreen::TokenCreator);
        let contracts_dashpay_screen =
            DashPayScreen::new(&active_context, DashPaySubscreen::Profile);
        let dashpay_contacts_screen =
            DashPayScreen::new(&active_context, DashPaySubscreen::Contacts);
        let dashpay_profile_screen = DashPayScreen::new(&active_context, DashPaySubscreen::Profile);
        let dashpay_payments_screen =
            DashPayScreen::new(&active_context, DashPaySubscreen::Payments);
        let dashpay_profile_search_screen = ProfileSearchScreen::new(active_context.clone());

        let network_chooser_screen = NetworkChooserScreen::new(&network_contexts, chosen_network);

        let wallets_balances_screen = WalletsBalancesScreen::new(&active_context);

        // Persisted setting; the effective `selected_main_screen` is computed
        // after the screen map is built (below) so we can fall back to a
        // known-registered screen if the persisted value is no longer
        // registered.
        let persisted_main_screen = settings.root_screen_type;

        // // Create a channel with a buffer size of 32 (adjust as needed)
        let (task_result_sender, task_result_receiver) =
            tokiompsc::channel(256).with_egui_ctx(ctx.clone());

        // Build the egui just-in-time secret prompt host and install it on
        // every network context BEFORE the eager wallet-backend init below
        // reads it into each backend's `SecretAccess`. The host enqueues
        // passphrase requests onto `secret_prompt_receiver`, which the frame
        // loop drains. One host serves every network (the request carries the
        // scope; the active network's backend prompts through it).
        let (secret_prompt_host, secret_prompt_receiver) = EguiSecretPromptHost::new(ctx.clone());
        let secret_prompt_host: Arc<dyn crate::wallet_backend::SecretPrompt> =
            Arc::new(secret_prompt_host);
        for app_ctx in network_contexts.values() {
            app_ctx.install_secret_prompt(Arc::clone(&secret_prompt_host));
        }

        // Eagerly build the wallet seam for every pre-created network context
        // (typically just the active one) so the SpvProvider can serve
        // chain-only lookups (e.g. `get_quorum_public_key`) before any
        // wallet is unlocked. Without this, the SDK retry loop tight-loops
        // at 10ms on `WalletBackendNotYetWired`. `PlatformWalletManager` is
        // wallet-independent at construction (Case B); persisted wallets
        // load watch-only via `load_from_persistor_seedless`, no unlock required
        // to display funds — the seed enters memory only on unlock.
        //
        // Auto-start of chain sync rides on wiring completion: for the active
        // network, when onboarding is done and the user opted in, the same
        // task that wires the backend goes on to start SPV. Folding the start
        // into the spawned init closes the boot race where a synchronous
        // `start_spv()` fired before the fire-and-forget wiring could finish.
        let boot_auto_start_spv = boot_auto_start_spv(
            onboarding_completed,
            settings.auto_start_spv,
            network_selection_required,
        );
        for (&net, app_ctx) in network_contexts.iter() {
            let auto_start = boot_auto_start_spv && net == chosen_network;
            Self::spawn_backend_init(
                &subtasks,
                task_result_sender.clone(),
                app_ctx.clone(),
                BackendInitReason::Boot,
                auto_start,
            );
        }

        // MCP server (feature-gated, opt-in via MCP_API_KEY env var)
        #[cfg(feature = "mcp")]
        let (mcp_app_context, mcp_server_pending_config) = {
            if let Some(mcp_config) = crate::mcp::McpConfig::from_env() {
                let initial_ctx = active_context.clone();
                let mcp_ctx = Arc::new(arc_swap::ArcSwap::new(initial_ctx));
                let pending_config = if !network_selection_required {
                    Self::spawn_mcp_server(&subtasks, mcp_ctx.clone(), mcp_config);
                    None
                } else {
                    tracing::debug!("MCP server deferred until network selection");
                    Some(mcp_config)
                };
                (Some(mcp_ctx), pending_config)
            } else {
                let reason = match std::env::var("MCP_API_KEY") {
                    Ok(ref k) if !k.is_empty() => "MCP_API_KEY is set but invalid (too short)",
                    _ => "MCP_API_KEY not set",
                };
                tracing::debug!("MCP server disabled ({reason})");
                (None, None)
            }
        };

        let main_screens: BTreeMap<RootScreenType, Screen> = [
            (
                RootScreenType::RootScreenIdentities,
                Screen::IdentitiesScreen(identities_screen),
            ),
            (
                RootScreenType::RootScreenDPNSActiveContests,
                Screen::DPNSScreen(dpns_active_contests_screen),
            ),
            (
                RootScreenType::RootScreenDPNSPastContests,
                Screen::DPNSScreen(dpns_past_contests_screen),
            ),
            (
                RootScreenType::RootScreenDPNSOwnedNames,
                Screen::DPNSScreen(dpns_my_usernames_screen),
            ),
            (
                RootScreenType::RootScreenDPNSScheduledVotes,
                Screen::DPNSScreen(dpns_scheduled_votes_screen),
            ),
            (
                RootScreenType::RootScreenWalletsBalances,
                Screen::WalletsBalancesScreen(wallets_balances_screen),
            ),
            (
                RootScreenType::RootScreenToolsTransitionVisualizerScreen,
                Screen::TransitionVisualizerScreen(transition_visualizer_screen),
            ),
            (
                RootScreenType::RootScreenToolsProofVisualizerScreen,
                Screen::ProofVisualizerScreen(proof_visualizer_screen),
            ),
            (
                RootScreenType::RootScreenToolsDocumentVisualizerScreen,
                Screen::DocumentVisualizerScreen(document_visualizer_screen),
            ),
            (
                RootScreenType::RootScreenToolsContractVisualizerScreen,
                Screen::ContractVisualizerScreen(contract_visualizer_screen),
            ),
            (
                RootScreenType::RootScreenToolsPlatformInfoScreen,
                Screen::PlatformInfoScreen(platform_info_screen),
            ),
            (
                RootScreenType::RootScreenToolsAddressBalanceScreen,
                Screen::AddressBalanceScreen(address_balance_screen),
            ),
            (
                RootScreenType::RootScreenToolsGroveSTARKScreen,
                Screen::GroveSTARKScreen(grovestark_screen),
            ),
            (
                RootScreenType::RootScreenDocumentQuery,
                Screen::DocumentQueryScreen(document_query_screen),
            ),
            (
                RootScreenType::RootScreenDashpay,
                Screen::DashPayScreen(contracts_dashpay_screen),
            ),
            (
                RootScreenType::RootScreenNetworkChooser,
                Screen::NetworkChooserScreen(network_chooser_screen),
            ),
            (
                RootScreenType::RootScreenMyTokenBalances,
                Screen::TokensScreen(Box::new(tokens_balances_screen)),
            ),
            (
                RootScreenType::RootScreenTokenSearch,
                Screen::TokensScreen(Box::new(token_search_screen)),
            ),
            (
                RootScreenType::RootScreenTokenCreator,
                Screen::TokensScreen(Box::new(token_creator_screen)),
            ),
            (
                RootScreenType::RootScreenDashPayContacts,
                Screen::DashPayScreen(dashpay_contacts_screen),
            ),
            (
                RootScreenType::RootScreenDashPayProfile,
                Screen::DashPayScreen(dashpay_profile_screen),
            ),
            (
                RootScreenType::RootScreenDashPayPayments,
                Screen::DashPayScreen(dashpay_payments_screen),
            ),
            (
                RootScreenType::RootScreenDashPayProfileSearch,
                Screen::DashPayProfileSearchScreen(dashpay_profile_search_screen),
            ),
            (
                // Always registered — the Masternodes tab is gated at runtime by
                // Expert Mode (the nav entry + route), not by a Cargo feature, so
                // the screen must always exist to switch into when Expert Mode
                // is on. Live de-gating falls back to `FALLBACK_ROOT_SCREEN`.
                RootScreenType::RootScreenMasternodes,
                Screen::MasternodesScreen(crate::ui::masternodes::MasternodesScreen::new(
                    &active_context,
                )),
            ),
        ]
        .into_iter()
        .chain({
            // Register the unified Identities hub screen.
            let hub = crate::ui::identity::IdentityHubScreen::new(&active_context);
            [(
                RootScreenType::RootScreenIdentityHub,
                Screen::IdentityHubScreen(hub),
            )]
        })
        .collect();

        // Resolve the effective selected root screen. If the persisted value is
        // no longer registered, fall back to `FALLBACK_ROOT_SCREEN` so
        // `active_root_screen_mut()` does not panic on first frame.
        let selected_main_screen = initial_root_screen(
            persisted_main_screen,
            main_screens.contains_key(&persisted_main_screen),
            network_selection_required,
        );

        let mut app_state = Self {
            main_screens,
            selected_main_screen,
            screen_stack: vec![],
            chosen_network,
            connection_status,
            network_contexts,
            network_switch_pending: None,
            network_switch_banner: None,
            network_selection_required,
            task_result_sender,
            task_result_receiver,
            theme: ThemeState::new(theme_preference),
            last_scheduled_vote_check: Instant::now(),
            scheduled_vote_sweep_deferred_since_ms: BTreeMap::new(),
            scheduled_vote_sweeps_in_progress: BTreeSet::new(),
            scheduled_vote_recovery_last_attempt: BTreeMap::new(),
            last_repaint_request: Instant::now(),
            subtasks,
            show_welcome_screen: show_welcome_screen(
                onboarding_completed,
                network_selection_required,
            ),
            welcome_screen: None,
            connection_banner: ConnectionBanner::new(),
            // Arm the block for the boot SPV sync when it auto-starts (F-SPV-A:
            // scoped to user-initiated sync, not ambient reconnect).
            spv_block: SpvBlockReconciler::new(boot_auto_start_spv),
            migration: MigrationReconciler::new(),
            shutdown_receiver: None,
            shutdown_started: None,
            shutdown_finished: false,
            accessibility: AccessibilityActivator::new(accessibility_enforced),
            #[cfg(feature = "mcp")]
            mcp_app_context,
            #[cfg(feature = "mcp")]
            mcp_server_pending_config,
            secret_prompt_receiver,
            active_secret_prompt: None,
            prompt_was_blocking: false,
        };

        // Initialize welcome screen if needed (uses whichever context is active)
        if app_state.show_welcome_screen {
            app_state.welcome_screen =
                Some(WelcomeScreen::new(app_state.current_app_context().clone()));
        } else {
            // Boot-time SPV auto-start is folded into the eager wallet-backend
            // init above (so it cannot fire before the backend is wired).

            // Refresh ALL main screens so they load data properly
            // This ensures screens like DashPay Profile have identities loaded
            // even if they're not the initially selected screen
            for screen in app_state.main_screens.values_mut() {
                screen.refresh_on_arrival();
            }
        }

        // The Orchard proving key is now owned by the upstream shielded
        // coordinator (`CachedOrchardProver`), warmed lazily on the first
        // shielded operation — DET no longer builds or caches it here.

        Ok(app_state)
    }

    /// Force UI animations off (or lift that override) for every network context.
    ///
    /// No override by default. Lifting one does not *guarantee* animation: the
    /// Power and Developer roles keep the UI still on their own.
    pub fn with_animations(self, enabled: bool) -> Self {
        for context in self.network_contexts.values() {
            context.set_animations_disabled(!enabled);
        }
        self
    }

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        self.network_contexts
            .get(&self.chosen_network)
            .unwrap_or_else(|| {
                panic!(
                    "BUG: chosen network is {:?} but its AppContext is missing",
                    self.chosen_network
                )
            })
    }

    fn context_available_for_network(&self, network: Network) -> bool {
        self.network_contexts.contains_key(&network)
    }

    fn enforce_network_context_invariant(&mut self) {
        if self.context_available_for_network(self.chosen_network) {
            return;
        }

        panic!(
            "BUG: selected network {:?} has no AppContext. Refusing to auto-switch networks.",
            self.chosen_network
        );
    }

    /// Spawn wallet-backend wiring for `app_ctx` and, when `start_spv`, chain
    /// sync — the single shape behind every eager-init site (boot, network
    /// switch, post-onboarding auto-start, manual Connect). Folding the start
    /// into the same spawned task closes the boot race where a synchronous
    /// `start_spv()` could fire before the fire-and-forget wiring finished.
    /// `reason` selects the task label and the log/banner wording.
    ///
    /// Associated (not `&mut self`) so the constructor's per-network loop can
    /// call it before `AppState` exists; the block-arming that user-initiated
    /// starts need stays at those callsites.
    fn spawn_backend_init(
        subtasks: &Arc<TaskManager>,
        sender: egui_mpsc::SenderAsync<TaskResult>,
        app_ctx: Arc<AppContext>,
        reason: BackendInitReason,
        start_spv: bool,
    ) {
        let _ = subtasks.spawn_sync(reason.task_name(), async move {
            if start_spv {
                let already_running = app_ctx
                    .wallet_backend()
                    .map(|b| b.is_started())
                    .unwrap_or(false);
                match app_ctx.ensure_wallet_backend_and_start_spv(sender).await {
                    Ok(()) => reason.log_spv_started(&app_ctx, already_running),
                    Err(e) => reason.on_spv_start_error(app_ctx.egui_ctx(), &e),
                }
            } else if let Err(e) = app_ctx.ensure_wallet_backend(sender).await {
                reason.on_wire_error(&e);
            }
        });
    }

    #[cfg(feature = "mcp")]
    fn spawn_mcp_server(
        subtasks: &Arc<TaskManager>,
        app_context: Arc<arc_swap::ArcSwap<AppContext>>,
        config: crate::mcp::McpConfig,
    ) {
        let cancel = subtasks.cancellation_token.clone();
        let _ = subtasks.spawn_sync("mcp-server", async move {
            if let Err(error) = crate::mcp::start_http_server(app_context, config, cancel).await {
                tracing::error!(%error, "MCP server failed");
            }
        });
        tracing::debug!("MCP server enabled");
    }

    // Handle the backend task and send the result through the channel.
    //
    // Uses spawn_blocking + block_on to avoid Send bound issues with platform
    // SDK types (DataContract/Sdk references across await points).
    fn handle_backend_task(&mut self, task: BackendTask) {
        let context = BackendTaskContext::from(&task);
        self.handle_backend_task_with_context(task, context);
    }

    fn handle_backend_task_with_context(&mut self, task: BackendTask, context: BackendTaskContext) {
        let request_id = crate::backend_task::dashpay_request_id(&task);
        let sender = self.task_result_sender.clone();
        let watcher_sender = sender.clone();
        let watcher_context = context.clone();
        let app_context = self.current_app_context().clone();
        let handle = tokio::runtime::Handle::current();
        let _ = self.subtasks.spawn_blocking_sync(
            "backend_task_join_watcher",
            move || {
                handle.block_on(async move {
                    let result = app_context.run_backend_task(task, sender.clone()).await;
                    if let Err(e) = sender
                        .send(TaskResult::from_backend_task_result(context, result))
                        .await
                    {
                        tracing::error!("Failed to send task result: {}", e);
                    }
                });
            },
            move |join_result| {
                forward_backend_task_join_error(
                    join_result,
                    watcher_sender,
                    request_id,
                    watcher_context,
                )
            },
        );
    }

    /// Handle the backend tasks and send the results through the channel
    fn handle_backend_tasks(&self, tasks: Vec<BackendTask>, mode: BackendTasksExecutionMode) {
        let sender = self.task_result_sender.clone();
        let watcher_sender = sender.clone();
        let contexts = tasks
            .iter()
            .map(BackendTaskContext::from)
            .collect::<Vec<_>>();
        let app_context = self.current_app_context().clone();
        let handle = tokio::runtime::Handle::current();

        let _ = self.subtasks.spawn_blocking_sync(
            "backend_tasks_join_watcher",
            move || {
                handle.block_on(async move {
                    let results = match mode {
                        BackendTasksExecutionMode::Sequential => {
                            app_context
                                .run_backend_tasks_sequential(tasks, sender.clone())
                                .await
                        }
                        BackendTasksExecutionMode::Concurrent => {
                            app_context
                                .run_backend_tasks_concurrent(tasks, sender.clone())
                                .await
                        }
                    };

                    for (context, result) in contexts.into_iter().zip(results) {
                        if let Err(e) = sender
                            .send(TaskResult::from_backend_task_result(context, result))
                            .await
                        {
                            tracing::error!("Failed to send task result: {}", e);
                        }
                    }
                });
            },
            move |join_result| {
                forward_backend_task_join_error(
                    join_result,
                    watcher_sender,
                    None,
                    BackendTaskContext::Unknown,
                )
            },
        );
    }

    pub fn active_root_screen_mut(&mut self) -> &mut Screen {
        // Live de-gating (§10.11): if the role dropped below Power while the
        // Masternodes tab was active, fall back to `FALLBACK_ROOT_SCREEN` so the
        // gated screen is never shown without its gate. That screen is always
        // registered, so the subsequent lookup cannot fail.
        if self.selected_main_screen == RootScreenType::RootScreenMasternodes
            && !FeatureGate::Masternodes.is_available(self.current_app_context())
        {
            self.select_main_screen(FALLBACK_ROOT_SCREEN);
        }
        self.main_screens
            .get_mut(&self.selected_main_screen)
            .expect("expected to get screen")
    }

    /// Make `root_screen_type` the selected root screen, telling the screen being
    /// left that it is losing visibility. Root screens are never dropped, so a
    /// screen holding secrets (the Masternodes load form's keys) depends on this
    /// notification to zeroize them.
    fn select_main_screen(&mut self, root_screen_type: RootScreenType) {
        if self.selected_main_screen == root_screen_type {
            return;
        }
        if let Some(left) = self.main_screens.get_mut(&self.selected_main_screen) {
            left.on_leave();
        }
        self.selected_main_screen = root_screen_type;
    }

    pub fn change_network(&mut self, network: Network) {
        // Block any new switch while one is already in progress.
        if self.network_switch_pending.is_some() {
            tracing::debug!(
                "Ignoring network switch to {:?} — switch to {:?} already pending",
                network,
                self.network_switch_pending
            );
            return;
        }

        // Fast path: context already exists — switch immediately.
        if self.context_available_for_network(network) {
            self.finalize_network_switch(network);
            return;
        }

        // Slow path: dispatch SwitchNetwork as a backend task. The result
        // (NetworkContextCreated) comes back through the task result channel
        // and is handled in update(). Same path used by MCP tools.
        self.network_switch_pending = Some(network);
        self.network_switch_banner = Some(MessageBanner::set_global(
            self.current_app_context().egui_ctx(),
            format!("Connecting to {network:?}..."),
            MessageType::Info,
        ));
        let start_spv = self.current_app_context().get_app_settings().auto_start_spv;
        self.handle_backend_task(BackendTask::SwitchNetwork { network, start_spv });
    }

    /// Complete the network switch after the context is available.
    fn finalize_network_switch(&mut self, network: Network) {
        let was_network_selection_required = self.network_selection_required;
        // Forget any session-cached secrets on the outgoing context before we
        // leave it. The outgoing per-network context stays cached in
        // `network_contexts` (its `WalletBackend` is NOT dropped on switch), so
        // this explicit, eager zeroize is what the JIT design relies on to keep
        // secrets from lingering across a network change — not a drop.
        if let Ok(backend) = self.current_app_context().wallet_backend() {
            backend.forget_all_secrets();
        }

        self.chosen_network = network;
        self.network_selection_required = false;

        let app_context = self.current_app_context().clone();

        if was_network_selection_required && !app_context.get_app_settings().onboarding_completed {
            self.show_welcome_screen = true;
            self.welcome_screen = Some(WelcomeScreen::new(app_context.clone()));
        }

        // Same eager wallet-backend init as at app start (Case B): chain-
        // only SDK lookups must work pre-unlock on the freshly-switched
        // context too, otherwise the SDK tight-loops on WalletBackendNotYetWired.
        //
        // Chain sync auto-starts on wiring completion (mirrors boot). The slow
        // path already started SPV inside the `SwitchNetwork` task, but the fast
        // path (cached context) reaches here without ever having started it — so
        // the auto-start must live here to cover both. All steps are idempotent:
        // re-wiring is a no-op and the backend's start latch prevents a second
        // run loop.
        Self::spawn_backend_init(
            &self.subtasks,
            self.task_result_sender.clone(),
            app_context.clone(),
            BackendInitReason::NetworkSwitch,
            app_context.get_app_settings().auto_start_spv,
        );

        // Update MCP server's context to follow network switch
        #[cfg(feature = "mcp")]
        if let Some(ref mcp_ctx) = self.mcp_app_context {
            mcp_ctx.store(app_context.clone());
            tracing::debug!("MCP context switched to {:?}", network);
        }
        #[cfg(feature = "mcp")]
        if let (Some(mcp_ctx), Some(config)) = (
            self.mcp_app_context.clone(),
            self.mcp_server_pending_config.take(),
        ) {
            Self::spawn_mcp_server(&self.subtasks, mcp_ctx, config);
        }

        // Deliberately clear stale banners from the previous network context.
        // A backend task completing after the switch could set a new banner in the new
        // network context — accepted risk for a local desktop app (cosmetic only).
        MessageBanner::clear_all_global(app_context.egui_ctx());
        // Drop any blocking overlay from the previous context so the new network
        // is never left behind a stale block. Also drop the SPV-sync overlay
        // bookkeeping so its handle never goes stale against the cleared `ctx.data`.
        ProgressOverlay::clear_all_global(app_context.egui_ctx());
        self.spv_block.reset();

        for screen in self.main_screens.values_mut() {
            screen.change_context(app_context.clone())
        }

        self.connection_status.reset();

        // Reset connection banner tracking so the next frame re-evaluates
        // the new network's state (even if it matches the old state).
        self.connection_banner.reset();

        // Reset the migration banner reconciler too: the new network's
        // `MigrationStatus` lives on the new `AppContext`, so the reconciler
        // must re-evaluate from scratch (otherwise a stale `Success` from the
        // previous network would suppress the new network's `Running` banner).
        self.migration.reset_for_switch();

        // Persist the network choice.
        match app_context.update_settings(RootScreenType::RootScreenNetworkChooser) {
            Ok(()) if was_network_selection_required => {
                if let Err(error) = crate::backend_task::migration::legacy_settings::finish_after_explicit_network_selection(app_context.app_kv().as_ref()) {
                    show_legacy_settings_import_warning(app_context.egui_ctx(), &error);
                }
            }
            Ok(()) => {}
            Err(error) => {
                tracing::warn!(error = ?error, "Could not persist the selected network");
                show_legacy_settings_import_warning(app_context.egui_ctx(), &error);
            }
        }
    }

    /// Whether a passphrase prompt owns the frame's full interaction surface.
    fn has_blocking_secret_prompt(&self, migration_state: &MigrationState) -> bool {
        self.active_secret_prompt.is_some() || MigrationReconciler::is_prompting(migration_state)
    }

    fn claim_overlay_input(&self, ctx: &egui::Context, migration_state: &MigrationState) {
        if !self.has_blocking_secret_prompt(migration_state) {
            ProgressOverlay::claim_input(ctx);
        }
    }

    /// Test seam (RQ-1): force a secret prompt to be active (or not) so a kittest
    /// can drive the REAL `update()` loop — including the
    /// [`claim_overlay_input`](Self::claim_overlay_input) gate and
    /// `render_secret_prompt` — and assert that the prompt above an overlay stays
    /// focusable/typeable. Compiled only under the `testing` feature.
    #[cfg(feature = "testing")]
    pub fn test_set_secret_prompt_active(&mut self, active: bool) {
        self.active_secret_prompt = active.then(ActivePrompt::test_stub);
    }

    /// Test seam (Task 9 / F-SPV-A): arm a user-initiated SPV-sync block episode,
    /// as the boot auto-start and the Connect button do.
    #[cfg(feature = "testing")]
    pub fn test_arm_spv_block(&mut self) {
        self.spv_block.arm();
    }

    /// Test seam (Task 9): run the REAL SPV-sync block driver once against the
    /// active context's (forced) connection state, in isolation from the
    /// throttled frame loop. Lets a kittest assert that an armed episode blocks,
    /// disarms on a terminal state, and that ambient (un-armed) sync never blocks.
    #[cfg(feature = "testing")]
    pub fn test_drive_spv_overlay(&mut self, ctx: &egui::Context) {
        let app_context = self.current_app_context().clone();
        self.spv_block.update(ctx, &app_context);
    }

    /// Test seam (F-SPV-A): run the REAL post-onboarding auto-start path
    /// ([`Self::try_auto_start_spv`], the method `AppAction::OnboardingComplete`
    /// invokes) so a kittest can lock that it arms the SPV-sync block.
    #[cfg(feature = "testing")]
    pub fn test_run_auto_start_spv(&mut self) {
        self.try_auto_start_spv();
    }

    /// Test seam (F-SPV-A): observe the SPV-sync block's armed flag.
    #[cfg(feature = "testing")]
    pub fn test_spv_block_armed(&self) -> bool {
        self.spv_block.armed()
    }

    /// Sweep orphaned overlay action ids whose owning overlay is gone. Screens own
    /// dispatch and cancellation today — they drain their own clicks via
    /// [`OverlayHandle::take_actions`](crate::ui::components::OverlayHandle::take_actions);
    /// this loop only reclaims orphans so they cannot accumulate in `ctx.data`.
    //
    // TODO(T7): the BackendTask system has no cooperative cancellation, so an
    // overlay button can only stop waiting, never abort a running operation. When
    // T7 lands (thread a per-operation CancellationToken through run_backend_task
    // and retain the abort handle in handle_backend_task), a screen can wire a
    // generic overlay button — e.g. one it labels "Cancel" — to a real abort.
    // Until then no production overlay attaches a button to a running task, and
    // this loop has no live cancellation role; the 120s watchdog
    // (see progress_overlay.rs) bounds every block in the meantime.
    fn drain_overlay_actions(&mut self, ctx: &egui::Context) {
        for action_id in ProgressOverlay::sweep_orphan_actions(ctx) {
            tracing::warn!(
                target = "ui::overlay",
                action_id = %action_id,
                "Overlay action received for an overlay that is no longer active — dropping"
            );
        }
    }

    pub fn visible_screen_mut(&mut self) -> &mut Screen {
        if self.screen_stack.is_empty() {
            self.active_root_screen_mut()
        } else {
            self.screen_stack
                .last_mut()
                .expect("invariant: screen_stack is non-empty in this branch")
        }
    }

    fn route_contact_request_result_to_hidden_hub(&mut self, result: &BackendTaskSuccessResult) {
        if identity_hub_is_visible(self.selected_main_screen, self.screen_stack.is_empty()) {
            return;
        }
        if let Some(Screen::IdentityHubScreen(hub)) = self
            .main_screens
            .get_mut(&RootScreenType::RootScreenIdentityHub)
        {
            hub.handle_contact_request_result(result);
        }
    }

    fn route_contact_request_error_to_hidden_hub(&mut self, error: &TaskError) {
        if identity_hub_is_visible(self.selected_main_screen, self.screen_stack.is_empty()) {
            return;
        }
        if let Some(Screen::IdentityHubScreen(hub)) = self
            .main_screens
            .get_mut(&RootScreenType::RootScreenIdentityHub)
        {
            hub.handle_contact_request_error(error);
        }
    }

    /// Promote at most one queued passphrase request before overlay handling.
    fn activate_secret_prompt(&mut self, ctx: &egui::Context) {
        if self.active_secret_prompt.is_none()
            && let Ok(queued) = self.secret_prompt_receiver.try_recv()
        {
            self.active_secret_prompt = Some(ActivePrompt::new(queued));
            ctx.request_repaint();
        }
    }

    fn render_secret_prompt(&mut self, ctx: &egui::Context) {
        if let Some(prompt) = &mut self.active_secret_prompt {
            let resolved = prompt.show(ctx);
            if resolved {
                self.active_secret_prompt = None;
                // A second request may be queued — repaint so it surfaces
                // without waiting for an idle wakeup.
                ctx.request_repaint();
            }
        }
    }

    fn set_main_screen(&mut self, root_screen_type: RootScreenType) {
        if !network_selection_allows_root(self.network_selection_required, root_screen_type) {
            return;
        }
        self.select_main_screen(root_screen_type);
        let active_screen = self.active_root_screen_mut();
        active_screen.reset_to_root_view();
        active_screen.refresh_on_arrival();
        self.current_app_context()
            .update_settings(root_screen_type)
            .ok();
    }

    /// Auto-start chain sync for the active context when the user opted in.
    ///
    /// Wires the wallet backend first (via the async chokepoint) so the start
    /// cannot race ahead of backend wiring. Used after onboarding completes;
    /// boot-time auto-start is handled inline by the eager wallet-backend init.
    ///
    /// Arms the SPV-sync block (F-SPV-A) when the start actually fires — this is
    /// a user-initiated sync just like the Connect button, so the blocking
    /// overlay must cover it. Boot auto-start arms via the constructor instead.
    fn try_auto_start_spv(&mut self) {
        if self.current_app_context().get_app_settings().auto_start_spv {
            // Fresh user-initiated episode: arm the block and re-arm the escape,
            // mirroring AppAction::StartSpv.
            self.spv_block.arm();
            Self::spawn_backend_init(
                &self.subtasks,
                self.task_result_sender.clone(),
                self.current_app_context().clone(),
                BackendInitReason::OnboardingAutoStart,
                true,
            );
        }
    }

    fn push_unique_context(contexts: &mut Vec<Arc<AppContext>>, context: Arc<AppContext>) {
        if !contexts
            .iter()
            .any(|existing| Arc::ptr_eq(existing, &context))
        {
            contexts.push(context);
        }
    }

    fn collect_created_context(contexts: &mut Vec<Arc<AppContext>>, task_result: TaskResult) {
        if let TaskResult::Success { result, .. } = task_result {
            match *result {
                BackendTaskSuccessResult::NetworkContextRegistered { context, .. }
                | BackendTaskSuccessResult::NetworkContextCreated { context, .. } => {
                    Self::push_unique_context(contexts, context);
                }
                _ => {}
            }
        }
    }

    async fn shutdown_wallet_backend_instances<B: ShutdownWalletBackend>(
        wallet_backends: &[Arc<B>],
        shutdown_timeout: Duration,
    ) -> ShutdownOutcome {
        for backend in wallet_backends {
            backend.forget_all_secrets();
        }

        let shutdowns = futures::future::join_all(
            wallet_backends
                .iter()
                .map(|wallet_backend| wallet_backend.shutdown()),
        );
        let outcome = if tokio::time::timeout(shutdown_timeout, shutdowns)
            .await
            .is_err()
        {
            tracing::warn!(
                timeout_secs = shutdown_timeout.as_secs(),
                "Wallet backend shutdown timed out; closing with degraded teardown"
            );
            ShutdownOutcome::WalletBackendTimedOut
        } else {
            ShutdownOutcome::Complete
        };

        for backend in wallet_backends {
            backend.forget_all_secrets();
        }

        outcome
    }

    async fn shutdown_wallet_backends(contexts: Vec<Arc<AppContext>>) -> ShutdownOutcome {
        let mut wallet_backends = Vec::new();
        for context in contexts {
            if let Ok(backend) = context.wallet_backend()
                && !wallet_backends
                    .iter()
                    .any(|existing| Arc::ptr_eq(existing, &backend))
            {
                wallet_backends.push(backend);
            }
        }

        Self::shutdown_wallet_backend_instances(&wallet_backends, WALLET_BACKEND_SHUTDOWN_TIMEOUT)
            .await
    }

    fn initial_shutdown_contexts(&self) -> Vec<Arc<AppContext>> {
        let mut contexts = Vec::new();
        for context in self.network_contexts.values() {
            Self::push_unique_context(&mut contexts, Arc::clone(context));
        }
        contexts
    }

    async fn finish_wallet_shutdown(
        mut contexts: Vec<Arc<AppContext>>,
        mut task_result_receiver: tokiompsc::Receiver<TaskResult>,
        #[cfg(feature = "mcp")] mcp_app_context: Option<Arc<arc_swap::ArcSwap<AppContext>>>,
    ) -> ShutdownOutcome {
        while let Ok(task_result) = task_result_receiver.try_recv() {
            Self::collect_created_context(&mut contexts, task_result);
        }

        #[cfg(feature = "mcp")]
        if let Some(mcp_app_context) = mcp_app_context {
            Self::push_unique_context(&mut contexts, mcp_app_context.load_full());
        }

        Self::shutdown_wallet_backends(contexts).await
    }

    async fn finish_shutdown_after_tasks<F>(
        task_shutdown_outcome: Option<TaskShutdownOutcome>,
        wallet_shutdown: F,
    ) -> ShutdownOutcome
    where
        F: std::future::Future<Output = ShutdownOutcome>,
    {
        match task_shutdown_outcome {
            Some(TaskShutdownOutcome::Complete) => wallet_shutdown.await,
            Some(TaskShutdownOutcome::BackendTasksTimedOut) => {
                ShutdownOutcome::BackendTasksTimedOut
            }
            None => ShutdownOutcome::TaskManagerFailed,
        }
    }

    fn start_async_shutdown(&mut self) -> tokio::sync::oneshot::Receiver<ShutdownOutcome> {
        let mut contexts = self.initial_shutdown_contexts();
        let mut task_shutdown = self.subtasks.shutdown_async();
        let (_empty_sender, empty_receiver) = tokiompsc::channel(1);
        let mut task_result_receiver =
            std::mem::replace(&mut self.task_result_receiver, empty_receiver);
        #[cfg(feature = "mcp")]
        let mcp_app_context = self.mcp_app_context.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let task_shutdown_outcome = loop {
                tokio::select! {
                    result = &mut task_shutdown => break result.ok(),
                    task_result = task_result_receiver.recv() => {
                        let Some(task_result) = task_result else {
                            break task_shutdown.await.ok();
                        };
                        Self::collect_created_context(&mut contexts, task_result);
                    }
                }
            };

            let outcome = Self::finish_shutdown_after_tasks(
                task_shutdown_outcome,
                Self::finish_wallet_shutdown(
                    contexts,
                    task_result_receiver,
                    #[cfg(feature = "mcp")]
                    mcp_app_context,
                ),
            )
            .await;
            let _ = tx.send(outcome);
        });

        rx
    }

    fn run_blocking_shutdown_fallback(&mut self) {
        const POLL_INTERVAL: Duration = Duration::from_millis(10);

        let mut contexts = self.initial_shutdown_contexts();
        let mut task_shutdown = self.subtasks.shutdown_async();
        let (_empty_sender, empty_receiver) = tokiompsc::channel(1);
        let mut task_result_receiver =
            std::mem::replace(&mut self.task_result_receiver, empty_receiver);
        #[cfg(feature = "mcp")]
        let mcp_app_context = self.mcp_app_context.clone();
        let task_shutdown_outcome = loop {
            while let Ok(task_result) = task_result_receiver.try_recv() {
                Self::collect_created_context(&mut contexts, task_result);
            }

            match task_shutdown.try_recv() {
                Ok(outcome) => break Some(outcome),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => break None,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(POLL_INTERVAL);
                }
            }
        };

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        tokio::spawn(async move {
            let outcome = Self::finish_shutdown_after_tasks(
                task_shutdown_outcome,
                Self::finish_wallet_shutdown(
                    contexts,
                    task_result_receiver,
                    #[cfg(feature = "mcp")]
                    mcp_app_context,
                ),
            )
            .await;
            let _ = tx.send(outcome);
        });

        match rx.recv_timeout(WALLET_BACKEND_SHUTDOWN_TIMEOUT + SHUTDOWN_DEADLINE_MARGIN) {
            Ok(ShutdownOutcome::Complete)
                if task_shutdown_outcome == Some(TaskShutdownOutcome::Complete) => {}
            Ok(outcome) => tracing::warn!(
                ?outcome,
                ?task_shutdown_outcome,
                "Blocking shutdown fallback completed with degraded teardown"
            ),
            Err(error) => tracing::warn!(
                ?error,
                "Blocking wallet backend shutdown did not report a terminal outcome"
            ),
        }
    }
}

impl App for AppState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // ── Graceful shutdown: intercept window close so the UI stays responsive ──
        // When the user closes the window we cancel the native close, show a banner,
        // and start an async shutdown. Once all tasks have finished (or timed out)
        // we issue Close ourselves.
        if self.shutdown_started.is_some() {
            // Shutdown already in progress — check if it's done.
            let should_close = match self.shutdown_receiver.as_mut() {
                Some(rx) => match rx.try_recv() {
                    Ok(outcome) => {
                        self.shutdown_finished = true;
                        match outcome {
                            ShutdownOutcome::Complete => {
                                tracing::debug!("Async shutdown finished, closing viewport");
                            }
                            ShutdownOutcome::TaskManagerFailed => tracing::warn!(
                                "Task shutdown failed; closing with degraded teardown"
                            ),
                            ShutdownOutcome::BackendTasksTimedOut => tracing::warn!(
                                "Backend task blocking work exceeded its deadline; closing with degraded teardown"
                            ),
                            ShutdownOutcome::WalletBackendTimedOut => tracing::warn!(
                                "Wallet backend shutdown exceeded its deadline; closing with degraded teardown"
                            ),
                        }
                        true
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        // Sender dropped without sending — shutdown task likely panicked.
                        tracing::warn!("Shutdown channel closed unexpectedly (possible panic)");
                        true
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                        // Still waiting — check hard deadline to prevent infinite loop.
                        if let Some(started) = self.shutdown_started {
                            let grace = shutdown_hard_deadline();
                            if started.elapsed() > grace {
                                tracing::warn!(
                                    "Shutdown hard deadline exceeded, force-closing viewport"
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                },
                None => true,
            };
            if should_close {
                self.shutdown_receiver.take();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint();
            }
            // Render a minimal UI that shows the shutdown banner.
            self.theme.poll_and_apply(ctx);
            crate::ui::components::styled::island_central_panel(ui, |_ui| {});
            return;
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            // Prevent the window from closing immediately.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            MessageBanner::set_global(
                ctx,
                "Shutting down background tasks — please wait…",
                MessageType::Warning,
            );
            tracing::debug!("Close requested, starting async shutdown");
            self.shutdown_receiver = Some(self.start_async_shutdown());
            self.shutdown_started = Some(std::time::Instant::now());
            ctx.request_repaint();
            return;
        }

        // On the first frames, trigger platform-level accessibility activation
        // so tools like Peekaboo can see the AccessKit tree without VoiceOver.
        self.accessibility.update(ctx);

        self.theme.poll_and_apply(ctx);

        self.enforce_network_context_invariant();
        let active_context = self.current_app_context().clone();
        let migration_state = active_context.migration_status().state();

        // Poll the receiver for any new task results
        while let Ok(task_result) = self.task_result_receiver.try_recv() {
            active_context
                .connection_status()
                .handle_task_result(&task_result, active_context.network);

            // Handle the result on the main thread
            match task_result {
                TaskResult::Success {
                    context,
                    result: message,
                } => {
                    let unboxed_message = *message;
                    self.route_contact_request_result_to_hidden_hub(&unboxed_message);
                    match unboxed_message {
                        BackendTaskSuccessResult::None => {}
                        BackendTaskSuccessResult::Refresh => {
                            self.visible_screen_mut().refresh();
                        }
                        BackendTaskSuccessResult::NetworkDatabaseCleared { network } => {
                            let network_label = chooser_network_label(network);
                            MessageBanner::set_global(
                                ctx,
                                format!(
                                    "Cleared {network_label} database. Restart or resync to rebuild state."
                                ),
                                MessageType::Success,
                            );
                            if let Some(screen) = self
                                .main_screens
                                .get_mut(&RootScreenType::RootScreenNetworkChooser)
                            {
                                screen.display_backend_task_result(
                                    &context,
                                    BackendTaskSuccessResult::NetworkDatabaseCleared { network },
                                );
                            }
                        }
                        BackendTaskSuccessResult::DashPayIncomingDetected(outputs) => {
                            // The EventBridge surfaced received outputs on a
                            // freshly-seen wallet transaction. Run the owner-
                            // scoped detect-match-record off the frame thread;
                            // matches come back as a `Refresh` to repaint the
                            // payment history, misses as `None`.
                            self.handle_backend_task(BackendTask::DashPayTask(Box::new(
                                DashPayTask::DetectIncomingContactPayments { outputs },
                            )));
                        }
                        BackendTaskSuccessResult::PlatformReadyDiscoverIdentities => {
                            // Platform is reachable: run the automatic all-wallets
                            // identity discovery sweep. The latch inside makes it a
                            // no-op if it already ran this session.
                            active_context.queue_all_wallets_identity_discovery();
                        }
                        BackendTaskSuccessResult::Message(ref msg) => {
                            // TODO: Some screens inspect Message text for error
                            // keywords and may override with an Error banner, causing a
                            // brief green-then-red flash. Refactor to pass structured error
                            // types through task results instead of string messages.
                            // See https://github.com/dashpay/dash-evo-tool/issues/660 .
                            MessageBanner::set_global(ctx, msg, MessageType::Success);
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                        BackendTaskSuccessResult::AssetLockBroadcast { ref txid } => {
                            let msg = format!(
                                "Asset lock transaction broadcast successfully. Transaction ID: {txid}"
                            );
                            MessageBanner::set_global(ctx, &msg, MessageType::Success);
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                        BackendTaskSuccessResult::DashPayAddressesRegistered {
                            addresses,
                            contacts,
                            errors,
                        } => {
                            let msg = if errors == 0 {
                                format!(
                                    "Registered {addresses} DashPay addresses for {contacts} contacts."
                                )
                            } else {
                                format!(
                                    "Registered {addresses} DashPay addresses for {contacts} contacts. {errors} addresses could not be registered."
                                )
                            };
                            MessageBanner::set_global(ctx, &msg, MessageType::Success);
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                        BackendTaskSuccessResult::IdentitiesLoaded { count } => {
                            let msg = if count == 1 {
                                "Successfully loaded 1 identity from your wallet.".to_string()
                            } else {
                                format!("Successfully loaded {count} identities from your wallet.")
                            };
                            MessageBanner::set_global(ctx, &msg, MessageType::Success);
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                        BackendTaskSuccessResult::Progress { .. } => {
                            // Progress updates only go to the screen — no global banner.
                            // The screen updates its existing banner handle in-place.
                            // TODO: Routes via visible_screen_mut(), so if the user
                            // navigates away from the originating screen, progress
                            // updates land on the wrong screen. Adding task-to-screen
                            // affinity would fix this (same limitation as Message).
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                        BackendTaskSuccessResult::UpdatedThemePreference(new_theme) => {
                            let detection_failed = self.theme.apply_new_preference(ctx, new_theme);
                            if detection_failed {
                                MessageBanner::set_global(
                                    ctx,
                                    "Could not detect your system theme. Using the previous theme for now — it will update automatically when detection succeeds.",
                                    MessageType::Warning,
                                );
                            } else {
                                MessageBanner::set_global(
                                    ctx,
                                    "Theme preference updated successfully",
                                    MessageType::Success,
                                );
                            }
                            self.visible_screen_mut().display_message(
                                "Theme preference updated successfully",
                                MessageType::Success,
                            );
                        }
                        BackendTaskSuccessResult::CastScheduledVote(ref vote) => {
                            let _ = self.current_app_context().mark_vote_executed(
                                vote.voter_id.as_slice(),
                                vote.contested_name.clone(),
                            );
                            MessageBanner::set_global(
                                ctx,
                                "Successfully cast scheduled vote",
                                MessageType::Success,
                            );
                            self.visible_screen_mut().display_message(
                                "Successfully cast scheduled vote",
                                MessageType::Success,
                            );
                            self.visible_screen_mut().refresh();
                        }
                        BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
                            network,
                            preserve_eligibility_since_ms,
                        } => {
                            self.scheduled_vote_sweeps_in_progress.remove(&network);
                            if clear_confirmed_vote_recovery_cutoff(
                                &mut self.scheduled_vote_sweep_deferred_since_ms,
                                network,
                                preserve_eligibility_since_ms,
                            ) {
                                self.scheduled_vote_recovery_last_attempt.remove(&network);
                            }
                        }
                        BackendTaskSuccessResult::NetworkContextCreated {
                            network,
                            context,
                            ..
                        } => {
                            self.network_contexts.insert(network, context);
                            self.network_switch_pending = None;
                            self.network_switch_banner.take_and_clear();
                            self.finalize_network_switch(network);
                        }
                        BackendTaskSuccessResult::NetworkContextRegistered { network, context } => {
                            context.install_secret_prompt(Arc::clone(&self.secret_prompt_host));
                            self.network_contexts.entry(network).or_insert(context);
                        }
                        BackendTaskSuccessResult::PlatformAddressSyncPushed { updates } => {
                            // Coordinator push: populate per-address platform_address_info
                            // for all loaded wallets so the per-address tab stays current
                            // without a manual Refresh. No banner — this fires every 15 s.
                            active_context.apply_platform_address_push(updates);
                        }
                        BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight => {
                            MessageBanner::set_global(
                                ctx,
                                "Token balances are already refreshing. Wait a moment before refreshing again.",
                                MessageType::Info,
                            );
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                        _ => {
                            // For all other success results, let the screen decide how to display
                            // the outcome without showing a generic global success banner.
                            self.visible_screen_mut()
                                .display_backend_task_result(&context, unboxed_message);
                        }
                    }
                }
                TaskResult::Error {
                    error: err @ TaskError::CoreWalletAutoDetected { .. },
                    ..
                } => {
                    let msg = err.to_string();
                    MessageBanner::set_global(ctx, &msg, MessageType::Success);
                    self.visible_screen_mut()
                        .display_message(&msg, MessageType::Success);
                    self.visible_screen_mut().refresh();
                }
                TaskResult::Error {
                    error: err @ TaskError::NetworkContextCreationFailed { .. },
                    ..
                } => {
                    self.network_switch_pending = None;
                    self.network_switch_banner.take_and_clear();
                    let current_context = self.current_app_context().clone();
                    if let Some(screen) = self
                        .main_screens
                        .get_mut(&RootScreenType::RootScreenNetworkChooser)
                    {
                        screen.change_context(current_context);
                    }
                    MessageBanner::set_global(ctx, err.to_string(), MessageType::Error)
                        .disable_auto_dismiss();
                }
                TaskResult::Error {
                    error:
                        TaskError::MigrationFailed { .. }
                        | TaskError::SavedDataTooOld { .. }
                        | TaskError::SavedDataTooNew { .. },
                    ..
                } => {
                    // The migration task already published `MigrationState::Failed`.
                    // Its reconciler supplies the typed details and applicable
                    // recovery path, so suppress the duplicate generic banner.
                }
                TaskResult::Error {
                    context,
                    error:
                        err @ (TaskError::ScheduledVoteSweepFailed { .. }
                        | TaskError::ScheduledVoteSweepAllAddressesExhausted { .. }),
                } => {
                    clear_scheduled_vote_sweep_guard_on_error(
                        &mut self.scheduled_vote_sweeps_in_progress,
                        &context,
                        &err,
                    );
                    self.visible_screen_mut()
                        .display_backend_task_error(&context, &err);
                    let handled = self.visible_screen_mut().display_task_error(&err);
                    if !handled && !scheduled_vote_sweep_is_quiet(&err) {
                        let msg = err.to_string();
                        let handle = MessageBanner::set_global(ctx, &msg, MessageType::Error);
                        handle.disable_auto_dismiss();
                        handle.with_details(&err);
                        self.visible_screen_mut()
                            .display_message(&msg, MessageType::Error);
                    }
                }
                TaskResult::Error {
                    context,
                    error: err,
                } => {
                    clear_scheduled_vote_sweep_guard_on_error(
                        &mut self.scheduled_vote_sweeps_in_progress,
                        &context,
                        &err,
                    );
                    self.route_contact_request_error_to_hidden_hub(&err);
                    let is_database_clear = context == BackendTaskContext::ClearNetworkDatabase;
                    let suppress_stale_error = !is_database_clear
                        && self
                            .visible_screen_mut()
                            .should_suppress_backend_task_error(&context, &err);
                    if is_database_clear {
                        if let Some(screen) = self
                            .main_screens
                            .get_mut(&RootScreenType::RootScreenNetworkChooser)
                        {
                            screen.display_backend_task_error(&context, &err);
                        }
                    } else {
                        self.visible_screen_mut()
                            .display_backend_task_error(&context, &err);
                    }
                    // Let the screen handle specific error types first.
                    // If handled, skip the generic error banner.
                    let handled = suppress_stale_error
                        || (!is_database_clear
                            && self.visible_screen_mut().display_task_error(&err));

                    if !handled {
                        let msg = err.to_string();
                        let handle = MessageBanner::set_global(ctx, &msg, MessageType::Error);
                        handle.disable_auto_dismiss();
                        // TaskError Debug output is shown to users, deliberately.
                        // Ensure inner error types don't expose secrets.
                        handle.with_details(&err);
                        if !is_database_clear {
                            self.visible_screen_mut()
                                .display_message(&msg, MessageType::Error);
                        }
                    }
                }
                TaskResult::Refresh => {
                    self.visible_screen_mut().refresh();
                }
                TaskResult::Repaint => {
                    // SenderAsync/SenderSync already requested a repaint when sending; avoid
                    // state-clearing screen refreshes for ambient events such as sync ticks.
                }
            }
        }

        // Schedule a periodic repaint every ~1 second so timed messages update
        // their countdown and other periodic UI elements stay current.
        // Throttled so we don't re-schedule on every frame during user interaction.
        if self.last_repaint_request.elapsed() >= Duration::from_secs(1) {
            ctx.request_repaint_after(Duration::from_secs(1));
            self.last_repaint_request = Instant::now();
        }

        // Periodically cast any scheduled masternode votes that have come due.
        // The poll itself — the DB query, local-identity load, and per-vote
        // casting — runs off the UI thread in the `CastDueScheduledVotes`
        // backend task; this tick only dispatches it. The DPNS Scheduled Votes
        // screen learns which votes are in progress / cast via
        // `display_task_result`, so a slow or failing query never stalls a frame.
        let now = Instant::now();
        let network = active_context.network;
        if !migration_allows_scheduled_vote_sweep(migration_state.as_ref()) {
            self.scheduled_vote_sweep_deferred_since_ms
                .entry(network)
                .or_insert_with(unix_time_ms);
        } else if !self.network_selection_required
            && !self.scheduled_vote_sweeps_in_progress.contains(&network)
        {
            let preserve_eligibility_since_ms = self
                .scheduled_vote_sweep_deferred_since_ms
                .get(&network)
                .copied();
            let recovery_due = preserve_eligibility_since_ms.is_some()
                && self
                    .scheduled_vote_recovery_last_attempt
                    .get(&network)
                    .is_none_or(|last| now.duration_since(*last) > Duration::from_secs(60));
            let periodic_due = preserve_eligibility_since_ms.is_none()
                && now.duration_since(self.last_scheduled_vote_check) > Duration::from_secs(60);
            if recovery_due || periodic_due {
                self.last_scheduled_vote_check = now;
                if preserve_eligibility_since_ms.is_some() {
                    self.scheduled_vote_recovery_last_attempt
                        .insert(network, now);
                }
                self.scheduled_vote_sweeps_in_progress.insert(network);
                self.handle_backend_task_with_context(
                    BackendTask::ContestedResourceTask(
                        ContestedResourceTask::CastDueScheduledVotes {
                            preserve_eligibility_since_ms,
                        },
                    ),
                    BackendTaskContext::ScheduledVoteSweep { network },
                );
            }
        }

        // Drive the SPV-sync block BEFORE claiming input and running the screen, so
        // a freshly-armed episode RAISES the overlay in time for THIS frame's input
        // claim + global render. Otherwise (raising after the claim + screen) the
        // frame right after Connect/arming is fully interactive and the block only
        // takes effect a frame later — the one-frame interactive gap. The connection
        // banner still reads the block state afterwards (it suppresses its redundant
        // Connecting/Syncing copy while the block is up).
        self.spv_block.update(ctx, &active_context);

        // Promote a queued prompt before the overlay input/render decision so
        // its first visible frame never shares a pointer sink or focus trap.
        self.activate_secret_prompt(ctx);

        // On the frame a passphrase prompt first becomes active — a just-in-time
        // unlock promoted above, or the migration password prompt — egui has
        // already resolved this frame's click against the previous, prompt-less
        // frame, before the modal installs its input sink. Drop that one pending
        // click so it cannot fall through to the screen beneath; the sink covers
        // every later frame.
        let prompt_blocking = self.has_blocking_secret_prompt(migration_state.as_ref());
        if prompt_blocking && !self.prompt_was_blocking {
            passphrase_modal::drop_activation_frame_pointer_click(ctx);
        }
        self.prompt_was_blocking = prompt_blocking;

        // Total input block at frame start: while a blocking overlay is up, claim
        // all keyboard + text input BEFORE the panels run — unless a
        // secret prompt is active above the overlay (it needs the keyboard).
        self.claim_overlay_input(ctx, migration_state.as_ref());

        // Show welcome screen if onboarding not completed
        let mut actions = Vec::new();
        if self.show_welcome_screen
            && let Some(welcome_screen) = &mut self.welcome_screen
        {
            actions.push(welcome_screen.ui(ui));
        } else {
            actions.push(self.visible_screen_mut().ui(ui));
        };

        // A blocking progress overlay remains active underneath a secret prompt,
        // but renders no dimmer, card, or focus trap until the prompt resolves.
        // Every passphrase prompt — cancellable or not — supplies its own
        // outside-window input barrier in its place (`passphrase_modal`).
        ProgressOverlay::render_global(
            ctx,
            self.has_blocking_secret_prompt(migration_state.as_ref()),
        );

        // Render any just-in-time passphrase prompt on top of the screen.
        self.render_secret_prompt(ctx);

        // Schedule connection status refresh
        actions.push(
            active_context
                .connection_status()
                .trigger_refresh(active_context.as_ref()),
        );

        // The SPV-sync block was already driven at frame start (above), before the
        // input claim + screen, to close the one-frame interactive gap. It still
        // runs before the connection banner, which suppresses its redundant
        // Connecting/Syncing text while the overlay is up.
        let spv_overlaying = self.spv_block.is_overlaying();
        if let Some(task) = self.connection_banner.update(
            ctx,
            &active_context,
            spv_overlaying,
            self.show_welcome_screen,
        ) {
            self.handle_backend_task(task);
        }
        if !self.network_selection_required
            && let Some(task) = self.migration.dispatch_cold_start(&active_context)
        {
            self.handle_backend_task(task);
        }
        if !self.network_selection_required {
            self.migration
                .update_banner(ctx, &active_context, migration_state.as_ref());
            self.migration.handle_esc(ctx);
            if let Some(task) = self.migration.drain_actions(ctx, self.chosen_network) {
                self.handle_backend_task(task);
            }
        }
        self.drain_overlay_actions(ctx);

        for action in actions {
            if !network_selection_allows_action(self.network_selection_required, &action) {
                tracing::debug!("Blocked an action until the user confirms a network");
                MessageBanner::set_global(
                    ctx,
                    "Choose a network before using this control.",
                    MessageType::Info,
                );
                continue;
            }
            match action {
                AppAction::None => {}
                AppAction::AddScreen(screen) => self.screen_stack.push(screen),
                AppAction::Refresh => self.visible_screen_mut().refresh(),
                AppAction::PopScreen => {
                    if !self.screen_stack.is_empty() {
                        self.screen_stack.pop();
                    }
                }
                AppAction::PopScreenAndRefresh => {
                    if !self.screen_stack.is_empty() {
                        self.screen_stack.pop();
                    }
                    if let Some(screen) = self.screen_stack.last_mut() {
                        screen.refresh();
                    } else {
                        self.active_root_screen_mut().refresh_on_arrival();
                    }
                }
                AppAction::GoToMainScreen => {
                    self.screen_stack = vec![];
                    self.active_root_screen_mut().refresh_on_arrival();
                }
                AppAction::BackendTask(task) => {
                    self.handle_backend_task(task);
                }
                AppAction::BackendTaskWithContext { task, context } => {
                    self.handle_backend_task_with_context(task, context);
                }
                AppAction::BackendTasks(tasks, mode) => {
                    self.handle_backend_tasks(tasks, mode);
                }
                AppAction::SetMainScreen(root_screen_type) => {
                    self.set_main_screen(root_screen_type);
                }
                AppAction::SetMainScreenThenGoToMainScreen(root_screen_type) => {
                    self.set_main_screen(root_screen_type);
                    self.screen_stack = vec![];
                }
                AppAction::SetMainScreenThenPopScreen(root_screen_type) => {
                    self.set_main_screen(root_screen_type);
                    if !self.screen_stack.is_empty() {
                        self.screen_stack.pop();
                    }
                }
                AppAction::SwitchNetwork(network) => {
                    self.change_network(network);
                }
                AppAction::PopThenAddScreenToMainScreen(root_screen_type, screen) => {
                    self.screen_stack = vec![screen];
                    self.set_main_screen(root_screen_type);
                }
                AppAction::StartSpv => {
                    // Arm the SPV-sync block for this user-initiated Connect (a
                    // fresh episode — re-arm the escape). The block conveys the
                    // "connecting" state, so no separate Info banner is set here
                    // (F-SPV-E: a dropped Info-banner handle could not be cleared
                    // by the overlay's banner suppression).
                    self.spv_block.arm();
                    Self::spawn_backend_init(
                        &self.subtasks,
                        self.task_result_sender.clone(),
                        self.current_app_context().clone(),
                        BackendInitReason::ManualConnect,
                        true,
                    );
                }
                AppAction::StopSpv => {
                    let app_ctx = self.current_app_context().clone();
                    // Claim the disconnect synchronously: this flips the
                    // indicator to Stopping on this frame (so the button
                    // disables immediately) and dedupes a fast second click —
                    // only the winner spawns the async teardown. No banner is
                    // needed for a user-initiated stop.
                    if app_ctx.connection_status().begin_spv_stop() {
                        let _ = self.subtasks.spawn_sync("spv_manual_stop", async move {
                            app_ctx.stop_spv().await;
                        });
                    }
                }
                AppAction::Custom(_) => {}
                AppAction::OnboardingComplete {
                    main_screen,
                    add_screen,
                } => {
                    self.show_welcome_screen = false;
                    self.welcome_screen = None;
                    self.set_main_screen(main_screen);
                    if let Some(screen_type) = add_screen {
                        let screen = screen_type.create_screen(self.current_app_context());
                        self.screen_stack.push(screen);
                    }
                    self.try_auto_start_spv();
                }
                AppAction::SwitchIdentityHubTab(tab) => {
                    // Resolve the visible screen. In-hub deep links are only
                    // meaningful when the user is actually on the hub, so we
                    // silently drop the action otherwise rather than hijack
                    // navigation.
                    if let crate::ui::Screen::IdentityHubScreen(hub) = self.visible_screen_mut() {
                        hub.select_tab(tab);
                        hub.refresh();
                    }
                }
            }
        }
    }

    fn on_exit(&mut self) {
        // On macOS, order windows out before winit tears down the event
        // handler. This lets AppKit properly clean up display-related KVO
        // observers (TouchBar, etc.) while views are still alive.
        crate::platform::order_out_all_windows();

        if self.shutdown_started.is_some() || self.shutdown_finished {
            tracing::debug!("on_exit: shutdown already attempted, skipping blocking fallback");
            return;
        }
        self.shutdown_started = Some(std::time::Instant::now());
        tracing::debug!("on_exit: fallback blocking shutdown");
        self.run_blocking_shutdown_fallback();
        tracing::debug!("App shutdown complete");
    }
}

#[cfg(test)]
mod migration_banner_tests {
    use super::*;

    /// A frame owns one migration snapshot even if the task publishes mid-frame.
    #[test]
    fn migration_frame_snapshot_is_stable_after_async_publish() {
        let status = crate::context::migration_status::MigrationStatus::new_idle();
        let frame_state = status.state();

        status.set_state(
            crate::context::migration_status::MigrationState::AwaitingWalletPasswords {
                wallets: Vec::new(),
            },
        );

        assert!(
            !MigrationReconciler::is_prompting(&frame_state),
            "a transition published mid-frame must wait for the next frame",
        );
        assert!(
            MigrationReconciler::is_prompting(&status.state()),
            "the next frame snapshot must observe the prompt",
        );
    }

    /// Scheduled-vote work resumes only after every migration pass has finished.
    #[test]
    fn scheduled_vote_sweep_waits_for_successful_migration_completion() {
        use crate::context::migration_status::{MigrationState, MigrationStep};

        assert!(!migration_allows_scheduled_vote_sweep(
            &MigrationState::Idle
        ));
        assert!(migration_allows_scheduled_vote_sweep(
            &MigrationState::Ready
        ));
        assert!(!migration_allows_scheduled_vote_sweep(
            &MigrationState::Running {
                step: MigrationStep::Identities,
            },
        ));
        assert!(!migration_allows_scheduled_vote_sweep(
            &MigrationState::AwaitingWalletPasswords {
                wallets: Vec::new(),
            },
        ));
        assert!(migration_allows_scheduled_vote_sweep(
            &MigrationState::Success,
        ));
        assert!(migration_allows_scheduled_vote_sweep(
            &MigrationState::SucceededWithUnreadableData {
                identities: 1,
                votes: 1,
                top_ups: 1,
            },
        ));
    }

    #[test]
    fn failed_legacy_settings_import_raises_a_sticky_warning() {
        use crate::backend_task::migration::legacy_settings::SettingsImportError;

        let ctx = egui::Context::default();
        let error = SettingsImportError::LegacyDataTooOld {
            found: 1,
            minimum_supported: 11,
        };

        show_legacy_settings_import_warning(&ctx, &error);

        assert!(MessageBanner::has_global(&ctx));
        MessageBanner::clear_global_message(&ctx, LEGACY_SETTINGS_IMPORT_WARNING);
    }

    #[test]
    fn legacy_settings_io_failure_requires_explicit_network_selection() {
        use crate::backend_task::migration::legacy_settings::SettingsImportError;
        use crate::wallet_backend::KvAdapterError;

        let read_error = SettingsImportError::LegacyRead {
            source: rusqlite::Error::InvalidQuery,
        };
        let write_error = SettingsImportError::Write {
            source: KvAdapterError::Truncated,
        };

        for error in [&read_error, &write_error] {
            let selection_required = legacy_settings_import_requires_network_selection(error);
            assert!(selection_required);
            assert_eq!(
                initial_root_screen(
                    RootScreenType::RootScreenWalletsBalances,
                    true,
                    selection_required,
                ),
                RootScreenType::RootScreenNetworkChooser,
            );
            assert!(!show_welcome_screen(false, selection_required));
            assert!(!boot_auto_start_spv(true, true, selection_required));
            assert!(!network_selection_allows_root(
                selection_required,
                RootScreenType::RootScreenWalletsBalances,
            ));
            assert!(network_selection_allows_root(
                selection_required,
                RootScreenType::RootScreenNetworkChooser,
            ));
            assert!(!network_selection_allows_action(
                selection_required,
                &AppAction::StartSpv,
            ));
            assert!(!network_selection_allows_action(
                selection_required,
                &AppAction::BackendTask(BackendTask::None),
            ));
            assert!(network_selection_allows_action(
                selection_required,
                &AppAction::SwitchNetwork(Network::Mainnet),
            ));
        }

        let version_error = SettingsImportError::LegacyDataTooOld {
            found: 1,
            minimum_supported: 11,
        };
        assert!(legacy_settings_import_requires_network_selection(
            &version_error
        ));
    }

    #[test]
    fn onboarding_resumes_after_required_network_selection() {
        assert!(!show_welcome_screen(false, true));
        assert!(show_welcome_screen(false, false));
        assert!(!show_welcome_screen(true, false));
    }

    #[test]
    fn deferred_vote_cutoff_clears_only_after_matching_success() {
        let mut deferred = BTreeMap::from([(Network::Testnet, 42)]);

        assert!(!clear_confirmed_vote_recovery_cutoff(
            &mut deferred,
            Network::Testnet,
            None,
        ));
        assert!(!clear_confirmed_vote_recovery_cutoff(
            &mut deferred,
            Network::Testnet,
            Some(41),
        ));
        assert_eq!(deferred.get(&Network::Testnet), Some(&42));

        assert!(clear_confirmed_vote_recovery_cutoff(
            &mut deferred,
            Network::Testnet,
            Some(42),
        ));
        assert!(!deferred.contains_key(&Network::Testnet));
    }

    #[test]
    fn unreadable_identity_copy_names_both_loading_paths() {
        let text = migration_unreadable_identities_text(2);
        assert!(text.contains("Identities screen"));
        assert!(text.contains("Masternodes tab"));
        assert!(text.contains("+ Load"));
    }

    #[test]
    fn unreadable_top_up_copy_is_actionable() {
        let text = migration_unreadable_data_text(0, 0, 2);
        assert!(text.contains("balance history"));
        assert!(text.contains("before adding more funds"));
        assert!(text.ends_with('.'));
    }

    /// TC-MIG-014 — every `MigrationStep` exposes a non-empty,
    /// sentence-shaped label so i18n extraction picks it up as one
    /// translation unit (no concatenation).
    #[test]
    fn migration_running_text_is_sentence_for_every_step() {
        for step in [
            MigrationStep::Detecting,
            MigrationStep::AppData,
            MigrationStep::SingleKey,
            MigrationStep::Shielded,
            MigrationStep::WalletSeeds,
            MigrationStep::WalletMeta,
            MigrationStep::Identities,
            MigrationStep::Finalize,
        ] {
            let text = migration_running_text(step);
            assert!(!text.is_empty(), "{step:?} has empty banner text");
            assert!(
                text.ends_with('.'),
                "{step:?} text `{text}` is not a complete sentence",
            );
        }
    }

    /// TC-MIG-001 / TC-MIG-013 — every step has a distinct sentence so
    /// the per-frame reconciler can detect transitions by text equality
    /// alone (`set_global` is idempotent for matching text).
    #[test]
    fn migration_running_text_distinct_per_step() {
        let labels = [
            migration_running_text(MigrationStep::Detecting),
            migration_running_text(MigrationStep::AppData),
            migration_running_text(MigrationStep::SingleKey),
            migration_running_text(MigrationStep::Shielded),
            migration_running_text(MigrationStep::WalletSeeds),
            migration_running_text(MigrationStep::WalletMeta),
            migration_running_text(MigrationStep::Identities),
            migration_running_text(MigrationStep::Finalize),
        ];
        let unique: std::collections::HashSet<&str> = labels.iter().copied().collect();
        assert_eq!(
            unique.len(),
            labels.len(),
            "duplicate banner text across MigrationStep variants",
        );
    }

    /// Retry action id is stable — kittest + production both match on
    /// this constant, so a typo would mean the click silently drops on
    /// the floor.
    #[test]
    fn migration_retry_action_id_is_stable() {
        assert_eq!(MIGRATION_RETRY_ACTION_ID, "migration:retry:finish_unwire");
    }

    /// Every banner action id is distinct. `drain_actions` dispatches on these
    /// strings, so a collision would silently route one banner's acknowledgement
    /// to another's task — retiring a warning the user was never shown.
    #[test]
    fn migration_action_ids_are_distinct() {
        let ids = [
            MIGRATION_RETRY_ACTION_ID,
            MIGRATION_VOTES_ACK_ACTION_ID,
            MIGRATION_IDENTITIES_ACK_ACTION_ID,
            MIGRATION_UNREADABLE_ACK_ACTION_ID,
        ];
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            ids.len(),
            "banner action ids must not collide"
        );
    }

    /// The combined-failure banner must surface BOTH signals in one message: the
    /// unreadable-identity count AND the app-data failure, plus the retry the
    /// app-data half needs. If it named only one, the other would be silently
    /// swallowed — exactly the bug this copy exists to prevent.
    #[test]
    fn migration_combined_failure_text_names_both_problems_and_the_retry() {
        let text = migration_failed_with_unreadable_identities_text(3);
        assert!(
            text.contains("identities"),
            "must name the identity problem"
        );
        assert!(
            text.contains('3'),
            "must carry the unreadable-identity count"
        );
        assert!(
            text.contains("did not finish"),
            "must name the app-data failure, not only the identities",
        );
        assert!(
            text.contains("Retry now"),
            "must offer the retry the app-data half needs",
        );
        assert!(text.ends_with('.'), "one complete sentence-shaped message");
    }

    /// Cold-start dispatch gate (the startup-race fix): dispatch only when the
    /// network has NOT already been dispatched AND its wallet backend is wired.
    /// The not-ready row is the regression guard — a switched-to network whose
    /// backend is still wiring must NOT dispatch (and so must not burn its
    /// per-network guard), so a later frame retries once the backend wires.
    #[test]
    fn cold_start_dispatch_gate_truth_table() {
        assert!(
            should_dispatch_cold_start(false, true),
            "fresh network with a wired backend must dispatch",
        );
        assert!(
            !should_dispatch_cold_start(false, false),
            "fresh network whose backend is still wiring must wait, not dispatch",
        );
        assert!(
            !should_dispatch_cold_start(true, true),
            "an already-dispatched network must not re-dispatch",
        );
        assert!(
            !should_dispatch_cold_start(true, false),
            "already-dispatched and not-ready must not dispatch",
        );
    }

    /// Readiness-timeout watchdog: the gate surfaces the stuck-preparation
    /// banner only after the backend has been unwired for at least the timeout,
    /// never before (premature firing would flash the banner on a normal boot,
    /// where wiring lags dispatch by a few frames). Synthetic durations so the
    /// test needs no real clock.
    #[test]
    fn cold_start_backend_wait_timeout_fires_only_after_grace() {
        let timeout = COLD_START_BACKEND_READY_TIMEOUT;

        // Not waiting at all never times out.
        assert!(
            !cold_start_backend_wait_timed_out(None, timeout),
            "a network that is not waiting must never time out",
        );

        // Inside the grace window: keep waiting silently.
        assert!(
            !cold_start_backend_wait_timed_out(Some(Duration::ZERO), timeout),
            "a just-started wait must not fire immediately",
        );
        assert!(
            !cold_start_backend_wait_timed_out(Some(timeout - Duration::from_millis(1)), timeout),
            "a wait one tick short of the timeout must not fire prematurely",
        );

        // At or past the window: fire.
        assert!(
            cold_start_backend_wait_timed_out(Some(timeout), timeout),
            "a wait that reaches the timeout must fire",
        );
        assert!(
            cold_start_backend_wait_timed_out(Some(timeout * 4), timeout),
            "a wait well past the timeout must fire",
        );
    }
}

#[cfg(test)]
mod contact_request_routing_tests {
    use super::*;

    #[test]
    fn hidden_hub_needs_authoritative_contact_result_forwarding() {
        assert!(!identity_hub_is_visible(
            RootScreenType::RootScreenWalletsBalances,
            true
        ));
        assert!(!identity_hub_is_visible(
            RootScreenType::RootScreenIdentityHub,
            false
        ));
        assert!(identity_hub_is_visible(
            RootScreenType::RootScreenIdentityHub,
            true
        ));
    }
}

#[cfg(test)]
mod spv_overlay_tests {
    use super::*;

    const ALL_STATES: [OverallConnectionState; 5] = [
        OverallConnectionState::Disconnected,
        OverallConnectionState::Connecting,
        OverallConnectionState::Syncing,
        OverallConnectionState::Synced,
        OverallConnectionState::Error,
    ];

    /// F-SPV-A — UN-armed (ambient sync, or already disarmed): NEVER block, for
    /// every state and dismissal. This is the regression guard: a mid-session
    /// reconnect or per-block Synced→Syncing flip must not hard-block.
    #[test]
    fn unarmed_never_blocks() {
        for dismissed in [false, true] {
            for state in ALL_STATES {
                assert_eq!(
                    spv_block_step(false, dismissed, state),
                    SpvBlockStep::Idle,
                    "un-armed {state:?} (dismissed={dismissed}) must not block"
                );
            }
        }
    }

    /// Armed + getting-connected (Connecting/Syncing/Disconnected) + not dismissed
    /// → hard block.
    #[test]
    fn armed_blocks_while_getting_connected() {
        for state in [
            OverallConnectionState::Disconnected,
            OverallConnectionState::Connecting,
            OverallConnectionState::Syncing,
        ] {
            assert_eq!(spv_block_step(true, false, state), SpvBlockStep::Block);
        }
    }

    /// C2 escape — armed + dismissed + getting-connected → Stand (no block, episode
    /// kept armed so sync keeps running and the user is just not trapped).
    #[test]
    fn armed_dismissed_stands_down_without_disarming() {
        for state in [
            OverallConnectionState::Disconnected,
            OverallConnectionState::Connecting,
            OverallConnectionState::Syncing,
        ] {
            assert_eq!(spv_block_step(true, true, state), SpvBlockStep::Stand);
        }
    }

    /// C1 / F-SPV-A — armed + terminal (Synced/Error) → Disarm, regardless of
    /// dismissal: lower and disarm so ambient sync afterwards never re-blocks.
    #[test]
    fn armed_terminal_state_disarms() {
        for dismissed in [false, true] {
            for state in [
                OverallConnectionState::Synced,
                OverallConnectionState::Error,
            ] {
                assert_eq!(spv_block_step(true, dismissed, state), SpvBlockStep::Disarm);
            }
        }
    }

    /// The escape action id is stable — production raises it and the SPV-sync
    /// block reconciler matches on it; a typo would drop the click.
    #[test]
    fn continue_background_action_id_is_stable() {
        assert_eq!(
            SPV_CONTINUE_BACKGROUND_ACTION,
            "spv:sync:continue_background"
        );
    }

    /// F-SPV-B — the block descriptions are jargon-free complete sentences (no
    /// "SPV"/"headers"/"masternodes"/raw heights/percentages).
    #[test]
    fn descriptions_are_jargon_free_sentences() {
        for desc in [SPV_CONNECTING_DESCRIPTION, SPV_SYNCING_DESCRIPTION] {
            assert!(desc.ends_with('.'), "`{desc}` must be a complete sentence");
            let lower = desc.to_lowercase();
            for jargon in ["header", "masternode", "filter", "spv", "rpc", "%", "/"] {
                assert!(
                    !lower.contains(jargon),
                    "`{desc}` leaks blockchain jargon `{jargon}` to the Everyday User"
                );
            }
        }
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct MockShutdownBackend {
        forget_calls: AtomicUsize,
        secret_cached: AtomicBool,
        shutdown_completes: bool,
    }

    impl ShutdownWalletBackend for MockShutdownBackend {
        fn forget_all_secrets(&self) {
            self.forget_calls.fetch_add(1, Ordering::Relaxed);
            self.secret_cached.store(false, Ordering::Release);
        }

        fn shutdown(&self) -> futures::future::BoxFuture<'_, ()> {
            self.secret_cached.store(true, Ordering::Release);
            if self.shutdown_completes {
                Box::pin(async {})
            } else {
                Box::pin(std::future::pending())
            }
        }
    }

    struct TestDataDir {
        prior: Option<String>,
        _temp_dir: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestDataDir {
        fn enter() -> Self {
            let lock = crate::test_support::DASH_EVO_DATA_DIR_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let temp_dir = tempfile::tempdir().expect("temporary app data directory");
            let prior = std::env::var("DASH_EVO_DATA_DIR").ok();
            // Safety: the process-global test lock serializes this environment override.
            unsafe { std::env::set_var("DASH_EVO_DATA_DIR", temp_dir.path()) };
            Self {
                prior,
                _temp_dir: temp_dir,
                _lock: lock,
            }
        }
    }

    impl Drop for TestDataDir {
        fn drop(&mut self) {
            // Safety: `_lock` remains held until after the prior value is restored.
            unsafe {
                match &self.prior {
                    Some(value) => std::env::set_var("DASH_EVO_DATA_DIR", value),
                    None => std::env::remove_var("DASH_EVO_DATA_DIR"),
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn second_exit_after_async_attempt_does_not_repeat_wallet_teardown() {
        use crate::wallet_backend::{RememberPolicy, SecretPlaintext, SecretScope};
        use zeroize::Zeroizing;

        let _data_dir = TestDataDir::enter();
        let mut app = AppState::new(egui::Context::default()).expect("test AppState");
        let context = app.current_app_context().clone();
        context
            .ensure_wallet_backend(app.task_result_sender.clone())
            .await
            .expect("wire test wallet backend");
        let backend = context.wallet_backend().expect("wired wallet backend");
        let secret_access = backend.secret_access();
        let scope = SecretScope::HdSeed {
            seed_hash: [0x42; 32],
        };
        let secret = Zeroizing::new([0x24; 64]);
        secret_access.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&secret),
            RememberPolicy::UntilAppClose,
        );

        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        assert!(
            app.subtasks
                .spawn_blocking_sync(
                    "slow-shutdown-regression-task",
                    move || {
                        started_tx.send(()).expect("report slow task start");
                        release_rx.recv().expect("wait for slow task release");
                    },
                    |_| async {},
                )
                .is_ok(),
            "slow shutdown task is accepted"
        );
        started_rx.recv().expect("slow task started");

        app.shutdown_started = Some(std::time::Instant::now());
        let mut shutdown = app.start_async_shutdown();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
                .await
                .is_err(),
            "wallet teardown waits for in-flight blocking work"
        );
        release_tx.send(()).expect("release slow task");
        let outcome = tokio::time::timeout(Duration::from_secs(5), &mut shutdown)
            .await
            .expect("async shutdown stays within its budget")
            .expect("async shutdown task reports an outcome");
        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert!(!secret_access.is_session_cached(&scope));

        secret_access.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&secret),
            RememberPolicy::UntilAppClose,
        );
        app.shutdown_finished = false;
        app.shutdown_receiver = None;
        eframe::App::on_exit(&mut app);

        assert!(
            secret_access.is_session_cached(&scope),
            "on_exit must not invoke wallet teardown again after any async attempt"
        );
        secret_access.forget_all();
    }

    #[tokio::test]
    async fn wallet_shutdown_clears_secrets_again_after_teardown_wait() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: true,
        });

        let outcome = AppState::shutdown_wallet_backend_instances(
            &[Arc::clone(&backend)],
            Duration::from_secs(1),
        )
        .await;

        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 2);
        assert!(
            !backend.secret_cached.load(Ordering::Acquire),
            "a secret re-cached during shutdown is cleared by the final pass"
        );
    }

    #[tokio::test]
    async fn wallet_shutdown_clears_secrets_again_after_teardown_timeout() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: false,
        });

        let outcome = AppState::shutdown_wallet_backend_instances(
            &[Arc::clone(&backend)],
            Duration::from_millis(10),
        )
        .await;

        assert_eq!(outcome, ShutdownOutcome::WalletBackendTimedOut);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 2);
        assert!(
            !backend.secret_cached.load(Ordering::Acquire),
            "the final clear still runs when backend shutdown exceeds its timeout"
        );
    }

    #[tokio::test]
    async fn backend_task_timeout_skips_wallet_teardown() {
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);

        let outcome = AppState::finish_shutdown_after_tasks(
            Some(TaskShutdownOutcome::BackendTasksTimedOut),
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                ShutdownOutcome::Complete
            },
        )
        .await;

        assert_eq!(outcome, ShutdownOutcome::BackendTasksTimedOut);
        assert_eq!(teardown_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn task_manager_failure_skips_wallet_teardown() {
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);

        let outcome = AppState::finish_shutdown_after_tasks(None, async move {
            calls.fetch_add(1, Ordering::Relaxed);
            ShutdownOutcome::Complete
        })
        .await;

        assert_eq!(outcome, ShutdownOutcome::TaskManagerFailed);
        assert_eq!(teardown_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn complete_task_shutdown_runs_wallet_teardown_once() {
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);

        let outcome = AppState::finish_shutdown_after_tasks(
            Some(TaskShutdownOutcome::Complete),
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                ShutdownOutcome::Complete
            },
        )
        .await;

        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert_eq!(teardown_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn collect_created_context_only_keeps_network_context_results() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let mut contexts = Vec::new();

        AppState::collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::NetworkContextRegistered {
                network: Network::Testnet,
                context: Arc::clone(&context),
            }),
        );
        assert_eq!(contexts.len(), 1);
        assert!(Arc::ptr_eq(&contexts[0], &context));

        AppState::collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::NetworkContextCreated {
                network: Network::Testnet,
                context: Arc::clone(&context),
                spv_started: false,
            }),
        );
        assert_eq!(contexts.len(), 1);
        assert!(Arc::ptr_eq(&contexts[0], &context));

        AppState::collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::NetworkContextCreated {
                network: Network::Testnet,
                context: Arc::clone(&context),
                spv_started: false,
            }),
        );
        assert_eq!(contexts.len(), 1);

        AppState::collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::None),
        );
        assert_eq!(contexts.len(), 1);

        AppState::collect_created_context(
            &mut contexts,
            TaskResult::unattributed_error(TaskError::NoIdentitiesFound),
        );
        assert_eq!(contexts.len(), 1);
    }

    #[test]
    fn viewport_deadline_is_derived_from_shutdown_phase_budgets() {
        assert_eq!(
            shutdown_hard_deadline(),
            TaskManager::graceful_shutdown_budget()
                + WALLET_BACKEND_SHUTDOWN_TIMEOUT
                + SHUTDOWN_DEADLINE_MARGIN
        );
    }
}
